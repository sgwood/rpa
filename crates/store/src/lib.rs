use std::sync::{Arc, Mutex};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use ai_rpa_core::{
    AdapterStatus, Capability, CommandAction, CommandDelivery, CommandRecord, CommandState,
    ControlMode, DeviceRecord, EventType, Provider, TaskRecord, TaskState, UnifiedEvent,
    derive_state, redact_text,
};
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

const SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS devices (
  id TEXT PRIMARY KEY,
  last_seen_at TEXT NOT NULL,
  record_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS adapters (
  provider TEXT PRIMARY KEY,
  last_event_at TEXT,
  record_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS tasks (
  id TEXT PRIMARY KEY,
  provider TEXT NOT NULL,
  device_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  state TEXT NOT NULL,
  project TEXT,
  updated_at TEXT NOT NULL,
  snapshot_json TEXT NOT NULL,
  UNIQUE(provider, device_id, session_id)
);
CREATE INDEX IF NOT EXISTS idx_tasks_updated ON tasks(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_tasks_state ON tasks(state, updated_at DESC);
CREATE TABLE IF NOT EXISTS events (
  event_id TEXT PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE,
  task_id TEXT NOT NULL REFERENCES tasks(id),
  occurred_at TEXT NOT NULL,
  event_type TEXT NOT NULL,
  event_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_task_time ON events(task_id, occurred_at);
CREATE TABLE IF NOT EXISTS quarantine (
  id TEXT PRIMARY KEY,
  provider TEXT,
  reason TEXT NOT NULL,
  payload_digest TEXT,
  received_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS commands (
  id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL REFERENCES tasks(id),
  device_id TEXT NOT NULL,
  action TEXT NOT NULL,
  state TEXT NOT NULL,
  body_ciphertext TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  lease_owner TEXT,
  lease_until TEXT,
  attempts INTEGER NOT NULL DEFAULT 0,
  record_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_commands_queue ON commands(task_id, state, created_at);
CREATE TABLE IF NOT EXISTS audit_log (
  id TEXT PRIMARY KEY,
  task_id TEXT,
  command_id TEXT,
  action TEXT NOT NULL,
  actor TEXT NOT NULL,
  summary TEXT NOT NULL,
  occurred_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_task ON audit_log(task_id, occurred_at);
CREATE TABLE IF NOT EXISTS outbox (
  id TEXT PRIMARY KEY,
  dedupe_key TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL,
  task_id TEXT,
  payload_json TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'PENDING',
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at TEXT NOT NULL,
  last_error TEXT,
  sent_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_outbox_due ON outbox(state, next_attempt_at);
CREATE TABLE IF NOT EXISTS sync_events (
  event_id TEXT PRIMARY KEY REFERENCES events(event_id) ON DELETE CASCADE,
  created_at TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  last_error TEXT
);
CREATE TABLE IF NOT EXISTS remote_command_sync (
  command_id TEXT PRIMARY KEY REFERENCES commands(id) ON DELETE CASCADE,
  last_reported_state TEXT
);
"#;

#[derive(Clone)]
pub struct Store {
    connection: Arc<Mutex<Connection>>,
    crypto: Arc<CryptoBox>,
}

#[derive(Clone)]
pub struct CryptoBox {
    cipher: Aes256Gcm,
}

impl CryptoBox {
    pub fn from_key(key: [u8; 32]) -> Self {
        Self {
            cipher: Aes256Gcm::new((&key).into()),
        }
    }

    pub fn generate() -> (Self, [u8; 32]) {
        let key = Aes256Gcm::generate_key(&mut OsRng);
        let mut bytes = [0_u8; 32];
        bytes.copy_from_slice(&key);
        (Self::from_key(bytes), bytes)
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| anyhow!("failed to encrypt command body"))?;
        let mut envelope = nonce.to_vec();
        envelope.extend(ciphertext);
        Ok(STANDARD.encode(envelope))
    }

    pub fn decrypt(&self, envelope: &str) -> Result<String> {
        let bytes = STANDARD
            .decode(envelope)
            .context("invalid encrypted command envelope")?;
        if bytes.len() < 12 {
            bail!("encrypted command envelope is too short");
        }
        let (nonce, ciphertext) = bytes.split_at(12);
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| anyhow!("failed to decrypt command body"))?;
        String::from_utf8(plaintext).context("command body is not utf-8")
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskFilter {
    pub provider: Option<Provider>,
    pub state: Option<TaskState>,
    pub device_id: Option<String>,
    pub project: Option<String>,
    pub control_mode: Option<ControlMode>,
    pub updated_after: Option<DateTime<Utc>>,
    pub updated_before: Option<DateTime<Utc>>,
    pub search: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestOutcome {
    pub duplicate: bool,
    pub task: TaskRecord,
    pub state_changed: bool,
    pub delivery: Option<CommandDelivery>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDetail {
    pub task: TaskRecord,
    pub events: Vec<UnifiedEvent>,
    pub commands: Vec<CommandRecord>,
    pub audit: Vec<AuditEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: Uuid,
    pub task_id: Option<Uuid>,
    pub command_id: Option<Uuid>,
    pub action: String,
    pub actor: String,
    pub summary: String,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxItem {
    pub id: Uuid,
    pub kind: String,
    pub task_id: Option<Uuid>,
    pub payload: Value,
    pub attempts: u32,
}

impl Store {
    pub fn open(path: impl AsRef<std::path::Path>, crypto: CryptoBox) -> Result<Self> {
        let mut connection = Connection::open(path).context("open sqlite database")?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .context("enable sqlite WAL")?;
        connection
            .pragma_update(None, "synchronous", "NORMAL")
            .context("configure sqlite synchronous mode")?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .context("configure sqlite busy timeout")?;
        connection
            .execute_batch(SCHEMA)
            .context("initialize schema")?;
        let sync_backfill_complete: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM meta WHERE key='central_sync_backfill_v1')",
            [],
            |row| row.get(0),
        )?;
        if !sync_backfill_complete {
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT OR IGNORE INTO sync_events(event_id, created_at)
                 SELECT event_id, occurred_at FROM events",
                [],
            )?;
            transaction.execute(
                "INSERT INTO meta(key,value) VALUES('central_sync_backfill_v1','complete')",
                [],
            )?;
            transaction.commit()?;
        }
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            crypto: Arc::new(crypto),
        })
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        connection
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .context("read metadata")
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        connection.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn upsert_device(&self, device: &DeviceRecord) -> Result<()> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        connection.execute(
            "INSERT INTO devices(id, last_seen_at, record_json) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET last_seen_at=excluded.last_seen_at, record_json=excluded.record_json",
            params![device.id, device.last_seen_at.to_rfc3339(), serde_json::to_string(device)?],
        )?;
        Ok(())
    }

    pub fn devices(&self) -> Result<Vec<DeviceRecord>> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        let mut statement =
            connection.prepare("SELECT record_json FROM devices ORDER BY last_seen_at DESC")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| serde_json::from_str(&row?).context("decode device record"))
            .collect()
    }

    pub fn upsert_adapter(&self, status: &AdapterStatus) -> Result<()> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        connection.execute(
            "INSERT INTO adapters(provider, last_event_at, record_json) VALUES (?1, ?2, ?3)
             ON CONFLICT(provider) DO UPDATE SET last_event_at=excluded.last_event_at, record_json=excluded.record_json",
            params![
                status.provider.as_str(),
                status.last_event_at.map(|value| value.to_rfc3339()),
                serde_json::to_string(status)?
            ],
        )?;
        Ok(())
    }

    pub fn adapters(&self) -> Result<Vec<AdapterStatus>> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        let mut statement =
            connection.prepare("SELECT record_json FROM adapters ORDER BY provider")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| serde_json::from_str(&row?).context("decode adapter status"))
            .collect()
    }

    pub fn quarantine(
        &self,
        provider: Option<Provider>,
        reason: &str,
        payload_digest: Option<&str>,
    ) -> Result<()> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        connection.execute(
            "INSERT INTO quarantine(id, provider, reason, payload_digest, received_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                Uuid::new_v4().to_string(),
                provider.map(|item| item.to_string()),
                redact_text(reason).text,
                payload_digest,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn ingest_event(&self, event: &UnifiedEvent) -> Result<IngestOutcome> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        let task_id = find_or_create_task(&transaction, event)?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO events(event_id, idempotency_key, task_id, occurred_at, event_type, event_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.event_id.to_string(),
                event.idempotency_key,
                task_id.to_string(),
                event.occurred_at.to_rfc3339(),
                event.event_type.to_string(),
                serde_json::to_string(event)?
            ],
        )?;
        let previous = task_by_id_tx(&transaction, task_id)?;

        if inserted == 0 {
            transaction.commit()?;
            return Ok(IngestOutcome {
                duplicate: true,
                task: previous,
                state_changed: false,
                delivery: None,
            });
        }

        transaction.execute(
            "INSERT OR IGNORE INTO sync_events(event_id, created_at) VALUES (?1, ?2)",
            params![event.event_id.to_string(), Utc::now().to_rfc3339()],
        )?;

        let events = events_for_task_tx(&transaction, task_id)?;
        let derived = derive_state(&events, previous.required_evidence_level);
        let state_changed = previous.state != derived.state;
        let old_state = previous.state;
        let mut capabilities = previous.capabilities.clone();
        for capability in &event.capabilities {
            if !capabilities.contains(capability) {
                capabilities.push(*capability);
            }
        }
        let task = TaskRecord {
            title: if event.title.ends_with(" task") {
                previous.title.clone()
            } else {
                event.title.clone()
            },
            workspace: event.workspace.clone().or(previous.workspace.clone()),
            project: event.project.clone().or(previous.project.clone()),
            control_mode: if event.control_mode == ai_rpa_core::ControlMode::Managed {
                event.control_mode
            } else {
                previous.control_mode
            },
            capabilities,
            required_evidence_level: previous
                .required_evidence_level
                .max(event.required_evidence_level),
            state: derived.state,
            confidence: derived.confidence,
            evidence_level: derived.evidence_level,
            evidence_summary: derived.evidence_summary,
            started_at: derived.started_at,
            updated_at: derived.updated_at,
            duration_ms: derived.duration_ms,
            last_event_type: derived.last_event_type,
            state_version: if state_changed {
                previous.state_version + 1
            } else {
                previous.state_version
            },
            ..previous
        };
        transaction.execute(
            "UPDATE tasks SET state=?2, project=?3, updated_at=?4, snapshot_json=?5 WHERE id=?1",
            params![
                task.id.to_string(),
                task.state.to_string(),
                task.project,
                task.updated_at.to_rfc3339(),
                serde_json::to_string(&task)?
            ],
        )?;

        if state_changed {
            append_audit_tx(
                &transaction,
                Some(task.id),
                None,
                "TASK_STATE_CHANGED",
                "state-engine",
                &format!("{} -> {}", old_state, task.state),
            )?;
            enqueue_notification_tx(&transaction, &task)?;
        }

        let delivery = if matches!(
            event.event_type,
            EventType::TurnStopped | EventType::SessionEnded | EventType::Result
        ) {
            deliver_next_tx(&transaction, &self.crypto, &task)?
        } else {
            None
        };
        transaction.commit()?;
        Ok(IngestOutcome {
            duplicate: false,
            task,
            state_changed,
            delivery,
        })
    }

    pub fn list_tasks(&self, filter: &TaskFilter) -> Result<Vec<TaskRecord>> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        let mut statement = connection
            .prepare("SELECT snapshot_json FROM tasks ORDER BY updated_at DESC LIMIT 10000")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let search = filter.search.as_deref().map(str::to_ascii_lowercase);
        let mut output = Vec::new();
        for row in rows {
            let task: TaskRecord = serde_json::from_str(&row?)?;
            if filter.provider.is_some_and(|value| value != task.provider)
                || filter.state.is_some_and(|value| value != task.state)
                || filter
                    .device_id
                    .as_deref()
                    .is_some_and(|value| value != task.device_id)
                || filter
                    .project
                    .as_deref()
                    .is_some_and(|value| task.project.as_deref() != Some(value))
                || filter
                    .control_mode
                    .is_some_and(|value| value != task.control_mode)
                || filter
                    .updated_after
                    .is_some_and(|value| task.updated_at < value)
                || filter
                    .updated_before
                    .is_some_and(|value| task.updated_at > value)
            {
                continue;
            }
            if search.as_deref().is_some_and(|needle| {
                !format!(
                    "{} {} {} {}",
                    task.title,
                    task.session_id,
                    task.workspace.as_deref().unwrap_or(""),
                    task.project.as_deref().unwrap_or("")
                )
                .to_ascii_lowercase()
                .contains(needle)
            }) {
                continue;
            }
            output.push(task);
            if output.len() >= filter.limit.unwrap_or(200).min(1000) {
                break;
            }
        }
        Ok(output)
    }

    pub fn task_detail(&self, task_id: Uuid) -> Result<Option<TaskDetail>> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        let task = task_by_id_connection(&connection, task_id).optional()?;
        let Some(task) = task else {
            return Ok(None);
        };
        let events = events_for_task_connection(&connection, task_id)?;
        let commands = commands_for_task_connection(&connection, task_id)?;
        let audit = audit_for_task_connection(&connection, task_id)?;
        Ok(Some(TaskDetail {
            task,
            events,
            commands,
            audit,
        }))
    }

    pub fn create_command(
        &self,
        task_id: Uuid,
        action: CommandAction,
        message: &str,
        created_by: &str,
        ttl_seconds: i64,
    ) -> Result<CommandRecord> {
        if message.trim().is_empty() {
            bail!("command message is required");
        }
        if message.len() > 32 * 1024 {
            bail!("command message exceeds 32 KiB");
        }
        if !(60..=86_400).contains(&ttl_seconds) {
            bail!("command TTL must be between 60 and 86400 seconds");
        }
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        let task = task_by_id_tx(&transaction, task_id)?;
        validate_action(&task, action)?;

        let pending_for_task: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM commands WHERE task_id=?1 AND state IN ('QUEUED','RETRY_WAIT','LEASED','DELIVERED','ACCEPTED')",
            [task_id.to_string()],
            |row| row.get(0),
        )?;
        if pending_for_task >= 5 {
            bail!("at most 5 pending continuation commands are allowed per task");
        }
        let pending_global: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM commands WHERE state IN ('QUEUED','RETRY_WAIT','LEASED','DELIVERED','ACCEPTED')",
            [],
            |row| row.get(0),
        )?;
        if pending_global >= 1000 {
            bail!("global continuation queue circuit breaker is open at 1000 pending commands");
        }

        let recent_deliveries: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM commands WHERE task_id=?1 AND state IN ('DELIVERED','ACCEPTED','COMPLETED') AND created_at >= ?2",
            params![task_id.to_string(), (Utc::now() - Duration::hours(1)).to_rfc3339()],
            |row| row.get(0),
        )?;
        if recent_deliveries >= 5 {
            bail!("automatic continuation circuit breaker is open after 5 deliveries in one hour");
        }

        let now = Utc::now();
        let mut record = CommandRecord {
            id: Uuid::new_v4(),
            task_id,
            provider: task.provider,
            device_id: task.device_id.clone(),
            session_id: task.session_id.clone(),
            action,
            state: CommandState::Queued,
            message: None,
            created_by: redact_text(created_by).text,
            created_at: now,
            expires_at: now + Duration::seconds(ttl_seconds),
            lease_owner: None,
            lease_until: None,
            attempts: 0,
            max_attempts: 5,
            result_summary: Some("encrypted command queued".to_owned()),
        };
        let ciphertext = self.crypto.encrypt(message)?;
        transaction.execute(
            "INSERT INTO commands(id, task_id, device_id, action, state, body_ciphertext, created_at, expires_at, attempts, record_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9)",
            params![
                record.id.to_string(),
                task_id.to_string(),
                record.device_id,
                action.to_string(),
                record.state.to_string(),
                ciphertext,
                record.created_at.to_rfc3339(),
                record.expires_at.to_rfc3339(),
                serde_json::to_string(&record)?
            ],
        )?;
        append_audit_tx(
            &transaction,
            Some(task_id),
            Some(record.id),
            "COMMAND_QUEUED",
            &record.created_by,
            &format!("{} command queued with TTL {}s", action, ttl_seconds),
        )?;
        transaction.commit()?;
        record.message = None;
        Ok(record)
    }

    pub fn lease_managed_command(
        &self,
        device_id: &str,
        lease_owner: &str,
        lease_seconds: i64,
    ) -> Result<Option<CommandRecord>> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire_commands_tx(&transaction)?;
        let candidate: Option<String> = transaction
            .query_row(
                "SELECT candidate.id FROM commands candidate
                 WHERE candidate.device_id=?1
                   AND candidate.action='RESUME_AND_SEND'
                   AND candidate.state IN ('QUEUED','RETRY_WAIT')
                   AND candidate.expires_at>?2
                   AND NOT EXISTS (
                     SELECT 1 FROM commands active
                     WHERE active.task_id=candidate.task_id AND active.state='LEASED'
                   )
                 ORDER BY candidate.created_at LIMIT 1",
                params![device_id, Utc::now().to_rfc3339()],
                |row| row.get(0),
            )
            .optional()?;
        let Some(candidate) = candidate else {
            transaction.commit()?;
            return Ok(None);
        };
        let command_id = Uuid::parse_str(&candidate)?;
        let mut record = command_by_id_tx(&transaction, command_id)?;
        record.state = CommandState::Leased;
        record.lease_owner = Some(redact_text(lease_owner).text);
        record.lease_until = Some(Utc::now() + Duration::seconds(lease_seconds));
        record.attempts += 1;
        update_command_tx(&transaction, &record)?;
        append_audit_tx(
            &transaction,
            Some(record.task_id),
            Some(record.id),
            "COMMAND_LEASED",
            lease_owner,
            "managed command leased",
        )?;
        transaction.commit()?;
        Ok(Some(record))
    }

    pub fn command_message(&self, command_id: Uuid) -> Result<String> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        let ciphertext: String = connection.query_row(
            "SELECT body_ciphertext FROM commands WHERE id=?1",
            [command_id.to_string()],
            |row| row.get(0),
        )?;
        self.crypto.decrypt(&ciphertext)
    }

    pub fn update_command(
        &self,
        command_id: Uuid,
        state: CommandState,
        result_summary: Option<&str>,
        actor: &str,
    ) -> Result<CommandRecord> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        let mut record = command_by_id_tx(&transaction, command_id)?;
        record.state = state;
        record.result_summary = result_summary.map(|value| redact_text(value).text);
        if state != CommandState::Leased {
            record.lease_owner = None;
            record.lease_until = None;
        }
        update_command_tx(&transaction, &record)?;
        append_audit_tx(
            &transaction,
            Some(record.task_id),
            Some(record.id),
            "COMMAND_STATE_CHANGED",
            actor,
            &format!("command changed to {state}"),
        )?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn pending_sync_events(&self, limit: usize) -> Result<Vec<UnifiedEvent>> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT events.event_json FROM sync_events
             JOIN events ON events.event_id=sync_events.event_id
             ORDER BY sync_events.created_at LIMIT ?1",
        )?;
        let rows = statement.query_map([limit.min(500) as i64], |row| row.get::<_, String>(0))?;
        rows.map(|row| serde_json::from_str(&row?).context("decode sync event"))
            .collect()
    }

    pub fn mark_sync_events_sent(&self, event_ids: &[Uuid]) -> Result<()> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        for event_id in event_ids {
            transaction.execute(
                "DELETE FROM sync_events WHERE event_id=?1",
                [event_id.to_string()],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn mark_sync_events_retry(&self, event_ids: &[Uuid], error: &str) -> Result<()> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        for event_id in event_ids {
            transaction.execute(
                "UPDATE sync_events SET attempts=attempts+1,last_error=?2 WHERE event_id=?1",
                params![event_id.to_string(), redact_text(error).text],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn import_remote_command(&self, remote: &ai_rpa_core::RemoteCommand) -> Result<bool> {
        let mut connection = self.connection.lock().expect("sqlite mutex poisoned");
        let transaction = connection.transaction()?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM commands WHERE id=?1)",
            [remote.id.to_string()],
            |row| row.get(0),
        )?;
        if exists {
            transaction.commit()?;
            return Ok(false);
        }
        let task_json: String = transaction
            .query_row(
                "SELECT snapshot_json FROM tasks WHERE provider=?1 AND device_id=?2 AND session_id=?3",
                params![remote.provider.as_str(), remote.device_id, remote.session_id],
                |row| row.get(0),
            )
            .optional()?
            .context("remote command target session is not present on this device")?;
        let task: TaskRecord = serde_json::from_str(&task_json)?;
        validate_action(&task, remote.action)?;
        if remote.expires_at <= Utc::now() {
            bail!("remote command already expired");
        }
        let record = CommandRecord {
            id: remote.id,
            task_id: task.id,
            provider: remote.provider,
            device_id: remote.device_id.clone(),
            session_id: remote.session_id.clone(),
            action: remote.action,
            state: CommandState::Queued,
            message: None,
            created_by: format!("central:{}", redact_text(&remote.created_by).text),
            created_at: remote.created_at,
            expires_at: remote.expires_at,
            lease_owner: None,
            lease_until: None,
            attempts: 0,
            max_attempts: 3,
            result_summary: None,
        };
        transaction.execute(
            "INSERT INTO commands(id,task_id,device_id,action,state,body_ciphertext,created_at,expires_at,attempts,record_json)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,0,?9)",
            params![
                record.id.to_string(),
                record.task_id.to_string(),
                record.device_id,
                record.action.to_string(),
                record.state.to_string(),
                self.crypto.encrypt(&remote.message)?,
                record.created_at.to_rfc3339(),
                record.expires_at.to_rfc3339(),
                serde_json::to_string(&record)?
            ],
        )?;
        transaction.execute(
            "INSERT INTO remote_command_sync(command_id,last_reported_state) VALUES(?1,'QUEUED')",
            [record.id.to_string()],
        )?;
        append_audit_tx(
            &transaction,
            Some(task.id),
            Some(record.id),
            "REMOTE_COMMAND_ACCEPTED",
            "central-sync",
            "authenticated remote command accepted into local queue",
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn remote_command_updates(&self) -> Result<Vec<CommandRecord>> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT commands.record_json FROM remote_command_sync
             JOIN commands ON commands.id=remote_command_sync.command_id
             WHERE commands.state NOT IN ('CREATED','QUEUED','LEASED')
               AND (remote_command_sync.last_reported_state IS NULL OR remote_command_sync.last_reported_state<>commands.state)
             ORDER BY commands.created_at LIMIT 100",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| serde_json::from_str(&row?).context("decode remote command update"))
            .collect()
    }

    pub fn mark_remote_command_reported(
        &self,
        command_id: Uuid,
        state: CommandState,
    ) -> Result<()> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        connection.execute(
            "UPDATE remote_command_sync SET last_reported_state=?2 WHERE command_id=?1",
            params![command_id.to_string(), state.to_string()],
        )?;
        Ok(())
    }

    pub fn due_outbox(&self, limit: usize) -> Result<Vec<OutboxItem>> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT id, kind, task_id, payload_json, attempts FROM outbox WHERE state IN ('PENDING','RETRY') AND next_attempt_at<=?1 ORDER BY next_attempt_at LIMIT ?2",
        )?;
        let rows = statement.query_map(params![Utc::now().to_rfc3339(), limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u32>(4)?,
            ))
        })?;
        let mut output = Vec::new();
        for row in rows {
            let (id, kind, task_id, payload, attempts) = row?;
            output.push(OutboxItem {
                id: Uuid::parse_str(&id)?,
                kind,
                task_id: task_id.map(|value| Uuid::parse_str(&value)).transpose()?,
                payload: serde_json::from_str(&payload)?,
                attempts,
            });
        }
        Ok(output)
    }

    pub fn mark_outbox_sent(&self, id: Uuid) -> Result<()> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        connection.execute(
            "UPDATE outbox SET state='SENT', sent_at=?2 WHERE id=?1",
            params![id.to_string(), Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn mark_outbox_retry(&self, id: Uuid, attempts: u32, error: &str) -> Result<()> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        let next_attempt = Utc::now() + Duration::seconds(2_i64.pow(attempts.min(8)));
        let state = if attempts >= 8 { "FAILED" } else { "RETRY" };
        connection.execute(
            "UPDATE outbox SET state=?2, attempts=?3, next_attempt_at=?4, last_error=?5 WHERE id=?1",
            params![
                id.to_string(),
                state,
                attempts,
                next_attempt.to_rfc3339(),
                redact_text(error).text
            ],
        )?;
        Ok(())
    }

    pub fn counts(&self) -> Result<Value> {
        let connection = self.connection.lock().expect("sqlite mutex poisoned");
        let task_count: i64 =
            connection.query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))?;
        let event_count: i64 =
            connection.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        let quarantine_count: i64 =
            connection.query_row("SELECT COUNT(*) FROM quarantine", [], |row| row.get(0))?;
        let command_count: i64 =
            connection.query_row("SELECT COUNT(*) FROM commands", [], |row| row.get(0))?;
        let pending_outbox: i64 = connection.query_row(
            "SELECT COUNT(*) FROM outbox WHERE state IN ('PENDING','RETRY')",
            [],
            |row| row.get(0),
        )?;
        Ok(json!({
            "tasks": task_count,
            "events": event_count,
            "quarantine": quarantine_count,
            "commands": command_count,
            "pendingOutbox": pending_outbox,
            "pendingCentralSync": connection.query_row(
                "SELECT COUNT(*) FROM sync_events", [], |row| row.get::<_, i64>(0)
            )?
        }))
    }
}

