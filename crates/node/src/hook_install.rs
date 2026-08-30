use std::{
    fs,
    path::{Path, PathBuf},
};

use ai_rpa_core::Provider;
use anyhow::{Context, Result, bail};
use chrono::Utc;
use directories::BaseDirs;
use serde::Serialize;
use serde_json::{Map, Value, json};

const MARKER: &str = " hook --provider ";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookInstallResult {
    pub provider: Provider,
    pub path: PathBuf,
    pub changed: bool,
    pub backup: Option<PathBuf>,
}

pub fn install_all(executable: &Path) -> Result<Vec<HookInstallResult>> {
    [
        Provider::Codex,
        Provider::Claude,
        Provider::Cursor,
        Provider::Antigravity,
    ]
    .into_iter()
    .map(|provider| install(provider, executable))
    .collect()
}

pub fn uninstall_all() -> Result<Vec<HookInstallResult>> {
    [
        Provider::Codex,
        Provider::Claude,
        Provider::Cursor,
        Provider::Antigravity,
    ]
    .into_iter()
    .map(uninstall)
    .collect()
}

pub fn install(provider: Provider, executable: &Path) -> Result<HookInstallResult> {
    if !executable.is_absolute() {
        bail!("hook executable path must be absolute");
    }
    let path = config_path(provider)?;
    let mut root = read_json(&path)?;
    remove_managed_entries(provider, &mut root)?;
    add_managed_entries(provider, &mut root, executable)?;
    write_if_changed(provider, path, root)
}

pub fn uninstall(provider: Provider) -> Result<HookInstallResult> {
    let path = config_path(provider)?;
    if !path.exists() {
        return Ok(HookInstallResult {
            provider,
            path,
            changed: false,
            backup: None,
        });
    }
    let mut root = read_json(&path)?;
    remove_managed_entries(provider, &mut root)?;
    write_if_changed(provider, path, root)
}

pub fn config_path(provider: Provider) -> Result<PathBuf> {
    let home = BaseDirs::new()
        .context("resolve user home directory")?
        .home_dir()
        .to_path_buf();
    Ok(match provider {
        Provider::Codex => home.join(".codex/hooks.json"),
        Provider::Claude => home.join(".claude/settings.json"),
        Provider::Cursor => home.join(".cursor/hooks.json"),
        Provider::Antigravity => home.join(".gemini/config/hooks.json"),
    })
}

pub fn configured(provider: Provider) -> Result<bool> {
    let path = config_path(provider)?;
    if !path.exists() {
        return Ok(false);
    }
    let root = read_json(&path)?;
    Ok(if provider == Provider::Antigravity {
        root.get("ai-rpa-monitor").is_some()
    } else {
        contains_marker(&root)
    })
}

fn read_json(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let value: Value = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "{} is not valid JSON; refusing to overwrite user configuration",
            path.display()
        )
    })?;
    if !value.is_object() {
        bail!("{} must contain a JSON object", path.display());
    }
    Ok(value)
}

fn add_managed_entries(provider: Provider, root: &mut Value, executable: &Path) -> Result<()> {
    match provider {
        Provider::Cursor => {
            let object = root_object(root)?;
            object.entry("version").or_insert(json!(1));
            let hooks = object_entry_object(object, "hooks")?;
            for event in ["sessionStart", "stop", "sessionEnd"] {
                array_entry(hooks, event)?.push(json!({
                    "command": platform_command(executable, provider, event),
                    "timeout": 5,
                    "loop_limit": if event == "stop" { json!(5) } else { Value::Null }
                }));
            }
        }
        Provider::Codex | Provider::Claude => {
            let object = root_object(root)?;
            let hooks = object_entry_object(object, "hooks")?;
            let events: &[&str] = if provider == Provider::Claude {
                &[
                    "SessionStart",
                    "PermissionRequest",
                    "Notification",
                    "Stop",
                    "StopFailure",
                    "SessionEnd",
                ]
            } else {
                &["SessionStart", "PermissionRequest", "Stop", "SessionEnd"]
            };
            for event in events {
                let unix = unix_command(executable, provider, event);
                let windows = windows_command(executable, provider, event);
                let mut definition = json!({
                    "hooks": [{
                        "type": "command",
                        "command": unix,
                        "commandWindows": windows,
                        "timeout": if *event == "SessionEnd" { 3 } else { 5 }
                    }]
                });
                if *event == "Notification" {
                    definition["matcher"] =
                        json!("permission_prompt|idle_prompt|elicitation_dialog");
                }
                array_entry(hooks, event)?.push(definition);
            }
        }
        Provider::Antigravity => {
            let object = root_object(root)?;
            object.insert(
                "ai-rpa-monitor".to_owned(),
                json!({
                    "enabled": true,
                    "PreInvocation": [{
                        "type": "command",
                        "command": platform_command(executable, provider, "PreInvocation"),
                        "timeout": 5
                    }],
                    "PostInvocation": [{
                        "type": "command",
                        "command": platform_command(executable, provider, "PostInvocation"),
                        "timeout": 5
                    }],
                    "Stop": [{
                        "type": "command",
                        "command": platform_command(executable, provider, "Stop"),
                        "timeout": 5
                    }]
                }),
            );
        }
    }
    Ok(())
}

