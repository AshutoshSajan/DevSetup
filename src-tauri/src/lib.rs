mod exec;
mod helper;
mod manifest;
mod plan;
mod preflight;
mod state;
mod system;

use manifest::{Manifest, Tool};
use serde::Serialize;
use std::collections::HashSet;

pub use helper::run as helper_run;

#[derive(Serialize)]
struct ToolView { #[serde(flatten)] tool: Tool, installed: bool, managed: bool }

#[tauri::command]
fn get_system() -> system::Distro { system::distro() }

#[tauri::command]
fn get_manifest() -> Manifest { manifest::load() }

#[tauri::command]
fn list_tools() -> Vec<ToolView> {
    let apt = system::apt_installed();
    let managed: HashSet<String> = state::load().managed.into_iter().collect();
    manifest::load().tools.into_iter().map(|t| {
        let installed = system::is_installed(&t, &apt);
        let managed = managed.contains(&t.id);
        ToolView { tool: t, installed, managed }
    }).collect()
}

#[tauri::command]
fn make_plan(selected: Vec<String>) -> plan::Plan { plan::build(&manifest::load(), &selected) }

#[tauri::command]
fn run_plan(app: tauri::AppHandle, install: Vec<String>, remove: Vec<String>, strict: bool) -> Result<(), String> {
    exec::start(app, install, remove, strict)
}

#[tauri::command]
async fn preflight(install: Vec<String>, remove: Vec<String>, strict: bool) -> Vec<preflight::Check> {
    tauri::async_runtime::spawn_blocking(move || preflight::run(install, remove, strict)).await.unwrap_or_default()
}

#[tauri::command]
fn cancel_run() { exec::cancel() }

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_system, get_manifest, list_tools, make_plan, run_plan, cancel_run, preflight])
        .run(tauri::generate_context!())
        .expect("error while running DevSetup");
}
