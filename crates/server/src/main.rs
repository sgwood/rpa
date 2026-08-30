use std::path::PathBuf;

use ai_rpa_server::{ServerConfig, run};
use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "ai-rpa-server", about = "AI RPA ctyun central control plane")]
struct Cli {
    #[arg(long, env = "AI_RPA_BIND", default_value = "0.0.0.0:8080")]
    bind: String,
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,
    #[arg(long, env = "AI_RPA_ADMIN_TOKEN")]
    admin_token: String,
    #[arg(long, env = "AI_RPA_DATA_KEY")]
    data_key: String,
    #[arg(long, env = "AI_RPA_UI_DIR")]
    ui_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run(ServerConfig {
        bind: cli.bind,
        database_url: cli.database_url,
        admin_token: cli.admin_token,
        data_key: cli.data_key,
        ui_dir: cli.ui_dir,
    })
    .await
}
