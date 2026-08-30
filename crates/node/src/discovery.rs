use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use ai_rpa_core::{AdapterStatus, ControlMode, Provider, adapter::provider_capabilities};
#[cfg(not(target_os = "macos"))]
use anyhow::Context;
use anyhow::{Result, bail};
use chrono::Utc;

use crate::hook_install;

pub fn discover_all() -> Vec<AdapterStatus> {
    Provider::ALL.into_iter().map(discover_provider).collect()
}

pub fn discover_provider(provider: Provider) -> AdapterStatus {
    let executable = candidates(provider).into_iter().find(|path| path.is_file());
    let running = process_running(provider);
    let version = executable.as_ref().and_then(version_of);
    let install_state = match (&executable, running) {
        (None, false) => "NOT_INSTALLED",
        (Some(_), false) => "INSTALLED_NOT_RUNNING",
        (_, true) => "RUNNING",
    }
    .to_owned();
    let hook_state = match hook_install::configured(provider) {
        Ok(true) => "CONFIGURED",
        Ok(false) => "NOT_CONFIGURED",
        Err(_) => "CONFIG_INVALID",
    }
    .to_owned();
    AdapterStatus {
        provider,
        install_state,
        executable: executable.map(|path| path.to_string_lossy().into_owned()),
        version,
        hook_state,
        last_event_at: None,
        capabilities: provider_capabilities(provider, ControlMode::Observed),
        message: if running {
            "process detected; hook health requires an event".to_owned()
        } else {
            "no active process detected".to_owned()
        },
    }
}

fn candidates(provider: Provider) -> Vec<PathBuf> {
    let names: &[&str] = match provider {
        Provider::Codex => &["codex"],
        Provider::Claude => &["claude"],
        Provider::Cursor => &["cursor"],
        Provider::Antigravity => &["agy", "antigravity", "antigravity-ide"],
    };
    let mut output = Vec::new();
    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path) {
            for name in names {
                #[cfg(windows)]
                output.push(directory.join(format!("{name}.exe")));
                output.push(directory.join(name));
            }
        }
    }
    #[cfg(windows)]
    {
        if let Some(local_app_data) = env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            let fixed: &[&str] = match provider {
                Provider::Codex => &["Programs/OpenAI/Codex/Codex.exe"],
                Provider::Claude => &["AnthropicClaude/claude.exe", "Programs/Claude/Claude.exe"],
                Provider::Cursor => &["Programs/cursor/Cursor.exe"],
                Provider::Antigravity => &[
                    "Programs/Antigravity/Antigravity.exe",
                    "Programs/Antigravity IDE/Antigravity IDE.exe",
                ],
            };
            output.extend(fixed.iter().map(|path| local_app_data.join(path)));
        }
        if let Some(program_files) = env::var_os("ProgramFiles").map(PathBuf::from) {
            let fixed: &[&str] = match provider {
                Provider::Codex => &["OpenAI/Codex/Codex.exe"],
                Provider::Claude => &["Claude/Claude.exe"],
                Provider::Cursor => &["Cursor/Cursor.exe"],
                Provider::Antigravity => &["Antigravity IDE/Antigravity IDE.exe"],
            };
            output.extend(fixed.iter().map(|path| program_files.join(path)));
        }
    }
    #[cfg(target_os = "macos")]
    {
        let fixed: &[&str] = match provider {
            Provider::Codex => &[
                "/Applications/ChatGPT.app/Contents/Resources/codex",
                "/Applications/Codex.app/Contents/Resources/codex",
            ],
            Provider::Claude => &["/Applications/Claude.app/Contents/MacOS/Claude"],
            Provider::Cursor => &["/Applications/Cursor.app/Contents/Resources/app/bin/cursor"],
            Provider::Antigravity => &[
                "/Applications/Antigravity.app/Contents/Resources/app/bin/agy",
                "/Applications/Antigravity.app/Contents/MacOS/Antigravity",
                "/Applications/Antigravity IDE.app/Contents/Resources/app/bin/antigravity-ide",
                "/Applications/Antigravity IDE.app/Contents/MacOS/Electron",
            ],
        };
        output.extend(fixed.iter().map(PathBuf::from));
    }
    output
}

