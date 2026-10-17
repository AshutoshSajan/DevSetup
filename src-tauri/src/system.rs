use crate::manifest::{Method, Tool};
use serde::Serialize;
use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct Distro { pub id: String, pub name: String, pub version: String }

pub fn distro() -> Distro {
    let txt = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let get = |k: &str| {
        let prefix = format!("{k}=");
        txt.lines().find_map(|l| l.strip_prefix(prefix.as_str()).map(|v| v.trim_matches('"').to_string())).unwrap_or_default()
    };
    Distro { id: get("ID"), name: get("PRETTY_NAME"), version: get("VERSION_ID") }
}

pub fn home() -> Option<PathBuf> { std::env::var_os("HOME").map(PathBuf::from) }

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(h) = home() {
        for d in [".local/bin", ".local/share/mise/shims", ".cargo/bin", ".bun/bin"] { dirs.push(h.join(d)); }
    }
    dirs
}

/// First executable called `cmd` in PATH or in the folders DevSetup installs tools into.
pub fn find_in_path(cmd: &str) -> Option<PathBuf> {
    search_dirs().into_iter().map(|d| d.join(cmd)).find(|p| {
        p.is_file() && std::fs::metadata(p).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    })
}

pub fn on_path(cmd: &str) -> bool { find_in_path(cmd).is_some() }

/// Names of installed Debian/Ubuntu packages, read once from the dpkg status file.
pub fn apt_installed() -> HashSet<String> {
    let txt = std::fs::read_to_string("/var/lib/dpkg/status").unwrap_or_default();
    let mut set = HashSet::new();
    for block in txt.split("\n\n") {
        let (mut name, mut ok) = (None, false);
        for l in block.lines() {
            if let Some(v) = l.strip_prefix("Package: ") { name = Some(v.trim().to_string()); }
            else if l == "Status: install ok installed" { ok = true; }
        }
        if let (Some(n), true) = (name, ok) { set.insert(n); }
    }
    set
}

fn flatpak_has(app_id: &str) -> bool {
    PathBuf::from("/var/lib/flatpak/app").join(app_id).exists()
        || home().map(|h| h.join(".local/share/flatpak/app").join(app_id).exists()).unwrap_or(false)
}

pub fn is_installed(t: &Tool, apt: &HashSet<String>) -> bool {
    let by_method = match &t.method {
        Method::Apt { package } => apt.contains(package),
        Method::Flatpak { app_id } => flatpak_has(app_id),
        _ => false,
    };
    by_method || (!t.check.is_empty() && on_path(&t.check))
}
