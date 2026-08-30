use std::{path::PathBuf, sync::Arc};

use ai_rpa_core::{CommandAction, Provider};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use clap::{Parser, Subcommand};
use serde_json::json;
use tokio::net::TcpListener;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

use crate::{
    AppState,
    api::{background_tasks, router},
    config::{AppPaths, DEFAULT_BIND, load_or_create_device, open_store},
    diagnostics, discovery, hook, hook_install, is_safe_local_bind,
};

#[derive(Debug, Parser)]
#[command(
    name = "ai-rpa",
    version,
    about = "AI task monitor and continuation node"
)]
struct Cli {
    #[arg(long, global = true, env = "AI_RPA_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[arg(long, global = true, env = "AI_RPA_DEV_MASTER_KEY")]
    development_key: Option<String>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Serve {
        #[arg(long, default_value = DEFAULT_BIND)]
        bind: String,
        #[arg(long)]
        ui_dir: Option<PathBuf>,
    },
    Hook {
        #[arg(long)]
        provider: Provider,
        #[arg(long)]
        event: String,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long, default_value = "http://127.0.0.1:3847")]
        node_url: String,
    },
    Doctor,
    Discover,
    InstallHooks {
        #[arg(long)]
        provider: Option<Provider>,
        #[arg(long)]
        executable: Option<PathBuf>,
    },
    UninstallHooks {
        #[arg(long)]
        provider: Option<Provider>,
    },
    ExportDiagnostics {
        #[arg(long)]
        output: PathBuf,
    },
    Command {
        #[arg(long)]
        task_id: Uuid,
        #[arg(long)]
        action: CommandAction,
        #[arg(long)]
        message: String,
        #[arg(long, default_value_t = 7200)]
        ttl_seconds: i64,
    },
}

pub fn is_cli_invocation() -> bool {
    std::env::args().skip(1).any(|argument| {
        matches!(
            argument.as_str(),
            "serve"
                | "hook"
                | "doctor"
                | "discover"
                | "install-hooks"
                | "uninstall-hooks"
                | "export-diagnostics"
                | "command"
                | "--help"
                | "--version"
                | "-h"
                | "-V"
        )
    })
}

pub fn create_state(
    data_dir: Option<PathBuf>,
    development_key: Option<&str>,
) -> Result<(AppState, AppPaths)> {
    let paths = AppPaths::resolve(data_dir)?;
    let store = open_store(&paths, development_key)?;
    let device = load_or_create_device(&store)?;
    Ok((
        AppState {
            store,
            device,
            started_at: Utc::now(),
            data_dir: Arc::new(paths.data_dir.clone()),
            ui_dir: None,
        },
        paths,
    ))
}

pub async fn run() -> Result<()> {
    let _ = tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("ai_rpa=info,tower_http=info")),
        )
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .try_init();

    let cli = Cli::parse();
    let (state, paths) = create_state(cli.data_dir, cli.development_key.as_deref())?;
    match cli.command.unwrap_or(Commands::Serve {
        bind: DEFAULT_BIND.to_owned(),
        ui_dir: None,
    }) {
        Commands::Serve { bind, ui_dir } => serve(state, &paths, &bind, ui_dir).await,
        Commands::Hook {
            provider,
            event,
            session_id,
            node_url,
        } => {
            let payload = hook::read_stdin_json()?;
            let raw = hook::to_raw_event(provider, &event, session_id.as_deref(), payload)?;
            match hook::submit(&node_url, &raw).await {
                Ok(response) => println!("{}", hook::hook_stdout(response.get("hookResponse"))),
                Err(error) => {
                    let path = hook::spool(&paths.spool, &raw)?;
                    eprintln!(
                        "AI RPA node unavailable; event spooled at {}: {error}",
                        path.display()
                    );
                    println!("{}", json!({}));
                }
            }
            Ok(())
        }
        Commands::Doctor => {
            println!(
                "{}",
                serde_json::to_string_pretty(&diagnostics::simple_doctor(
                    &state.store,
                    &state.device
                )?)?
            );
            Ok(())
        }
        Commands::Discover => {
            println!(
                "{}",
                serde_json::to_string_pretty(&discovery::refresh(&state.store)?)?
            );
            Ok(())
        }
        Commands::InstallHooks {
            provider,
            executable,
        } => {
            let executable = executable.map(Ok).unwrap_or_else(std::env::current_exe)?;
            let results = if let Some(provider) = provider {
                vec![hook_install::install(provider, &executable)?]
            } else {
                hook_install::install_all(&executable)?
            };
            println!("{}", serde_json::to_string_pretty(&results)?);
            Ok(())
        }
        Commands::UninstallHooks { provider } => {
            let results = if let Some(provider) = provider {
                vec![hook_install::uninstall(provider)?]
            } else {
                hook_install::uninstall_all()?
            };
            println!("{}", serde_json::to_string_pretty(&results)?);
            Ok(())
        }
        Commands::ExportDiagnostics { output } => {
            diagnostics::export(&state, &output)?;
            println!("{}", output.display());
            Ok(())
        }
        Commands::Command {
            task_id,
            action,
            message,
            ttl_seconds,
        } => {
            let command =
                state
                    .store
                    .create_command(task_id, action, &message, "local-cli", ttl_seconds)?;
            println!("{}", serde_json::to_string_pretty(&command)?);
            Ok(())
        }
    }
}

pub async fn serve(
    mut state: AppState,
    paths: &AppPaths,
    bind: &str,
    ui_dir: Option<PathBuf>,
) -> Result<()> {
    if !is_safe_local_bind(bind) {
        bail!("P0 node may only bind to loopback; refused {bind}");
    }
    if let Some(directory) = ui_dir {
        state.ui_dir = Some(Arc::new(directory));
    }
    discovery::refresh(&state.store)?;
    let local_url = format!("http://{bind}");
    if let Err(error) = hook::drain_spool(&paths.spool, &local_url).await {
        tracing::warn!(%error, "initial spool drain failed");
    }
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("bind local API to {bind}"))?;
    tracing::info!(%bind, device_id = %state.device.id, "AI RPA node started");
    tokio::spawn(background_tasks(state.clone()));
    axum::serve(listener, router(state)).await?;
    Ok(())
}
