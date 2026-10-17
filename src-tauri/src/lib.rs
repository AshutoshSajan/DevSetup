mod clean;
mod doctor;
mod exec;
mod gitwiz;
mod helper;
mod manifest;
mod plan;
mod preflight;
mod review;
mod scan;
mod setup;
mod shellcfg;
mod state;
mod system;
mod update;

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
async fn scan_updates() -> Vec<update::UpdateItem> {
    tauri::async_runtime::spawn_blocking(update::scan).await.unwrap_or_default()
}

#[tauri::command]
fn run_updates(app: tauri::AppHandle, ids: Vec<String>) -> Result<(), String> { update::start(app, ids) }

#[tauri::command]
async fn scan_doctor() -> Vec<doctor::Finding> {
    tauri::async_runtime::spawn_blocking(doctor::scan).await.unwrap_or_default()
}

#[tauri::command]
fn run_fix(app: tauri::AppHandle, id: String) -> Result<(), String> { doctor::start_fix(app, id) }

#[tauri::command]
async fn git_state() -> gitwiz::GitState {
    tauri::async_runtime::spawn_blocking(gitwiz::state).await.unwrap_or_default()
}

#[tauri::command]
fn git_save_identity(name: String, email: String, default_main: bool) -> Result<(), String> { gitwiz::save_identity(&name, &email, default_main) }

#[tauri::command]
async fn git_generate_key(email: String, passphrase: String) -> Result<String, String> {
    let r = tauri::async_runtime::spawn_blocking(move || gitwiz::generate_key(&email, &passphrase)).await.map_err(|e| e.to_string())?;
    r
}

#[tauri::command]
async fn git_test_ssh() -> gitwiz::SshTest {
    tauri::async_runtime::spawn_blocking(gitwiz::test_ssh).await.unwrap_or_default()
}

#[tauri::command]
async fn git_add_key(path: String, kind: String) -> Result<String, String> {
    let r = tauri::async_runtime::spawn_blocking(move || gitwiz::add_key(&path, &kind)).await.map_err(|e| e.to_string())?;
    r
}

#[tauri::command]
async fn git_setup_credentials() -> Result<String, String> {
    let r = tauri::async_runtime::spawn_blocking(gitwiz::setup_git_credentials).await.map_err(|e| e.to_string())?;
    r
}

#[tauri::command]
fn git_set_signing(enable: bool, key: String, email: String) -> Result<(), String> { gitwiz::set_signing(enable, &key, &email) }

#[tauri::command]
fn open_url(which: String) -> Result<(), String> { gitwiz::open_url(&which) }

#[tauri::command]
fn gh_login(app: tauri::AppHandle, protocol: String) -> Result<(), String> { gitwiz::start_login(app, protocol) }

#[tauri::command]
async fn shell_state() -> shellcfg::ShellState {
    tauri::async_runtime::spawn_blocking(shellcfg::state).await.unwrap_or_default()
}

#[tauri::command]
fn shell_preview(shell: String, opts: shellcfg::Options) -> Result<shellcfg::Preview, String> { shellcfg::preview(&shell, &opts) }

#[tauri::command]
fn shell_apply(shell: String, opts: shellcfg::Options) -> Result<String, String> { shellcfg::apply(&shell, &opts) }

#[tauri::command]
fn shell_remove(shell: String) -> Result<String, String> { shellcfg::remove(&shell) }

#[tauri::command]
fn set_login_shell(app: tauri::AppHandle, shell: String) -> Result<(), String> { shellcfg::start_login_shell(app, shell) }

#[tauri::command]
fn cancel_run() { exec::cancel() }

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![get_system, get_manifest, list_tools, make_plan, run_plan, cancel_run, preflight, plan_details, export_setup, parse_setup, read_setup_file, write_setup_file, fetch_setup, scan_machine, scan_clean, run_clean, scan_updates, run_updates, scan_doctor, run_fix, git_state, git_save_identity, git_generate_key, git_test_ssh, git_add_key, git_setup_credentials, git_set_signing, open_url, gh_login, shell_state, shell_preview, shell_apply, shell_remove, set_login_shell])
        .run(tauri::generate_context!())
        .expect("error while running DevSetup");
}
