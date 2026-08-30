use std::{collections::BTreeMap, time::Duration};

use ai_rpa_core::{
    AdapterStatus, CommandAction, CommandState, Provider, RawEventInput, TaskRecord, TaskState,
    normalize_event, provider_hook_response,
};
use ai_rpa_store::TaskFilter;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use uuid::Uuid;

use crate::{AppState, diagnostics, discovery, hook_install, notify};

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/dashboard", get(dashboard))
        .route("/devices", get(devices))
        .route("/adapters", get(adapters))
        .route("/events", post(ingest_event))
        .route("/hooks/install", post(install_hooks))
        .route("/hooks/uninstall", post(uninstall_hooks))
        .route("/tasks", get(tasks))
        .route("/tasks/{id}", get(task_detail))
        .route("/tasks/{id}/open", post(open_task))
        .route("/tasks/{id}/commands", post(create_command))
        .route("/commands/{id}/ack", post(ack_command))
        .route("/diagnostics", get(diagnostics_report))
        .route("/diagnostics/export", get(export_diagnostics))
        .route("/settings/feishu", post(set_feishu))
        .route("/notifications/flush", post(flush_notifications));
    let mut app = Router::new()
        .nest("/api", api)
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin([
                    HeaderValue::from_static("http://localhost:1420"),
                    HeaderValue::from_static("http://127.0.0.1:1420"),
                    HeaderValue::from_static("tauri://localhost"),
                    HeaderValue::from_static("http://tauri.localhost"),
                    HeaderValue::from_static("https://tauri.localhost"),
                ])
                .allow_headers(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any),
        )
        .with_state(state.clone());
    if let Some(directory) = &state.ui_dir {
        app = app.fallback_service(
            ServeDir::new(directory.as_ref()).append_index_html_on_directories(true),
        );
    }
    app
}

#[derive(Debug)]
pub struct ApiError(anyhow::Error, StatusCode);

impl ApiError {
    fn bad_request(error: impl Into<anyhow::Error>) -> Self {
        Self(error.into(), StatusCode::BAD_REQUEST)
    }

    fn not_found(message: &str) -> Self {
        Self(anyhow::anyhow!(message.to_owned()), StatusCode::NOT_FOUND)
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self(error, StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self(error.into(), StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.1,
            Json(json!({"error": {"message": self.0.to_string()}})),
        )
            .into_response()
    }
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "deviceId": state.device.id,
        "uptimeSeconds": (Utc::now() - state.started_at).num_seconds().max(0)
    }))
}

async fn devices(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({"items": state.store.devices()?})))
}

async fn install_hooks() -> Result<Json<Value>, ApiError> {
    let executable = std::env::current_exe().map_err(anyhow::Error::from)?;
    let results = hook_install::install_all(&executable).map_err(ApiError::bad_request)?;
    Ok(Json(json!({"items": results})))
}

async fn uninstall_hooks() -> Result<Json<Value>, ApiError> {
    let results = hook_install::uninstall_all().map_err(ApiError::bad_request)?;
    Ok(Json(json!({"items": results})))
}

async fn adapters(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({"items": discovery::refresh(&state.store)?})))
}

async fn ingest_event(
    State(state): State<AppState>,
    Json(input): Json<RawEventInput>,
) -> Result<Json<Value>, ApiError> {
    let provider = input.provider;
    let event = match normalize_event(input, &state.device.id) {
        Ok(event) => event,
        Err(error) => {
            state
                .store
                .quarantine(Some(provider), &error.to_string(), None)?;
            return Err(ApiError::bad_request(error));
        }
    };
    let outcome = state.store.ingest_event(&event)?;
    let mut status = discovery::discover_provider(provider);
    status.last_event_at = Some(event.received_at);
    status.hook_state = "HEALTHY".to_owned();
    status.message = format!("last event: {}", event.event_type);
    state.store.upsert_adapter(&status)?;
    let hook_response = outcome
        .delivery
        .as_ref()
        .map(|delivery| provider_hook_response(provider, delivery));
    Ok(Json(json!({
        "duplicate": outcome.duplicate,
        "task": outcome.task,
        "stateChanged": outcome.state_changed,
        "hookResponse": hook_response
    })))
}

async fn tasks(
    State(state): State<AppState>,
    Query(filter): Query<TaskFilter>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({"items": state.store.list_tasks(&filter)?})))
}

