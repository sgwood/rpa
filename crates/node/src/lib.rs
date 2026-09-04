pub mod api;
pub mod cli;
pub mod codex;
pub mod command_runner;
pub mod config;
pub mod diagnostics;
pub mod discovery;
pub mod hook;
pub mod hook_install;
pub mod notify;
pub mod sync;

use std::{path::Path, sync::Arc};

use ai_rpa_core::DeviceRecord;
use ai_rpa_store::Store;
use chrono::Utc;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub device: DeviceRecord,
    pub started_at: chrono::DateTime<Utc>,
    pub data_dir: Arc<std::path::PathBuf>,
    pub ui_dir: Option<Arc<std::path::PathBuf>>,
}

impl AppState {
    pub fn task_url(&self, task_id: uuid::Uuid) -> String {
        format!("http://127.0.0.1:3847/tasks/{task_id}")
    }
}

pub fn is_safe_local_bind(bind: &str) -> bool {
    bind.starts_with("127.0.0.1:") || bind.starts_with("[::1]:") || bind.starts_with("localhost:")
}

pub fn ensure_private_directory(path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
