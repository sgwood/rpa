use std::{path::Path, process::Stdio, time::Duration};

use ai_rpa_core::{
    CommandRecord, CommandState, ControlMode, Provider, RawEventInput, normalize_event,
};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde_json::json;
use tokio::{io::AsyncWriteExt, process::Command, time::timeout};

use crate::{AppState, discovery};

pub async fn run_once(state: &AppState) -> Result<bool> {
    let owner = format!("{}:{}", state.device.id, std::process::id());
    let Some(command) = state
        .store
        .lease_managed_command(&state.device.id, &owner, 120)?
    else {
        return Ok(false);
    };
    let result = async {
        let message = state.store.command_message(command.id)?;
        let executable = discovery::discover_provider(command.provider)
            .executable
            .context("provider executable is not available")?;
        execute(
            command.provider,
            Path::new(&executable),
            &command.session_id,
            &message,
        )
        .await
    }
    .await;
    match result {
        Ok(summary) => {
            state.store.update_command(
                command.id,
                CommandState::Completed,
                Some(&summary),
                "managed-runner",
            )?;
            record_task_outcome(state, &command, true)?;
        }
        Err(error) => {
            let next_state = if command.attempts >= command.max_attempts {
                CommandState::Failed
            } else {
                CommandState::RetryWait
            };
            state.store.update_command(
                command.id,
                next_state,
                Some(&error.to_string()),
                "managed-runner",
            )?;
            if next_state == CommandState::Failed {
                record_task_outcome(state, &command, false)?;
            }
        }
    }
    Ok(true)
}

fn record_task_outcome(state: &AppState, command: &CommandRecord, success: bool) -> Result<()> {
    let detail = state
        .store
        .task_detail(command.task_id)?
        .context("managed command task no longer exists")?;
    let event = normalize_event(
        RawEventInput {
            provider: command.provider,
            event_type: if success { "Result" } else { "Failed" }.to_owned(),
            event_id: None,
            idempotency_key: Some(format!(
                "managed-command:{}:{}",
                command.id,
                if success { "result" } else { "failed" }
            )),
            device_id: Some(command.device_id.clone()),
            session_id: command.session_id.clone(),
            turn_id: None,
            occurred_at: Some(Utc::now()),
            title: Some(detail.task.title),
            workspace: detail.task.workspace,
            project: detail.task.project,
            control_mode: ControlMode::Managed,
            required_evidence_level: detail.task.required_evidence_level,
            payload: if success {
                json!({"exit_code": 0, "structured_success": true})
            } else {
                json!({"error_code": "managed_command_failed"})
            },
        },
        &state.device.id,
    )?;
    state.store.ingest_event(&event)?;
    Ok(())
}

async fn execute(
    provider: Provider,
    executable: &Path,
    session_id: &str,
    message: &str,
) -> Result<String> {
    let mut command = Command::new(executable);
    let stdin_payload = match provider {
        Provider::Codex => {
            command.args(["exec", "resume", session_id, "-", "--json"]);
            message.to_owned()
        }
        Provider::Claude => {
            command.args(["--print", "--resume", session_id, "--output-format", "json"]);
            message.to_owned()
        }
        Provider::Antigravity => {
            command.args([
                "--conversation",
                session_id,
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
            ]);
            format!(
                "{}\n",
                serde_json::json!({
                    "event": "user",
                    "message": { "content": message }
                })
            )
        }
        Provider::Cursor => bail!("Cursor observed sessions do not support managed resume in P0"),
    };
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().context("start managed provider command")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(stdin_payload.as_bytes()).await?;
        stdin.shutdown().await?;
    }
    let output = timeout(Duration::from_secs(30 * 60), child.wait_with_output())
        .await
        .context("managed provider command timed out")??;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        bail!(
            "provider exited with {}; inspect its local logs",
            output.status
        );
    }
    let _ = stdout;
    Ok("provider command completed successfully".to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ai_rpa_core::{
        CommandAction, DeviceRecord, EvidenceLevel, RawEventInput, TaskState, normalize_event,
    };
    use ai_rpa_store::{CryptoBox, Store};
    use chrono::Utc;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn managed_success_creates_idempotent_e2_result() {
        let directory = TempDir::new().unwrap();
        let store = Store::open(
            directory.path().join("test.db"),
            CryptoBox::from_key([3_u8; 32]),
        )
        .unwrap();
        let device = DeviceRecord {
            id: "device-1".to_owned(),
            os: "test".to_owned(),
            arch: "test".to_owned(),
            hostname: "test-device".to_owned(),
            logical_environment: "test".to_owned(),
            node_version: "test".to_owned(),
            last_seen_at: Utc::now(),
        };
        store.upsert_device(&device).unwrap();
        let state = AppState {
            store,
            device,
            started_at: Utc::now(),
            data_dir: Arc::new(directory.path().to_path_buf()),
            ui_dir: None,
        };
        let start = normalize_event(
            RawEventInput {
                provider: Provider::Codex,
                event_type: "SessionStart".to_owned(),
                event_id: None,
                idempotency_key: Some("start".to_owned()),
                device_id: Some("device-1".to_owned()),
                session_id: "session-1".to_owned(),
                turn_id: None,
                occurred_at: None,
                title: Some("managed task".to_owned()),
                workspace: None,
                project: None,
                control_mode: ControlMode::Managed,
                required_evidence_level: EvidenceLevel::E2,
                payload: json!({}),
            },
            "device-1",
        )
        .unwrap();
        let task = state.store.ingest_event(&start).unwrap().task;
        let command = state
            .store
            .create_command(
                task.id,
                CommandAction::ResumeAndSend,
                "continue",
                "test",
                3600,
            )
            .unwrap();

        record_task_outcome(&state, &command, true).unwrap();
        record_task_outcome(&state, &command, true).unwrap();

        let detail = state.store.task_detail(task.id).unwrap().unwrap();
        assert_eq!(detail.task.state, TaskState::Succeeded);
        assert_eq!(detail.task.evidence_level, EvidenceLevel::E2);
        assert_eq!(detail.events.len(), 2);
    }
}
