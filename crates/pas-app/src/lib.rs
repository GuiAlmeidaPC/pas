use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pas_engine::{ColumnInfo, DatasetInfo, DatasetPage, Event, Library, Session};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};

mod oauth;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub libnames: Vec<ProjectLibname>,
    /// Files that belong to this project (regardless of whether they're
    /// currently open in the editor). Drives the Project tree.
    #[serde(default)]
    pub programs: Vec<TabConfig>,
    /// Snapshot of which programs are open in tabs right now. On project
    /// open we use this to restore the working set; on save we capture
    /// the current editor state.
    #[serde(default)]
    pub open_tabs: Vec<TabConfig>,
    #[serde(default)]
    pub active_tab: Option<String>,
    #[serde(default)]
    pub layout: Layout,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectLibname {
    pub name: String,
    /// "memory" | "duckdb" | "dir"
    pub kind: String,
    #[serde(default)]
    pub path: String,
    /// Only for `dir`: "parquet" | "csv"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabConfig {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Layout {
    #[serde(default)]
    pub sidebar_width: Option<u32>,
    #[serde(default)]
    pub bottom_height: Option<u32>,
    #[serde(default)]
    pub bottom_width: Option<u32>,
    #[serde(default)]
    pub orientation: Option<String>,
}

pub struct AppState {
    session: Arc<Session>,
    project_root: Mutex<Option<PathBuf>>,
    ai_config: Mutex<Option<StoredAiConfig>>,
    chatgpt_tokens: Mutex<Option<oauth::StoredTokens>>,
    allowed_paths: Mutex<HashSet<PathBuf>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfigInput {
    pub provider: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub custom_url: Option<String>,
    /// "api_key" (default) | "chatgpt". Only meaningful for the openai provider.
    #[serde(default)]
    pub auth_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCompletionRequest {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub custom_url: Option<String>,
    #[serde(default)]
    pub auth_mode: Option<String>,
    pub system_prompt: String,
    pub messages: Vec<AiMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AiStreamPayload {
    request_id: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAiConfig {
    provider: String,
    api_key: String,
    model: String,
    custom_url: Option<String>,
    auth_mode: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct SubmitEventPayload {
    submission_id: String,
    event: Event,
}

#[tauri::command]
async fn submit(
    program: String,
    submission_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<String, String> {
    let session = state.session.clone();
    let id = submission_id.clone();
    let app_for_panic = app.clone();
    let id_for_panic = submission_id.clone();

    // Run synchronously off the UI thread. DuckDB calls are blocking.
    let handle = tokio::task::spawn_blocking(move || {
        let events = session.submit(&program);
        for event in events {
            let _ = app.emit(
                "pas://event",
                SubmitEventPayload {
                    submission_id: id.clone(),
                    event,
                },
            );
        }
    });

    // Safety net: the engine already converts per-statement panics into Error
    // events, but if execution ever unwinds anyway the blocking task aborts
    // without emitting a terminal `Done`, leaving the UI spinning forever.
    // Watch the task and synthesize an error + `Done` so a run can always end.
    tokio::spawn(async move {
        if handle.await.is_err() {
            let _ = app_for_panic.emit(
                "pas://event",
                SubmitEventPayload {
                    submission_id: id_for_panic.clone(),
                    event: Event::Error {
                        text: "internal engine error: the engine panicked while running \
                               this program; results may be incomplete"
                            .to_string(),
                        source_span: None,
                    },
                },
            );
            let _ = app_for_panic.emit(
                "pas://event",
                SubmitEventPayload {
                    submission_id: id_for_panic,
                    event: Event::Done,
                },
            );
        }
    });

    Ok(submission_id)
}

#[tauri::command]
async fn submit_files(
    mut programs: Vec<TabConfig>,
    submission_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<String, String> {
    let project_root = state
        .project_root
        .lock()
        .map_err(|_| "project lock poisoned")?
        .clone();
    for prog in &mut programs {
        if prog.content.is_some() {
            continue;
        }
        let path = normalize_path(&prog.path)?;
        let resolved = if path.is_absolute() {
            path
        } else if let Some(root) = &project_root {
            root.join(path)
        } else {
            path
        };
        if let Some(root) = &project_root {
            let canonical_parent = canonical_existing_parent(&resolved)?;
            if !canonical_parent.starts_with(root) {
                return Err(format!(
                    "{} is outside the active project directory",
                    resolved.display()
                ));
            }
        }
        prog.path = resolved.to_string_lossy().to_string();
    }

    let session = state.session.clone();
    let id = submission_id.clone();

    tokio::task::spawn_blocking(move || {
        for prog in programs {
            let path = prog.path;
            let _ = app.emit(
                "pas://event",
                SubmitEventPayload {
                    submission_id: id.clone(),
                    event: Event::Note {
                        text: format!("Running file: {}", path),
                    },
                },
            );

            let content = if let Some(c) = prog.content {
                c
            } else {
                match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = app.emit(
                            "pas://event",
                            SubmitEventPayload {
                                submission_id: id.clone(),
                                event: Event::Error {
                                    text: format!("Failed to read {}: {}", path, e),
                                    source_span: None,
                                },
                            },
                        );
                        break;
                    }
                }
            };

            let events = session.submit(&content);
            let mut has_error = false;
            for event in events {
                if matches!(event, Event::Error { .. }) {
                    has_error = true;
                }
                // Don't emit individual Done events from inner submits,
                // we'll emit one at the very end.
                if !matches!(event, Event::Done) {
                    let _ = app.emit(
                        "pas://event",
                        SubmitEventPayload {
                            submission_id: id.clone(),
                            event,
                        },
                    );
                }
            }

            if has_error {
                let _ = app.emit(
                    "pas://event",
                    SubmitEventPayload {
                        submission_id: id.clone(),
                        event: Event::Note {
                            text: "Stopping execution due to error.".to_string(),
                        },
                    },
                );
                break;
            }
        }
        let _ = app.emit(
            "pas://event",
            SubmitEventPayload {
                submission_id: id,
                event: Event::Done,
            },
        );
    });

    Ok(submission_id)
}

#[tauri::command]
fn cancel(state: State<'_, AppState>) -> Result<(), String> {
    state.session.request_cancel();
    Ok(())
}

#[tauri::command]
fn engine_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
fn list_libraries(state: State<'_, AppState>) -> Vec<Library> {
    state.session.list_libraries()
}

#[tauri::command]
fn list_datasets(libref: String, state: State<'_, AppState>) -> Result<Vec<DatasetInfo>, String> {
    state
        .session
        .list_datasets(&libref)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn dataset_schema(
    libref: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<Vec<ColumnInfo>, String> {
    state
        .session
        .dataset_schema(&libref, &name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn dataset_page(
    libref: String,
    name: String,
    offset: u64,
    limit: u64,
    filters: Option<std::collections::HashMap<String, String>>,
    state: State<'_, AppState>,
) -> Result<DatasetPage, String> {
    state
        .session
        .dataset_page(&libref, &name, offset, limit, filters.as_ref())
        .map_err(|e| e.to_string())
}

fn normalize_path(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    if path.as_os_str().is_empty() {
        return Err("empty path".to_string());
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("paths containing '..' are not allowed".to_string());
    }
    Ok(path)
}

fn resolve_project_path(path: PathBuf, state: &State<'_, AppState>) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path);
    }
    let Some(root) = state
        .project_root
        .lock()
        .map_err(|_| "project lock poisoned")?
        .clone()
    else {
        return Ok(path);
    };
    Ok(root.join(path))
}

fn extension_is(path: &Path, allowed: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| allowed.iter().any(|a| ext.eq_ignore_ascii_case(a)))
}

fn canonical_existing_parent(path: &Path) -> Result<PathBuf, String> {
    let mut cur = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or_else(|| Path::new("")).to_path_buf()
    };
    loop {
        if cur.as_os_str().is_empty() {
            cur = PathBuf::from(".");
        }
        if cur.exists() {
            return cur
                .canonicalize()
                .map_err(|e| format!("canonicalize {}: {}", cur.display(), e));
        }
        if !cur.pop() {
            return Err(format!("no existing parent for {}", path.display()));
        }
    }
}

fn canonicalize_path(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        path.canonicalize()
            .map_err(|e| format!("canonicalize: {}", e))
    } else if let Some(parent) = path.parent() {
        let canonical_parent = parent
            .canonicalize()
            .map_err(|e| format!("canonicalize parent: {}", e))?;
        let filename = path.file_name().ok_or_else(|| "no file name".to_string())?;
        Ok(canonical_parent.join(filename))
    } else {
        path.canonicalize()
            .map_err(|e| format!("canonicalize: {}", e))
    }
}

fn ensure_under_project_root(path: &Path, state: &AppState) -> Result<(), String> {
    let canonical = canonicalize_path(path)?;

    // Check if explicitly allowed in allowed_paths first!
    {
        let allowed = state
            .allowed_paths
            .lock()
            .map_err(|_| "allowed paths lock poisoned")?;
        if allowed.contains(&canonical) {
            return Ok(());
        }
    }

    let Some(root) = state
        .project_root
        .lock()
        .map_err(|_| "project lock poisoned")?
        .clone()
    else {
        return Err(format!(
            "Access denied: path {} is not in allowed paths allowlist and no active project",
            path.display()
        ));
    };

    let canonical_parent = canonical_existing_parent(&canonical)?;
    if canonical_parent.starts_with(&root) {
        return Ok(());
    }

    Err(format!(
        "{} is outside the active project directory and not in allowed paths allowlist",
        path.display()
    ))
}

#[tauri::command]
fn read_file(path: String, state: State<'_, AppState>) -> Result<String, String> {
    let path = resolve_project_path(normalize_path(&path)?, &state)?;
    if !extension_is(&path, &["sas"]) {
        return Err("only .sas program files can be read with read_file".to_string());
    }
    ensure_under_project_root(&path, &state)?;
    let canonical = canonicalize_path(&path)?;
    std::fs::read_to_string(&canonical).map_err(|e| format!("{}: {}", canonical.display(), e))
}

#[tauri::command]
fn write_file(path: String, content: String, state: State<'_, AppState>) -> Result<(), String> {
    let path = resolve_project_path(normalize_path(&path)?, &state)?;
    if !extension_is(&path, &["sas"]) {
        return Err("only .sas program files can be written with write_file".to_string());
    }
    ensure_under_project_root(&path, &state)?;
    let canonical = canonicalize_path(&path)?;
    if let Some(parent) = canonical.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {}", parent.display(), e))?;
        }
    }
    std::fs::write(&canonical, content).map_err(|e| format!("{}: {}", canonical.display(), e))
}

#[tauri::command]
fn read_project(path: String, state: State<'_, AppState>) -> Result<ProjectConfig, String> {
    let path = normalize_path(&path)?;
    if !extension_is(&path, &["json"]) || !path.to_string_lossy().ends_with(".pas.json") {
        return Err("project files must use the .pas.json extension".to_string());
    }
    let canonical = canonicalize_path(&path)?;

    // Verify the project path itself is explicitly in the allowlist!
    {
        let allowed = state
            .allowed_paths
            .lock()
            .map_err(|_| "allowed paths lock poisoned")?;
        if !allowed.contains(&canonical) {
            return Err("Access denied: project file not in allowed paths allowlist".to_string());
        }
    }

    let text = std::fs::read_to_string(&canonical)
        .map_err(|e| format!("{}: {}", canonical.display(), e))?;
    let project: ProjectConfig =
        serde_json::from_str(&text).map_err(|e| format!("parse {}: {}", canonical.display(), e))?;
    let root = canonical
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .canonicalize()
        .map_err(|e| format!("canonicalize project directory: {}", e))?;

    *state
        .project_root
        .lock()
        .map_err(|_| "project lock poisoned")? = Some(root.clone());

    // Register project file, project root, and all sub-programs inside allowed_paths
    {
        let mut allowed = state
            .allowed_paths
            .lock()
            .map_err(|_| "allowed paths lock poisoned")?;
        allowed.insert(canonical);
        allowed.insert(root.clone());
        for p in &project.programs {
            if let Ok(p_buf) = normalize_path(&p.path) {
                let abs = if p_buf.is_absolute() {
                    p_buf
                } else {
                    root.join(p_buf)
                };
                if let Ok(canon) = canonicalize_path(&abs) {
                    allowed.insert(canon);
                }
            }
        }
        for t in &project.open_tabs {
            if let Ok(p_buf) = normalize_path(&t.path) {
                let abs = if p_buf.is_absolute() {
                    p_buf
                } else {
                    root.join(p_buf)
                };
                if let Ok(canon) = canonicalize_path(&abs) {
                    allowed.insert(canon);
                }
            }
        }
    }

    Ok(project)
}

#[tauri::command]
fn save_project(
    path: String,
    project: ProjectConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let path = normalize_path(&path)?;
    if !extension_is(&path, &["json"]) || !path.to_string_lossy().ends_with(".pas.json") {
        return Err("project files must use the .pas.json extension".to_string());
    }
    let canonical = canonicalize_path(&path)?;

    // Verify the project path itself is explicitly in the allowlist!
    {
        let allowed = state
            .allowed_paths
            .lock()
            .map_err(|_| "allowed paths lock poisoned")?;
        if !allowed.contains(&canonical) {
            return Err("Access denied: project file not in allowed paths allowlist".to_string());
        }
    }

    let text = serde_json::to_string_pretty(&project).map_err(|e| e.to_string())?;
    if let Some(parent) = canonical.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {}", parent.display(), e))?;
        }
    }
    std::fs::write(&canonical, text).map_err(|e| format!("{}: {}", canonical.display(), e))?;
    let root = canonical
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .canonicalize()
        .map_err(|e| format!("canonicalize project directory: {}", e))?;

    *state
        .project_root
        .lock()
        .map_err(|_| "project lock poisoned")? = Some(root.clone());

    // Register project file, project root, and all sub-programs inside allowed_paths
    {
        let mut allowed = state
            .allowed_paths
            .lock()
            .map_err(|_| "allowed paths lock poisoned")?;
        allowed.insert(canonical);
        allowed.insert(root.clone());
        for p in &project.programs {
            if let Ok(p_buf) = normalize_path(&p.path) {
                let abs = if p_buf.is_absolute() {
                    p_buf
                } else {
                    root.join(p_buf)
                };
                if let Ok(canon) = canonicalize_path(&abs) {
                    allowed.insert(canon);
                }
            }
        }
        for t in &project.open_tabs {
            if let Ok(p_buf) = normalize_path(&t.path) {
                let abs = if p_buf.is_absolute() {
                    p_buf
                } else {
                    root.join(p_buf)
                };
                if let Ok(canon) = canonicalize_path(&abs) {
                    allowed.insert(canon);
                }
            }
        }
    }

    Ok(())
}

#[tauri::command]
fn apply_project_libnames(
    libnames: Vec<ProjectLibname>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<String, String> {
    // Synthesize SAS libname statements and run them through submit().
    let mut prog = String::new();
    for l in &libnames {
        let path = l.path.replace('\'', "''");
        let kind = match l.kind.as_str() {
            "duckdb" => "duckdb ",
            "dir" => "dir ",
            "memory" => continue, // WORK is implicit
            other => return Err(format!("unknown libname kind: {}", other)),
        };
        let fmt = match &l.format {
            Some(f) => format!(" format={}", f),
            None => String::new(),
        };
        prog.push_str(&format!("libname {} {}'{}'{};\n", l.name, kind, path, fmt));
    }
    if prog.is_empty() {
        return Ok(String::new());
    }
    let submission_id = uuid::Uuid::new_v4().to_string();
    let id = submission_id.clone();
    let session = state.session.clone();
    tokio::task::spawn_blocking(move || {
        let events = session.submit(&prog);
        for event in events {
            let _ = app.emit(
                "pas://event",
                SubmitEventPayload {
                    submission_id: id.clone(),
                    event,
                },
            );
        }
    });
    Ok(submission_id)
}

#[tauri::command]
fn dataset_page_arrow(
    libref: String,
    name: String,
    offset: u64,
    limit: u64,
    filters: Option<std::collections::HashMap<String, String>>,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    let bytes = state
        .session
        .dataset_page_arrow(&libref, &name, offset, limit, filters.as_ref())
        .map_err(|e| e.to_string())?;
    Ok(tauri::ipc::Response::new(bytes))
}

fn validate_ai_provider(provider: &str) -> Result<(), String> {
    match provider {
        "openai" | "anthropic" | "gemini" | "deepseek" | "openrouter" => Ok(()),
        other => Err(format!("unsupported AI provider: {}", other)),
    }
}

fn validate_https_url(url: &str) -> Result<(), String> {
    if url.starts_with("https://") {
        Ok(())
    } else {
        Err("AI endpoints must use https://".to_string())
    }
}

#[tauri::command]
fn set_ai_config(
    config: AiConfigInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_ai_provider(&config.provider)?;
    let is_chatgpt = config.provider == "openai" && config.auth_mode.as_deref() == Some("chatgpt");
    if !is_chatgpt && config.api_key.trim().is_empty() {
        return Err("API key is required".to_string());
    }
    if config.model.trim().is_empty() {
        return Err("model is required".to_string());
    }
    if let Some(url) = config
        .custom_url
        .as_deref()
        .filter(|u| !u.trim().is_empty())
    {
        validate_https_url(url)?;
    }
    let stored = StoredAiConfig {
        provider: config.provider,
        api_key: config.api_key,
        model: config.model,
        custom_url: config.custom_url.filter(|u| !u.trim().is_empty()),
        auth_mode: config.auth_mode,
    };
    *state
        .ai_config
        .lock()
        .map_err(|_| "AI config lock poisoned")? = Some(stored.clone());
    save_ai_config_to_disk(&app, &stored)?;
    Ok(())
}

#[tauri::command]
fn clear_ai_config(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    *state
        .ai_config
        .lock()
        .map_err(|_| "AI config lock poisoned")? = None;
    delete_ai_config_from_disk(&app);
    Ok(())
}

#[tauri::command]
fn get_ai_config(state: State<'_, AppState>) -> Result<Option<AiConfigInput>, String> {
    let guard = state
        .ai_config
        .lock()
        .map_err(|_| "AI config lock poisoned")?;
    Ok(guard.as_ref().map(|s| AiConfigInput {
        provider: s.provider.clone(),
        api_key: s.api_key.clone(),
        model: s.model.clone(),
        custom_url: s.custom_url.clone(),
        auth_mode: s.auth_mode.clone(),
    }))
}

fn chatgpt_token_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("chatgpt_tokens.enc"))
}

