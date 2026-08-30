use std::{collections::HashMap, sync::Arc, time::Duration};

use ai_rpa_core::{CommandAction, DeviceRecord, NodeToServerMessage, ServerToNodeMessage};
use axum::{
    Json, Router,
    body::Body,
    extract::{
        DefaultBodyLimit, Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::{SinkExt, StreamExt};
use rand::random;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx_core::query_scalar::query_scalar;
use tokio::sync::{RwLock, mpsc};
use tower_http::{
    cors::CorsLayer, services::ServeDir, set_header::SetResponseHeaderLayer, trace::TraceLayer,
};
use tracing::{error, warn};
use uuid::Uuid;

use crate::store::{CentralStore, CentralTaskFilter, token_hash};

#[derive(Clone)]
pub struct AppState {
    pub store: CentralStore,
    pub admin_token_hash: String,
    pub connections: Arc<RwLock<HashMap<String, mpsc::Sender<ServerToNodeMessage>>>>,
}

pub fn router(state: AppState, ui_dir: Option<std::path::PathBuf>) -> Router {
    let api = Router::new()
        .route("/session", get(session))
        .route("/dashboard", get(dashboard))
        .route("/devices", get(devices))
        .route("/devices/enrollment-codes", post(create_enrollment_code))
        .route("/devices/{id}", patch(rename_device))
        .route("/devices/{id}/revoke", post(revoke_device))
        .route("/tasks", get(tasks))
        .route("/tasks/{id}", get(task_detail))
        .route("/tasks/{id}/commands", post(create_command))
        .route("/tasks/{id}/open", post(remote_open_task));
    let mut app = Router::new()
        .route("/health", get(health))
        .route("/v1/devices/enroll", post(enroll))
        .route("/v1/nodes/connect", get(node_connect))
        .nest("/api", api)
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static(
                "default-src 'self'; connect-src 'self' wss:; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
            ),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("no-referrer"),
        ))
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
        .with_state(state);
    if let Some(directory) = ui_dir {
        app = app.fallback_service(ServeDir::new(directory).append_index_html_on_directories(true));
    }
    app
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "请登录中央控制台".to_owned(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        let trace_id = Uuid::new_v4();
        error!(%trace_id, %error, "central API failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("服务器暂时不可用（追踪号 {trace_id}）"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"error": {"message": self.message}})),
        )
            .into_response()
    }
}

fn authorize_admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let token = bearer(headers).ok_or_else(ApiError::unauthorized)?;
    let presented = token_hash(token);
    if presented.len() != state.admin_token_hash.len()
        || !presented
            .bytes()
            .zip(state.admin_token_hash.bytes())
            .fold(true, |same, (left, right)| same & (left == right))
    {
        return Err(ApiError::unauthorized());
    }
    Ok(())
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|value| !value.is_empty())
}

async fn health(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    query_scalar::<sqlx_postgres::Postgres, i32>("SELECT 1")
        .fetch_one(&state.store.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(
        json!({"status":"ok","mode":"CENTRAL","version":env!("CARGO_PKG_VERSION")}),
    ))
}

async fn session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    authorize_admin(&state, &headers)?;
    Ok(Json(
        json!({"mode":"CENTRAL","authenticated":true,"version":env!("CARGO_PKG_VERSION")}),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnrollRequest {
    enrollment_code: String,
    device: DeviceRecord,
    alias: Option<String>,
}

async fn enroll(
    State(state): State<AppState>,
    Json(input): Json<EnrollRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let code = input.enrollment_code.trim().to_ascii_uppercase();
    if code.len() != 10 || !code.chars().all(|value| value.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request("配对码格式不正确"));
    }
    if input.device.id.is_empty()
        || input.device.id.len() > 128
        || !input
            .device
            .id
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '-' | '.'))
        || input.device.hostname.chars().count() > 255
        || input.device.os.chars().count() > 64
        || input.device.arch.chars().count() > 64
        || input.device.logical_environment.chars().count() > 128
        || input.device.node_version.chars().count() > 64
    {
        return Err(ApiError::bad_request("设备身份字段格式不正确"));
    }
    let token = URL_SAFE_NO_PAD.encode(random::<[u8; 32]>());
    let alias = input
        .alias
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&input.device.hostname);
    if alias.chars().count() > 64 {
        return Err(ApiError::bad_request("设备名称需要 1～64 个字符"));
    }
    state
        .store
        .enroll_device(
            &token_hash(&code),
            &input.device,
            alias,
            &token_hash(&token),
        )
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "deviceId": input.device.id,
            "nodeToken": token,
            "heartbeatSeconds": 30
        })),
    ))
}

