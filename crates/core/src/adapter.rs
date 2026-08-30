use std::{collections::BTreeMap, str::FromStr};

use chrono::Utc;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    Capability, CommandDelivery, ControlMode, EventType, EvidenceLevel, Provider, RawEventInput,
    UnifiedEvent, redact_text,
};

const MAX_SESSION_ID: usize = 512;

#[derive(Debug, Error)]
pub enum NormalizeError {
    #[error("sessionId is required")]
    MissingSession,
    #[error("sessionId is too long")]
    SessionTooLong,
    #[error("unsupported or unknown event type: {0}")]
    UnknownEventType(String),
}

pub fn provider_capabilities(provider: Provider, mode: ControlMode) -> Vec<Capability> {
    let mut capabilities = vec![Capability::SendNext, Capability::OpenAndPrefill];
    if mode == ControlMode::Managed
        && matches!(
            provider,
            Provider::Codex | Provider::Claude | Provider::Antigravity
        )
    {
        capabilities.push(Capability::ResumeAndSend);
    }
    capabilities
}

pub fn normalize_event(
    input: RawEventInput,
    default_device_id: &str,
) -> Result<UnifiedEvent, NormalizeError> {
    let session_id = input.session_id.trim().to_owned();
    if session_id.is_empty() {
        return Err(NormalizeError::MissingSession);
    }
    if session_id.len() > MAX_SESSION_ID {
        return Err(NormalizeError::SessionTooLong);
    }

    let event_type = map_event_type(input.provider, &input.event_type);
    if event_type == EventType::Unknown {
        return Err(NormalizeError::UnknownEventType(input.event_type));
    }

    let occurred_at = input.occurred_at.unwrap_or_else(Utc::now);
    let received_at = Utc::now();
    let device_id = input
        .device_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default_device_id.to_owned());
    let evidence_level = infer_evidence_level(event_type, &input.payload);
    let evidence_summary = safe_summary(event_type, evidence_level, &input.payload);
    let attributes = safe_attributes(&input.payload);
    let title = input
        .title
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback_title(input.provider, input.workspace.as_deref()));
    let event_id = input.event_id.unwrap_or_else(Uuid::new_v4);
    let idempotency_key = input.idempotency_key.unwrap_or_else(|| {
        let mut hasher = Sha256::new();
        hasher.update(input.provider.as_str());
        hasher.update(b"|");
        hasher.update(&device_id);
        hasher.update(b"|");
        hasher.update(&session_id);
        hasher.update(b"|");
        hasher.update(input.turn_id.as_deref().unwrap_or(""));
        hasher.update(b"|");
        hasher.update(occurred_at.to_rfc3339());
        hasher.update(b"|");
        hasher.update(event_type.to_string());
        hex::encode(hasher.finalize())
    });

    Ok(UnifiedEvent {
        schema_version: 1,
        event_id,
        idempotency_key,
        provider: input.provider,
        device_id,
        session_id,
        turn_id: input.turn_id,
        occurred_at,
        received_at,
        event_type,
        control_mode: input.control_mode,
        capabilities: provider_capabilities(input.provider, input.control_mode),
        evidence_level,
        evidence_summary,
        title: redact_text(&title).text,
        workspace: input.workspace.map(|value| redact_text(&value).text),
        project: input.project.map(|value| redact_text(&value).text),
        required_evidence_level: input.required_evidence_level,
        attributes,
    })
}

fn fallback_title(provider: Provider, workspace: Option<&str>) -> String {
    let workspace_name = workspace
        .map(str::trim)
        .map(|value| value.trim_end_matches(['/', '\\']))
        .filter(|value| !value.is_empty())
        .and_then(|value| value.rsplit(['/', '\\']).next())
        .filter(|value| !value.is_empty());
    workspace_name.map_or_else(
        || format!("{} task", provider),
        |name| format!("{} · {name}", provider),
    )
}