fn load_chatgpt_tokens(app: &AppHandle) -> Option<oauth::StoredTokens> {
    let path = chatgpt_token_path(app).ok()?;
    let blob = std::fs::read(&path).ok()?;
    let key = oauth::derive_key(path.parent()?);
    oauth::decrypt_tokens(&key, &blob).ok()
}

fn save_chatgpt_tokens(app: &AppHandle, tokens: &oauth::StoredTokens) -> Result<(), String> {
    let path = chatgpt_token_path(app)?;
    let key = oauth::derive_key(path.parent().ok_or("no parent dir for token file")?);
    let blob = oauth::encrypt_tokens(&key, tokens)?;
    std::fs::write(&path, blob).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// AI config persistence (encrypted at rest)
// ---------------------------------------------------------------------------

fn ai_config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("ai_config.enc"))
}

fn encrypt_ai_config(key: &[u8; 32], config: &StoredAiConfig) -> Result<Vec<u8>, String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    use rand::RngCore;
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let plaintext = serde_json::to_vec(config).map_err(|e| e.to_string())?;
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
        .map_err(|e| format!("encrypt: {e}"))?;
    let mut out = nonce_bytes.to_vec();
    out.extend_from_slice(&ct);
    Ok(out)
}

fn decrypt_ai_config(key: &[u8; 32], blob: &[u8]) -> Option<StoredAiConfig> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    if blob.len() < 12 {
        return None;
    }
    let (nonce_bytes, ct) = blob.split_at(12);
    let cipher = Aes256Gcm::new(key.into());
    let plaintext = cipher.decrypt(Nonce::from_slice(nonce_bytes), ct).ok()?;
    serde_json::from_slice(&plaintext).ok()
}

