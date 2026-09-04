use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use ai_rpa_core::{
    ControlMode, EvidenceLevel, Provider, RawEventInput, TaskRecord, normalize_event, redact_text,
};
use ai_rpa_store::{Store, TaskFilter};
use anyhow::{Context, Result, anyhow, bail};
use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdout, Command},
    task::JoinHandle,
    time::timeout,
};
use uuid::Uuid;

use crate::{AppState, api::ApiError, discovery};

const PROJECTS_META_KEY: &str = "codex_projects_v1";
const MAX_PROJECTS: usize = 100;
const MAX_PROJECTS_PER_RUN: usize = 8;
const MAX_PROMPT_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexProject {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexStatus {
    pub state: CodexAvailability,
    pub version: Option<String>,
    pub executable: Option<String>,
    pub authenticated: bool,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CodexAvailability {
    Ready,
    NotInstalled,
    NotAuthenticated,
    Broken,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterProjectRequest {
    name: String,
    path: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodexSandbox {
    ReadOnly,
    WorkspaceWrite,
}

impl CodexSandbox {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
        }
    }
}

fn default_timeout_seconds() -> u64 {
    3600
}

fn default_sandbox() -> CodexSandbox {
    CodexSandbox::ReadOnly
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRunsRequest {
    title: String,
    prompt: String,
    project_ids: Vec<String>,
    #[serde(default = "default_timeout_seconds")]
    timeout_seconds: u64,
    #[serde(default = "default_sandbox")]
    sandbox: CodexSandbox,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunAssignment {
    project_id: String,
    project_name: String,
    task_id: Uuid,
    session_id: String,
    state: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunError {
    project_id: String,
    project_name: String,
    message: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexTasksQuery {
    project_ids: Option<String>,
}

pub async fn status() -> Json<CodexStatus> {
    Json(detect_status().await)
}

pub async fn projects(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "items": load_projects(&state.store)? })))
}

pub async fn register_project(
    State(state): State<AppState>,
    Json(request): Json<RegisterProjectRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let project = register(&state.store, request)
        .await
        .map_err(ApiError::bad_request)?;
    Ok((StatusCode::CREATED, Json(json!({ "project": project }))))
}

pub async fn delete_project(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let deleted = remove_project(&state.store, &id)?;
    Ok(Json(json!({ "deleted": deleted })))
}

pub async fn tasks(
    State(state): State<AppState>,
    Query(query): Query<CodexTasksQuery>,
) -> Result<Json<Value>, ApiError> {
    let projects = load_projects(&state.store)?;
    let known: HashSet<&str> = projects.iter().map(|project| project.id.as_str()).collect();
    let selected = parse_project_ids(query.project_ids.as_deref())?;
    if let Some(selected) = &selected
        && let Some(unknown) = selected.iter().find(|id| !known.contains(id.as_str()))
    {
        return Err(ApiError::bad_request(anyhow!(
            "unknown Codex project: {unknown}"
        )));
    }
    let mut items = state.store.list_tasks(&TaskFilter {
        provider: Some(Provider::Codex),
        control_mode: Some(ControlMode::Managed),
        limit: Some(500),
        ..TaskFilter::default()
    })?;
    items.retain(|task| {
        let Some(project_id) = task.project.as_deref() else {
            return false;
        };
        selected.as_ref().map_or_else(
            || known.contains(project_id),
            |ids| ids.contains(project_id),
        )
    });
    Ok(Json(json!({ "items": items })))
}

pub async fn start_runs(
    State(state): State<AppState>,
    Json(request): Json<StartRunsRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    validate_run_request(&request).map_err(ApiError::bad_request)?;
    let status = detect_status().await;
    if status.state != CodexAvailability::Ready {
        return Err(ApiError::unavailable(anyhow!(status.message)));
    }
    let executable = PathBuf::from(status.executable.context("Codex executable is missing")?);
    let all_projects = load_projects(&state.store)?;
    let mut selected = Vec::new();
    for id in &request.project_ids {
        let project = all_projects
            .iter()
            .find(|project| &project.id == id)
            .cloned()
            .ok_or_else(|| ApiError::bad_request(anyhow!("unknown Codex project: {id}")))?;
        selected.push(project);
    }

    let mut assignments = Vec::new();
    let mut errors = Vec::new();
    for project in selected {
        match start_project(
            state.clone(),
            &executable,
            project.clone(),
            request.title.trim(),
            &request.prompt,
            request.timeout_seconds,
            request.sandbox,
        )
        .await
        {
            Ok(assignment) => assignments.push(assignment),
            Err(error) => errors.push(RunError {
                project_id: project.id,
                project_name: project.name,
                message: public_error(&error),
            }),
        }
    }
    if assignments.is_empty() {
        let summary = errors
            .iter()
            .map(|error| format!("{}: {}", error.project_name, error.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ApiError::unavailable(anyhow!(summary)));
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "assignments": assignments, "errors": errors })),
    ))
}