async fn task_detail(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let detail = state
        .store
        .task_detail(id)?
        .ok_or_else(|| ApiError::not_found("task not found"))?;
    Ok(Json(serde_json::to_value(detail)?))
}

async fn open_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let detail = state
        .store
        .task_detail(id)?
        .ok_or_else(|| ApiError::not_found("task not found"))?;
    discovery::open_provider(detail.task.provider, detail.task.workspace.as_deref())
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({"opened": true})))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateCommandRequest {
    action: CommandAction,
    message: String,
    #[serde(default = "default_actor")]
    created_by: String,
    #[serde(default = "default_ttl")]
    ttl_seconds: i64,
}

fn default_actor() -> String {
    "local-user".to_owned()
}

fn default_ttl() -> i64 {
    7200
}

async fn create_command(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<CreateCommandRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let command = state
        .store
        .create_command(
            id,
            request.action,
            &request.message,
            &request.created_by,
            request.ttl_seconds,
        )
        .map_err(ApiError::bad_request)?;
    Ok((StatusCode::CREATED, Json(json!({"command": command}))))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AckRequest {
    state: CommandState,
    result_summary: Option<String>,
    #[serde(default = "default_actor")]
    actor: String,
}

async fn ack_command(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<AckRequest>,
) -> Result<Json<Value>, ApiError> {
    let command = state.store.update_command(
        id,
        request.state,
        request.result_summary.as_deref(),
        &request.actor,
    )?;
    Ok(Json(json!({"command": command})))
}

async fn diagnostics_report(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(diagnostics::run(&state)?)?))
}

