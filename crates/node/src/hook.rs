use std::{fs, io::Read, path::Path, time::Duration};

use ai_rpa_core::{ControlMode, EvidenceLevel, Provider, RawEventInput};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use reqwest::Client;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub fn read_stdin_json() -> Result<Value> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        bail!("hook payload exceeds 1 MiB");
    }
    serde_json::from_slice(&bytes).context("hook input is not valid JSON")
}

pub fn to_raw_event(
    provider: Provider,
    event_type: &str,
    session_override: Option<&str>,
    mut payload: Value,
) -> Result<RawEventInput> {
    let session_id = session_override
        .map(str::to_owned)
        .or_else(|| {
            first_string(
                &payload,
                &[
                    "session_id",
                    "sessionId",
                    "conversation_id",
                    "conversationId",
                    "thread_id",
                    "threadId",
                ],
            )
        })
        .context("hook payload does not contain a session/conversation/thread id")?;
    let turn_id = first_string(
        &payload,
        &["turn_id", "turnId", "agent_run_id", "agentRunId"],
    );
    let title = first_string(&payload, &["title", "task_title", "taskTitle"]);
    let workspace = first_string(&payload, &["cwd", "workspace", "workspacePath"]);
    let project = first_string(&payload, &["project", "projectName"]);
    let managed = payload
        .get("managed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let event_type = classify_event(provider, event_type, &mut payload);
    Ok(RawEventInput {
        provider,
        event_type,
        event_id: None,
        idempotency_key: first_string(&payload, &["event_id", "eventId"]),
        device_id: None,
        session_id,
        turn_id,
        occurred_at: None,
        title,
        workspace,
        project,
        control_mode: if managed {
            ControlMode::Managed
        } else {
            ControlMode::Observed
        },
        required_evidence_level: first_string(
            &payload,
            &["requiredEvidenceLevel", "required_evidence_level"],
        )
        .and_then(|value| value.parse().ok())
        .unwrap_or(EvidenceLevel::E2),
        payload: sanitized_payload(&payload),
    })
}

fn sanitized_payload(payload: &Value) -> Value {
    let mut safe = serde_json::Map::new();
    let mappings = [
        ("status", "status"),
        ("terminationReason", "termination_reason"),
        ("termination_reason", "termination_reason"),
        ("fullyIdle", "fully_idle"),
        ("fully_idle", "fully_idle"),
        ("modelName", "model"),
        ("model", "model"),
        ("stop_hook_active", "stop_hook_active"),
        ("structured_success", "structured_success"),
        ("tests_passed", "tests_passed"),
        ("artifact_hash", "artifact_hash"),
        ("exit_code", "exit_code"),
        ("error_code", "error_code"),
        ("notification_type", "notification_type"),
    ];
    for (source, target) in mappings {
        let Some(value) = payload.get(source) else {
            continue;
        };
        if matches!(
            value,
            Value::String(_) | Value::Bool(_) | Value::Number(_) | Value::Null
        ) {
            safe.insert(target.to_owned(), value.clone());
        }
    }
    Value::Object(safe)
}

fn classify_event(provider: Provider, event_type: &str, payload: &mut Value) -> String {
    let normalized = event_type.to_ascii_lowercase();
    if normalized.contains("failure")
        || payload
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| status.eq_ignore_ascii_case("error"))
        || payload
            .get("terminationReason")
            .or_else(|| payload.get("termination_reason"))
            .and_then(Value::as_str)
            .is_some_and(|reason| reason.eq_ignore_ascii_case("error"))
    {
        return "Failed".to_owned();
    }
    if payload
        .get("status")
        .or_else(|| payload.get("reason"))
        .and_then(Value::as_str)
        .is_some_and(|status| status.eq_ignore_ascii_case("aborted"))
    {
        return "Cancelled".to_owned();
    }
    if normalized == "notification" {
        return match payload.get("notification_type").and_then(Value::as_str) {
            Some("permission_prompt" | "idle_prompt" | "elicitation_dialog") => {
                "WaitingUser".to_owned()
            }
            _ => "Heartbeat".to_owned(),
        };
    }
    let completed_stop = normalized == "stop"
        && provider == Provider::Cursor
        && payload
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| status.eq_ignore_ascii_case("completed"));
    if completed_stop {
        if let Some(object) = payload.as_object_mut() {
            object.insert("structured_success".to_owned(), Value::Bool(true));
        }
        return "Result".to_owned();
    }
    event_type.to_owned()
}

pub async fn submit(base_url: &str, event: &RawEventInput) -> Result<Value> {
    let client = Client::builder()
        .connect_timeout(Duration::from_millis(150))
        .timeout(Duration::from_millis(500))
        .build()?;
    let response = client
        .post(format!("{base_url}/api/events"))
        .json(event)
        .send()
        .await
        .context("submit hook event to local node")?;
    if !response.status().is_success() {
        bail!("local node returned HTTP {}", response.status());
    }
    response.json().await.context("decode local node response")
}