fn validate_run_request(request: &StartRunsRequest) -> Result<()> {
    let title = request.title.trim();
    if title.is_empty() || title.chars().count() > 120 {
        bail!("task title must contain 1 to 120 characters");
    }
    if request.prompt.trim().is_empty() || request.prompt.len() > MAX_PROMPT_BYTES {
        bail!("task prompt must contain 1 byte to 32 KiB");
    }
    if request.project_ids.is_empty() || request.project_ids.len() > MAX_PROJECTS_PER_RUN {
        bail!("select between 1 and {MAX_PROJECTS_PER_RUN} projects");
    }
    let unique: HashSet<&str> = request.project_ids.iter().map(String::as_str).collect();
    if unique.len() != request.project_ids.len() {
        bail!("project selection contains duplicates");
    }
    if !matches!(request.timeout_seconds, 3600 | 7200 | 10800) {
        bail!("timeout must be 1, 2, or 3 hours");
    }
    Ok(())
}

async fn detect_status() -> CodexStatus {
    let candidates = discovery::executable_candidates(Provider::Codex);
    if candidates.is_empty() {
        return CodexStatus {
            state: CodexAvailability::NotInstalled,
            version: None,
            executable: None,
            authenticated: false,
            message: "未找到 Codex CLI；请安装 Codex 桌面端或 Codex CLI。".to_owned(),
        };
    }
    let mut last_error = None;
    for executable in candidates {
        match command_output(&executable, &["--version"]).await {
            Ok(version) => match command_output(&executable, &["login", "status"]).await {
                Ok(login) if login.to_ascii_lowercase().contains("logged in") => {
                    return CodexStatus {
                        state: CodexAvailability::Ready,
                        version: first_line(&version),
                        executable: Some(executable.to_string_lossy().into_owned()),
                        authenticated: true,
                        message: "Codex CLI 可用且已登录，可以分配任务。".to_owned(),
                    };
                }
                Ok(_) | Err(_) => {
                    return CodexStatus {
                        state: CodexAvailability::NotAuthenticated,
                        version: first_line(&version),
                        executable: Some(executable.to_string_lossy().into_owned()),
                        authenticated: false,
                        message: "Codex CLI 已安装但尚未登录，请先在 Codex 中完成登录。".to_owned(),
                    };
                }
            },
            Err(error) => last_error = Some(error),
        }
    }
    CodexStatus {
        state: CodexAvailability::Broken,
        version: None,
        executable: None,
        authenticated: false,
        message: format!(
            "找到 Codex 入口但无法启动：{}",
            last_error
                .as_ref()
                .map(public_error)
                .unwrap_or_else(|| "未知错误".to_owned())
        ),
    }
}

async fn command_output(executable: &Path, arguments: &[&str]) -> Result<String> {
    let output = timeout(
        Duration::from_secs(4),
        Command::new(executable).args(arguments).output(),
    )
    .await
    .context("Codex command timed out")?
    .context("start Codex command")?;
    let text = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).into_owned()
    } else {
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    if !output.status.success() {
        bail!("Codex exited with {}: {}", output.status, concise(&text));
    }
    Ok(text)
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(120).collect())
}

