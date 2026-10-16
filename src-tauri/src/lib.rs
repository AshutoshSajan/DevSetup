mod clean;
mod exec;
mod helper;
mod manifest;
mod plan;
mod preflight;
mod review;
mod scan;
mod setup;
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
async fn plan_details(install: Vec<String>, remove: Vec<String>, strict: bool) -> review::Details {
    tauri::async_runtime::spawn_blocking(move || review::details(install, remove, strict)).await.unwrap_or_default()
}

#[tauri::command]
fn export_setup(name: String, roles: Vec<String>, tools: Vec<String>, notes: Vec<String>) -> String { setup::to_yaml(&name, &roles, &tools, &notes) }

#[tauri::command]
fn parse_setup(text: String) -> Result<setup::Imported, String> { setup::parse(&text, &manifest::load()) }

#[tauri::command]
fn read_setup_file(path: String) -> Result<String, String> { setup::read_file(&path) }

#[tauri::command]
fn write_setup_file(path: String, content: String) -> Result<(), String> { setup::write_file(&path, &content) }

#[tauri::command]
async fn fetch_setup(source: String) -> Result<String, String> {
    let r = tauri::async_runtime::spawn_blocking(move || setup::fetch(&source)).await.map_err(|e| e.to_string())?;
    r
}

#[tauri::command]
async fn scan_machine() -> scan::Scan {
    tauri::async_runtime::spawn_blocking(scan::scan).await.unwrap_or_default()
}

#[tauri::command]
async fn scan_clean() -> Vec<clean::CleanItem> {
    tauri::async_runtime::spawn_blocking(clean::scan).await.unwrap_or_default()
}

#[tauri::command]
fn run_clean(app: tauri::AppHandle, ids: Vec<String>) -> Result<(), String> { clean::start(app, ids) }

#[tauri::command]
fn cancel_run() { exec::cancel() }

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![get_system, get_manifest, list_tools, make_plan, run_plan, cancel_run, preflight, plan_details, export_setup, parse_setup, read_setup_file, write_setup_file, fetch_setup, scan_machine, scan_clean, run_clean])
        .run(tauri::generate_context!())
        .expect("error while running DevSetup");
}