async fn dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    authorize_admin(&state, &headers)?;
    Ok(Json(
        state.store.dashboard().await.map_err(ApiError::internal)?,
    ))
}

async fn devices(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    authorize_admin(&state, &headers)?;
    Ok(Json(
        json!({"items":state.store.devices().await.map_err(ApiError::internal)?}),
    ))
}

async fn create_enrollment_code(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize_admin(&state, &headers)?;
    let code = hex::encode(random::<[u8; 5]>()).to_ascii_uppercase();
    let expires_at = Utc::now() + ChronoDuration::minutes(15);
    state
        .store
        .create_enrollment_code(&token_hash(&code), expires_at)
        .await
        .map_err(ApiError::internal)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"code":code,"expiresAt":expires_at})),
    ))
}

#[derive(Debug, Deserialize)]
struct RenameRequest {
    alias: String,
}

async fn rename_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<RenameRequest>,
) -> Result<Json<Value>, ApiError> {
    authorize_admin(&state, &headers)?;
    let alias = input.alias.trim();
    if alias.is_empty() || alias.chars().count() > 64 {
        return Err(ApiError::bad_request("设备名称需要 1～64 个字符"));
    }
    state
        .store
        .rename_device(&id, alias)
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(Json(json!({"updated":true})))
}

async fn revoke_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    authorize_admin(&state, &headers)?;
    state
        .store
        .revoke_device(&id)
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if let Some(sender) = state.connections.write().await.remove(&id) {
        let _ = sender
            .send(ServerToNodeMessage::Error {
                code: "DEVICE_REVOKED".to_owned(),
                message: "此设备已从中央控制台撤销".to_owned(),
            })
            .await;
    }
    Ok(Json(json!({"revoked":true})))
}

async fn tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<CentralTaskFilter>,
) -> Result<Json<Value>, ApiError> {
    authorize_admin(&state, &headers)?;
    Ok(Json(
        json!({"items":state.store.list_tasks(&filter).await.map_err(ApiError::internal)?}),
    ))
}

