use std::collections::{BTreeMap, HashMap};

use ai_rpa_core::{
    AdapterStatus, Capability, CommandAction, CommandRecord, CommandState, ControlMode,
    DeviceRecord, Provider, RemoteCommand, TaskRecord, TaskState, UnifiedEvent, derive_state,
    redact_text,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx_core::{row::Row, transaction::Transaction};
use sqlx_postgres::{PgPool, Postgres};
use uuid::Uuid;

use crate::crypto::ServerCrypto;

mod sqlx {
    pub use sqlx_core::{query::query, query_scalar::query_scalar};
}

#[derive(Clone)]
pub struct CentralStore {
    pub pool: PgPool,
    crypto: ServerCrypto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    #[serde(flatten)]
    pub device: DeviceRecord,
    pub alias: String,
    pub online: bool,
    pub revoked: bool,
    pub connected_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CentralTaskFilter {
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
pub struct AuditView {
    pub id: Uuid,
    pub task_id: Option<Uuid>,
    pub command_id: Option<Uuid>,
    pub action: String,
    pub actor: String,
    pub summary: String,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CentralTaskDetail {
    pub task: TaskRecord,
    pub events: Vec<UnifiedEvent>,
    pub commands: Vec<CommandRecord>,
    pub audit: Vec<AuditView>,
}

impl CentralStore {
    pub fn new(pool: PgPool, crypto: ServerCrypto) -> Self {
        Self { pool, crypto }
    }

    pub async fn create_enrollment_code(
        &self,
        code_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query("INSERT INTO enrollment_codes(code_hash, expires_at) VALUES ($1, $2)")
            .bind(code_hash)
            .bind(expires_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn enroll_device(
        &self,
        code_hash: &str,
        device: &DeviceRecord,
        alias: &str,
        token_hash: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let code = sqlx::query(
            "SELECT expires_at, max_uses, uses FROM enrollment_codes WHERE code_hash=$1 FOR UPDATE",
        )
        .bind(code_hash)
        .fetch_optional(&mut *tx)
        .await?
        .context("invalid enrollment code")?;
        let expires_at: DateTime<Utc> = code.try_get("expires_at")?;
        let max_uses: i32 = code.try_get("max_uses")?;
        let uses: i32 = code.try_get("uses")?;
        if expires_at <= Utc::now() || uses >= max_uses {
            bail!("enrollment code expired or already used");
        }
        sqlx::query("UPDATE enrollment_codes SET uses=uses+1 WHERE code_hash=$1")
            .bind(code_hash)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO devices(id, alias, os, arch, hostname, logical_environment, node_version, token_hash, last_seen_at, record_json)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
             ON CONFLICT(id) DO UPDATE SET alias=excluded.alias, os=excluded.os, arch=excluded.arch,
               hostname=excluded.hostname, logical_environment=excluded.logical_environment,
               node_version=excluded.node_version, token_hash=excluded.token_hash,
               last_seen_at=excluded.last_seen_at, revoked_at=NULL, record_json=excluded.record_json",
        )
        .bind(&device.id)
        .bind(alias)
        .bind(&device.os)
        .bind(&device.arch)
        .bind(&device.hostname)
        .bind(&device.logical_environment)
        .bind(&device.node_version)
        .bind(token_hash)
        .bind(device.last_seen_at)
        .bind(serde_json::to_value(device)?)
        .execute(&mut *tx)
        .await?;
        append_audit(
            &mut tx,
            None,
            None,
            "DEVICE_ENROLLED",
            "enrollment",
            &device.id,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn verify_device_token(&self, device_id: &str, token_hash: &str) -> Result<bool> {
        let stored = sqlx::query_scalar::<_, String>(
            "SELECT token_hash FROM devices WHERE id=$1 AND revoked_at IS NULL",
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(stored.is_some_and(|value| constant_time_eq(value.as_bytes(), token_hash.as_bytes())))
    }

    pub async fn set_connected(&self, device_id: &str, connected: bool) -> Result<()> {
        if connected {
            sqlx::query("UPDATE devices SET connected_at=now(), last_seen_at=now() WHERE id=$1")
                .bind(device_id)
                .execute(&self.pool)
                .await?;
        } else {
            sqlx::query("UPDATE devices SET connected_at=NULL WHERE id=$1")
                .bind(device_id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    pub async fn heartbeat(
        &self,
        authenticated_device_id: &str,
        device: &DeviceRecord,
        adapters: &[AdapterStatus],
    ) -> Result<()> {
        if authenticated_device_id != device.id {
            bail!("heartbeat device id does not match authenticated device");
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE devices SET os=$2, arch=$3, hostname=$4, logical_environment=$5,
             node_version=$6, last_seen_at=now(), connected_at=now(), record_json=$7 WHERE id=$1 AND revoked_at IS NULL",
        )
        .bind(&device.id)
        .bind(&device.os)
        .bind(&device.arch)
        .bind(&device.hostname)
        .bind(&device.logical_environment)
        .bind(&device.node_version)
        .bind(serde_json::to_value(device)?)
        .execute(&mut *tx)
        .await?;
        for adapter in adapters {
            sqlx::query(
                "INSERT INTO adapters(device_id, provider, last_event_at, record_json) VALUES ($1,$2,$3,$4)
                 ON CONFLICT(device_id,provider) DO UPDATE SET last_event_at=excluded.last_event_at, record_json=excluded.record_json",
            )
            .bind(&device.id)
            .bind(adapter.provider.as_str())
            .bind(adapter.last_event_at)
            .bind(serde_json::to_value(adapter)?)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn ingest_events(
        &self,
        authenticated_device_id: &str,
        events: &[UnifiedEvent],
    ) -> Result<usize> {
        let mut accepted = 0;
        for event in events {
            if event.device_id != authenticated_device_id {
                bail!("event device id does not match authenticated device");
            }
            if self.ingest_event(event).await? {
                accepted += 1;
            }
        }
        Ok(accepted)
    }

    async fn ingest_event(&self, event: &UnifiedEvent) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let existing = sqlx::query_scalar::<_, Value>(
            "SELECT snapshot_json FROM tasks WHERE provider=$1 AND device_id=$2 AND session_id=$3",
        )
        .bind(event.provider.as_str())
        .bind(&event.device_id)
        .bind(&event.session_id)
        .fetch_optional(&mut *tx)
        .await?;
        let mut task = if let Some(value) = existing {
            serde_json::from_value::<TaskRecord>(value)?
        } else {
            let task = TaskRecord {
                id: Uuid::new_v4(),
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
            sqlx::query(
                "INSERT INTO tasks(id,provider,device_id,session_id,state,project,updated_at,snapshot_json)
                 VALUES($1,$2,$3,$4,$5,$6,$7,$8)",
            )
            .bind(task.id)
            .bind(task.provider.as_str())
            .bind(&task.device_id)
            .bind(&task.session_id)
            .bind(task.state.to_string())
            .bind(&task.project)
            .bind(task.updated_at)
            .bind(serde_json::to_value(&task)?)
            .execute(&mut *tx)
            .await?;
            task
        };
        let inserted = sqlx::query(
            "INSERT INTO events(event_id,idempotency_key,task_id,occurred_at,event_type,event_json)
             VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING",
        )
        .bind(event.event_id)
        .bind(&event.idempotency_key)
        .bind(task.id)
        .bind(event.occurred_at)
        .bind(event.event_type.to_string())
        .bind(serde_json::to_value(event)?)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted == 0 {
            tx.commit().await?;
            return Ok(false);
        }
        let event_values = sqlx::query_scalar::<_, Value>(
            "SELECT event_json FROM events WHERE task_id=$1 ORDER BY occurred_at,event_id",
        )
        .bind(task.id)
        .fetch_all(&mut *tx)
        .await?;
        let ordered = event_values
            .into_iter()
            .map(serde_json::from_value)
            .collect::<serde_json::Result<Vec<UnifiedEvent>>>()?;
        let derived = derive_state(
            &ordered,
            task.required_evidence_level
                .max(event.required_evidence_level),
        );
        let old_state = task.state;
        for capability in &event.capabilities {
            if !task.capabilities.contains(capability) {
                task.capabilities.push(*capability);
            }
        }
        task.title = if event.title.ends_with(" task") {
            task.title
        } else {
            event.title.clone()
        };
        task.workspace = event.workspace.clone().or(task.workspace);
        task.project = event.project.clone().or(task.project);
        if event.control_mode == ControlMode::Managed {
            task.control_mode = ControlMode::Managed;
        }
        task.required_evidence_level = task
            .required_evidence_level
            .max(event.required_evidence_level);
        task.state = derived.state;
        task.confidence = derived.confidence;
        task.evidence_level = derived.evidence_level;
        task.evidence_summary = derived.evidence_summary;
        task.started_at = derived.started_at;
        task.updated_at = derived.updated_at;
        task.duration_ms = derived.duration_ms;
        task.last_event_type = derived.last_event_type;
        if old_state != task.state {
            task.state_version += 1;
            append_audit(
                &mut tx,
                Some(task.id),
                None,
                "TASK_STATE_CHANGED",
                "central-state-engine",
                &format!("{old_state} -> {}", task.state),
            )
            .await?;
        }
        sqlx::query(
            "UPDATE tasks SET state=$2,project=$3,updated_at=$4,snapshot_json=$5 WHERE id=$1",
        )
        .bind(task.id)
        .bind(task.state.to_string())
        .bind(&task.project)
        .bind(task.updated_at)
        .bind(serde_json::to_value(&task)?)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn devices(&self) -> Result<Vec<DeviceView>> {
        let rows = sqlx::query(
            "SELECT alias, record_json, connected_at, revoked_at FROM devices ORDER BY last_seen_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        let cutoff = Utc::now() - Duration::seconds(75);
        rows.into_iter()
            .map(|row| {
                let device: DeviceRecord = serde_json::from_value(row.try_get("record_json")?)?;
                let connected_at: Option<DateTime<Utc>> = row.try_get("connected_at")?;
                let revoked_at: Option<DateTime<Utc>> = row.try_get("revoked_at")?;
                Ok(DeviceView {
                    online: revoked_at.is_none()
                        && connected_at.is_some_and(|value| value >= cutoff)
                        && device.last_seen_at >= cutoff,
                    revoked: revoked_at.is_some(),
                    alias: row.try_get("alias")?,
                    connected_at,
                    device,
                })
            })
            .collect()
    }

    pub async fn rename_device(&self, device_id: &str, alias: &str) -> Result<()> {
        let changed = sqlx::query("UPDATE devices SET alias=$2 WHERE id=$1")
            .bind(device_id)
            .bind(alias)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if changed == 0 {
            bail!("device not found");
        }
        Ok(())
    }

    pub async fn revoke_device(&self, device_id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE devices SET revoked_at=now(), connected_at=NULL WHERE id=$1 AND revoked_at IS NULL",
        )
        .bind(device_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if changed == 0 {
            bail!("device not found or already revoked");
        }
        append_audit(&mut tx, None, None, "DEVICE_REVOKED", "admin", device_id).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_tasks(&self, filter: &CentralTaskFilter) -> Result<Vec<TaskRecord>> {
        let values = sqlx::query_scalar::<_, Value>(
            "SELECT snapshot_json FROM tasks ORDER BY updated_at DESC LIMIT 10000",
        )
        .fetch_all(&self.pool)
        .await?;
        let needle = filter.search.as_deref().map(str::to_ascii_lowercase);
        let mut output = Vec::new();
        for value in values {
            let task: TaskRecord = serde_json::from_value(value)?;
            if filter.provider.is_some_and(|v| v != task.provider)
                || filter.state.is_some_and(|v| v != task.state)
                || filter
                    .device_id
                    .as_deref()
                    .is_some_and(|v| v != task.device_id)
                || filter
                    .project
                    .as_deref()
                    .is_some_and(|v| task.project.as_deref() != Some(v))
                || filter.control_mode.is_some_and(|v| v != task.control_mode)
                || filter.updated_after.is_some_and(|v| task.updated_at < v)
                || filter.updated_before.is_some_and(|v| task.updated_at > v)
            {
                continue;
            }
            if needle.as_deref().is_some_and(|value| {
                !format!(
                    "{} {} {} {}",
                    task.title,
                    task.session_id,
                    task.workspace.as_deref().unwrap_or(""),
                    task.project.as_deref().unwrap_or("")
                )
                .to_ascii_lowercase()
                .contains(value)
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

    pub async fn task_detail(&self, id: Uuid) -> Result<Option<CentralTaskDetail>> {
        let task_value =
            sqlx::query_scalar::<_, Value>("SELECT snapshot_json FROM tasks WHERE id=$1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        let Some(task_value) = task_value else {
            return Ok(None);
        };
        let task = serde_json::from_value(task_value)?;
        let events = sqlx::query_scalar::<_, Value>(
            "SELECT event_json FROM events WHERE task_id=$1 ORDER BY occurred_at,event_id",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(serde_json::from_value)
        .collect::<serde_json::Result<_>>()?;
        let commands = sqlx::query_scalar::<_, Value>(
            "SELECT record_json FROM commands WHERE task_id=$1 ORDER BY created_at DESC",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(serde_json::from_value)
        .collect::<serde_json::Result<_>>()?;
        let audit_rows = sqlx::query(
            "SELECT id,task_id,command_id,action,actor,summary,occurred_at FROM audit_log WHERE task_id=$1 ORDER BY occurred_at",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        let audit = audit_rows
            .into_iter()
            .map(|row| {
                Ok(AuditView {
                    id: row.try_get("id")?,
                    task_id: row.try_get("task_id")?,
                    command_id: row.try_get("command_id")?,
                    action: row.try_get("action")?,
                    actor: row.try_get("actor")?,
                    summary: row.try_get("summary")?,
                    occurred_at: row.try_get("occurred_at")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(CentralTaskDetail {
            task,
            events,
            commands,
            audit,
        }))
    }

    pub async fn create_command(
        &self,
        task_id: Uuid,
        action: CommandAction,
        message: &str,
        created_by: &str,
        ttl_seconds: i64,
    ) -> Result<RemoteCommand> {
        if message.trim().is_empty() || message.len() > 32 * 1024 {
            bail!("message must contain 1..32768 bytes");
        }
        if action == CommandAction::OpenAndPrefill {
            bail!("OPEN_AND_PREFILL is only available from the target computer");
        }
        if !(60..=86_400).contains(&ttl_seconds) {
            bail!("ttlSeconds must be between 60 and 86400");
        }
        let task = self
            .task_detail(task_id)
            .await?
            .context("task not found")?
            .task;
        validate_action(&task, action)?;
        let pending: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM commands WHERE task_id=$1 AND state IN ('QUEUED','RETRY_WAIT','DELIVERED','ACCEPTED')",
        )
        .bind(task_id)
        .fetch_one(&self.pool)
        .await?;
        if pending >= 5 {
            bail!("at most 5 pending continuation commands are allowed per task");
        }
        let now = Utc::now();
        let remote = RemoteCommand {
            id: Uuid::new_v4(),
            central_task_id: task_id,
            provider: task.provider,
            device_id: task.device_id.clone(),
            session_id: task.session_id.clone(),
            action,
            message: message.to_owned(),
            created_by: redact_text(created_by).text,
            created_at: now,
            expires_at: now + Duration::seconds(ttl_seconds),
        };
        let record = CommandRecord {
            id: remote.id,
            task_id,
            provider: remote.provider,
            device_id: remote.device_id.clone(),
            session_id: remote.session_id.clone(),
            action,
            state: CommandState::Queued,
            message: None,
            created_by: remote.created_by.clone(),
            created_at: remote.created_at,
            expires_at: remote.expires_at,
            lease_owner: None,
            lease_until: None,
            attempts: 0,
            max_attempts: 3,
            result_summary: None,
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO commands(id,task_id,device_id,action,state,body_ciphertext,created_at,expires_at,record_json)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(record.id)
        .bind(record.task_id)
        .bind(&record.device_id)
        .bind(record.action.to_string())
        .bind(record.state.to_string())
        .bind(self.crypto.encrypt(&remote.message)?)
        .bind(record.created_at)
        .bind(record.expires_at)
        .bind(serde_json::to_value(&record)?)
        .execute(&mut *tx)
        .await?;
        append_audit(
            &mut tx,
            Some(task_id),
            Some(record.id),
            "REMOTE_COMMAND_CREATED",
            created_by,
            "remote AI continuation queued",
        )
        .await?;
        tx.commit().await?;
        Ok(remote)
    }

    pub async fn pending_commands(&self, device_id: &str) -> Result<Vec<RemoteCommand>> {
        sqlx::query("UPDATE commands SET state='EXPIRED' WHERE device_id=$1 AND state IN ('QUEUED','RETRY_WAIT') AND expires_at<=now()")
            .bind(device_id)
            .execute(&self.pool)
            .await?;
        let rows = sqlx::query(
            "SELECT body_ciphertext,record_json FROM commands
             WHERE device_id=$1 AND state IN ('QUEUED','RETRY_WAIT') AND expires_at>now()
             ORDER BY created_at LIMIT 20",
        )
        .bind(device_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let record: CommandRecord = serde_json::from_value(row.try_get("record_json")?)?;
                Ok(RemoteCommand {
                    id: record.id,
                    central_task_id: record.task_id,
                    provider: record.provider,
                    device_id: record.device_id,
                    session_id: record.session_id,
                    action: record.action,
                    message: self.crypto.decrypt(row.try_get("body_ciphertext")?)?,
                    created_by: record.created_by,
                    created_at: record.created_at,
                    expires_at: record.expires_at,
                })
            })
            .collect()
    }

    pub async fn ack_command(
        &self,
        device_id: &str,
        command_id: Uuid,
        state: CommandState,
        result_summary: Option<&str>,
    ) -> Result<()> {
        if matches!(
            state,
            CommandState::Created | CommandState::Queued | CommandState::Leased
        ) {
            bail!("node cannot acknowledge command as {state}");
        }
        let value = sqlx::query_scalar::<_, Value>(
            "SELECT record_json FROM commands WHERE id=$1 AND device_id=$2",
        )
        .bind(command_id)
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await?
        .context("command does not belong to authenticated device")?;
        let mut record: CommandRecord = serde_json::from_value(value)?;
        record.state = state;
        record.result_summary = result_summary.map(|value| redact_text(value).text);
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE commands SET state=$2,record_json=$3 WHERE id=$1")
            .bind(command_id)
            .bind(state.to_string())
            .bind(serde_json::to_value(&record)?)
            .execute(&mut *tx)
            .await?;
        append_audit(
            &mut tx,
            Some(record.task_id),
            Some(command_id),
            "REMOTE_COMMAND_ACK",
            device_id,
            &format!("node reported {state}"),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn dashboard(&self) -> Result<Value> {
        let tasks = self
            .list_tasks(&CentralTaskFilter {
                limit: Some(1000),
                ..Default::default()
            })
            .await?;
        let devices = self.devices().await?;
        let adapter_values = sqlx::query_scalar::<_, Value>("SELECT record_json FROM adapters")
            .fetch_all(&self.pool)
            .await?;
        let mut by_provider: HashMap<Provider, AdapterStatus> = HashMap::new();
        for value in adapter_values {
            let adapter: AdapterStatus = serde_json::from_value(value)?;
            by_provider
                .entry(adapter.provider)
                .and_modify(|current| {
                    if adapter.install_state == "RUNNING" {
                        current.install_state = "RUNNING".to_owned();
                    }
                    if adapter.hook_state == "HEALTHY" {
                        current.hook_state = "HEALTHY".to_owned();
                    }
                    if adapter.last_event_at > current.last_event_at {
                        current.last_event_at = adapter.last_event_at;
                    }
                    for capability in &adapter.capabilities {
                        if !current.capabilities.contains(capability) {
                            current.capabilities.push(*capability);
                        }
                    }
                })
                .or_insert(adapter);
        }
        let adapters: Vec<_> = Provider::ALL
            .into_iter()
            .filter_map(|provider| by_provider.remove(&provider))
            .collect();
        let mut counts = BTreeMap::new();
        for task in &tasks {
            *counts.entry(task.state.to_string()).or_insert(0usize) += 1;
        }
        let since = Utc::now() - Duration::hours(24);
        let completed: Vec<_> = tasks
            .iter()
            .filter(|task| task.updated_at >= since && task.state.is_terminal())
            .collect();
        let completion_rate = if completed.is_empty() {
            0.0
        } else {
            completed
                .iter()
                .filter(|task| task.state == TaskState::Succeeded)
                .count() as f64
                / completed.len() as f64
        };
        let mut durations: Vec<_> = completed
            .iter()
            .filter_map(|task| task.duration_ms)
            .collect();
        durations.sort_unstable();
        let p95 = (!durations.is_empty()).then(|| {
            let index = ((durations.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
            durations[index.min(durations.len() - 1)]
        });
        let active: Vec<_> = tasks
            .iter()
            .filter(|task| matches!(task.state, TaskState::Running | TaskState::WaitingUser))
            .collect();
        let observed_at = Utc::now();
        let providers: Vec<_> = adapters
            .iter()
            .map(|adapter| {
                let active_count = active
                    .iter()
                    .filter(|task| task.provider == adapter.provider)
                    .count();
                json!({
                    "provider": adapter.provider,
                    "connectionState": adapter.install_state,
                    "trackingState": if active_count > 0 { "LIVE" } else { "READY" },
                    "activeTaskCount": active_count,
                    "lastEventAt": adapter.last_event_at
                })
            })
            .collect();
        let live_tasks: Vec<_> = active
            .iter()
            .map(|task| {
                let mut value = serde_json::to_value(task).expect("task serializes");
                if let Value::Object(object) = &mut value {
                    let age = (observed_at - task.updated_at).num_seconds().max(0);
                    object.insert("source".to_owned(), json!("HOOK_EVENT"));
                    object.insert("stale".to_owned(), json!(age > 300));
                    object.insert("ageSeconds".to_owned(), json!(age));
                }
                value
            })
            .collect();
        let attention: Vec<_> = tasks
            .iter()
            .filter(|task| {
                matches!(
                    task.state,
                    TaskState::WaitingUser | TaskState::Failed | TaskState::Unknown
                )
            })
            .take(10)
            .cloned()
            .collect();
        Ok(json!({
            "counts": counts,
            "completionRate24h": completion_rate,
            "p95DurationMs": p95,
            "devices": devices,
            "adapters": adapters,
            "live": {
                "observedAt": observed_at,
                "pollIntervalMs": 2000,
                "connectedProviderCount": adapters.iter().filter(|a| a.install_state == "RUNNING").count(),
                "monitoredProviderCount": adapters.iter().filter(|a| matches!(a.hook_state.as_str(), "CONFIGURED" | "HEALTHY")).count(),
                "executingTaskCount": active.iter().filter(|task| task.state == TaskState::Running).count(),
                "waitingTaskCount": active.iter().filter(|task| task.state == TaskState::WaitingUser).count(),
                "providers": providers,
                "tasks": live_tasks
            },
            "attention": attention,
            "recent": tasks.iter().take(10).collect::<Vec<_>>()
        }))
    }
}

pub fn token_hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

fn validate_action(task: &TaskRecord, action: CommandAction) -> Result<()> {
    let capability = match action {
        CommandAction::SendNext => Capability::SendNext,
        CommandAction::ResumeAndSend => Capability::ResumeAndSend,
        CommandAction::OpenAndPrefill => Capability::OpenAndPrefill,
    };
    if !task.capabilities.contains(&capability) {
        bail!("task does not advertise requested capability");
    }
    if action == CommandAction::ResumeAndSend && task.control_mode != ControlMode::Managed {
        bail!("RESUME_AND_SEND is only available for managed sessions");
    }
    Ok(())
}

async fn append_audit(
    tx: &mut Transaction<'_, Postgres>,
    task_id: Option<Uuid>,
    command_id: Option<Uuid>,
    action: &str,
    actor: &str,
    summary: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log(id,task_id,command_id,action,actor,summary,occurred_at) VALUES($1,$2,$3,$4,$5,$6,now())",
    )
    .bind(Uuid::new_v4())
    .bind(task_id)
    .bind(command_id)
    .bind(action)
    .bind(redact_text(actor).text)
    .bind(redact_text(summary).text)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_stable_and_constant_time_comparison_is_exact() {
        assert_eq!(token_hash("same"), token_hash("same"));
        assert_ne!(token_hash("same"), token_hash("other"));
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
