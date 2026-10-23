// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod jobs;
#[cfg(target_os = "macos")]
mod menu;

use std::fmt::Display;
use std::sync::Arc;

use jobs::{EventSink, JobInfo, JobManager, JobRequest, JobSummary, ProgressEvent};
use porter_backends::Registry;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_opener::OpenerExt;

struct AppState {
    registry: Arc<Registry>,
    jobs: Arc<JobManager>,
    /// Why saved remotes could not be loaded, shown in the sidebar.
    config_error: Option<String>,
}

/// Forwards job events to the UI.
struct UiSink(AppHandle);

impl EventSink for UiSink {
    fn started(&self, info: &JobInfo) {
        let _ = self.0.emit("job-started", info);
    }
    fn progress(&self, event: &ProgressEvent) {
        let _ = self.0.emit("job-progress", event);
    }
    fn finished(&self, summary: &JobSummary) {
        let _ = self.0.emit("job-finished", summary);
    }
}

type CmdResult<T> = Result<T, String>;

fn message(e: impl Display) -> String {
    e.to_string()
}

#[tauri::command]
async fn list_dir(state: State<'_, AppState>, location: String) -> CmdResult<api::Listing> {
    api::list_dir(&state.registry, &location)
        .await
        .map_err(message)
}

#[tauri::command]
async fn stat(state: State<'_, AppState>, location: String) -> CmdResult<api::Item> {
    api::stat(&state.registry, &location).await.map_err(message)
}

#[tauri::command]
async fn sidebar(state: State<'_, AppState>) -> CmdResult<api::Sidebar> {
    api::sidebar(&state.registry, state.config_error.clone())
        .await
        .map_err(message)
}

#[tauri::command]
async fn mkdir(state: State<'_, AppState>, parent: String, name: String) -> CmdResult<String> {
    api::mkdir(&state.registry, &parent, &name)
        .await
        .map_err(message)
}

#[tauri::command]
async fn rename(state: State<'_, AppState>, location: String, name: String) -> CmdResult<String> {
    api::rename(&state.registry, &location, &name)
        .await
        .map_err(message)
}

#[tauri::command]
fn open(app: AppHandle, state: State<'_, AppState>, location: String) -> CmdResult<()> {
    let path = api::local_path(&state.registry, &location).map_err(message)?;
    app.opener().open_path(path, None::<&str>).map_err(message)
}

#[tauri::command]
async fn start_job(
    app: AppHandle,
    state: State<'_, AppState>,
    request: JobRequest,
) -> CmdResult<JobInfo> {
    state
        .jobs
        .start(&state.registry, request, Arc::new(UiSink(app)))
        .map_err(message)
}

#[tauri::command]
fn cancel_job(state: State<'_, AppState>, id: u64) -> bool {
    state.jobs.cancel(id)
}

fn main() {
    let (registry, config_error) = match Registry::load() {
        Ok(registry) => (registry, None),
        Err(e) => (Registry::default(), Some(e.to_string())),
    };
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.menu(menu::build).on_menu_event(menu::on_event);
    builder
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            registry: Arc::new(registry),
            jobs: Arc::default(),
            config_error,
        })
        .invoke_handler(tauri::generate_handler![
            list_dir, stat, sidebar, mkdir, rename, open, start_job, cancel_job
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Porter");
}