async fn task_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    authorize_admin(&state, &headers)?;
    let detail = state
        .store
        .task_detail(id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("任务不存在"))?;
    Ok(Json(
        serde_json::to_value(detail).map_err(ApiError::internal)?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateCommandRequest {
    action: CommandAction,
    message: String,
    #[serde(default = "default_ttl")]
    ttl_seconds: i64,
}

fn default_ttl() -> i64 {
    7200
}

async fn create_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<CreateCommandRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize_admin(&state, &headers)?;
    let command = state
        .store
        .create_command(
            id,
            input.action,
            &input.message,
            "central-admin",
            input.ttl_seconds,
        )
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if let Some(sender) = state
        .connections
        .read()
        .await
        .get(&command.device_id)
        .cloned()
    {
        let _ = sender
            .send(ServerToNodeMessage::Command(command.clone()))
            .await;
    }
    let mut public = serde_json::to_value(&command).map_err(ApiError::internal)?;
    if let Value::Object(object) = &mut public {
        object.remove("message");
    }
    Ok((StatusCode::CREATED, Json(json!({"command":public}))))
}

async fn remote_open_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    authorize_admin(&state, &headers)?;
    if state
        .store
        .task_detail(id)
        .await
        .map_err(ApiError::internal)?
        .is_none()
    {
        return Err(ApiError::not_found("任务不存在"));
    }
    Ok(Json(
        json!({"opened":false,"reason":"远程控制台无法替你操作目标电脑窗口，任务内容已复制"}),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeConnectQuery {
    device_id: String,
}

async fn node_connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<NodeConnectQuery>,
    upgrade: WebSocketUpgrade,
) -> Result<Response<Body>, ApiError> {
    let token = bearer(&headers).ok_or_else(ApiError::unauthorized)?;
    let valid = state
        .store
        .verify_device_token(&query.device_id, &token_hash(token))
        .await
        .map_err(ApiError::internal)?;
    if !valid {
        return Err(ApiError::unauthorized());
    }
    let device_id = query.device_id;
    Ok(upgrade
        .max_message_size(1024 * 1024)
        .max_frame_size(1024 * 1024)
        .on_upgrade(move |socket| node_socket(state, device_id, socket)))
}

async fn node_socket(state: AppState, device_id: String, socket: WebSocket) {
    let (sender, mut receiver) = mpsc::channel::<ServerToNodeMessage>(64);
    state
        .connections
        .write()
        .await
        .insert(device_id.clone(), sender.clone());
    if let Err(error) = state.store.set_connected(&device_id, true).await {
        warn!(%device_id, %error, "failed to mark device connected");
    }
    let (mut output, mut input) = socket.split();
    let _ = sender
        .send(ServerToNodeMessage::Welcome {
            heartbeat_seconds: 30,
            server_time: Utc::now(),
        })
        .await;
    if let Ok(commands) = state.store.pending_commands(&device_id).await {
        for command in commands {
            let _ = sender.send(ServerToNodeMessage::Command(command)).await;
        }
    }
    let mut ping = tokio::time::interval(Duration::from_secs(25));
    loop {
        tokio::select! {
            outbound = receiver.recv() => {
                let Some(outbound) = outbound else { break };
                match serde_json::to_string(&outbound) {
                    Ok(encoded) => {
                        if output.send(Message::Text(encoded.into())).await.is_err() { break; }
                    }
                    Err(error) => warn!(%device_id, %error, "failed to encode server message"),
                }
            }
            inbound = input.next() => {
                let Some(Ok(inbound)) = inbound else { break };
                match inbound {
                    Message::Text(text) => {
                        if let Err(error) = handle_node_message(&state, &device_id, &sender, text.as_str()).await {
                            warn!(%device_id, %error, "invalid node message");
                            let _ = sender.send(ServerToNodeMessage::Error { code:"INVALID_MESSAGE".to_owned(), message:error.to_string() }).await;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            _ = ping.tick() => {
                if sender.send(ServerToNodeMessage::Ping { server_time: Utc::now() }).await.is_err() { break; }
            }
        }
    }
    let should_mark_offline = {
        let mut connections = state.connections.write().await;
        if connections
            .get(&device_id)
            .is_some_and(|active| active.same_channel(&sender))
        {
            connections.remove(&device_id);
            true
        } else {
            false
        }
    };
    if should_mark_offline {
        let _ = state.store.set_connected(&device_id, false).await;
    }
}

async fn handle_node_message(
    state: &AppState,
    device_id: &str,
    sender: &mpsc::Sender<ServerToNodeMessage>,
    text: &str,
) -> anyhow::Result<()> {
    match serde_json::from_str::<NodeToServerMessage>(text)? {
        NodeToServerMessage::Heartbeat {
            device, adapters, ..
        } => {
            state.store.heartbeat(device_id, &device, &adapters).await?;
            for command in state.store.pending_commands(device_id).await? {
                sender.send(ServerToNodeMessage::Command(command)).await?;
            }
        }
        NodeToServerMessage::EventBatch { batch_id, events } => {
            let accepted = state.store.ingest_events(device_id, &events).await?;
            sender
                .send(ServerToNodeMessage::EventBatchAck { batch_id, accepted })
                .await?;
        }
        NodeToServerMessage::CommandAck {
            command_id,
            state: command_state,
            result_summary,
            ..
        } => {
            state
                .store
                .ack_command(
                    device_id,
                    command_id,
                    command_state,
                    result_summary.as_deref(),
                )
                .await?;
            sender
                .send(ServerToNodeMessage::CommandAckReceipt {
                    command_id,
                    state: command_state,
                })
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_parser_rejects_non_bearer_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Basic abc"),
        );
        assert!(bearer(&headers).is_none());
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer token"),
        );
        assert_eq!(bearer(&headers), Some("token"));
    }
}
