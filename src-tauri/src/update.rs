//! Updater: finds available updates (read-only) and runs them. The UI only ever sends item ids;
//! what each id does is fixed here or comes from the built-in manifest.
use crate::exec::{self, cancelled, log, step, stream};
use crate::manifest::{self, Method};
use crate::system;
use serde::Serialize;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Default, Clone)]
pub struct UpdateItem {
    pub id: String,
    pub name: String,
    pub description: String,
    pub command: String,
    /// Number of updates found. None = cannot be checked ahead of time.
    pub count: Option<usize>,
    pub detail: String,
    pub needs_root: bool,
    pub available: bool,
    pub reason: String,
}

enum Run {
    /// Fixed operation run by the root helper.
    Root,
    /// Command run as the user.
    Sh(String),
}

fn catalog() -> Vec<(UpdateItem, Run)> {
    let mk = |id: &str, name: &str, desc: &str, cmd: &str, root: bool| UpdateItem {
        id: id.into(), name: name.into(), description: desc.into(), command: cmd.into(), needs_root: root, available: true, ..Default::default()
    };
    let mut v = vec![
        (mk("apt", "System packages", "Every upgradable apt package (apt upgrade). Package lists are refreshed first, so more may turn up than the count shows.", "apt-get update && apt-get upgrade -y", true), Run::Root),
        (mk("flatpak", "Flatpak apps and runtimes", "Your user Flatpak installs.", "flatpak update --user -y --noninteractive", false), Run::Sh("flatpak update --user -y --noninteractive".into())),
        (mk("mise", "mise tools", "Every tool installed with mise.", "mise upgrade", false), Run::Sh("mise upgrade".into())),
        (mk("npm", "Global npm packages", "Every package installed with npm -g.", "npm update -g", false), Run::Sh("npm update -g".into())),
        (mk("pipx", "pipx apps", "Every app installed with pipx. Apps that are up to date are skipped.", "pipx upgrade-all", false), Run::Sh("pipx upgrade-all".into())),
    ];
    // Tools installed by an install script that can update themselves.
    let m = manifest::load();
    let apt = system::apt_installed();
    for t in &m.tools {
        if let Method::Script { update: Some(cmd), .. } = &t.method {
            if system::is_installed(t, &apt) {
                v.push((mk(&format!("self-{}", t.id), &format!("{} (its own updater)", t.name), "Runs the tool's built-in update command. It does nothing if the tool is already current.", cmd, false), Run::Sh(cmd.clone())));
            }
        }
    }
    v
}

// ---------- parsing ----------

/// Package names from the "Inst name [old] (new ...)" lines of `apt-get -s upgrade`.
fn parse_inst(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.strip_prefix("Inst ")).filter_map(|l| l.split_whitespace().next()).map(|s| s.to_string()).collect()
}

/// Keys of the top-level JSON object printed by `mise outdated --json` or `npm outdated -g --json`.
/// Empty output means nothing is outdated. None means the output could not be understood.
fn json_keys(text: &str) -> Option<Vec<String>> {
    if text.trim().is_empty() { return Some(vec![]); }
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut keys: Vec<String> = v.as_object()?.keys().cloned().collect();
    keys.sort();
    Some(keys)
}

fn valid_id(s: &str) -> bool { !s.is_empty() && s.len() <= 120 && s.chars().all(|c| c.is_ascii_alphanumeric() || "@._/-+".contains(c)) }