fn save_ai_config_to_disk(app: &AppHandle, config: &StoredAiConfig) -> Result<(), String> {
    let path = ai_config_path(app)?;
    let key = oauth::derive_key(path.parent().ok_or("no parent dir for ai config file")?);
    let blob = encrypt_ai_config(&key, config)?;
    std::fs::write(&path, blob).map_err(|e| e.to_string())
}

fn load_ai_config_from_disk(app: &AppHandle) -> Option<StoredAiConfig> {
    let path = ai_config_path(app).ok()?;
    let key = oauth::derive_key(path.parent()?);
    let blob = std::fs::read(&path).ok()?;
    decrypt_ai_config(&key, &blob)
}

fn delete_ai_config_from_disk(app: &AppHandle) {
    if let Ok(path) = ai_config_path(app) {
        let _ = std::fs::remove_file(path);
    }
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OauthStatus {
    signed_in: bool,
    email: Option<String>,
}

#[tauri::command]
async fn openai_oauth_login(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<OauthStatus, String> {
    let tokens = oauth::interactive_login().await?;
    save_chatgpt_tokens(&app, &tokens)?;
    let status = OauthStatus {
        signed_in: true,
        email: tokens.email.clone(),
    };
    *state
        .chatgpt_tokens
        .lock()
        .map_err(|_| "token lock poisoned")? = Some(tokens);
    Ok(status)
}

#[tauri::command]
fn openai_oauth_status(state: State<'_, AppState>) -> Result<OauthStatus, String> {
    let guard = state
        .chatgpt_tokens
        .lock()
        .map_err(|_| "token lock poisoned")?;
    Ok(match guard.as_ref() {
        Some(t) => OauthStatus {
            signed_in: true,
            email: t.email.clone(),
        },
        None => OauthStatus {
            signed_in: false,
            email: None,
        },
    })
}

#[tauri::command]
fn openai_oauth_logout(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    *state
        .chatgpt_tokens
        .lock()
        .map_err(|_| "token lock poisoned")? = None;
    if let Ok(path) = chatgpt_token_path(&app) {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

#[tauri::command]
async fn ai_completion(
    request: AiCompletionRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    validate_ai_provider(&request.provider)?;

    // ChatGPT OAuth path: use the Codex Responses API with the stored token.
    if request.provider == "openai" && request.auth_mode.as_deref() == Some("chatgpt") {
        let mut tokens = state
            .chatgpt_tokens
            .lock()
            .map_err(|_| "token lock poisoned")?
            .clone()
            .ok_or("Not signed in with ChatGPT")?;
        let client = reqwest::Client::new();
        let access = oauth::valid_access_token(&mut tokens, &client).await?;
        save_chatgpt_tokens(&app, &tokens)?;
        *state
            .chatgpt_tokens
            .lock()
            .map_err(|_| "token lock poisoned")? = Some(tokens.clone());
        let model = if request.model.trim().is_empty() {
            "gpt-5.5"
        } else {
            request.model.as_str()
        };
        let body = oauth::build_responses_body(model, &request.system_prompt, &request.messages);
        return oauth::responses_completion(&client, &access, &tokens.account_id, &body).await;
    }

    let stored = state
        .ai_config
        .lock()
        .map_err(|_| "AI config lock poisoned")?
        .clone()
        .ok_or_else(|| "AI Setup required".to_string())?;

    if stored.provider != request.provider {
        return Err("saved AI provider does not match the request".to_string());
    }

    let model = if request.model.trim().is_empty() {
        stored.model.as_str()
    } else {
        request.model.as_str()
    };
    let custom_url = request
        .custom_url
        .as_deref()
        .filter(|u| !u.trim().is_empty())
        .or(stored.custom_url.as_deref());
    if let Some(url) = custom_url {
        validate_https_url(url)?;
    }

    let (url, headers, body) = build_ai_request(
        &stored.provider,
        &stored.api_key,
        model,
        custom_url,
        &request.system_prompt,
        &request.messages,
    )?;
    let client = reqwest::Client::new();
    let res = client
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("AI request failed: {}", e))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| format!("AI response read failed: {}", e))?;
    if !status.is_success() {
        return Err(format!(
            "API Error ({}): {}",
            status.as_u16(),
            parse_ai_error(&text)
        ));
    }
    parse_ai_response(&stored.provider, &text)
}

#[tauri::command]
async fn ai_completion_stream(
    request: AiCompletionRequest,
    request_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let emit_chunk = |text: String| {
        let _ = app.emit(
            "pas://ai-stream",
            AiStreamPayload {
                request_id: request_id.clone(),
                kind: "chunk",
                text: Some(text),
                message: None,
            },
        );
    };

    let result = ai_completion_stream_inner(request, &app, state, emit_chunk).await;
    match &result {
        Ok(_) => {
            let _ = app.emit(
                "pas://ai-stream",
                AiStreamPayload {
                    request_id,
                    kind: "done",
                    text: None,
                    message: None,
                },
            );
        }
        Err(message) => {
            let _ = app.emit(
                "pas://ai-stream",
                AiStreamPayload {
                    request_id,
                    kind: "error",
                    text: None,
                    message: Some(message.clone()),
                },
            );
        }
    }
    result
}

async fn ai_completion_stream_inner<F>(
    request: AiCompletionRequest,
    app: &AppHandle,
    state: State<'_, AppState>,
    mut emit_chunk: F,
) -> Result<String, String>
where
    F: FnMut(String),
{
    validate_ai_provider(&request.provider)?;

    if request.provider == "openai" && request.auth_mode.as_deref() == Some("chatgpt") {
        let mut tokens = state
            .chatgpt_tokens
            .lock()
            .map_err(|_| "token lock poisoned")?
            .clone()
            .ok_or("Not signed in with ChatGPT")?;
        let client = reqwest::Client::new();
        let access = oauth::valid_access_token(&mut tokens, &client).await?;
        save_chatgpt_tokens(app, &tokens)?;
        *state
            .chatgpt_tokens
            .lock()
            .map_err(|_| "token lock poisoned")? = Some(tokens.clone());
        let model = if request.model.trim().is_empty() {
            "gpt-5.5"
        } else {
            request.model.as_str()
        };
        let body = oauth::build_responses_body(model, &request.system_prompt, &request.messages);
        return oauth::responses_completion_stream(
            &client,
            &access,
            &tokens.account_id,
            &body,
            emit_chunk,
        )
        .await;
    }

    let stored = state
        .ai_config
        .lock()
        .map_err(|_| "AI config lock poisoned")?
        .clone()
        .ok_or_else(|| "AI Setup required".to_string())?;

    if stored.provider != request.provider {
        return Err("saved AI provider does not match the request".to_string());
    }

    let model = if request.model.trim().is_empty() {
        stored.model.as_str()
    } else {
        request.model.as_str()
    };
    let custom_url = request
        .custom_url
        .as_deref()
        .filter(|u| !u.trim().is_empty())
        .or(stored.custom_url.as_deref());
    if let Some(url) = custom_url {
        validate_https_url(url)?;
    }

    if stored.provider == "gemini" {
        let text = ai_completion(request, app.clone(), state).await?;
        emit_chunk(text.clone());
        return Ok(text);
    }

    let (url, headers, body) = build_ai_stream_request(
        &stored.provider,
        &stored.api_key,
        model,
        custom_url,
        &request.system_prompt,
        &request.messages,
    )?;
    let client = reqwest::Client::new();
    let mut res = client
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("AI request failed: {}", e))?;
    let status = res.status();
    if !status.is_success() {
        let text = res
            .text()
            .await
            .map_err(|e| format!("AI response read failed: {}", e))?;
        return Err(format!(
            "API Error ({}): {}",
            status.as_u16(),
            parse_ai_error(&text)
        ));
    }

    let mut buffer = String::new();
    let mut out = String::new();
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|e| format!("AI stream read failed: {}", e))?
    {
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        drain_sse_events(&stored.provider, &mut buffer, |delta| {
            out.push_str(delta);
            emit_chunk(delta.to_string());
        })?;
    }
    drain_sse_events(&stored.provider, &mut buffer, |delta| {
        out.push_str(delta);
        emit_chunk(delta.to_string());
    })?;
    if out.is_empty() {
        return Err("AI response did not contain text".to_string());
    }
    Ok(out)
}