async fn export_diagnostics(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(diagnostics::export_payload(&state)?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FeishuSettingsRequest {
    webhook: String,
}

async fn set_feishu(Json(request): Json<FeishuSettingsRequest>) -> Result<Json<Value>, ApiError> {
    notify::configure(request.webhook.trim()).map_err(ApiError::bad_request)?;
    Ok(Json(json!({"configured": true})))
}

async fn flush_notifications(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let notifier = notify::FeishuNotifier::load();
    Ok(Json(serde_json::to_value(
        notify::flush(&state.store, &notifier).await?,
    )?))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Dashboard {
    counts: BTreeMap<String, usize>,
    completion_rate_24h: f64,
    p95_duration_ms: Option<i64>,
    devices: Vec<ai_rpa_core::DeviceRecord>,
    adapters: Vec<AdapterStatus>,
    live: LiveOverview,
    attention: Vec<ai_rpa_core::TaskRecord>,
    recent: Vec<ai_rpa_core::TaskRecord>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveOverview {
    observed_at: DateTime<Utc>,
    poll_interval_ms: u64,
    connected_provider_count: usize,
    monitored_provider_count: usize,
    executing_task_count: usize,
    waiting_task_count: usize,
    providers: Vec<LiveProviderSummary>,
    tasks: Vec<LiveTaskSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveProviderSummary {
    provider: Provider,
    connection_state: String,
    tracking_state: String,
    active_task_count: usize,
    last_event_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveTaskSummary {
    #[serde(flatten)]
    task: TaskRecord,
    source: &'static str,
    stale: bool,
    age_seconds: i64,
}

const LIVE_POLL_INTERVAL_MS: u64 = 2_000;
const LIVE_EVENT_FRESHNESS_SECONDS: i64 = 300;

fn build_live_overview(
    adapters: &[AdapterStatus],
    tasks: &[TaskRecord],
    observed_at: DateTime<Utc>,
) -> LiveOverview {
    let active: Vec<_> = tasks
        .iter()
        .filter(|task| matches!(task.state, TaskState::Running | TaskState::WaitingUser))
        .collect();
    let providers = adapters
        .iter()
        .map(|adapter| {
            let active_task_count = active
                .iter()
                .filter(|task| task.provider == adapter.provider)
                .count();
            let tracking_state = if adapter.install_state != "RUNNING" {
                "OFFLINE"
            } else if !matches!(adapter.hook_state.as_str(), "CONFIGURED" | "HEALTHY") {
                "NOT_CONFIGURED"
            } else if active_task_count > 0
                || adapter.last_event_at.is_some_and(|last_event| {
                    (observed_at - last_event).num_seconds() <= LIVE_EVENT_FRESHNESS_SECONDS
                })
            {
                "LIVE"
            } else if adapter.last_event_at.is_some() {
                "STALE"
            } else {
                "READY"
            };
            LiveProviderSummary {
                provider: adapter.provider,
                connection_state: adapter.install_state.clone(),
                tracking_state: tracking_state.to_owned(),
                active_task_count,
                last_event_at: adapter.last_event_at,
            }
        })
        .collect();
    let live_tasks = active
        .iter()
        .map(|task| {
            let age_seconds = (observed_at - task.updated_at).num_seconds().max(0);
            LiveTaskSummary {
                task: (*task).clone(),
                source: "HOOK_EVENT",
                stale: age_seconds > LIVE_EVENT_FRESHNESS_SECONDS,
                age_seconds,
            }
        })
        .collect();
    LiveOverview {
        observed_at,
        poll_interval_ms: LIVE_POLL_INTERVAL_MS,
        connected_provider_count: adapters
            .iter()
            .filter(|adapter| adapter.install_state == "RUNNING")
            .count(),
        monitored_provider_count: adapters
            .iter()
            .filter(|adapter| matches!(adapter.hook_state.as_str(), "CONFIGURED" | "HEALTHY"))
            .count(),
        executing_task_count: active
            .iter()
            .filter(|task| task.state == TaskState::Running)
            .count(),
        waiting_task_count: active
            .iter()
            .filter(|task| task.state == TaskState::WaitingUser)
            .count(),
        providers,
        tasks: live_tasks,
    }
}

async fn dashboard(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let all = state.store.list_tasks(&TaskFilter {
        limit: Some(1000),
        ..TaskFilter::default()
    })?;
    let mut counts = BTreeMap::new();
    for item in &all {
        *counts.entry(item.state.to_string()).or_insert(0) += 1;
    }
    let since = Utc::now() - chrono::Duration::hours(24);
    let completed: Vec<_> = all
        .iter()
        .filter(|item| item.updated_at >= since && item.state.is_terminal())
        .collect();
    let completion_rate_24h = if completed.is_empty() {
        0.0
    } else {
        completed
            .iter()
            .filter(|item| item.state == TaskState::Succeeded)
            .count() as f64
            / completed.len() as f64
    };
    let mut durations: Vec<i64> = completed
        .iter()
        .filter_map(|item| item.duration_ms)
        .collect();
    durations.sort_unstable();
    let p95_duration_ms = (!durations.is_empty()).then(|| {
        let index = ((durations.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
        durations[index.min(durations.len() - 1)]
    });
    let attention = all
        .iter()
        .filter(|item| {
            matches!(
                item.state,
                TaskState::WaitingUser | TaskState::Failed | TaskState::Unknown
            )
        })
        .take(10)
        .cloned()
        .collect();
    let adapters = match state.store.adapters()? {
        cached if !cached.is_empty() => cached,
        _ => discovery::refresh(&state.store)?,
    };
    let live = build_live_overview(&adapters, &all, Utc::now());
    Ok(Json(serde_json::to_value(Dashboard {
        counts,
        completion_rate_24h,
        p95_duration_ms,
        devices: state.store.devices()?,
        adapters,
        live,
        attention,
        recent: all.into_iter().take(10).collect(),
    })?))
}

pub async fn background_tasks(state: AppState) {
    let notifier = notify::FeishuNotifier::load();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
    let mut notifications = tokio::time::interval(Duration::from_secs(10));
    let mut managed = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                let mut device = state.device.clone();
                device.last_seen_at = Utc::now();
                if let Err(error) = state.store.upsert_device(&device) {
                    tracing::warn!(%error, "failed to persist heartbeat");
                }
                if let Err(error) = discovery::refresh(&state.store) {
                    tracing::warn!(%error, "failed to refresh adapters");
                }
                let spool = state.data_dir.join("spool");
                if let Err(error) = crate::hook::drain_spool(&spool, "http://127.0.0.1:3847").await {
                    tracing::warn!(%error, "failed to drain offline hook spool");
                }
            }
            _ = notifications.tick() => {
                if let Err(error) = notify::flush(&state.store, &notifier).await {
                    tracing::warn!(%error, "failed to flush notification outbox");
                }
            }
            _ = managed.tick() => {
                if let Err(error) = crate::command_runner::run_once(&state).await {
                    tracing::warn!(%error, "managed command runner failed");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_rpa_core::{Capability, ControlMode, EventType, EvidenceLevel};
    use ai_rpa_store::{CryptoBox, Store};
    use axum::{body::Body, http::Request};
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;
    use tower::ServiceExt;

    fn test_state() -> (TempDir, AppState) {
        let directory = TempDir::new().unwrap();
        let store = Store::open(
            directory.path().join("test.db"),
            CryptoBox::from_key([3_u8; 32]),
        )
        .unwrap();
        let device = ai_rpa_core::DeviceRecord {
            id: "device-test".to_owned(),
            os: "test".to_owned(),
            arch: "test".to_owned(),
            hostname: "test".to_owned(),
            logical_environment: "test".to_owned(),
            node_version: "0.1.0".to_owned(),
            last_seen_at: Utc::now(),
        };
        store.upsert_device(&device).unwrap();
        (
            directory,
            AppState {
                store,
                device,
                started_at: Utc::now(),
                data_dir: std::sync::Arc::new(std::path::PathBuf::from(".")),
                ui_dir: None,
            },
        )
    }

    #[tokio::test]
    async fn health_endpoint_is_available() {
        let (_directory, state) = test_state();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn live_overview_counts_connected_tools_and_active_tasks() {
        let now = Utc.with_ymd_and_hms(2026, 8, 30, 12, 0, 0).unwrap();
        let adapters = [
            AdapterStatus {
                provider: Provider::Codex,
                install_state: "RUNNING".to_owned(),
                executable: None,
                version: None,
                hook_state: "HEALTHY".to_owned(),
                last_event_at: Some(now - chrono::Duration::seconds(20)),
                capabilities: vec![Capability::SendNext],
                message: "healthy".to_owned(),
            },
            AdapterStatus {
                provider: Provider::Claude,
                install_state: "RUNNING".to_owned(),
                executable: None,
                version: None,
                hook_state: "CONFIGURED".to_owned(),
                last_event_at: None,
                capabilities: vec![Capability::SendNext],
                message: "ready".to_owned(),
            },
            AdapterStatus {
                provider: Provider::Cursor,
                install_state: "INSTALLED_NOT_RUNNING".to_owned(),
                executable: None,
                version: None,
                hook_state: "CONFIGURED".to_owned(),
                last_event_at: None,
                capabilities: vec![Capability::SendNext],
                message: "offline".to_owned(),
            },
        ];
        let task = |provider, state, title: &str, age| TaskRecord {
            id: Uuid::new_v4(),
            provider,
            device_id: "device-test".to_owned(),
            session_id: Uuid::new_v4().to_string(),
            title: title.to_owned(),
            workspace: None,
            project: None,
            control_mode: ControlMode::Observed,
            capabilities: vec![Capability::SendNext],
            state,
            confidence: "MEDIUM".to_owned(),
            required_evidence_level: EvidenceLevel::E2,
            evidence_level: EvidenceLevel::E1,
            evidence_summary: None,
            started_at: Some(now - chrono::Duration::minutes(10)),
            updated_at: now - chrono::Duration::seconds(age),
            duration_ms: None,
            last_event_type: EventType::TurnStarted,
            state_version: 1,
        };
        let tasks = [
            task(Provider::Codex, TaskState::Running, "编码任务", 30),
            task(Provider::Claude, TaskState::WaitingUser, "等待批准", 600),
            task(Provider::Cursor, TaskState::Succeeded, "已完成", 10),
        ];

        let overview = build_live_overview(&adapters, &tasks, now);

        assert_eq!(overview.connected_provider_count, 2);
        assert_eq!(overview.monitored_provider_count, 3);
        assert_eq!(overview.executing_task_count, 1);
        assert_eq!(overview.waiting_task_count, 1);
        assert_eq!(overview.tasks.len(), 2);
        assert!(!overview.tasks[0].stale);
        assert!(overview.tasks[1].stale);
        assert_eq!(overview.providers[0].tracking_state, "LIVE");
        assert_eq!(overview.providers[1].tracking_state, "LIVE");
        assert_eq!(overview.providers[2].tracking_state, "OFFLINE");
    }

    #[tokio::test]
    async fn invalid_event_is_quarantined() {
        let (_directory, state) = test_state();
        let body = json!({
            "provider": "CODEX",
            "eventType": "new_unknown_event",
            "sessionId": "session"
        });
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/events")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(state.store.counts().unwrap()["quarantine"], 1);
    }
}