fn remove_managed_entries(provider: Provider, root: &mut Value) -> Result<()> {
    if provider == Provider::Antigravity {
        root_object(root)?.remove("ai-rpa-monitor");
        return Ok(());
    }
    let Some(hooks) = root
        .as_object_mut()
        .and_then(|object| object.get_mut("hooks"))
    else {
        return Ok(());
    };
    let hooks = hooks
        .as_object_mut()
        .context("hooks must be a JSON object")?;
    for definitions in hooks.values_mut() {
        let Some(definitions) = definitions.as_array_mut() else {
            continue;
        };
        definitions.retain(|definition| !contains_marker(definition));
    }
    hooks.retain(|_, definitions| {
        definitions
            .as_array()
            .is_none_or(|definitions| !definitions.is_empty())
    });
    Ok(())
}

fn contains_marker(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains(MARKER),
        Value::Array(items) => items.iter().any(contains_marker),
        Value::Object(object) => object.values().any(contains_marker),
        _ => false,
    }
}

fn platform_command(executable: &Path, provider: Provider, event: &str) -> String {
    #[cfg(windows)]
    return windows_command(executable, provider, event);
    #[cfg(not(windows))]
    unix_command(executable, provider, event)
}

fn unix_command(executable: &Path, provider: Provider, event: &str) -> String {
    format!(
        "{} hook --provider {} --event {}",
        shell_quote(&executable.to_string_lossy()),
        provider,
        event
    )
}

fn windows_command(executable: &Path, provider: Provider, event: &str) -> String {
    format!(
        "\"{}\" hook --provider {} --event {}",
        executable.to_string_lossy().replace('"', "\"\""),
        provider,
        event
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn root_object(root: &mut Value) -> Result<&mut Map<String, Value>> {
    root.as_object_mut()
        .context("configuration root must be an object")
}

fn object_entry_object<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
) -> Result<&'a mut Map<String, Value>> {
    object
        .entry(key.to_owned())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .with_context(|| format!("{key} must be a JSON object"))
}

fn array_entry<'a>(object: &'a mut Map<String, Value>, key: &str) -> Result<&'a mut Vec<Value>> {
    object
        .entry(key.to_owned())
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .with_context(|| format!("{key} hook definition must be an array"))
}

fn write_if_changed(provider: Provider, path: PathBuf, value: Value) -> Result<HookInstallResult> {
    let encoded = serde_json::to_vec_pretty(&value)?;
    let existing = fs::read(&path).ok();
    if existing.as_deref() == Some(encoded.as_slice()) {
        return Ok(HookInstallResult {
            provider,
            path,
            changed: false,
            backup: None,
        });
    }
    let backup = if path.exists() {
        let backup = path.with_extension(format!(
            "json.ai-rpa.bak.{}",
            Utc::now().format("%Y%m%d%H%M%S%3f")
        ));
        fs::copy(&path, &backup).with_context(|| format!("backup {}", path.display()))?;
        Some(backup)
    } else {
        None
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.ai-rpa.tmp");
    fs::write(&temporary, &encoded)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temporary, &path)?;
    Ok(HookInstallResult {
        provider,
        path,
        changed: true,
        backup,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn install_at(provider: Provider, path: &Path, executable: &Path) -> Result<Value> {
        let mut root = read_json(path)?;
        remove_managed_entries(provider, &mut root)?;
        add_managed_entries(provider, &mut root, executable)?;
        Ok(root)
    }

    #[test]
    fn cursor_merge_preserves_user_hooks_and_is_idempotent() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("hooks.json");
        fs::write(
            &path,
            r#"{"version":1,"hooks":{"stop":[{"command":"custom-stop"}]}}"#,
        )
        .unwrap();
        let executable = Path::new("/Applications/AI RPA/ai-rpa");
        let first = install_at(Provider::Cursor, &path, executable).unwrap();
        let mut second = first.clone();
        remove_managed_entries(Provider::Cursor, &mut second).unwrap();
        add_managed_entries(Provider::Cursor, &mut second, executable).unwrap();
        assert_eq!(first, second);
        assert_eq!(first["hooks"]["stop"].as_array().unwrap().len(), 2);
        assert!(first.to_string().contains("custom-stop"));
    }

    #[test]
    fn claude_merge_keeps_unrelated_settings() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("settings.json");
        fs::write(&path, r#"{"model":"sonnet","hooks":{}}"#).unwrap();
        let merged =
            install_at(Provider::Claude, &path, Path::new("/usr/local/bin/ai-rpa")).unwrap();
        assert_eq!(merged["model"], "sonnet");
        assert!(merged["hooks"]["Stop"].is_array());
        assert!(merged["hooks"]["StopFailure"].is_array());
    }

    #[test]
    fn malformed_json_is_never_overwritten() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("hooks.json");
        fs::write(&path, "{broken").unwrap();
        let error =
            install_at(Provider::Codex, &path, Path::new("/usr/local/bin/ai-rpa")).unwrap_err();
        assert!(error.to_string().contains("refusing to overwrite"));
        assert_eq!(fs::read_to_string(path).unwrap(), "{broken");
    }

    #[test]
    fn antigravity_owns_only_its_named_entry() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("hooks.json");
        fs::write(&path, r#"{"company-policy":{"enabled":true}}"#).unwrap();
        let mut merged = install_at(
            Provider::Antigravity,
            &path,
            Path::new("/usr/local/bin/ai-rpa"),
        )
        .unwrap();
        remove_managed_entries(Provider::Antigravity, &mut merged).unwrap();
        assert!(merged.get("company-policy").is_some());
        assert!(merged.get("ai-rpa-monitor").is_none());
    }
}