fn build_ai_request(
    provider: &str,
    api_key: &str,
    model: &str,
    custom_url: Option<&str>,
    system_prompt: &str,
    history: &[AiMessage],
) -> Result<(String, HeaderMap, serde_json::Value), String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let bearer = HeaderValue::from_str(&format!("Bearer {}", api_key))
        .map_err(|e| format!("invalid API key header: {}", e))?;

    let url;
    let body;
    match provider {
        "openai" | "deepseek" | "openrouter" => {
            url = custom_url
                .unwrap_or(match provider {
                    "deepseek" => "https://api.deepseek.com/v1/chat/completions",
                    "openrouter" => "https://openrouter.ai/api/v1/chat/completions",
                    _ => "https://api.openai.com/v1/chat/completions",
                })
                .to_string();
            headers.insert(reqwest::header::AUTHORIZATION, bearer);
            if provider == "openrouter" {
                headers.insert(
                    HeaderName::from_static("http-referer"),
                    HeaderValue::from_static("https://pas.app"),
                );
                headers.insert(
                    HeaderName::from_static("x-title"),
                    HeaderValue::from_static("PAS"),
                );
            }
            let messages: Vec<_> = std::iter::once(json!({
                "role": "system",
                "content": system_prompt,
            }))
            .chain(history.iter().map(|m| {
                json!({
                    "role": m.role,
                    "content": m.content,
                })
            }))
            .collect();
            body = json!({ "model": model, "messages": messages });
        }
        "anthropic" => {
            url = custom_url
                .unwrap_or("https://api.anthropic.com/v1/messages")
                .to_string();
            let api_key = HeaderValue::from_str(api_key)
                .map_err(|e| format!("invalid API key header: {}", e))?;
            headers.insert(HeaderName::from_static("x-api-key"), api_key);
            headers.insert(
                HeaderName::from_static("anthropic-version"),
                HeaderValue::from_static("2023-06-01"),
            );
            let messages: Vec<_> = history
                .iter()
                .map(|m| {
                    json!({
                        "role": if m.role == "assistant" { "assistant" } else { "user" },
                        "content": m.content,
                    })
                })
                .collect();
            body = json!({
                "model": model,
                "max_tokens": 4096,
                "system": system_prompt,
                "messages": messages,
            });
        }
        "gemini" => {
            let escaped_model = model.replace('/', "%2F");
            url = custom_url
                .map(str::to_string)
                .unwrap_or_else(|| {
                    format!(
                        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                        escaped_model, api_key
                    )
                });
            let mut contents = vec![
                json!({
                    "role": "user",
                    "parts": [{ "text": format!("{}\n\nUnderstood. Please prompt me for the code task.", system_prompt) }],
                }),
                json!({
                    "role": "model",
                    "parts": [{ "text": "Understood. I will act as a SAS/PAS programming assistant." }],
                }),
            ];
            contents.extend(history.iter().map(|m| {
                json!({
                    "role": if m.role == "user" { "user" } else { "model" },
                    "parts": [{ "text": m.content }],
                })
            }));
            body = json!({ "contents": contents });
        }
        other => return Err(format!("unsupported AI provider: {}", other)),
    }
    Ok((url, headers, body))
}

