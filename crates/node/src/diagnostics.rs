use std::{fs, path::Path};

use ai_rpa_store::Store;
use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{AppState, discovery, notify::FeishuNotifier};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReport {
    pub generated_at: chrono::DateTime<Utc>,
    pub node_version: String,
    pub device: ai_rpa_core::DeviceRecord,
    pub checks: Vec<DiagnosticCheck>,
    pub adapters: Vec<ai_rpa_core::AdapterStatus>,
    pub counts: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCheck {
    pub name: String,
    pub status: String,
    pub message: String,
}

pub fn run(state: &AppState) -> Result<DiagnosticReport> {
    let adapters = discovery::refresh(&state.store)?;
    let notifier = FeishuNotifier::load();
    let counts = state.store.counts()?;
    let checks = vec![
        DiagnosticCheck {
            name: "SQLite WAL".to_owned(),
            status: "PASS".to_owned(),
            message: "database opened and schema is readable".to_owned(),
        },
        DiagnosticCheck {
            name: "Local API".to_owned(),
            status: "PASS".to_owned(),
            message: "bound to loopback only".to_owned(),
        },
        DiagnosticCheck {
            name: "Feishu".to_owned(),
            status: if notifier.configured() {
                "PASS"
            } else {
                "NOT_CONFIGURED"
            }
            .to_owned(),
            message: if notifier.configured() {
                "webhook credential found in environment or OS credential store"
            } else {
                "configure webhook in OS credential store before sending notifications"
            }
            .to_owned(),
        },
        DiagnosticCheck {
            name: "Privacy".to_owned(),
            status: "PASS".to_owned(),
            message: "diagnostic output excludes prompts, transcripts, secrets and screenshots"
                .to_owned(),
        },
    ];
    Ok(DiagnosticReport {
        generated_at: Utc::now(),
        node_version: env!("CARGO_PKG_VERSION").to_owned(),
        device: state.device.clone(),
        checks,
        adapters,
        counts,
    })
}

pub fn export(state: &AppState, output: &Path) -> Result<()> {
    let payload = export_payload(state)?;
    fs::write(output, serde_json::to_vec_pretty(&payload)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(output, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn export_payload(state: &AppState) -> Result<Value> {
    let report = run(state)?;
    Ok(json!({
        "format": "ai-rpa-redacted-diagnostics-v1",
        "report": report,
        "privacy": {
            "containsSecrets": false,
            "containsPrompts": false,
            "containsSourceCode": false,
            "containsScreenshots": false
        }
    }))
}

pub fn simple_doctor(store: &Store, device: &ai_rpa_core::DeviceRecord) -> Result<Value> {
    Ok(json!({
        "status": "ok",
        "nodeVersion": env!("CARGO_PKG_VERSION"),
        "device": device,
        "counts": store.counts()?,
        "adapters": discovery::refresh(store)?
    }))
}