fn load_projects(store: &Store) -> Result<Vec<CodexProject>> {
    let Some(raw) = store.get_meta(PROJECTS_META_KEY)? else {
        return Ok(Vec::new());
    };
    serde_json::from_str(&raw).context("decode Codex project registry")
}

fn save_projects(store: &Store, projects: &[CodexProject]) -> Result<()> {
    store.set_meta(PROJECTS_META_KEY, &serde_json::to_string(projects)?)
}

async fn register(store: &Store, request: RegisterProjectRequest) -> Result<CodexProject> {
    let name = request.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        bail!("project name must contain 1 to 80 characters");
    }
    let path = std::fs::canonicalize(request.path.trim()).context("project path does not exist")?;
    if !path.is_dir() {
        bail!("project path must be a directory");
    }
    let git = timeout(
        Duration::from_secs(5),
        Command::new("git")
            .args(["-C"])
            .arg(&path)
            .args(["rev-parse", "--is-inside-work-tree"])
            .output(),
    )
    .await
    .context("git project validation timed out")?
    .context("run git project validation")?;
    if !git.status.success() || String::from_utf8_lossy(&git.stdout).trim() != "true" {
        bail!("project path must be inside a Git worktree");
    }
    let id = project_id(&path);
    let mut projects = load_projects(store)?;
    if let Some(existing) = projects.iter_mut().find(|project| project.id == id) {
        existing.name = name.to_owned();
        let project = existing.clone();
        save_projects(store, &projects)?;
        return Ok(project);
    }
    if projects.len() >= MAX_PROJECTS {
        bail!("at most {MAX_PROJECTS} Codex projects can be registered");
    }
    let project = CodexProject {
        id,
        name: name.to_owned(),
        path,
        created_at: Utc::now(),
    };
    projects.push(project.clone());
    projects.sort_by(|left, right| left.name.cmp(&right.name));
    save_projects(store, &projects)?;
    Ok(project)
}

fn project_id(path: &Path) -> String {
    let value = path.to_string_lossy();
    #[cfg(windows)]
    let value = value.to_ascii_lowercase();
    let digest = Sha256::digest(value.as_bytes());
    format!("codex-{}", &hex::encode(digest)[..16])
}

fn remove_project(store: &Store, id: &str) -> Result<bool> {
    let mut projects = load_projects(store)?;
    let before = projects.len();
    projects.retain(|project| project.id != id);
    let deleted = projects.len() != before;
    if deleted {
        save_projects(store, &projects)?;
    }
    Ok(deleted)
}

fn parse_project_ids(value: Option<&str>) -> Result<Option<HashSet<String>>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let ids: HashSet<String> = value
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect();
    if ids.is_empty() || ids.len() > MAX_PROJECTS_PER_RUN {
        return Err(ApiError::bad_request(anyhow!(
            "select between 1 and {MAX_PROJECTS_PER_RUN} projects"
        )));
    }
    Ok(Some(ids))
}

async fn start_project(
    state: AppState,
    executable: &Path,
    project: CodexProject,
    title: &str,
    prompt: &str,
    timeout_seconds: u64,
    sandbox: CodexSandbox,
) -> Result<RunAssignment> {
    let mut child = Command::new(executable)
        .args(["exec", "--json", "--sandbox", sandbox.as_str(), "--cd"])
        .arg(&project.path)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("start Codex task")?;
    let mut stdin = child.stdin.take().context("open Codex stdin")?;
    stdin
        .write_all(prompt.as_bytes())
        .await
        .context("send task to Codex")?;
    stdin.shutdown().await.context("close Codex stdin")?;
    drop(stdin);

    let stdout = child.stdout.take().context("open Codex event stream")?;
    let stderr = child.stderr.take().context("open Codex error stream")?;
    let mut lines = BufReader::new(stdout).lines();
    let mut stderr_task = tokio::spawn(read_limited(stderr));
    let session_id = match timeout(Duration::from_secs(20), find_thread_id(&mut lines)).await {
        Ok(Ok(session_id)) => session_id,
        Ok(Err(error)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let stderr = take_stderr(&mut stderr_task).await;
            return Err(error.context(concise(&stderr)));
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let stderr = take_stderr(&mut stderr_task).await;
            bail!(
                "Codex did not start a thread within 20 seconds: {}",
                concise(&stderr)
            );
        }
    };

    let task_id = record_event(
        EventIdentity {
            state: &state,
            project: &project,
            title,
            session_id: &session_id,
        },
        "SessionStart",
        format!("codex-run:{session_id}:start"),
        json!({}),
    )?
    .id;
    let context = RunnerContext {
        state: state.clone(),
        project: project.clone(),
        title: title.to_owned(),
        session_id: session_id.clone(),
        timeout_seconds,
    };
    tokio::spawn(async move {
        finish_project(context, child, lines, stderr_task).await;
    });
    Ok(RunAssignment {
        project_id: project.id,
        project_name: project.name,
        task_id,
        session_id,
        state: "RUNNING",
    })
}