fn build_ai_stream_request(
    provider: &str,
    api_key: &str,
    model: &str,
    custom_url: Option<&str>,
    system_prompt: &str,
    history: &[AiMessage],
) -> Result<(String, HeaderMap, serde_json::Value), String> {
    let (url, headers, mut body) =
        build_ai_request(provider, api_key, model, custom_url, system_prompt, history)?;
    match provider {
        "openai" | "deepseek" | "openrouter" | "anthropic" => {
            body["stream"] = json!(true);
        }
        _ => {}
    }
    Ok((url, headers, body))
}

fn drain_sse_events<F>(provider: &str, buffer: &mut String, mut on_delta: F) -> Result<(), String>
where
    F: FnMut(&str),
{
    while let Some(idx) = buffer.find("\n\n") {
        let event = buffer[..idx].to_string();
        buffer.replace_range(..idx + 2, "");
        parse_sse_event(provider, &event, &mut on_delta)?;
    }
    if !buffer.trim().is_empty() && !buffer.contains('\n') {
        return Ok(());
    }
    if buffer.ends_with('\n') {
        let event = std::mem::take(buffer);
        parse_sse_event(provider, &event, &mut on_delta)?;
    }
    Ok(())
}

fn parse_sse_event<F>(provider: &str, event: &str, on_delta: &mut F) -> Result<(), String>
where
    F: FnMut(&str),
{
    for line in event.lines() {
        let line = line.trim_start();
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(data).map_err(|e| format!("parse AI stream event: {}", e))?;
        if let Some(delta) = ai_stream_delta(provider, &v) {
            on_delta(delta);
        }
    }
    Ok(())
}

