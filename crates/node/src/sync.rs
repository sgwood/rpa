use std::{collections::HashMap, time::Duration};

use ai_rpa_core::{CommandState, NodeToServerMessage, ServerToNodeMessage};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use rand::random;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::HeaderValue},
};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{AppState, config, discovery};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnrollRequest<'a> {
    enrollment_code: &'a str,
    device: &'a ai_rpa_core::DeviceRecord,
    alias: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnrollResponse {
    node_token: String,
}

pub async fn enroll(state: &AppState, server_url: &str, code: &str, alias: &str) -> Result<()> {
    let server_url = server_url.trim().trim_end_matches('/');
    if !(server_url.starts_with("https://")
        || cfg!(debug_assertions) && server_url.starts_with("http://"))
    {
        bail!("central server must use HTTPS outside development builds");
    }
    let response = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()?
        .post(format!("{server_url}/v1/devices/enroll"))
        .json(&EnrollRequest {
            enrollment_code: code,
            device: &state.device,
            alias,
        })
        .send()
        .await
        .context("connect to central enrollment API")?;
    if !response.status().is_success() {
        let message = response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|value| {
                value
                    .pointer("/error/message")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "central enrollment was rejected".to_owned());
        bail!(message);
    }
    let body: EnrollResponse = response
        .json()
        .await
        .context("decode enrollment response")?;
    config::save_central_connection(
        &state.store,
        &state.device,
        server_url,
        alias,
        &body.node_token,
    )?;
    Ok(())
}

pub async fn run(state: AppState) {
    let mut backoff = 1_u64;
    loop {
        match config::load_central_connection(&state.store, &state.device) {
            Ok(Some(connection)) if !connection.server_url.is_empty() => {
                match connect_once(&state, &connection, &mut backoff).await {
                    Ok(()) => backoff = 1,
                    Err(error) => {
                        warn!(%error, delay_seconds=backoff, "central sync disconnected");
                        let jitter = Duration::from_millis(u64::from(random::<u16>() % 1000));
                        tokio::time::sleep(Duration::from_secs(backoff) + jitter).await;
                        backoff = (backoff * 2).min(60);
                    }
                }
            }
            Ok(_) => {
                backoff = 1;
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Err(error) => {
                warn!(%error, "failed to load central connection");
                tokio::time::sleep(Duration::from_secs(15)).await;
            }
        }
    }
}

async fn connect_once(
    state: &AppState,
    connection: &config::CentralConnection,
    backoff: &mut u64,
) -> Result<()> {
    let ws_base = if let Some(rest) = connection.server_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = connection.server_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        bail!("central server URL has an unsupported scheme");
    };
    let url = format!(
        "{}/v1/nodes/connect?deviceId={}",
        ws_base.trim_end_matches('/'),
        state.device.id
    );
    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {}", connection.node_token))?,
    );
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(15), connect_async(request))
        .await
        .context("central WSS connection timed out")?
        .context("open central WSS")?;
    *backoff = 1;
    info!(server=%connection.server_url, device_id=%state.device.id, "central sync connected");
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    let mut last_heartbeat = Instant::now() - Duration::from_secs(60);
    let mut batches: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if last_heartbeat.elapsed() >= Duration::from_secs(25) {
                    let mut device = state.device.clone();
                    device.last_seen_at = Utc::now();
                    let adapters = match state.store.adapters()? {
                        cached if !cached.is_empty() => cached,
                        _ => discovery::refresh(&state.store)?,
                    };
                    send(&mut socket, &NodeToServerMessage::Heartbeat {
                        device,
                        adapters,
                        sent_at: Utc::now(),
                    }).await?;
                    last_heartbeat = Instant::now();
                }
                if batches.is_empty() {
                    let events = state.store.pending_sync_events(200)?;
                    if !events.is_empty() {
                        let batch_id = Uuid::new_v4();
                        let ids = events.iter().map(|event| event.event_id).collect();
                        send(&mut socket, &NodeToServerMessage::EventBatch { batch_id, events }).await?;
                        batches.insert(batch_id, ids);
                    }
                }
                for command in state.store.remote_command_updates()? {
                    send(&mut socket, &NodeToServerMessage::CommandAck {
                        command_id: command.id,
                        state: command.state,
                        result_summary: command.result_summary,
                        sent_at: Utc::now(),
                    }).await?;
                }
            }
            incoming = socket.next() => {
                let Some(incoming) = incoming else { bail!("central WSS closed") };
                match incoming? {
                    Message::Text(text) => {
                        handle_server_message(state, &mut socket, &mut batches, text.as_str()).await?;
                    }
                    Message::Close(frame) => bail!("central WSS closed: {frame:?}"),
                    Message::Ping(payload) => socket.send(Message::Pong(payload)).await?,
                    _ => {}
                }
            }
        }
    }
}

async fn handle_server_message<S>(
    state: &AppState,
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    batches: &mut HashMap<Uuid, Vec<Uuid>>,
    text: &str,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match serde_json::from_str::<ServerToNodeMessage>(text)? {
        ServerToNodeMessage::Welcome { .. } | ServerToNodeMessage::Ping { .. } => {}
        ServerToNodeMessage::EventBatchAck { batch_id, .. } => {
            if let Some(event_ids) = batches.remove(&batch_id) {
                state.store.mark_sync_events_sent(&event_ids)?;
            }
        }
        ServerToNodeMessage::Command(command) => {
            let command_id = command.id;
            let imported = state.store.import_remote_command(&command)?;
            send(
                socket,
                &NodeToServerMessage::CommandAck {
                    command_id,
                    state: CommandState::Accepted,
                    result_summary: Some(if imported {
                        "remote command accepted by target device".to_owned()
                    } else {
                        "remote command was already accepted".to_owned()
                    }),
                    sent_at: Utc::now(),
                },
            )
            .await?;
        }
        ServerToNodeMessage::CommandAckReceipt {
            command_id,
            state: command_state,
        } => {
            state
                .store
                .mark_remote_command_reported(command_id, command_state)?;
        }
        ServerToNodeMessage::Error { code, message } => {
            bail!("central error {code}: {message}");
        }
    }
    Ok(())
}

async fn send<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    message: &NodeToServerMessage,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    socket
        .send(Message::Text(serde_json::to_string(message)?.into()))
        .await?;
    Ok(())
}
