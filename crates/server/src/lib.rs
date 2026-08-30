pub mod api;
pub mod crypto;
pub mod store;

use std::{path::PathBuf, str::FromStr};

use anyhow::{Context, Result, bail};
use sqlx_core::raw_sql::raw_sql;
use sqlx_postgres::{PgConnectOptions, PgPoolOptions};
use tokio::net::TcpListener;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    api::AppState,
    crypto::ServerCrypto,
    store::{CentralStore, token_hash},
};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: String,
    pub database_url: String,
    pub admin_token: String,
    pub data_key: String,
    pub ui_dir: Option<PathBuf>,
}

pub async fn run(config: ServerConfig) -> Result<()> {
    let _ = tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("ai_rpa_server=info,tower_http=info")),
        )
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .try_init();
    if config.admin_token.len() < 24 {
        bail!("AI_RPA_ADMIN_TOKEN must contain at least 24 characters");
    }
    let options = PgConnectOptions::from_str(&config.database_url)?;
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect_with(options)
        .await
        .context("connect to PostgreSQL")?;
    raw_sql(include_str!("../migrations/0001_initial.sql"))
        .execute(&pool)
        .await
        .context("initialize central schema")?;
    let state = AppState {
        store: CentralStore::new(pool, ServerCrypto::from_base64(&config.data_key)?),
        admin_token_hash: token_hash(&config.admin_token),
        connections: Default::default(),
    };
    let listener = TcpListener::bind(&config.bind)
        .await
        .context("bind central server")?;
    tracing::info!(bind=%config.bind, "AI RPA central server started");
    axum::serve(listener, api::router(state, config.ui_dir)).await?;
    Ok(())
}