fn ai_stream_delta<'a>(provider: &str, v: &'a serde_json::Value) -> Option<&'a str> {
    match provider {
        "openai" | "deepseek" | "openrouter" => v
            .pointer("/choices/0/delta/content")
            .and_then(|x| x.as_str())
            .or_else(|| v.get("delta").and_then(|x| x.as_str())),
        "anthropic" => v
            .pointer("/delta/text")
            .and_then(|x| x.as_str())
            .or_else(|| {
                v.pointer("/content_block/delta/text")
                    .and_then(|x| x.as_str())
            }),
        _ => None,
    }
}

fn parse_ai_error(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|json| {
            json.pointer("/error/message")
                .or_else(|| json.get("message"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| text.to_string())
}

fn parse_ai_response(provider: &str, text: &str) -> Result<String, String> {
    let data: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("parse AI response: {}", e))?;
    let out = match provider {
        "openai" | "deepseek" | "openrouter" => data
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str()),
        "anthropic" => data.pointer("/content/0/text").and_then(|v| v.as_str()),
        "gemini" => data
            .pointer("/candidates/0/content/parts/0/text")
            .and_then(|v| v.as_str()),
        _ => None,
    };
    out.map(str::to_string)
        .ok_or_else(|| "AI response did not contain text".to_string())
}

/// Build the request to a provider's model-list endpoint.
fn build_models_request(
    provider: &str,
    api_key: &str,
    custom_url: Option<&str>,
) -> Result<(String, HeaderMap), String> {
    let mut headers = HeaderMap::new();
    let url = match provider {
        "openai" | "deepseek" | "openrouter" => {
            let bearer = HeaderValue::from_str(&format!("Bearer {}", api_key))
                .map_err(|e| format!("invalid API key header: {}", e))?;
            headers.insert(reqwest::header::AUTHORIZATION, bearer);
            // Derive the models endpoint from a custom completions URL when given.
            custom_url
                .and_then(|u| u.strip_suffix("/chat/completions"))
                .map(|base| format!("{base}/models"))
                .unwrap_or_else(|| {
                    match provider {
                        "deepseek" => "https://api.deepseek.com/v1/models",
                        "openrouter" => "https://openrouter.ai/api/v1/models",
                        _ => "https://api.openai.com/v1/models",
                    }
                    .to_string()
                })
        }
        "anthropic" => {
            let key = HeaderValue::from_str(api_key)
                .map_err(|e| format!("invalid API key header: {}", e))?;
            headers.insert(HeaderName::from_static("x-api-key"), key);
            headers.insert(
                HeaderName::from_static("anthropic-version"),
                HeaderValue::from_static("2023-06-01"),
            );
            "https://api.anthropic.com/v1/models".to_string()
        }
        "gemini" => format!(
            "https://generativelanguage.googleapis.com/v1beta/models?key={}",
            api_key
        ),
        other => return Err(format!("unsupported AI provider: {}", other)),
    };
    Ok((url, headers))
}