fn run_sh(script: &str) -> String {
    exec::sh(script, &[]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

fn has(cmd: &str) -> bool { !run_sh(&format!("command -v {cmd}")).trim().is_empty() }

fn names_detail(names: &[String]) -> String {
    let more = if names.len() > 12 { ", …" } else { "" };
    format!("{}{}", names.iter().take(12).cloned().collect::<Vec<_>>().join(", "), more)
}

// ---------- scanning (read-only) ----------

fn unavailable(it: &mut UpdateItem, why: &str) { it.available = false; it.reason = why.to_string(); }

fn scan_apt(it: &mut UpdateItem) {
    if !system::on_path("apt-get") || !Path::new("/var/lib/dpkg/status").exists() { return unavailable(it, "Needs an apt-based system (Debian, Ubuntu)."); }
    let Ok(o) = Command::new("apt-get").args(["-s", "upgrade"]).env("LC_ALL", "C").output() else { return unavailable(it, "Could not run apt-get."); };
    let names = parse_inst(&String::from_utf8_lossy(&o.stdout));
    it.count = Some(names.len());
    it.detail = if names.is_empty() { "Nothing to upgrade with the current package lists.".to_string() } else { names_detail(&names) };
}

fn scan_flatpak(it: &mut UpdateItem) {
    if !has("flatpak") { return unavailable(it, "Flatpak is not installed."); }
    let text = run_sh("timeout 20 flatpak remote-ls --user --updates --columns=application 2>/dev/null");
    let names: Vec<String> = text.lines().map(|l| l.trim().to_string()).filter(|l| valid_id(l)).collect();
    it.count = Some(names.len());
    it.detail = if names.is_empty() { "Nothing to update.".to_string() } else { names_detail(&names) };
}

fn scan_mise(it: &mut UpdateItem) {
    if !has("mise") { return unavailable(it, "mise is not installed."); }
    match json_keys(&run_sh("timeout 30 mise outdated --json 2>/dev/null")) {
        Some(names) => {
            it.count = Some(names.len());
            it.detail = if names.is_empty() { "Nothing to update.".to_string() } else { names_detail(&names) };
            if names.iter().any(|n| n == "node") {
                it.detail.push_str(". Upgrading Node installs a new Node version: global npm packages such as Codex CLI are not carried over and must be installed again afterwards.");
            }
        }
        None => it.detail = "Could not check ahead of time. mise upgrade skips tools that are current.".to_string(),
    }
}

fn scan_npm(it: &mut UpdateItem) {
    if !has("npm") { return unavailable(it, "npm is not installed."); }
    match json_keys(&run_sh("timeout 30 npm outdated -g --json 2>/dev/null")) {
        Some(names) => {
            it.count = Some(names.len());
            it.detail = if names.is_empty() { "Nothing to update.".to_string() } else { names_detail(&names) };
        }
        None => it.detail = "Could not check ahead of time.".to_string(),
    }
}

pub fn scan() -> Vec<UpdateItem> {
    catalog().into_iter().map(|(mut it, _)| {
        let id = it.id.clone();
        match id.as_str() {
            "apt" => scan_apt(&mut it),
            "flatpak" => scan_flatpak(&mut it),
            "mise" => scan_mise(&mut it),
            "npm" => scan_npm(&mut it),
            "pipx" => if !has("pipx") { unavailable(&mut it, "pipx is not installed."); },
            _ => {}
        }
        it
    }).collect()
}

// ---------- running ----------

fn run(app: &AppHandle, ids: &[String]) -> usize {
    let defs = catalog();
    let chosen: Vec<&(UpdateItem, Run)> = ids.iter().filter_map(|id| defs.iter().find(|(it, _)| it.id == id.as_str())).collect();
    let mut failed = 0;
    for (it, _) in chosen.iter().copied() { step(app, &it.id, "queued", ""); }

    // System packages: one pkexec call, one password prompt. Not cancellable once started.
    let root: Vec<&UpdateItem> = chosen.iter().copied().filter(|(_, r)| matches!(r, Run::Root)).map(|(it, _)| it).collect();
    if !root.is_empty() {
        if cancelled() {
            for it in &root { step(app, &it.id, "cancelled", ""); }
        } else {
            for it in &root { step(app, &it.id, "running", ""); }
            let mut cmd = Command::new("pkexec");
            match std::env::current_exe() {
                Ok(exe) => { cmd.arg(exe).arg("--helper").arg("upgrade"); }
                Err(e) => log(app, "apt", "stderr", &format!("cannot find own executable: {e}")),
            }
            let ok = stream(app, "apt", cmd, false);
            for it in &root {
                if ok { step(app, &it.id, "done", ""); } else { step(app, &it.id, "failed", "see log"); failed += 1; }
            }
        }
    }

    for (it, r) in chosen.iter().copied() {
        let Run::Sh(script) = r else { continue };
        if cancelled() { step(app, &it.id, "cancelled", ""); continue; }
        step(app, &it.id, "running", "");
        if stream(app, &it.id, exec::sh(script, &[]), true) { step(app, &it.id, "done", ""); }
        else if cancelled() { step(app, &it.id, "cancelled", "stopped"); }
        else { step(app, &it.id, "failed", "see log"); failed += 1; }
    }
    failed
}

pub fn start(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    if exec::RUNNING.swap(true, Ordering::SeqCst) { return Err("a run is already in progress".into()); }
    exec::CANCEL.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let failed = run(&app, &ids);
        let was_cancelled = cancelled();
        exec::RUNNING.store(false, Ordering::SeqCst);
        let _ = app.emit("run-done", exec::Done { ok: failed == 0 && !was_cancelled, failed, cancelled: was_cancelled });
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_apt_simulation_and_json_lists() {
        let apt = "Inst libc6 [2.36-9] (2.36-10 Debian:12/stable [amd64])\nInst git [1:2.39] (1:2.40 Debian:12/stable [amd64])\nConf git (1:2.40 Debian:12/stable [amd64])\n";
        assert_eq!(parse_inst(apt), vec!["libc6", "git"]);
        assert_eq!(json_keys(r#"{"node":{"current":"22.1"},"go":{"current":"1.22"}}"#), Some(vec!["go".to_string(), "node".to_string()]));
        assert_eq!(json_keys("  \n"), Some(vec![]));
        assert_eq!(json_keys("[1,2]"), None);
        assert_eq!(json_keys("not json"), None);
    }

    #[test]
    fn script_tools_with_an_update_command_get_their_own_item_when_installed() {
        // Every update command in the manifest must be a non-empty string.
        for t in manifest::load().tools {
            if let Method::Script { update: Some(u), .. } = &t.method { assert!(!u.trim().is_empty(), "{} has an empty update command", t.id); }
        }
        let ids: Vec<String> = catalog().into_iter().map(|(it, _)| it.id).collect();
        for fixed in ["apt", "flatpak", "mise", "npm", "pipx"] { assert!(ids.iter().any(|i| i == fixed), "missing {fixed}"); }
    }
}
