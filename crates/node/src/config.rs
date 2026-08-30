use std::{env, path::PathBuf, process::Command};

use ai_rpa_core::DeviceRecord;
use ai_rpa_store::{CryptoBox, Store};
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::Utc;
use directories::ProjectDirs;
use uuid::Uuid;

use crate::ensure_private_directory;

pub const DEFAULT_BIND: &str = "127.0.0.1:3847";
pub const KEYRING_SERVICE: &str = "com.stargold.ai-rpa";
const CENTRAL_URL_KEY: &str = "central_server_url";
const CENTRAL_ALIAS_KEY: &str = "central_device_alias";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CentralConnection {
    pub server_url: String,
    pub device_alias: String,
    #[serde(skip_serializing)]
    pub node_token: String,
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub database: PathBuf,
    pub spool: PathBuf,
    pub diagnostics: PathBuf,
}

impl AppPaths {
    pub fn resolve(override_dir: Option<PathBuf>) -> Result<Self> {
        let data_dir = if let Some(path) = override_dir {
            path
        } else {
            ProjectDirs::from("com", "stargold", "ai-rpa")
                .context("cannot determine application data directory")?
                .data_local_dir()
                .to_path_buf()
        };
        ensure_private_directory(&data_dir)?;
        let spool = data_dir.join("spool");
        let diagnostics = data_dir.join("diagnostics");
        ensure_private_directory(&spool)?;
        ensure_private_directory(&diagnostics)?;
        Ok(Self {
            database: data_dir.join("ai-rpa.db"),
            data_dir,
            spool,
            diagnostics,
        })
    }
}

pub fn open_store(paths: &AppPaths, development_key: Option<&str>) -> Result<Store> {
    let crypto = load_crypto(development_key)?;
    Store::open(&paths.database, crypto)
}

fn load_crypto(development_key: Option<&str>) -> Result<CryptoBox> {
    let encoded_override = development_key
        .map(str::to_owned)
        .or_else(|| env::var("AI_RPA_MASTER_KEY").ok());
    if let Some(value) = encoded_override {
        let bytes = STANDARD
            .decode(value)
            .context("AI_RPA_MASTER_KEY must be base64")?;
        if bytes.len() != 32 {
            bail!("AI_RPA_MASTER_KEY must decode to 32 bytes");
        }
        let mut key = [0_u8; 32];
        key.copy_from_slice(&bytes);
        return Ok(CryptoBox::from_key(key));
    }

    let entry = keyring::Entry::new(KEYRING_SERVICE, "local-master-key")?;
    match entry.get_password() {
        Ok(encoded) => {
            let bytes = STANDARD
                .decode(encoded)
                .context("invalid master key in OS credential store")?;
            if bytes.len() != 32 {
                bail!("invalid master key length in OS credential store");
            }
            let mut key = [0_u8; 32];
            key.copy_from_slice(&bytes);
            Ok(CryptoBox::from_key(key))
        }
        Err(keyring::Error::NoEntry) => {
            let (crypto, key) = CryptoBox::generate();
            entry
                .set_password(&STANDARD.encode(key))
                .context("save master key in OS credential store")?;
            Ok(crypto)
        }
        Err(error) => Err(error).context("read master key from OS credential store"),
    }
}

pub fn load_or_create_device(store: &Store) -> Result<DeviceRecord> {
    let logical_environment = logical_environment();
    let meta_key = format!("device_id:{logical_environment}");
    let id = match store.get_meta(&meta_key)? {
        Some(value) => value,
        None => {
            let value = format!("dev_{}", Uuid::new_v4().simple());
            store.set_meta(&meta_key, &value)?;
            value
        }
    };
    let hostname = system_hostname();
    let device = DeviceRecord {
        id,
        os: env::consts::OS.to_owned(),
        arch: env::consts::ARCH.to_owned(),
        hostname,
        logical_environment,
        node_version: env!("CARGO_PKG_VERSION").to_owned(),
        last_seen_at: Utc::now(),
    };
    store.upsert_device(&device)?;
    Ok(device)
}

pub fn save_central_connection(
    store: &Store,
    device: &DeviceRecord,
    server_url: &str,
    alias: &str,
    token: &str,
) -> Result<()> {
    let normalized = server_url.trim().trim_end_matches('/');
    if !(normalized.starts_with("https://")
        || cfg!(debug_assertions) && normalized.starts_with("http://"))
    {
        bail!("central server must use HTTPS outside development builds");
    }
    store.set_meta(CENTRAL_URL_KEY, normalized)?;
    store.set_meta(CENTRAL_ALIAS_KEY, alias.trim())?;
    keyring::Entry::new(
        KEYRING_SERVICE,
        &format!("central-node-token:{}", device.id),
    )?
    .set_password(token)
    .context("save central node token in OS credential store")?;
    Ok(())
}

pub fn load_central_connection(
    store: &Store,
    device: &DeviceRecord,
) -> Result<Option<CentralConnection>> {
    let Some(server_url) = store
        .get_meta(CENTRAL_URL_KEY)?
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(None);
    };
    let device_alias = store
        .get_meta(CENTRAL_ALIAS_KEY)?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| device.hostname.clone());
    let node_token = env::var("AI_RPA_NODE_TOKEN").ok().or_else(|| {
        keyring::Entry::new(
            KEYRING_SERVICE,
            &format!("central-node-token:{}", device.id),
        )
        .ok()?
        .get_password()
        .ok()
    });
    Ok(node_token.map(|node_token| CentralConnection {
        server_url,
        device_alias,
        node_token,
    }))
}

pub fn clear_central_connection(store: &Store, device: &DeviceRecord) -> Result<()> {
    store.set_meta(CENTRAL_URL_KEY, "")?;
    store.set_meta(CENTRAL_ALIAS_KEY, "")?;
    if let Ok(entry) = keyring::Entry::new(
        KEYRING_SERVICE,
        &format!("central-node-token:{}", device.id),
    ) {
        let _ = entry.delete_credential();
    }
    Ok(())
}

fn system_hostname() -> String {
    env::var("COMPUTERNAME")
        .or_else(|_| env::var("HOSTNAME"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            Command::new("hostname")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "未知设备".to_owned())
}

fn logical_environment() -> String {
    if cfg!(target_os = "windows") {
        "windows-native".to_owned()
    } else if env::var_os("WSL_DISTRO_NAME").is_some() || env::var_os("WSL_INTEROP").is_some() {
        format!(
            "wsl:{}",
            env::var("WSL_DISTRO_NAME").unwrap_or_else(|_| "unknown".to_owned())
        )
    } else {
        env::consts::OS.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_environment_has_non_empty_identity() {
        assert!(!logical_environment().is_empty());
        assert!(!system_hostname().is_empty());
    }
}