async fn find_thread_id(lines: &mut Lines<BufReader<ChildStdout>>) -> Result<String> {
    while let Some(line) = lines.next_line().await.context("read Codex event stream")? {
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) == Some("thread.started") {
            let id = event
                .get("thread_id")
                .or_else(|| event.get("threadId"))
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .context("Codex thread.started event did not contain a thread id")?;
            return Ok(id.to_owned());
        }
    }
    bail!("Codex event stream ended before thread.started")
}

#[derive(Debug, Default)]
struct StreamResult {
    turn_completed: bool,
    failure: Option<String>,
    final_message: Option<String>,
}

struct RunnerContext {
    state: AppState,
    project: CodexProject,
    title: String,
    session_id: String,
    timeout_seconds: u64,
}

async fn finish_project(
    context: RunnerContext,
    mut child: Child,
    mut lines: Lines<BufReader<ChildStdout>>,
    mut stderr_task: JoinHandle<String>,
) {
    let run = timeout(Duration::from_secs(context.timeout_seconds), async {
        let stream = read_stream(&mut lines).await?;
        let exit = child.wait().await.context("wait for Codex task")?;
        let stderr = take_stderr(&mut stderr_task).await;
        Ok::<_, anyhow::Error>((stream, exit, stderr))
    })
    .await;

    let (event_type, payload) = match run {
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            (
                "Failed",
                json!({
                    "error_code": "task_timeout",
                    "summary": format!("任务执行超过 {} 小时，已由中控台停止。", context.timeout_seconds / 3600)
                }),
            )
        }
        Ok(Err(error)) => (
            "Failed",
            json!({ "error_code": "codex_stream_error", "summary": public_error(&error) }),
        ),
        Ok(Ok((stream, exit, stderr))) if !exit.success() => (
            "Failed",
            json!({
                "exit_code": exit.code().unwrap_or(-1),
                "error_code": "codex_non_zero_exit",
                "summary": stream.failure.unwrap_or_else(|| concise(&stderr))
            }),
        ),
        Ok(Ok((stream, _, _))) if stream.failure.is_some() => (
            "Failed",
            json!({
                "error_code": "codex_turn_failed",
                "summary": stream.failure.unwrap_or_else(|| "Codex turn failed".to_owned())
            }),
        ),
        Ok(Ok((stream, _, _))) if stream.turn_completed => (
            "Result",
            json!({
                "exit_code": 0,
                "structured_success": true,
                "summary": stream.final_message.unwrap_or_else(|| "Codex 任务已完成。".to_owned())
            }),
        ),
        Ok(Ok(_)) => (
            "Failed",
            json!({
                "error_code": "missing_completion_event",
                "summary": "Codex 进程已退出，但没有收到 turn.completed 完成事件。"
            }),
        ),
    };
    if let Err(error) = record_event(
        EventIdentity {
            state: &context.state,
            project: &context.project,
            title: &context.title,
            session_id: &context.session_id,
        },
        event_type,
        format!("codex-run:{}:final", context.session_id),
        payload,
    ) {
        tracing::error!(session_id = %context.session_id, error = %error, "failed to record Codex task result");
    }
}