pub fn map_event_type(provider: Provider, raw: &str) -> EventType {
    let normalized = raw
        .trim()
        .to_ascii_lowercase()
        .replace(['-', '.', ' '], "_");

    if normalized.parse::<EventType>().is_ok() {
        return normalized.parse().unwrap_or(EventType::Unknown);
    }
    if normalized.contains("cancel") || normalized.contains("interrupt") {
        return EventType::Cancelled;
    }
    if normalized == "waitinguser"
        || normalized.contains("permission")
        || normalized.contains("approval")
        || normalized.contains("login")
        || normalized.contains("wait_user")
        || normalized.contains("input_required")
    {
        return EventType::WaitingUser;
    }
    if normalized.contains("failure")
        || normalized.contains("failed")
        || normalized.contains("error")
    {
        return EventType::Failed;
    }
    if normalized.contains("session_end") || normalized == "sessionend" {
        return EventType::SessionEnded;
    }
    if normalized.contains("session_start") || normalized == "sessionstart" {
        return EventType::SessionStarted;
    }
    if normalized == "preinvocation" || normalized == "pre_invocation" {
        return EventType::TurnStarted;
    }
    if normalized == "postinvocation" || normalized == "post_invocation" {
        return EventType::Heartbeat;
    }
    if normalized.contains("task_completed")
        || normalized.contains("taskcompleted")
        || normalized.contains("result")
        || normalized == "success"
    {
        return EventType::Result;
    }
    if normalized.contains("stop")
        || normalized.contains("idle")
        || (provider == Provider::Antigravity && normalized.contains("fully_idle"))
    {
        return EventType::TurnStopped;
    }
    if normalized.contains("start")
        || normalized.contains("submit_prompt")
        || normalized.contains("before_agent")
    {
        return EventType::TurnStarted;
    }
    if normalized.contains("heartbeat") {
        return EventType::Heartbeat;
    }
    EventType::Unknown
}

fn infer_evidence_level(event_type: EventType, payload: &Value) -> EvidenceLevel {
    if let Some(level) = payload
        .get("evidence_level")
        .or_else(|| payload.get("evidenceLevel"))
        .and_then(Value::as_str)
        .and_then(|value| EvidenceLevel::from_str(value).ok())
    {
        return level;
    }
    if payload.get("tests_passed").and_then(Value::as_bool) == Some(true)
        || payload
            .get("artifact_hash")
            .and_then(Value::as_str)
            .is_some()
    {
        return EvidenceLevel::E3;
    }
    if event_type == EventType::Result
        || payload.get("exit_code").and_then(Value::as_i64) == Some(0)
        || payload.get("structured_success").and_then(Value::as_bool) == Some(true)
    {
        return EvidenceLevel::E2;
    }
    if matches!(event_type, EventType::TurnStopped | EventType::SessionEnded) {
        return EvidenceLevel::E1;
    }
    EvidenceLevel::E0
}

fn safe_summary(
    event_type: EventType,
    evidence_level: EvidenceLevel,
    payload: &Value,
) -> Option<String> {
    let candidate = payload
        .get("summary")
        .or_else(|| payload.get("reason"))
        .or_else(|| payload.get("termination_reason"))
        .and_then(Value::as_str)
        .map(|value| redact_text(value).text);
    candidate.or_else(|| {
        Some(format!(
            "{} event with {} evidence",
            event_type, evidence_level
        ))
    })
}

fn safe_attributes(payload: &Value) -> BTreeMap<String, Value> {
    const SAFE_KEYS: &[&str] = &[
        "exit_code",
        "termination_reason",
        "tests_passed",
        "artifact_hash",
        "stop_hook_active",
        "fully_idle",
        "model",
        "error_code",
        "structured_success",
    ];
    let mut output = BTreeMap::new();
    for key in SAFE_KEYS {
        if let Some(value) = payload.get(*key) {
            let safe = match value {
                Value::String(text) => Value::String(redact_text(text).text),
                Value::Bool(_) | Value::Number(_) | Value::Null => value.clone(),
                _ => continue,
            };
            output.insert((*key).to_owned(), safe);
        }
    }
    output
}

