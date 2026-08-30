use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{EventType, EvidenceLevel, TaskState, UnifiedEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedState {
    pub state: TaskState,
    pub confidence: String,
    pub evidence_level: EvidenceLevel,
    pub evidence_summary: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub duration_ms: Option<i64>,
    pub last_event_type: EventType,
}

pub fn derive_state(events: &[UnifiedEvent], required: EvidenceLevel) -> DerivedState {
    let mut ordered: Vec<&UnifiedEvent> = events.iter().collect();
    ordered.sort_by_key(|event| (event.occurred_at, event.received_at, event.event_id));
    let now = Utc::now();
    let mut state = TaskState::Unknown;
    let mut started_at = None;
    let mut evidence_level = EvidenceLevel::E0;
    let mut evidence_summary = None;
    let mut last_event_type = EventType::Unknown;
    let mut updated_at = now;

    for event in &ordered {
        evidence_level = evidence_level.max(event.evidence_level);
        if event.evidence_summary.is_some() {
            evidence_summary.clone_from(&event.evidence_summary);
        }
        last_event_type = event.event_type;
        updated_at = event.occurred_at;
        match event.event_type {
            EventType::SessionStarted | EventType::TurnStarted => {
                state = TaskState::Running;
                started_at.get_or_insert(event.occurred_at);
            }
            EventType::WaitingUser => state = TaskState::WaitingUser,
            EventType::Result => {
                state = if event.evidence_level >= required.max(EvidenceLevel::E2) {
                    TaskState::Succeeded
                } else {
                    TaskState::Unknown
                };
            }
            EventType::TurnStopped | EventType::SessionEnded => {
                if state != TaskState::Succeeded {
                    state = TaskState::Unknown;
                }
            }
            EventType::Failed => state = TaskState::Failed,
            EventType::Cancelled => state = TaskState::Cancelled,
            EventType::Heartbeat | EventType::Unknown => {}
        }
    }

    // Explicit failure and cancellation evidence is never hidden by a later optimistic result.
    if ordered
        .iter()
        .any(|event| event.event_type == EventType::Failed || non_zero_exit(event))
    {
        state = TaskState::Failed;
    }
    if ordered
        .iter()
        .any(|event| event.event_type == EventType::Cancelled)
    {
        state = TaskState::Cancelled;
    }

    let confidence = match state {
        TaskState::Succeeded if evidence_level >= EvidenceLevel::E3 => "HIGH",
        TaskState::Succeeded | TaskState::Failed | TaskState::Cancelled => "MEDIUM",
        TaskState::Running | TaskState::WaitingUser => "MEDIUM",
        TaskState::Unknown => "LOW",
    }
    .to_owned();
    let duration_ms = started_at.map(|started| (updated_at - started).num_milliseconds().max(0));

    DerivedState {
        state,
        confidence,
        evidence_level,
        evidence_summary,
        started_at,
        updated_at,
        duration_ms,
        last_event_type,
    }
}

fn non_zero_exit(event: &&UnifiedEvent) -> bool {
    event
        .attributes
        .get("exit_code")
        .and_then(serde_json::Value::as_i64)
        .is_some_and(|value| value != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ControlMode, Provider, RawEventInput, normalize_event};
    use chrono::{Duration, TimeZone};
    use serde_json::json;

    fn event(kind: &str, offset: i64, payload: serde_json::Value) -> UnifiedEvent {
        normalize_event(
            RawEventInput {
                provider: Provider::Codex,
                event_type: kind.to_owned(),
                event_id: None,
                idempotency_key: None,
                device_id: None,
                session_id: "session".to_owned(),
                turn_id: Some("turn".to_owned()),
                occurred_at: Some(
                    Utc.with_ymd_and_hms(2026, 8, 29, 10, 0, 0).unwrap()
                        + Duration::seconds(offset),
                ),
                title: None,
                workspace: None,
                project: None,
                control_mode: ControlMode::Managed,
                required_evidence_level: EvidenceLevel::E2,
                payload,
            },
            "device",
        )
        .unwrap()
    }

    #[test]
    fn stop_without_result_is_unknown() {
        let events = vec![event("start", 0, json!({})), event("stop", 10, json!({}))];
        assert_eq!(
            derive_state(&events, EvidenceLevel::E2).state,
            TaskState::Unknown
        );
    }

    #[test]
    fn result_requires_configured_evidence() {
        let events = vec![
            event("start", 0, json!({})),
            event("result", 10, json!({"structured_success": true})),
        ];
        assert_eq!(
            derive_state(&events, EvidenceLevel::E2).state,
            TaskState::Succeeded
        );
        assert_eq!(
            derive_state(&events, EvidenceLevel::E3).state,
            TaskState::Unknown
        );
    }

    #[test]
    fn late_error_corrects_success() {
        let events = vec![
            event("start", 0, json!({})),
            event("result", 20, json!({"tests_passed": true})),
            event("tool_error", 10, json!({"exit_code": 1})),
        ];
        assert_eq!(
            derive_state(&events, EvidenceLevel::E3).state,
            TaskState::Failed
        );
    }

    #[test]
    fn waiting_can_return_to_running() {
        let events = vec![
            event("permission_request", 0, json!({})),
            event("turn_started", 10, json!({})),
        ];
        assert_eq!(
            derive_state(&events, EvidenceLevel::E2).state,
            TaskState::Running
        );
    }
}
