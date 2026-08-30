use std::{collections::BTreeMap, fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Provider {
    Codex,
    Claude,
    Cursor,
    Antigravity,
}

impl Provider {
    pub const ALL: [Self; 4] = [Self::Codex, Self::Claude, Self::Cursor, Self::Antigravity];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::Antigravity => "antigravity",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Provider {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude" | "claude-code" => Ok(Self::Claude),
            "cursor" | "cursor-ide" => Ok(Self::Cursor),
            "antigravity" | "agy" | "antigravity-ide" => Ok(Self::Antigravity),
            other => Err(format!("unsupported provider: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskState {
    Running,
    WaitingUser,
    Failed,
    Succeeded,
    Cancelled,
    Unknown,
}

impl TaskState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Failed | Self::Succeeded | Self::Cancelled)
    }
}

impl fmt::Display for TaskState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = serde_json::to_value(self).expect("task state serializes");
        f.write_str(value.as_str().expect("task state is a string"))
    }
}

impl FromStr for TaskState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(Value::String(value.to_ascii_uppercase()))
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ControlMode {
    #[default]
    Observed,
    Managed,
}

impl fmt::Display for ControlMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Observed => f.write_str("OBSERVED"),
            Self::Managed => f.write_str("MANAGED"),
        }
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceLevel {
    #[default]
    E0,
    E1,
    E2,
    E3,
}

impl fmt::Display for EvidenceLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::E0 => f.write_str("E0"),
            Self::E1 => f.write_str("E1"),
            Self::E2 => f.write_str("E2"),
            Self::E3 => f.write_str("E3"),
        }
    }
}

impl FromStr for EvidenceLevel {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_uppercase().as_str() {
            "E0" => Ok(Self::E0),
            "E1" => Ok(Self::E1),
            "E2" => Ok(Self::E2),
            "E3" => Ok(Self::E3),
            other => Err(format!("invalid evidence level: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventType {
    SessionStarted,
    TurnStarted,
    TurnStopped,
    Result,
    Failed,
    WaitingUser,
    Cancelled,
    SessionEnded,
    Heartbeat,
    Unknown,
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = serde_json::to_value(self).expect("event type serializes");
        f.write_str(value.as_str().expect("event type is a string"))
    }
}

impl FromStr for EventType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(Value::String(value.to_ascii_uppercase()))
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Capability {
    SendNext,
    ResumeAndSend,
    SteerActive,
    OpenAndPrefill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CommandAction {
    SendNext,
    ResumeAndSend,
    OpenAndPrefill,
}

impl fmt::Display for CommandAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = serde_json::to_value(self).expect("command action serializes");
        f.write_str(value.as_str().expect("command action is a string"))
    }
}

impl FromStr for CommandAction {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(Value::String(value.to_ascii_uppercase()))
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CommandState {
    Created,
    Queued,
    Leased,
    Delivered,
    Accepted,
    Completed,
    RetryWait,
    UnknownDelivery,
    Expired,
    Failed,
    Cancelled,
}

impl fmt::Display for CommandState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = serde_json::to_value(self).expect("command state serializes");
        f.write_str(value.as_str().expect("command state is a string"))
    }
}

impl FromStr for CommandState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(Value::String(value.to_ascii_uppercase()))
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawEventInput {
    pub provider: Provider,
    pub event_type: String,
    #[serde(default)]
    pub event_id: Option<Uuid>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    pub session_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub occurred_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub control_mode: ControlMode,
    #[serde(default)]
    pub required_evidence_level: EvidenceLevel,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedEvent {
    pub schema_version: u32,
    pub event_id: Uuid,
    pub idempotency_key: String,
    pub provider: Provider,
    pub device_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub occurred_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub event_type: EventType,
    pub control_mode: ControlMode,
    pub capabilities: Vec<Capability>,
    pub evidence_level: EvidenceLevel,
    pub evidence_summary: Option<String>,
    pub title: String,
    pub workspace: Option<String>,
    pub project: Option<String>,
    pub required_evidence_level: EvidenceLevel,
    pub attributes: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: Uuid,
    pub provider: Provider,
    pub device_id: String,
    pub session_id: String,
    pub title: String,
    pub workspace: Option<String>,
    pub project: Option<String>,
    pub control_mode: ControlMode,
    pub capabilities: Vec<Capability>,
    pub state: TaskState,
    pub confidence: String,
    pub required_evidence_level: EvidenceLevel,
    pub evidence_level: EvidenceLevel,
    pub evidence_summary: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub duration_ms: Option<i64>,
    pub last_event_type: EventType,
    pub state_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRecord {
    pub id: Uuid,
    pub task_id: Uuid,
    pub provider: Provider,
    pub device_id: String,
    pub session_id: String,
    pub action: CommandAction,
    pub state: CommandState,
    pub message: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub lease_owner: Option<String>,
    pub lease_until: Option<DateTime<Utc>>,
    pub attempts: u32,
    pub max_attempts: u32,
    pub result_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandDelivery {
    pub command_id: Uuid,
    pub action: CommandAction,
    pub message: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRecord {
    pub id: String,
    pub os: String,
    pub arch: String,
    pub hostname: String,
    pub logical_environment: String,
    pub node_version: String,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterStatus {
    pub provider: Provider,
    pub install_state: String,
    pub executable: Option<String>,
    pub version: Option<String>,
    pub hook_state: String,
    pub last_event_at: Option<DateTime<Utc>>,
    pub capabilities: Vec<Capability>,
    pub message: String,
}
