use std::sync::Arc;

use pas_engine::{ColumnInfo, DatasetInfo, DatasetPage, Event, Library, Session};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub libnames: Vec<ProjectLibname>,
    #[serde(default)]
    pub open_tabs: Vec<TabConfig>,
    #[serde(default)]
    pub active_tab: Option<String>,
    #[serde(default)]
    pub layout: Layout,
}

fn default_version() -> u32 { 1 }

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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Layout {
    #[serde(default)]
    pub sidebar_width: Option<u32>,
    #[serde(default)]
    pub bottom_height: Option<u32>,
}

pub struct AppState {
    session: Arc<Session>,
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

    // Run synchronously off the UI thread. DuckDB calls are blocking.
    tokio::task::spawn_blocking(move || {
        let events = session.submit(&program);
        for event in events {
            let _ = app.emit(
                "pas://event",
                SubmitEventPayload { submission_id: id.clone(), event },
            );
        }
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
    state.session.list_datasets(&libref).map_err(|e| e.to_string())
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

#[tauri::command]
fn read_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("{}: {}", path, e))
}

#[tauri::command]
fn write_file(path: String, content: String) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(&path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {}", parent.display(), e))?;
        }
    }
    std::fs::write(&path, content).map_err(|e| format!("{}: {}", path, e))
}

#[tauri::command]
fn read_project(path: String) -> Result<ProjectConfig, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {}", path, e))?;
    serde_json::from_str(&text).map_err(|e| format!("parse {}: {}", path, e))
}

#[tauri::command]
fn save_project(path: String, project: ProjectConfig) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&project).map_err(|e| e.to_string())?;
    if let Some(parent) = std::path::Path::new(&path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {}", parent.display(), e))?;
        }
    }
    std::fs::write(&path, text).map_err(|e| format!("{}: {}", path, e))
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
                SubmitEventPayload { submission_id: id.clone(), event },
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::try_init().ok();

    let session = Session::new_in_memory().expect("create engine");
    let state = AppState { session: Arc::new(session) };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .setup(|app| {
            #[cfg(debug_assertions)]
            if let Some(window) = app.get_webview_window("main") {
                window.open_devtools();
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
            apply_project_libnames
        ])
        .run(tauri::generate_context!())
        .expect("error while running PAS");
}