pub fn provider_hook_response(provider: Provider, delivery: &CommandDelivery) -> Value {
    match provider {
        Provider::Cursor => json!({
            "followup_message": delivery.message,
        }),
        Provider::Claude => json!({
            "decision": "block",
            "reason": delivery.message,
        }),
        Provider::Antigravity => json!({
            "decision": "continue",
            "reason": delivery.message,
        }),
        Provider::Codex => json!({
            "decision": "block",
            "reason": delivery.message,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommandAction, RawEventInput};
    use chrono::TimeZone;

    fn input(provider: Provider, event_type: &str) -> RawEventInput {
        RawEventInput {
            provider,
            event_type: event_type.to_owned(),
            event_id: None,
            idempotency_key: None,
            device_id: None,
            session_id: "session-1".to_owned(),
            turn_id: Some("turn-1".to_owned()),
            occurred_at: Some(Utc.with_ymd_and_hms(2026, 8, 29, 12, 0, 0).unwrap()),
            title: Some("Model probe".to_owned()),
            workspace: None,
            project: None,
            control_mode: ControlMode::Managed,
            required_evidence_level: EvidenceLevel::E2,
            payload: Value::Null,
        }
    }

    #[test]
    fn maps_all_provider_stop_events() {
        for (provider, raw) in [
            (Provider::Codex, "turn.stop"),
            (Provider::Claude, "Stop"),
            (Provider::Cursor, "stop"),
            (Provider::Antigravity, "fullyIdle"),
        ] {
            assert_eq!(map_event_type(provider, raw), EventType::TurnStopped);
        }
    }

    #[test]
    fn maps_hook_waiting_user_camel_case() {
        assert_eq!(
            map_event_type(Provider::Claude, "WaitingUser"),
            EventType::WaitingUser
        );
    }

    #[test]
    fn stop_alone_is_only_e1() {
        let event = normalize_event(input(Provider::Codex, "stop"), "device-1").unwrap();
        assert_eq!(event.evidence_level, EvidenceLevel::E1);
    }

    #[test]
    fn continuation_shapes_match_provider_contracts() {
        let delivery = CommandDelivery {
            command_id: Uuid::new_v4(),
            action: CommandAction::SendNext,
            message: "continue safely".to_owned(),
            expires_at: Utc::now(),
        };
        assert_eq!(
            provider_hook_response(Provider::Cursor, &delivery)["followup_message"],
            "continue safely"
        );
        for provider in [Provider::Codex, Provider::Claude] {
            let response = provider_hook_response(provider, &delivery);
            assert_eq!(response["decision"], "block");
            assert_eq!(response["reason"], "continue safely");
        }
        let antigravity = provider_hook_response(Provider::Antigravity, &delivery);
        assert_eq!(antigravity["decision"], "continue");
        assert_eq!(antigravity["reason"], "continue safely");
    }

    #[test]
    fn ignores_prompt_and_secret_payload_fields() {
        let mut raw = input(Provider::Claude, "TaskCompleted");
        raw.payload = json!({
            "prompt": "private source",
            "api_key": "sk-secret",
            "summary": "done",
            "structured_success": true
        });
        let event = normalize_event(raw, "device-1").unwrap();
        assert!(!event.attributes.contains_key("prompt"));
        assert!(!event.attributes.contains_key("api_key"));
        assert_eq!(event.evidence_level, EvidenceLevel::E2);
    }

    #[test]
    fn cursor_response_uses_followup_message() {
        let delivery = CommandDelivery {
            command_id: Uuid::nil(),
            action: CommandAction::SendNext,
            message: "next".to_owned(),
            expires_at: Utc::now(),
        };
        assert_eq!(
            provider_hook_response(Provider::Cursor, &delivery)["followup_message"],
            "next"
        );
    }

    #[test]
    fn unnamed_task_uses_workspace_name_without_using_prompt_content() {
        let mut raw = input(Provider::Cursor, "session_start");
        raw.title = None;
        raw.workspace = Some(r"C:\work\ai-console".to_owned());
        raw.payload = json!({"prompt":"private prompt"});

        let event = normalize_event(raw, "device-1").unwrap();

        assert_eq!(event.title, "cursor · ai-console");
        assert!(
            !serde_json::to_string(&event)
                .unwrap()
                .contains("private prompt")
        );
    }
}