async fn read_stream(lines: &mut Lines<BufReader<ChildStdout>>) -> Result<StreamResult> {
    let mut result = StreamResult::default();
    while let Some(line) = lines.next_line().await.context("read Codex event stream")? {
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("turn.completed") => result.turn_completed = true,
            Some("turn.failed") | Some("error") => {
                result.failure =
                    event_message(&event).or_else(|| Some("Codex turn failed".to_owned()));
            }
            Some("item.completed") => {
                if let Some(item) = event.get("item")
                    && item.get("type").and_then(Value::as_str) == Some("agent_message")
                {
                    result.final_message = item
                        .get("text")
                        .or_else(|| item.get("content"))
                        .and_then(Value::as_str)
                        .map(|text| text.to_owned());
                }
            }
            _ => {}
        }
    }
    Ok(result)
}

fn event_message(event: &Value) -> Option<String> {
    event
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| event.pointer("/error/message").and_then(Value::as_str))
        .map(str::to_owned)
}

struct EventIdentity<'a> {
    state: &'a AppState,
    project: &'a CodexProject,
    title: &'a str,
    session_id: &'a str,
}

fn record_event(
    identity: EventIdentity<'_>,
    event_type: &str,
    idempotency_key: String,
    payload: Value,
) -> Result<TaskRecord> {
    let event = normalize_event(
        RawEventInput {
            provider: Provider::Codex,
            event_type: event_type.to_owned(),
            event_id: None,
            idempotency_key: Some(idempotency_key),
            device_id: Some(identity.state.device.id.clone()),
            session_id: identity.session_id.to_owned(),
            turn_id: None,
            occurred_at: Some(Utc::now()),
            title: Some(identity.title.to_owned()),
            workspace: Some(identity.project.name.clone()),
            project: Some(identity.project.id.clone()),
            control_mode: ControlMode::Managed,
            required_evidence_level: EvidenceLevel::E2,
            payload,
        },
        &identity.state.device.id,
    )?;
    Ok(identity.state.store.ingest_event(&event)?.task)
}

async fn read_limited<R>(reader: R) -> String
where
    R: AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    let _ = reader.take(64 * 1024).read_to_end(&mut bytes).await;
    String::from_utf8_lossy(&bytes).into_owned()
}

async fn take_stderr(task: &mut JoinHandle<String>) -> String {
    timeout(Duration::from_secs(2), task)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
}

fn public_error(error: &anyhow::Error) -> String {
    concise(&error.to_string())
}