fn find_or_create_task(transaction: &Transaction<'_>, event: &UnifiedEvent) -> Result<Uuid> {
    let existing: Option<String> = transaction
        .query_row(
            "SELECT id FROM tasks WHERE provider=?1 AND device_id=?2 AND session_id=?3",
            params![event.provider.as_str(), event.device_id, event.session_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(existing) = existing {
        return Ok(Uuid::parse_str(&existing)?);
    }
    let id = Uuid::new_v4();
    let task = TaskRecord {
        id,
        provider: event.provider,
        device_id: event.device_id.clone(),
        session_id: event.session_id.clone(),
        title: event.title.clone(),
        workspace: event.workspace.clone(),
        project: event.project.clone(),
        control_mode: event.control_mode,
        capabilities: event.capabilities.clone(),
        state: TaskState::Unknown,
        confidence: "LOW".to_owned(),
        required_evidence_level: event.required_evidence_level,
        evidence_level: event.evidence_level,
        evidence_summary: event.evidence_summary.clone(),
        started_at: None,
        updated_at: event.occurred_at,
        duration_ms: None,
        last_event_type: event.event_type,
        state_version: 0,
    };
    transaction.execute(
        "INSERT INTO tasks(id, provider, device_id, session_id, state, project, updated_at, snapshot_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id.to_string(),
            event.provider.as_str(),
            event.device_id,
            event.session_id,
            task.state.to_string(),
            event.project,
            task.updated_at.to_rfc3339(),
            serde_json::to_string(&task)?
        ],
    )?;
    Ok(id)
}

fn task_by_id_tx(transaction: &Transaction<'_>, id: Uuid) -> Result<TaskRecord> {
    let json: String = transaction.query_row(
        "SELECT snapshot_json FROM tasks WHERE id=?1",
        [id.to_string()],
        |row| row.get(0),
    )?;
    serde_json::from_str(&json).context("decode task snapshot")
}

fn task_by_id_connection(connection: &Connection, id: Uuid) -> rusqlite::Result<TaskRecord> {
    connection.query_row(
        "SELECT snapshot_json FROM tasks WHERE id=?1",
        [id.to_string()],
        |row| {
            let json: String = row.get(0)?;
            serde_json::from_str(&json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    json.len(),
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        },
    )
}

fn events_for_task_tx(transaction: &Transaction<'_>, task_id: Uuid) -> Result<Vec<UnifiedEvent>> {
    let mut statement = transaction
        .prepare("SELECT event_json FROM events WHERE task_id=?1 ORDER BY occurred_at, event_id")?;
    let rows = statement.query_map([task_id.to_string()], |row| row.get::<_, String>(0))?;
    rows.map(|row| serde_json::from_str(&row?).context("decode event"))
        .collect()
}

fn events_for_task_connection(connection: &Connection, task_id: Uuid) -> Result<Vec<UnifiedEvent>> {
    let mut statement = connection
        .prepare("SELECT event_json FROM events WHERE task_id=?1 ORDER BY occurred_at, event_id")?;
    let rows = statement.query_map([task_id.to_string()], |row| row.get::<_, String>(0))?;
    rows.map(|row| serde_json::from_str(&row?).context("decode event"))
        .collect()
}

fn commands_for_task_connection(
    connection: &Connection,
    task_id: Uuid,
) -> Result<Vec<CommandRecord>> {
    let mut statement = connection
        .prepare("SELECT record_json FROM commands WHERE task_id=?1 ORDER BY created_at DESC")?;
    let rows = statement.query_map([task_id.to_string()], |row| row.get::<_, String>(0))?;
    rows.map(|row| serde_json::from_str(&row?).context("decode command"))
        .collect()
}

fn audit_for_task_connection(connection: &Connection, task_id: Uuid) -> Result<Vec<AuditEntry>> {
    let mut statement = connection.prepare(
        "SELECT id, task_id, command_id, action, actor, summary, occurred_at FROM audit_log WHERE task_id=?1 ORDER BY occurred_at",
    )?;
    let rows = statement.query_map([task_id.to_string()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    let mut output = Vec::new();
    for row in rows {
        let (id, task, command, action, actor, summary, occurred_at) = row?;
        output.push(AuditEntry {
            id: Uuid::parse_str(&id)?,
            task_id: task.map(|value| Uuid::parse_str(&value)).transpose()?,
            command_id: command.map(|value| Uuid::parse_str(&value)).transpose()?,
            action,
            actor,
            summary,
            occurred_at: DateTime::parse_from_rfc3339(&occurred_at)?.with_timezone(&Utc),
        });
    }
    Ok(output)
}

fn append_audit_tx(
    transaction: &Transaction<'_>,
    task_id: Option<Uuid>,
    command_id: Option<Uuid>,
    action: &str,
    actor: &str,
    summary: &str,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO audit_log(id, task_id, command_id, action, actor, summary, occurred_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            Uuid::new_v4().to_string(),
            task_id.map(|value| value.to_string()),
            command_id.map(|value| value.to_string()),
            action,
            redact_text(actor).text,
            redact_text(summary).text,
            Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

fn enqueue_notification_tx(transaction: &Transaction<'_>, task: &TaskRecord) -> Result<()> {
    let (kind, available_at) = match task.state {
        TaskState::Failed => ("FAILED_IMMEDIATE", Utc::now()),
        TaskState::WaitingUser => ("WAITING_USER_IMMEDIATE", Utc::now()),
        TaskState::Succeeded => ("SUCCEEDED_DIGEST", next_digest_boundary(Utc::now())),
        _ => return Ok(()),
    };
    let dedupe = format!("{}:{}:{}", task.id, task.state, task.state_version);
    let payload = json!({
        "taskId": task.id,
        "provider": task.provider,
        "title": task.title,
        "state": task.state,
        "durationMs": task.duration_ms,
        "evidenceLevel": task.evidence_level,
        "summary": task.evidence_summary,
        "nextStep": match task.state {
            TaskState::WaitingUser => "打开任务并处理权限或输入",
            TaskState::Failed => "查看失败证据并决定是否继续",
            TaskState::Succeeded => "查看结果或排队下一任务",
            _ => "查看任务详情"
        }
    });
    transaction.execute(
        "INSERT OR IGNORE INTO outbox(id, dedupe_key, kind, task_id, payload_json, next_attempt_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            Uuid::new_v4().to_string(),
            dedupe,
            kind,
            task.id.to_string(),
            serde_json::to_string(&payload)?,
            available_at.to_rfc3339()
        ],
    )?;
    Ok(())
}

fn next_digest_boundary(now: DateTime<Utc>) -> DateTime<Utc> {
    let seconds = now.timestamp();
    let next = (seconds.div_euclid(300) + 1) * 300;
    DateTime::from_timestamp(next, 0).unwrap_or(now + Duration::minutes(5))
}

fn validate_action(task: &TaskRecord, action: CommandAction) -> Result<()> {
    let capability = match action {
        CommandAction::SendNext => Capability::SendNext,
        CommandAction::ResumeAndSend => Capability::ResumeAndSend,
        CommandAction::OpenAndPrefill => Capability::OpenAndPrefill,
    };
    if !task.capabilities.contains(&capability) {
        bail!("task does not advertise capability {capability:?}");
    }
    if action == CommandAction::ResumeAndSend
        && task.control_mode != ai_rpa_core::ControlMode::Managed
    {
        bail!("RESUME_AND_SEND is only available for managed sessions");
    }
    Ok(())
}

fn deliver_next_tx(
    transaction: &Transaction<'_>,
    crypto: &CryptoBox,
    task: &TaskRecord,
) -> Result<Option<CommandDelivery>> {
    expire_commands_tx(transaction)?;
    let active: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM commands WHERE task_id=?1 AND action='SEND_NEXT' AND state IN ('LEASED','DELIVERED','ACCEPTED')",
        [task.id.to_string()],
        |row| row.get(0),
    )?;
    if active > 0 {
        return Ok(None);
    }
    let row: Option<(String, String, String)> = transaction
        .query_row(
            "SELECT id, body_ciphertext, record_json FROM commands WHERE task_id=?1 AND action='SEND_NEXT' AND state IN ('QUEUED','RETRY_WAIT') AND expires_at>?2 ORDER BY created_at LIMIT 1",
            params![task.id.to_string(), Utc::now().to_rfc3339()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((id, ciphertext, record_json)) = row else {
        return Ok(None);
    };
    let mut record: CommandRecord = serde_json::from_str(&record_json)?;
    record.state = CommandState::Delivered;
    record.attempts += 1;
    update_command_tx(transaction, &record)?;
    append_audit_tx(
        transaction,
        Some(task.id),
        Some(record.id),
        "COMMAND_DELIVERED",
        "hook-runner",
        "SEND_NEXT delivered by stop hook",
    )?;
    Ok(Some(CommandDelivery {
        command_id: Uuid::parse_str(&id)?,
        action: record.action,
        message: crypto.decrypt(&ciphertext)?,
        expires_at: record.expires_at,
    }))
}

fn expire_commands_tx(transaction: &Transaction<'_>) -> Result<()> {
    let now_at = Utc::now();
    let now = now_at.to_rfc3339();
    let mut leased_statement = transaction.prepare(
        "SELECT id, record_json FROM commands WHERE state='LEASED' AND lease_until IS NOT NULL AND lease_until<=?1 AND expires_at>?1",
    )?;
    let leased_rows: Vec<(String, String)> = leased_statement
        .query_map([&now], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(leased_statement);
    for (id, record_json) in leased_rows {
        let mut record: CommandRecord = serde_json::from_str(&record_json)?;
        record.state = if record.attempts >= record.max_attempts {
            CommandState::UnknownDelivery
        } else {
            CommandState::RetryWait
        };
        record.lease_owner = None;
        record.lease_until = None;
        transaction.execute(
            "UPDATE commands SET state=?2, lease_owner=NULL, lease_until=NULL, record_json=?3 WHERE id=?1",
            params![id, record.state.to_string(), serde_json::to_string(&record)?],
        )?;
        append_audit_tx(
            transaction,
            Some(record.task_id),
            Some(record.id),
            "COMMAND_LEASE_EXPIRED",
            "lease-recovery",
            &format!("command recovered as {}", record.state),
        )?;
    }
    let mut statement = transaction.prepare(
        "SELECT id, record_json FROM commands WHERE state IN ('QUEUED','RETRY_WAIT','LEASED') AND expires_at<=?1",
    )?;
    let rows: Vec<(String, String)> = statement
        .query_map([&now], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(statement);
    for (id, record_json) in rows {
        let mut record: CommandRecord = serde_json::from_str(&record_json)?;
        record.state = CommandState::Expired;
        record.lease_owner = None;
        record.lease_until = None;
        transaction.execute(
            "UPDATE commands SET state='EXPIRED', lease_owner=NULL, lease_until=NULL, record_json=?2 WHERE id=?1",
            params![id, serde_json::to_string(&record)?],
        )?;
    }
    Ok(())
}

fn command_by_id_tx(transaction: &Transaction<'_>, id: Uuid) -> Result<CommandRecord> {
    let json: String = transaction.query_row(
        "SELECT record_json FROM commands WHERE id=?1",
        [id.to_string()],
        |row| row.get(0),
    )?;
    serde_json::from_str(&json).context("decode command")
}

fn update_command_tx(transaction: &Transaction<'_>, record: &CommandRecord) -> Result<()> {
    transaction.execute(
        "UPDATE commands SET state=?2, lease_owner=?3, lease_until=?4, attempts=?5, record_json=?6 WHERE id=?1",
        params![
            record.id.to_string(),
            record.state.to_string(),
            record.lease_owner,
            record.lease_until.map(|value| value.to_rfc3339()),
            record.attempts,
            serde_json::to_string(record)?
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_rpa_core::{ControlMode, EvidenceLevel, RawEventInput, normalize_event};
    use serde_json::json;
    use tempfile::TempDir;

    fn store() -> (TempDir, Store) {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("test.db");
        (
            directory,
            Store::open(path, CryptoBox::from_key([7_u8; 32])).unwrap(),
        )
    }

    fn raw(kind: &str, idempotency: &str, payload: Value) -> UnifiedEvent {
        normalize_event(
            RawEventInput {
                provider: Provider::Cursor,
                event_type: kind.to_owned(),
                event_id: None,
                idempotency_key: Some(idempotency.to_owned()),
                device_id: None,
                session_id: "session-1".to_owned(),
                turn_id: Some("turn-1".to_owned()),
                occurred_at: None,
                title: Some("Cursor task".to_owned()),
                workspace: Some("/Users/alice/private".to_owned()),
                project: Some("rpa".to_owned()),
                control_mode: ControlMode::Managed,
                required_evidence_level: EvidenceLevel::E2,
                payload,
            },
            "device-1",
        )
        .unwrap()
    }

    #[test]
    fn duplicate_event_has_no_duplicate_transition() {
        let (_directory, store) = store();
        let event = raw("start", "same", json!({}));
        let first = store.ingest_event(&event).unwrap();
        let second = store.ingest_event(&event).unwrap();
        assert!(!first.duplicate);
        assert!(second.duplicate);
        assert_eq!(store.counts().unwrap()["events"], 1);
    }

    #[test]
    fn stop_delivers_send_next_exactly_once() {
        let (_directory, store) = store();
        let start = store
            .ingest_event(&raw("start", "start", json!({})))
            .unwrap();
        let command = store
            .create_command(
                start.task.id,
                CommandAction::SendNext,
                "run tests",
                "tester",
                3600,
            )
            .unwrap();
        let stop = raw("stop", "stop", json!({}));
        let first = store.ingest_event(&stop).unwrap();
        let second = store.ingest_event(&stop).unwrap();
        assert_eq!(first.delivery.unwrap().message, "run tests");
        assert!(second.delivery.is_none());
        let detail = store.task_detail(start.task.id).unwrap().unwrap();
        assert_eq!(detail.commands[0].id, command.id);
        assert_eq!(detail.commands[0].state, CommandState::Delivered);
    }

    #[test]
    fn observed_session_cannot_resume() {
        let (_directory, store) = store();
        let mut event = raw("start", "start", json!({}));
        event.control_mode = ControlMode::Observed;
        event.capabilities = vec![Capability::SendNext, Capability::OpenAndPrefill];
        let task = store.ingest_event(&event).unwrap().task;
        let error = store
            .create_command(
                task.id,
                CommandAction::ResumeAndSend,
                "continue",
                "tester",
                3600,
            )
            .unwrap_err();
        assert!(error.to_string().contains("capability"));
    }

    #[test]
    fn encrypted_command_body_is_round_trippable() {
        let crypto = CryptoBox::from_key([9_u8; 32]);
        let ciphertext = crypto.encrypt("private prompt").unwrap();
        assert!(!ciphertext.contains("private"));
        assert_eq!(crypto.decrypt(&ciphertext).unwrap(), "private prompt");
    }

    #[test]
    fn later_managed_event_upgrades_task_capabilities() {
        let (_directory, store) = store();
        let mut observed = raw("start", "observed", json!({}));
        observed.control_mode = ai_rpa_core::ControlMode::Observed;
        observed.capabilities = vec![Capability::SendNext, Capability::OpenAndPrefill];
        let task = store.ingest_event(&observed).unwrap().task;
        assert_eq!(task.control_mode, ai_rpa_core::ControlMode::Observed);

        let mut managed = raw("heartbeat", "managed", json!({}));
        managed.control_mode = ai_rpa_core::ControlMode::Managed;
        managed.capabilities.push(Capability::ResumeAndSend);
        let task = store.ingest_event(&managed).unwrap().task;
        assert_eq!(task.control_mode, ai_rpa_core::ControlMode::Managed);
        assert!(task.capabilities.contains(&Capability::ResumeAndSend));
    }

    #[test]
    fn duplicate_storm_remains_exactly_once() {
        let (_directory, store) = store();
        let event = raw("start", "storm-key", json!({}));
        for _ in 0..1000 {
            store.ingest_event(&event).unwrap();
        }
        let counts = store.counts().unwrap();
        assert_eq!(counts["events"], 1);
        assert_eq!(counts["tasks"], 1);
    }

    #[test]
    fn expired_lease_is_recovered_and_released() {
        let (_directory, store) = store();
        let mut managed = raw("start", "lease-start", json!({}));
        managed.provider = Provider::Codex;
        managed.capabilities.push(Capability::ResumeAndSend);
        let task = store.ingest_event(&managed).unwrap().task;
        store
            .create_command(
                task.id,
                CommandAction::ResumeAndSend,
                "continue",
                "tester",
                3600,
            )
            .unwrap();
        let first = store
            .lease_managed_command("device-1", "runner-1", 0)
            .unwrap()
            .unwrap();
        let second = store
            .lease_managed_command("device-1", "runner-2", 60)
            .unwrap()
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.attempts, 2);
        assert_eq!(second.lease_owner.as_deref(), Some("runner-2"));
    }

    #[test]
    fn task_filters_cover_control_mode_project_device_and_time() {
        let (_directory, store) = store();
        let task = store
            .ingest_event(&raw("start", "filter-start", json!({})))
            .unwrap()
            .task;
        let items = store
            .list_tasks(&TaskFilter {
                device_id: Some("device-1".to_owned()),
                project: Some("rpa".to_owned()),
                control_mode: Some(ControlMode::Managed),
                updated_after: Some(Utc::now() - Duration::minutes(1)),
                updated_before: Some(Utc::now() + Duration::minutes(1)),
                ..TaskFilter::default()
            })
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, task.id);
        assert!(
            store
                .list_tasks(&TaskFilter {
                    control_mode: Some(ControlMode::Observed),
                    ..TaskFilter::default()
                })
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn one_managed_lease_per_task_and_pending_queue_is_bounded() {
        let (_directory, primary_store) = store();
        let mut managed = raw("start", "single-lease", json!({}));
        managed.provider = Provider::Codex;
        managed.capabilities.push(Capability::ResumeAndSend);
        let task = primary_store.ingest_event(&managed).unwrap().task;
        let first = primary_store
            .create_command(
                task.id,
                CommandAction::ResumeAndSend,
                "first",
                "tester",
                3600,
            )
            .unwrap();
        primary_store
            .create_command(
                task.id,
                CommandAction::ResumeAndSend,
                "second",
                "tester",
                3600,
            )
            .unwrap();
        let leased = primary_store
            .lease_managed_command("device-1", "runner-1", 60)
            .unwrap()
            .unwrap();
        assert_eq!(leased.id, first.id);
        assert!(
            primary_store
                .lease_managed_command("device-1", "runner-2", 60)
                .unwrap()
                .is_none()
        );
        primary_store
            .update_command(first.id, CommandState::Completed, None, "tester")
            .unwrap();
        assert!(
            primary_store
                .lease_managed_command("device-1", "runner-2", 60)
                .unwrap()
                .is_some()
        );

        let (_other_directory, other_store) = store();
        let other_task = other_store
            .ingest_event(&raw("start", "bounded-queue", json!({})))
            .unwrap()
            .task;
        for index in 0..5 {
            other_store
                .create_command(
                    other_task.id,
                    CommandAction::SendNext,
                    &format!("next-{index}"),
                    "tester",
                    3600,
                )
                .unwrap();
        }
        let error = other_store
            .create_command(
                other_task.id,
                CommandAction::SendNext,
                "overflow",
                "tester",
                3600,
            )
            .unwrap_err();
        assert!(error.to_string().contains("at most 5 pending"));
    }

    #[test]
    fn central_sync_is_durable_and_remote_commands_are_idempotent() {
        let (directory, store) = store();
        let task = store
            .ingest_event(&raw("start", "sync-event", json!({})))
            .unwrap()
            .task;
        let pending = store.pending_sync_events(10).unwrap();
        assert_eq!(pending.len(), 1);
        store
            .mark_sync_events_retry(&[pending[0].event_id], "temporary network error")
            .unwrap();
        assert_eq!(store.pending_sync_events(10).unwrap().len(), 1);
        store.mark_sync_events_sent(&[pending[0].event_id]).unwrap();
        assert!(store.pending_sync_events(10).unwrap().is_empty());
        drop(store);
        let store = Store::open(
            directory.path().join("test.db"),
            CryptoBox::from_key([7_u8; 32]),
        )
        .unwrap();
        assert!(store.pending_sync_events(10).unwrap().is_empty());

        let remote = ai_rpa_core::RemoteCommand {
            id: Uuid::new_v4(),
            central_task_id: Uuid::new_v4(),
            provider: task.provider,
            device_id: task.device_id.clone(),
            session_id: task.session_id.clone(),
            action: CommandAction::SendNext,
            message: "继续验证".to_owned(),
            created_by: "central-admin".to_owned(),
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::hours(1),
        };
        assert!(store.import_remote_command(&remote).unwrap());
        assert!(!store.import_remote_command(&remote).unwrap());
        assert!(store.remote_command_updates().unwrap().is_empty());
        let delivered = store
            .ingest_event(&raw("stop", "sync-stop", json!({})))
            .unwrap()
            .delivery
            .unwrap();
        assert_eq!(delivered.command_id, remote.id);
        assert_eq!(delivered.message, "继续验证");
        let updates = store.remote_command_updates().unwrap();
        assert_eq!(updates[0].state, CommandState::Delivered);
        store
            .mark_remote_command_reported(remote.id, CommandState::Delivered)
            .unwrap();
        assert!(store.remote_command_updates().unwrap().is_empty());
    }
}