/// Heuristic: keep models usable for chat, drop embeddings/audio/image/etc.
fn is_chat_model(provider: &str, id: &str) -> bool {
    let l = id.to_lowercase();
    const EXCLUDE: &[&str] = &[
        "embedding",
        "whisper",
        "tts",
        "dall-e",
        "moderation",
        "audio",
        "realtime",
        "transcribe",
        "image",
        "-search",
        "guard",
    ];
    if EXCLUDE.iter().any(|e| l.contains(e)) {
        return false;
    }
    match provider {
        "openai" => {
            (l.starts_with("gpt")
                || l.starts_with("chatgpt")
                || l.starts_with("o1")
                || l.starts_with("o3")
                || l.starts_with("o4"))
                && !l.contains("instruct")
        }
        _ => true,
    }
}

/// Extract chat-capable model ids from a provider's model-list response.
fn parse_models_response(provider: &str, text: &str) -> Result<Vec<String>, String> {
    let data: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("parse models response: {}", e))?;
    let mut ids: Vec<String> = match provider {
        "openai" | "deepseek" | "openrouter" | "anthropic" => data
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        "gemini" => data
            .get("models")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|m| {
                        m.get("supportedGenerationMethods")
                            .and_then(|v| v.as_array())
                            .map(|ms| ms.iter().any(|x| x.as_str() == Some("generateContent")))
                            .unwrap_or(true)
                    })
                    .filter_map(|m| {
                        m.get("name")
                            .and_then(|v| v.as_str())
                            .map(|n| n.strip_prefix("models/").unwrap_or(n).to_string())
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    ids.retain(|id| is_chat_model(provider, id));
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// List the chat models available to the stored credential. The UI falls back
/// to its curated list when this errors (offline, no key, unsupported).
#[tauri::command]
async fn list_ai_models(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let stored = state
        .ai_config
        .lock()
        .map_err(|_| "AI config lock poisoned")?
        .clone()
        .ok_or_else(|| "AI Setup required".to_string())?;
    if stored.api_key.trim().is_empty() {
        return Err("no API key for model listing".to_string());
    }
    let (url, headers) = build_models_request(
        &stored.provider,
        &stored.api_key,
        stored.custom_url.as_deref(),
    )?;
    let client = reqwest::Client::new();
    let res = client
        .get(url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("model list request failed: {}", e))?;
    let status = res.status();
    let text = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "model list error ({}): {}",
            status.as_u16(),
            parse_ai_error(&text)
        ));
    }
    parse_models_response(&stored.provider, &text)
}