pub fn spool(spool_dir: &Path, event: &RawEventInput) -> Result<std::path::PathBuf> {
    fs::create_dir_all(spool_dir)?;
    let path = spool_dir.join(format!(
        "{}-{}.json",
        Utc::now().timestamp_millis(),
        Uuid::new_v4()
    ));
    fs::write(&path, serde_json::to_vec(event)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

pub async fn drain_spool(spool_dir: &Path, base_url: &str) -> Result<usize> {
    let mut drained = 0;
    for entry in fs::read_dir(spool_dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(entry.path())?;
        let event: RawEventInput = match serde_json::from_slice(&bytes) {
            Ok(event) => event,
            Err(_) => {
                let digest = hex::encode(Sha256::digest(&bytes));
                fs::rename(
                    entry.path(),
                    spool_dir.join(format!("quarantine-{digest}.bad")),
                )?;
                continue;
            }
        };
        if submit(base_url, &event).await.is_ok() {
            fs::remove_file(entry.path())?;
            drained += 1;
        }
    }
    Ok(drained)
}

pub fn hook_stdout(provider_response: Option<&Value>) -> String {
    provider_response
        .cloned()
        .unwrap_or_else(|| json!({}))
        .to_string()
}

fn first_string(payload: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| payload.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_vendor_stop_outcomes_without_guessing_cursor_abort() {
        let cursor_done = to_raw_event(
            Provider::Cursor,
            "stop",
            None,
            json!({"session_id":"s1","status":"completed"}),
        )
        .unwrap();
        assert_eq!(cursor_done.event_type, "Result");
        assert_eq!(cursor_done.payload["structured_success"], true);

        let cursor_aborted = to_raw_event(
            Provider::Cursor,
            "stop",
            None,
            json!({"session_id":"s2","status":"aborted"}),
        )
        .unwrap();
        assert_eq!(cursor_aborted.event_type, "Cancelled");

        let antigravity_error = to_raw_event(
            Provider::Antigravity,
            "Stop",
            None,
            json!({"conversationId":"s3","terminationReason":"error","fullyIdle":true}),
        )
        .unwrap();
        assert_eq!(antigravity_error.event_type, "Failed");

        let codex_stop =
            to_raw_event(Provider::Codex, "Stop", None, json!({"session_id":"s4"})).unwrap();
        let codex_stop = ai_rpa_core::normalize_event(codex_stop, "device").unwrap();
        assert_eq!(codex_stop.event_type, ai_rpa_core::EventType::TurnStopped);
        assert_eq!(codex_stop.evidence_level, EvidenceLevel::E1);
    }

    #[test]
    fn claude_permission_notification_waits_for_user_without_storing_message() {
        let raw = to_raw_event(
            Provider::Claude,
            "Notification",
            None,
            json!({
                "session_id":"s1",
                "notification_type":"permission_prompt",
                "message":"private permission detail"
            }),
        )
        .unwrap();
        assert_eq!(raw.event_type, "WaitingUser");
        assert!(
            serde_json::to_string(&raw)
                .unwrap()
                .contains("permission_prompt")
        );
        assert!(
            !serde_json::to_string(&raw)
                .unwrap()
                .contains("private permission detail")
        );
    }

    #[test]
    fn strips_prompt_transcript_and_assistant_text_before_transport_or_spool() {
        let raw = to_raw_event(
            Provider::Codex,
            "Stop",
            None,
            json!({
                "session_id":"safe-session",
                "model":"gpt-test",
                "prompt":"private prompt",
                "last_assistant_message":"private answer",
                "transcript_path":"/Users/alice/private.jsonl"
            }),
        )
        .unwrap();
        let encoded = serde_json::to_string(&raw).unwrap();
        assert!(!encoded.contains("private prompt"));
        assert!(!encoded.contains("private answer"));
        assert!(!encoded.contains("private.jsonl"));
        assert_eq!(raw.payload["model"], "gpt-test");
    }

    #[test]
    fn stop_contracts_keep_identity_without_inventing_success() {
        let fixtures = [
            (
                Provider::Codex,
                "Stop",
                json!({"session_id":"codex-session","turn_id":"turn-1"}),
                "codex-session",
            ),
            (
                Provider::Claude,
                "Stop",
                json!({"session_id":"claude-session","hook_event_name":"Stop"}),
                "claude-session",
            ),
            (
                Provider::Cursor,
                "stop",
                json!({"conversation_id":"cursor-session","status":"completed","loop_count":0}),
                "cursor-session",
            ),
            (
                Provider::Antigravity,
                "Stop",
                json!({"conversationId":"agy-session","terminationReason":"model_stop","fullyIdle":true}),
                "agy-session",
            ),
        ];
        for (provider, event, fixture, expected_session) in fixtures {
            let raw = to_raw_event(provider, event, None, fixture).unwrap();
            assert_eq!(raw.session_id, expected_session);
            if provider == Provider::Cursor {
                assert_eq!(raw.event_type, "Result");
                assert_eq!(raw.payload["structured_success"], true);
            } else {
                assert_eq!(raw.event_type, "Stop");
                assert!(raw.payload.get("structured_success").is_none());
            }
        }
    }
}