fn concise(value: &str) -> String {
    let redacted = redact_text(value).text;
    let compact = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return "未提供错误详情".to_owned();
    }
    compact.chars().take(500).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ai_rpa_core::DeviceRecord;
    use ai_rpa_store::CryptoBox;
    use tempfile::TempDir;

    use super::*;

    fn test_store(directory: &TempDir) -> Store {
        Store::open(
            directory.path().join("test.db"),
            CryptoBox::from_key([8_u8; 32]),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn project_registry_is_canonical_and_idempotent() {
        let directory = TempDir::new().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(&project)
                .status()
                .unwrap()
                .success()
        );
        let store = test_store(&directory);
        let first = register(
            &store,
            RegisterProjectRequest {
                name: "Alpha".to_owned(),
                path: project.to_string_lossy().into_owned(),
            },
        )
        .await
        .unwrap();
        let second = register(
            &store,
            RegisterProjectRequest {
                name: "Renamed".to_owned(),
                path: project.join(".").to_string_lossy().into_owned(),
            },
        )
        .await
        .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(load_projects(&store).unwrap().len(), 1);
        assert_eq!(load_projects(&store).unwrap()[0].name, "Renamed");
        assert!(remove_project(&store, &first.id).unwrap());
        assert!(!remove_project(&store, &first.id).unwrap());
    }

    #[tokio::test]
    async fn rejects_non_git_project() {
        let directory = TempDir::new().unwrap();
        let store = test_store(&directory);
        let result = register(
            &store,
            RegisterProjectRequest {
                name: "No Git".to_owned(),
                path: directory.path().to_string_lossy().into_owned(),
            },
        )
        .await;
        assert!(result.unwrap_err().to_string().contains("Git worktree"));
    }

    #[test]
    fn run_request_allows_only_safe_timeouts_and_unique_projects() {
        let valid = StartRunsRequest {
            title: "Review".to_owned(),
            prompt: "Inspect the project".to_owned(),
            project_ids: vec!["one".to_owned(), "two".to_owned()],
            timeout_seconds: 3600,
            sandbox: CodexSandbox::ReadOnly,
        };
        assert!(validate_run_request(&valid).is_ok());
        let duplicate = StartRunsRequest {
            project_ids: vec!["one".to_owned(), "one".to_owned()],
            ..valid
        };
        assert!(validate_run_request(&duplicate).is_err());
    }

    #[test]
    fn records_only_structured_completion_as_success() {
        let directory = TempDir::new().unwrap();
        let store = test_store(&directory);
        let device = DeviceRecord {
            id: "device-1".to_owned(),
            os: "test".to_owned(),
            arch: "test".to_owned(),
            hostname: "test".to_owned(),
            logical_environment: "test".to_owned(),
            node_version: "test".to_owned(),
            last_seen_at: Utc::now(),
        };
        store.upsert_device(&device).unwrap();
        let state = AppState {
            store,
            device,
            started_at: Utc::now(),
            data_dir: Arc::new(directory.path().to_path_buf()),
            ui_dir: None,
        };
        let project = CodexProject {
            id: "codex-project".to_owned(),
            name: "Project".to_owned(),
            path: directory.path().to_path_buf(),
            created_at: Utc::now(),
        };
        record_event(
            EventIdentity {
                state: &state,
                project: &project,
                title: "Task",
                session_id: "session",
            },
            "SessionStart",
            "start".to_owned(),
            json!({}),
        )
        .unwrap();
        let result = record_event(
            EventIdentity {
                state: &state,
                project: &project,
                title: "Task",
                session_id: "session",
            },
            "Result",
            "result".to_owned(),
            json!({ "exit_code": 0, "structured_success": true }),
        )
        .unwrap();
        assert_eq!(result.state, ai_rpa_core::TaskState::Succeeded);
        assert_eq!(result.evidence_level, EvidenceLevel::E2);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_run_closes_stdin_and_reaches_structured_completion() {
        use std::os::unix::fs::PermissionsExt;

        let directory = TempDir::new().unwrap();
        let executable = directory.path().join("fake-codex");
        std::fs::write(
            &executable,
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"type\":\"thread.started\",\"thread_id\":\"fake-thread\"}' '{\"type\":\"turn.started\"}' '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"done\"}}' '{\"type\":\"turn.completed\"}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = test_store(&directory);
        let device = DeviceRecord {
            id: "device-1".to_owned(),
            os: "test".to_owned(),
            arch: "test".to_owned(),
            hostname: "test".to_owned(),
            logical_environment: "test".to_owned(),
            node_version: "test".to_owned(),
            last_seen_at: Utc::now(),
        };
        store.upsert_device(&device).unwrap();
        let state = AppState {
            store,
            device,
            started_at: Utc::now(),
            data_dir: Arc::new(directory.path().to_path_buf()),
            ui_dir: None,
        };
        let project = CodexProject {
            id: "codex-project".to_owned(),
            name: "Project".to_owned(),
            path: directory.path().to_path_buf(),
            created_at: Utc::now(),
        };
        let assignment = start_project(
            state.clone(),
            &executable,
            project,
            "Task",
            "Prompt",
            3600,
            CodexSandbox::ReadOnly,
        )
        .await
        .unwrap();
        for _ in 0..50 {
            let task = state
                .store
                .task_detail(assignment.task_id)
                .unwrap()
                .unwrap()
                .task;
            if task.state == ai_rpa_core::TaskState::Succeeded {
                assert_eq!(task.evidence_level, EvidenceLevel::E2);
                assert_eq!(task.evidence_summary.as_deref(), Some("done"));
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("managed Codex run did not complete");
    }
}