#[tauri::command]
async fn pick_project_file(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let file_path = app
        .dialog()
        .file()
        .add_filter("PAS Project", &["pas.json", "json"])
        .blocking_pick_file();
    if let Some(fp) = file_path {
        let path = fp.into_path().map_err(|e| e.to_string())?;
        let canonical = canonicalize_path(&path)?;
        state
            .allowed_paths
            .lock()
            .map_err(|_| "allowed paths lock poisoned")?
            .insert(canonical.clone());
        Ok(Some(canonical.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}

#[tauri::command]
async fn pick_sas_file(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let file_path = app
        .dialog()
        .file()
        .add_filter("SAS", &["sas"])
        .add_filter("All files", &["*"])
        .blocking_pick_file();
    if let Some(fp) = file_path {
        let path = fp.into_path().map_err(|e| e.to_string())?;
        let canonical = canonicalize_path(&path)?;
        state
            .allowed_paths
            .lock()
            .map_err(|_| "allowed paths lock poisoned")?
            .insert(canonical.clone());
        Ok(Some(canonical.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}

#[tauri::command]
async fn pick_save_sas_file(
    app: AppHandle,
    state: State<'_, AppState>,
    default_path: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let mut builder = app.dialog().file().add_filter("SAS", &["sas"]);
    if let Some(dp) = default_path {
        builder = builder.set_file_name(dp);
    }
    let file_path = builder.blocking_save_file();
    if let Some(fp) = file_path {
        let path = fp.into_path().map_err(|e| e.to_string())?;
        let canonical = canonicalize_path(&path)?;
        state
            .allowed_paths
            .lock()
            .map_err(|_| "allowed paths lock poisoned")?
            .insert(canonical.clone());
        Ok(Some(canonical.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}

#[tauri::command]
async fn pick_save_project_file(
    app: AppHandle,
    state: State<'_, AppState>,
    default_path: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let mut builder = app
        .dialog()
        .file()
        .add_filter("PAS Project", &["pas.json", "json"]);
    if let Some(dp) = default_path {
        builder = builder.set_file_name(dp);
    }
    let file_path = builder.blocking_save_file();
    if let Some(fp) = file_path {
        let path = fp.into_path().map_err(|e| e.to_string())?;
        let canonical = canonicalize_path(&path)?;
        state
            .allowed_paths
            .lock()
            .map_err(|_| "allowed paths lock poisoned")?
            .insert(canonical.clone());
        Ok(Some(canonical.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::try_init().ok();

    let session = Session::new_in_memory().expect("create engine");
    let state = AppState {
        session: Arc::new(session),
        project_root: Mutex::new(None),
        ai_config: Mutex::new(None),
        chatgpt_tokens: Mutex::new(None),
        allowed_paths: Mutex::new(HashSet::new()),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .setup(|app| {
            #[cfg(debug_assertions)]
            if let Some(window) = app.get_webview_window("main") {
                window.open_devtools();
            }
            // Restore a previously saved ChatGPT OAuth session, if any.
            if let Some(tokens) = load_chatgpt_tokens(app.handle()) {
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(mut g) = state.chatgpt_tokens.lock() {
                        *g = Some(tokens);
                    }
                }
            }
            // Restore a previously saved AI config (including API key), if any.
            if let Some(cfg) = load_ai_config_from_disk(app.handle()) {
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(mut g) = state.ai_config.lock() {
                        *g = Some(cfg);
                    }
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            submit,
            cancel,
            engine_version,
            list_libraries,
            list_datasets,
            dataset_schema,
            dataset_page,
            dataset_page_arrow,
            read_file,
            write_file,
            read_project,
            save_project,
            submit_files,
            apply_project_libnames,
            set_ai_config,
            clear_ai_config,
            get_ai_config,
            ai_completion,
            ai_completion_stream,
            list_ai_models,
            openai_oauth_login,
            openai_oauth_status,
            openai_oauth_logout,
            pick_project_file,
            pick_sas_file,
            pick_save_sas_file,
            pick_save_project_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running PAS");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn parses_openai_models_and_filters_non_chat() {
        let body = r#"{"data":[
            {"id":"gpt-4o"},
            {"id":"gpt-3.5-turbo-instruct"},
            {"id":"text-embedding-3-small"},
            {"id":"o3-mini"},
            {"id":"dall-e-3"},
            {"id":"gpt-4o-audio-preview"}
        ]}"#;
        let models = parse_models_response("openai", body).unwrap();
        assert_eq!(models, vec!["gpt-4o".to_string(), "o3-mini".to_string()]);
    }

    #[test]
    fn parses_gemini_models_by_generate_content() {
        let body = r#"{"models":[
            {"name":"models/gemini-1.5-pro","supportedGenerationMethods":["generateContent"]},
            {"name":"models/embedding-001","supportedGenerationMethods":["embedContent"]}
        ]}"#;
        let models = parse_models_response("gemini", body).unwrap();
        assert_eq!(models, vec!["gemini-1.5-pro".to_string()]);
    }

    #[test]
    fn builds_models_url_from_custom_completions_url() {
        let (url, _) = build_models_request(
            "openai",
            "k",
            Some("https://proxy.example.com/v1/chat/completions"),
        )
        .unwrap();
        assert_eq!(url, "https://proxy.example.com/v1/models");
    }

    #[test]
    fn stream_request_enables_openai_compatible_streaming() {
        let (_, _, body) = build_ai_stream_request(
            "openai",
            "k",
            "gpt-4o",
            None,
            "system",
            &[AiMessage {
                role: "user".into(),
                content: "hello".into(),
            }],
        )
        .unwrap();
        assert_eq!(body.get("stream").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn parses_provider_stream_deltas() {
        let openai = serde_json::json!({
            "choices": [{ "delta": { "content": "hello" } }]
        });
        assert_eq!(ai_stream_delta("openai", &openai), Some("hello"));

        let anthropic = serde_json::json!({
            "type": "content_block_delta",
            "delta": { "type": "text_delta", "text": "world" }
        });
        assert_eq!(ai_stream_delta("anthropic", &anthropic), Some("world"));
    }

    #[test]
    fn drains_sse_events_across_chunk_boundaries() {
        let mut buffer = "data: {\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\n".to_string();
        buffer.push_str("data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n");
        let mut out = String::new();
        drain_sse_events("openai", &mut buffer, |delta| out.push_str(delta)).unwrap();
        assert_eq!(out, "hello");
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_canonicalize_path() {
        let temp_dir = std::env::temp_dir().join(format!("pas_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        let file_path = temp_dir.join("test.sas");

        // Non-existent file but existing parent
        let res = canonicalize_path(&file_path);
        assert!(res.is_ok());
        let canonical = res.unwrap();
        assert_eq!(canonical.file_name().unwrap(), "test.sas");

        // Non-existent parent directory traversal should fail
        let bad_path = Path::new("/nonexistent_dir_123_xyz/../test.sas");
        let res_bad = canonicalize_path(bad_path);
        assert!(res_bad.is_err());

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_ensure_under_project_root() {
        let temp_dir = std::env::temp_dir().join(format!("pas_test_{}", uuid::Uuid::new_v4()));
        let root_path = temp_dir.join("my_project");
        fs::create_dir_all(&root_path).unwrap();
        let canonical_root = root_path.canonicalize().unwrap();

        let session = Arc::new(Session::new_in_memory().unwrap());
        let state = AppState {
            session,
            project_root: Mutex::new(Some(canonical_root.clone())),
            ai_config: Mutex::new(None),
            chatgpt_tokens: Mutex::new(None),
            allowed_paths: Mutex::new(HashSet::new()),
        };

        // Case 1: Path inside project root is allowed
        let test_file = canonical_root.join("program.sas");
        let res = ensure_under_project_root(&test_file, &state);
        assert!(res.is_ok(), "Should allow files inside active project root");

        // Case 2: Path outside project root is blocked
        let outside_dir = temp_dir.join("other_folder");
        fs::create_dir_all(&outside_dir).unwrap();
        let outside_file = outside_dir.canonicalize().unwrap().join("stolen.sas");
        let res = ensure_under_project_root(&outside_file, &state);
        assert!(
            res.is_err(),
            "Should deny files outside active project root"
        );

        // Case 3: Path explicitly in allowed_paths override is allowed
        {
            let mut allowed = state.allowed_paths.lock().unwrap();
            allowed.insert(outside_file.clone());
        }
        let res = ensure_under_project_root(&outside_file, &state);
        assert!(
            res.is_ok(),
            "Should allow files in allowed_paths allowlist override"
        );

        fs::remove_dir_all(&temp_dir).ok();
    }
}
