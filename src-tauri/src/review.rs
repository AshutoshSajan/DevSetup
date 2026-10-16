//! Details for the review screen: the exact commands in execution order, notes, and apt package sizes.
use crate::manifest::{self, Method, Tool};
use crate::{exec, state, system};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::process::Command;

#[derive(Serialize)]
pub struct StepInfo { pub id: String, pub name: String, pub phase: &'static str, pub command: String, pub notes: Vec<String> }

#[derive(Serialize, Default)]
pub struct Details { pub steps: Vec<StepInfo>, pub skipped: Vec<String> }

fn human(b: u64) -> String {
    if b >= 1_000_000_000 { format!("{:.1} GB", b as f64 / 1e9) }
    else if b >= 1_000_000 { format!("{:.1} MB", b as f64 / 1e6) }
    else { format!("{:.0} kB", b as f64 / 1e3) }
}

/// package name -> (download bytes, installed bytes), from `apt-cache show` output.
fn parse_apt_show(text: &str) -> HashMap<String, (u64, u64)> {
    let mut out = HashMap::new();
    for block in text.split("\n\n") {
        let (mut name, mut size, mut inst) = (None, None, None);
        for l in block.lines() {
            if let Some(v) = l.strip_prefix("Package: ") { name = Some(v.trim().to_string()); }
            else if let Some(v) = l.strip_prefix("Size: ") { size = v.trim().parse::<u64>().ok(); }
            else if let Some(v) = l.strip_prefix("Installed-Size: ") { inst = v.trim().parse::<u64>().ok().map(|k| k * 1024); }
        }
        if let (Some(n), Some(s)) = (name, size) { out.insert(n, (s, inst.unwrap_or(0))); }
    }
    out
}

fn apt_sizes(pkgs: &[String]) -> HashMap<String, (u64, u64)> {
    if pkgs.is_empty() { return HashMap::new(); }
    match Command::new("apt-cache").args(["show", "--no-all-versions"]).args(pkgs).env("LC_ALL", "C").output() {
        Ok(o) => parse_apt_show(&String::from_utf8_lossy(&o.stdout)),
        Err(_) => HashMap::new(),
    }
}

fn apt_pkg(t: &Tool) -> Option<String> {
    if let Method::Apt { package } = &t.method { Some(package.clone()) } else { None }
}

pub fn details(install: Vec<String>, remove: Vec<String>, strict: bool) -> Details {
    let m = manifest::load();
    let apt = system::apt_installed();
    let managed: HashSet<String> = state::load().managed.into_iter().collect();
    let pl = exec::plan_steps(&m, &apt, &managed, &install, &remove);
    let mut d = Details::default();

    for (t, by) in &pl.blocked { d.skipped.push(format!("{} stays installed: {} needs it", t.name, by)); }

    // Phase 0: user-level removals, dependents first.
    let user_rm: Vec<Tool> = pl.to_remove.iter().filter(|t| apt_pkg(t).is_none()).cloned().collect();
    for t in exec::removal_order(&user_rm) {
        match exec::remove_cmd(&t) {
            Some(c) => d.steps.push(StepInfo { id: t.id.clone(), name: t.name.clone(), phase: "remove", command: exec::render(&c), notes: vec![] }),
            None => d.skipped.push(format!("{}: remove manually (no uninstall command known)", t.name)),
        }
    }

    // Phase 1: one root call for all apt work.
    let apt_in: Vec<&Tool> = pl.to_install.iter().filter(|t| apt_pkg(t).is_some()).collect();
    let apt_rm: Vec<&Tool> = pl.to_remove.iter().filter(|t| apt_pkg(t).is_some()).collect();
    if !apt_in.is_empty() || !apt_rm.is_empty() {
        let ins: Vec<String> = apt_in.iter().filter_map(|t| apt_pkg(t)).collect();
        let rms: Vec<String> = apt_rm.iter().filter_map(|t| apt_pkg(t)).collect();
        let mut lines: Vec<String> = vec![];
        if !ins.is_empty() { lines.push("apt-get update".into()); lines.push(format!("apt-get install -y {}", ins.join(" "))); }
        if !rms.is_empty() { lines.push(format!("apt-get remove -y {}", rms.join(" "))); }
        let sizes = apt_sizes(&ins);
        let mut notes: Vec<String> = vec![];
        for t in &apt_in {
            if let Some(p) = apt_pkg(t) {
                if let Some((dl, inst)) = sizes.get(&p) { notes.push(format!("{}: package {} to download, {} installed (dependencies not included)", t.name, human(*dl), human(*inst))); }
            }
        }
        d.steps.push(StepInfo {
            id: "apt".into(), name: "System packages (apt)".into(), phase: "apt",
            command: format!("as root through pkexec, one password prompt:\n{}", lines.join("\n")), notes,
        });
    }

    // Phase 2: user-level installs in dependency order.
    for t in pl.to_install.iter().filter(|t| apt_pkg(t).is_none()) {
        let mut notes: Vec<String> = vec![];
        let command = match &t.method {
            Method::Script { url, sha256, args, .. } => {
                let chk = match sha256.as_deref() { Some(h) if !h.is_empty() => format!("check SHA-256 {h}"), _ => "no checksum pinned".to_string() };
                let with = if args.is_empty() { String::new() } else { format!(" with: {}", args.join(" ")) };
                if t.script_is_pinned() { notes.push("pinned: refuses to run if the download does not match".into()); }
                else if strict { notes.push("will not run: strict mode requires a pinned checksum".into()); }
                else { notes.push("not pinned: runs whatever the server returns today".into()); }
                format!("download {url}, {chk}, then run it{with}")
            }
            Method::Flatpak { .. } => { notes.push("Flatpak shows the download size when it starts".into()); exec::install_cmd(t).map(|c| exec::render(&c)).unwrap_or_default() }
            _ => exec::install_cmd(t).map(|c| exec::render(&c)).unwrap_or_default(),
        };
        d.steps.push(StepInfo { id: t.id.clone(), name: t.name.clone(), phase: "install", command, notes });
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(human(3_679_758), "3.7 MB");
        assert_eq!(human(226_502), "227 kB");
        assert_eq!(human(2_500_000_000), "2.5 GB");
    }

    #[test]
    fn parses_apt_cache_show() {
        let text = "Package: git\nVersion: 1:2.39\nInstalled-Size: 21692\nSize: 3679758\n\nPackage: curl\nInstalled-Size: 521\nSize: 226502\n";
        let m = parse_apt_show(text);
        assert_eq!(m["git"], (3_679_758, 21_692 * 1024));
        assert_eq!(m["curl"], (226_502, 521 * 1024));
        assert!(!m.contains_key("nope"));
    }
}