fn process_running(provider: Provider) -> bool {
    #[cfg(unix)]
    let output = Command::new("ps").args(["ax", "-o", "command="]).output();
    #[cfg(windows)]
    let output = Command::new("tasklist").output();
    let Ok(output) = output else {
        return false;
    };
    let processes = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let needles: &[&str] = match provider {
        Provider::Codex => &["codex", "chatgpt.app"],
        Provider::Claude => &["claude"],
        Provider::Cursor => &["cursor"],
        Provider::Antigravity => &["antigravity", "agy"],
    };
    needles.iter().any(|needle| processes.contains(needle))
}

fn version_of(path: &PathBuf) -> Option<String> {
    #[cfg(target_os = "macos")]
    if let Some(version) = macos_app_version(path) {
        return Some(version);
    }

    let mut child = Command::new(path)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    for _ in 0..20 {
        if child.try_wait().ok().flatten().is_some() {
            let output = child.wait_with_output().ok()?;
            return output
                .status
                .success()
                .then(|| version_from_output(&output.stdout, &output.stderr))
                .flatten();
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

fn version_from_output(stdout: &[u8], stderr: &[u8]) -> Option<String> {
    let text = if stdout.is_empty() {
        String::from_utf8_lossy(stderr)
    } else {
        String::from_utf8_lossy(stdout)
    };
    let version = text.lines().next()?.trim();
    (!version.is_empty()).then(|| version.chars().take(120).collect())
}

#[cfg(target_os = "macos")]
fn macos_app_version(executable: &Path) -> Option<String> {
    let text = executable.to_string_lossy();
    let marker = ".app/Contents/";
    let split = text.find(marker)?;
    let plist = format!("{}.app/Contents/Info.plist", &text[..split]);
    let output = Command::new("/usr/bin/plutil")
        .args([
            "-extract",
            "CFBundleShortVersionString",
            "raw",
            "-o",
            "-",
            &plist,
        ])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub fn refresh(store: &ai_rpa_store::Store) -> anyhow::Result<Vec<AdapterStatus>> {
    let mut statuses = discover_all();
    let persisted = store.adapters().unwrap_or_default();
    for status in &mut statuses {
        if let Some(previous) = persisted
            .iter()
            .find(|item| item.provider == status.provider)
        {
            status.last_event_at = previous.last_event_at;
            if previous.last_event_at.is_some() && status.hook_state != "CONFIG_INVALID" {
                status.hook_state = "HEALTHY".to_owned();
                status.message = format!(
                    "last event received at {}",
                    previous.last_event_at.unwrap_or_else(Utc::now)
                );
            }
        }
        store.upsert_adapter(status)?;
    }
    Ok(statuses)
}

pub fn open_provider(provider: Provider, workspace: Option<&str>) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let names: &[&str] = match provider {
            Provider::Codex => &["Codex", "ChatGPT"],
            Provider::Claude => &["Claude"],
            Provider::Cursor => &["Cursor"],
            Provider::Antigravity => &["Antigravity", "Antigravity IDE"],
        };
        for name in names {
            let mut command = Command::new("/usr/bin/open");
            command.args(["-a", name]);
            if let Some(path) = workspace.filter(|path| !path.trim().is_empty()) {
                command.arg(path);
            }
            if command.status().is_ok_and(|status| status.success()) {
                return Ok(());
            }
        }
        bail!("could not open the installed {provider} application");
    }
    #[cfg(not(target_os = "macos"))]
    {
        let status = discover_provider(provider);
        let executable = status
            .executable
            .context("provider executable is not installed")?;
        let mut command = Command::new(executable);
        if let Some(path) = workspace.filter(|path| !path.trim().is_empty()) {
            command.arg(path);
        }
        command.spawn().context("open provider application")?;
        Ok(())
    }
}
