//! Cleaner: free disk space from unused packages and caches. The UI only ever sends item ids;
//! what each id does is fixed here, and folders are only ever emptied inside the user's home.
use crate::exec::{self, cancelled, log, step, stream};
use crate::system;
use serde::Serialize;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Default, Clone)]
pub struct CleanItem {
    pub id: String,
    pub name: String,
    pub description: String,
    pub command: String,
    /// Estimated bytes freed. None = unknown.
    pub bytes: Option<u64>,
    pub detail: String,
    pub needs_root: bool,
    /// "safe" items are pre-ticked; "review" items can remove things you may still want.
    pub risk: &'static str,
    pub available: bool,
    pub reason: String,
}

enum Action {
    /// Fixed operation run by the root helper.
    Root(&'static str),
    /// Command run as the user.
    Sh(&'static str),
    /// Empty these folders, relative to the home folder.
    Dirs(&'static [&'static str]),
}

struct Def { id: &'static str, name: &'static str, description: &'static str, risk: &'static str, action: Action }

fn catalog() -> Vec<Def> {
    vec![
        Def { id: "apt-autoremove", name: "Unused system packages", risk: "review", action: Action::Root("autoremove"),
              description: "Packages installed only as dependencies that nothing needs any more (apt autoremove). Check the list before you clean." },
        Def { id: "apt-cache", name: "Downloaded package files", risk: "safe", action: Action::Root("apt-cache"),
              description: "Package files apt downloaded and no longer needs (apt clean)." },
        Def { id: "docker-prune", name: "Docker: stopped containers, dangling images, unused networks, build cache", risk: "safe", action: Action::Sh("docker system prune -f"),
              description: "docker system prune. Volumes are never touched." },
        Def { id: "docker-images", name: "Docker: all unused images", risk: "review", action: Action::Sh("docker image prune -a -f"),
              description: "docker image prune -a. Images no container uses are removed and downloaded again when needed." },
        Def { id: "flatpak-unused", name: "Flatpak: unused runtimes", risk: "safe", action: Action::Sh("flatpak uninstall --user --unused -y --noninteractive"),
              description: "Runtimes that no installed Flatpak app needs (your user installs)." },
        Def { id: "npm-cache", name: "npm cache", risk: "safe", action: Action::Sh("npm cache clean --force"),
              description: "Downloaded npm packages. They are downloaded again when needed." },
        Def { id: "pip-cache", name: "pip cache", risk: "safe", action: Action::Dirs(&[".cache/pip"]),
              description: "Downloaded Python packages in ~/.cache/pip." },
        Def { id: "mise-cache", name: "mise cache", risk: "safe", action: Action::Dirs(&[".cache/mise"]),
              description: "mise downloads and metadata in ~/.cache/mise. Installed tools are not removed." },
        Def { id: "cargo-cache", name: "Cargo registry cache", risk: "safe", action: Action::Dirs(&[".cargo/registry/cache", ".cargo/registry/src"]),
              description: "Downloaded crates and their unpacked sources. Cargo downloads them again when needed." },
    ]
}

fn describe(a: &Action) -> String {
    match a {
        Action::Root(op) => if *op == "autoremove" { "apt-get autoremove -y".to_string() } else { "apt-get clean".to_string() },
        Action::Sh(s) => s.to_string(),
        Action::Dirs(rels) => format!("delete the contents of {}", rels.iter().map(|r| format!("~/{r}")).collect::<Vec<_>>().join(" and ")),
    }
}

// ---------- safe file handling ----------

/// Joins a fixed relative path onto the home folder. Refuses anything that could leave it.
fn safe_target(home: &Path, rel: &str) -> Option<PathBuf> {
    let r = Path::new(rel);
    if rel.is_empty() || r.is_absolute() || !r.components().all(|c| matches!(c, Component::Normal(_))) { return None; }
    if !home.is_absolute() || home.parent().is_none() { return None; } // never "/"
    Some(home.join(r))
}

/// Total size of regular files below `p`. Symbolic links are not followed.
fn dir_size(p: &Path) -> u64 {
    let (mut total, mut seen, mut stack) = (0u64, 0usize, vec![p.to_path_buf()]);
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            seen += 1;
            if seen > 2_000_000 { return total; }
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() { stack.push(e.path()); } else { total += md.len(); }
        }
    }
    total
}

/// Deletes everything inside `dir` but keeps `dir` itself. Refuses a symbolic link or a non-folder.
/// Links inside are removed as links, never followed.
fn clear_dir_contents(dir: &Path) -> Result<usize, String> {
    let md = std::fs::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if md.file_type().is_symlink() || !md.is_dir() { return Err(format!("{} is not a plain folder", dir.display())); }
    let mut n = 0;
    for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten() {
        let p = e.path();
        let is_dir = e.file_type().map_err(|e| format!("{}: {e}", p.display()))?.is_dir();
        let r = if is_dir { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        r.map_err(|e| format!("{}: {e}", p.display()))?;
        n += 1;
    }
    Ok(n)
}

// ---------- parsing ----------

/// Bytes from apt's "After this operation, 12.3 MB disk space will be freed."
fn parse_freed(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.contains("After this operation,") && l.contains("freed"))?;
    let rest = &line[line.find("After this operation,")? + "After this operation,".len()..];
    let mut it = rest.split_whitespace();
    let num: f64 = it.next()?.replace(',', "").parse().ok()?;
    let unit: String = it.next()?.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let mult = match unit.as_str() { "B" => 1.0, "kB" => 1e3, "MB" => 1e6, "GB" => 1e9, _ => return None };
    Some((num * mult).round() as u64)
}

/// Package names from the "Remv name [version]" lines of `apt-get -s`.
fn parse_remv(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.strip_prefix("Remv ")).filter_map(|l| l.split_whitespace().next()).map(|s| s.to_string()).collect()
}

/// Bytes from Docker sizes such as "1.2GB (45%)", "512MB" or "0B".
fn parse_docker_size(s: &str) -> Option<u64> {
    let t = s.split_whitespace().next()?;
    let i = t.find(|c: char| c.is_ascii_alphabetic())?;
    let (num, unit) = t.split_at(i);
    let n: f64 = num.parse().ok()?;
    let mult = match unit.to_ascii_lowercase().as_str() { "b" => 1.0, "kb" => 1e3, "mb" => 1e6, "gb" => 1e9, "tb" => 1e12, _ => return None };
    Some((n * mult).round() as u64)
}

fn run_sh(script: &str) -> String {
    exec::sh(script, &[]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

// ---------- scanning (read-only) ----------

fn unavailable(it: &mut CleanItem, why: &str) { it.available = false; it.reason = why.to_string(); }

fn scan_autoremove(it: &mut CleanItem) {
    if !system::on_path("apt-get") || !Path::new("/var/lib/dpkg/status").exists() { return unavailable(it, "Needs an apt-based system (Debian, Ubuntu)."); }
    let Ok(o) = Command::new("apt-get").args(["-s", "autoremove"]).env("LC_ALL", "C").output() else { return unavailable(it, "Could not run apt-get."); };
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    let pkgs = parse_remv(&text);
    it.bytes = Some(parse_freed(&text).unwrap_or(0));
    it.detail = if pkgs.is_empty() { "Nothing to remove.".to_string() } else {
        let more = if pkgs.len() > 12 { ", …" } else { "" };
        format!("{} package{}: {}{}", pkgs.len(), if pkgs.len() == 1 { "" } else { "s" }, pkgs.iter().take(12).cloned().collect::<Vec<_>>().join(", "), more)
    };
}

fn scan_docker(it: &mut CleanItem, all_images: bool) {
    let df = run_sh("timeout 10 docker system df --format '{{json .}}' 2>&1");
    let rows: Vec<serde_json::Value> = df.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    if rows.is_empty() {
        let first = df.lines().find(|l| !l.trim().is_empty()).unwrap_or("Docker is not installed or not running.");
        return unavailable(it, &first.chars().take(160).collect::<String>());
    }
    let reclaim = |kind: &str| -> u64 {
        rows.iter().find(|r| r["Type"] == kind).and_then(|r| r["Reclaimable"].as_str()).and_then(parse_docker_size).unwrap_or(0)
    };
    if all_images {
        it.bytes = Some(reclaim("Images"));
        it.detail = "Docker's own estimate of the images no container uses.".to_string();
    } else {
        let dangling: u64 = run_sh("timeout 10 docker image ls -f dangling=true --format '{{.Size}}' 2>/dev/null").lines().filter_map(parse_docker_size).sum();
        it.bytes = Some(reclaim("Containers") + reclaim("Build Cache") + dangling);
        it.detail = "Estimate: stopped containers, unused build cache and dangling images.".to_string();
    }
}

fn scan_flatpak(it: &mut CleanItem) {
    if run_sh("command -v flatpak").trim().is_empty() { return unavailable(it, "Flatpak is not installed."); }
    let text = run_sh("timeout 15 flatpak uninstall --user --unused --assumeno --noninteractive 2>&1");
    if text.contains("Nothing unused") { it.bytes = Some(0); it.detail = "Nothing unused.".to_string(); return; }
    let n = text.lines().filter(|l| {
        let t = l.trim_start();
        t.find('.').map_or(false, |i| i > 0 && t[..i].chars().all(|c| c.is_ascii_digit()))
    }).count();
    it.detail = if n > 0 { format!("{n} unused runtime{} found. Flatpak does not report their size.", if n == 1 { "" } else { "s" }) }
                else { "Flatpak removes whatever it finds unused. It does not report the size.".to_string() };
}

fn scan_npm(it: &mut CleanItem, home: Option<&Path>) {
    if run_sh("command -v npm").trim().is_empty() { return unavailable(it, "npm is not installed."); }
    let configured = run_sh("timeout 10 npm config get cache 2>/dev/null").trim().to_string();
    let dir = if configured.starts_with('/') { PathBuf::from(configured) } else { home.map(|h| h.join(".npm")).unwrap_or_default() };
    it.bytes = Some(dir_size(&dir));
}

pub fn scan() -> Vec<CleanItem> {
    let home = system::home();
    catalog().into_iter().map(|d| {
        let mut it = CleanItem {
            id: d.id.to_string(), name: d.name.to_string(), description: d.description.to_string(), command: describe(&d.action),
            needs_root: matches!(d.action, Action::Root(_)), risk: d.risk, available: true, ..Default::default()
        };
        match d.id {
            "apt-autoremove" => scan_autoremove(&mut it),
            "apt-cache" => it.bytes = Some(dir_size(Path::new("/var/cache/apt/archives"))),
            "docker-prune" => scan_docker(&mut it, false),
            "docker-images" => scan_docker(&mut it, true),
            "flatpak-unused" => scan_flatpak(&mut it),
            "npm-cache" => scan_npm(&mut it, home.as_deref()),
            _ => if let Action::Dirs(rels) = &d.action {
                it.bytes = Some(rels.iter().filter_map(|r| home.as_deref().and_then(|h| safe_target(h, r))).map(|p| dir_size(&p)).sum());
            },
        }
        it
    }).collect()
}

// ---------- running ----------

fn clear_dirs(app: &AppHandle, id: &str, home: Option<&Path>, rels: &[&str]) -> bool {
    let mut ok = true;
    for rel in rels {
        let Some(p) = home.and_then(|h| safe_target(h, rel)) else { log(app, id, "stderr", &format!("Refusing to touch ~/{rel}")); ok = false; continue; };
        if !p.exists() { log(app, id, "stdout", &format!("{} does not exist, nothing to clean", p.display())); continue; }
        match clear_dir_contents(&p) {
            Ok(n) => log(app, id, "stdout", &format!("Emptied {} ({n} entries)", p.display())),
            Err(e) => { log(app, id, "stderr", &e); ok = false; }
        }
    }
    ok
}

fn run(app: &AppHandle, ids: &[String]) -> usize {
    let defs = catalog();
    let home = system::home();
    let chosen: Vec<&Def> = ids.iter().filter_map(|id| defs.iter().find(|d| d.id == id.as_str())).collect();
    let mut failed = 0;
    for d in &chosen { step(app, d.id, "queued", ""); }

    // Root operations: one pkexec call, one password prompt. Not cancellable once started.
    let root: Vec<&Def> = chosen.iter().copied().filter(|d| matches!(d.action, Action::Root(_))).collect();
    if !root.is_empty() {
        if cancelled() {
            for d in &root { step(app, d.id, "cancelled", ""); }
        } else {
            for d in &root { step(app, d.id, "running", ""); }
            let mut cmd = Command::new("pkexec");
            match std::env::current_exe() {
                Ok(exe) => { cmd.arg(exe).arg("--helper").arg("clean"); }
                Err(e) => log(app, "apt", "stderr", &format!("cannot find own executable: {e}")),
            }
            for d in &root { if let Action::Root(op) = &d.action { cmd.arg(*op); } }
            let ok = stream(app, "apt", cmd, false);
            for d in &root {
                if ok { step(app, d.id, "done", ""); } else { step(app, d.id, "failed", "see log"); failed += 1; }
            }
        }
    }

    for d in chosen.iter().filter(|d| !matches!(d.action, Action::Root(_))) {
        if cancelled() { step(app, d.id, "cancelled", ""); continue; }
        step(app, d.id, "running", "");
        let ok = match &d.action {
            Action::Sh(script) => stream(app, d.id, exec::sh(script, &[]), true),
            Action::Dirs(rels) => clear_dirs(app, d.id, home.as_deref(), rels),
            Action::Root(_) => true,
        };
        if ok { step(app, d.id, "done", ""); }
        else if cancelled() { step(app, d.id, "cancelled", "stopped"); }
        else { step(app, d.id, "failed", "see log"); failed += 1; }
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
    fn parses_apt_and_docker_output() {
        let apt = "Remv libfoo [1.0]\nRemv libbar [2.1-3]\nAfter this operation, 41.0 MB disk space will be freed.\n";
        assert_eq!(parse_remv(apt), vec!["libfoo", "libbar"]);
        assert_eq!(parse_freed(apt), Some(41_000_000));
        assert_eq!(parse_freed("After this operation, 1,024 kB disk space will be freed."), Some(1_024_000));
        assert_eq!(parse_freed("0 upgraded, 0 newly installed, 0 to remove"), None);
        assert_eq!(parse_docker_size("1.2GB (45%)"), Some(1_200_000_000));
        assert_eq!(parse_docker_size("3.4kB"), Some(3_400));
        assert_eq!(parse_docker_size("0B"), Some(0));
        assert_eq!(parse_docker_size("garbage"), None);
    }

    #[test]
    fn targets_never_leave_home() {
        let home = Path::new("/home/u");
        assert_eq!(safe_target(home, ".cache/pip"), Some(PathBuf::from("/home/u/.cache/pip")));
        for bad in ["", "..", "../etc", ".cache/../../etc", "/etc"] { assert!(safe_target(home, bad).is_none(), "{bad}"); }
        assert!(safe_target(Path::new("/"), ".cache").is_none());
        assert!(safe_target(Path::new("relative"), ".cache").is_none());
    }

    #[test]
    fn clearing_a_folder_keeps_it_and_never_follows_links() {
        let base = std::env::temp_dir().join(format!("devsetup-clean-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (target, outside) = (base.join("cache"), base.join("outside.txt"));
        std::fs::create_dir_all(target.join("sub")).unwrap();
        std::fs::write(target.join("a.txt"), "x").unwrap();
        std::fs::write(target.join("sub/b.txt"), "yy").unwrap();
        std::fs::write(&outside, "keep").unwrap();
        std::os::unix::fs::symlink(&outside, target.join("link")).unwrap();

        assert_eq!(clear_dir_contents(&target).unwrap(), 3);
        assert!(target.is_dir() && std::fs::read_dir(&target).unwrap().next().is_none());
        assert!(outside.exists(), "a link inside the folder must not delete its target");

        let linked = base.join("linkdir");
        std::os::unix::fs::symlink(&target, &linked).unwrap();
        assert!(clear_dir_contents(&linked).is_err(), "a symlinked folder is refused");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn measures_folder_size() {
        let base = std::env::temp_dir().join(format!("devsetup-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("d")).unwrap();
        std::fs::write(base.join("a"), "123").unwrap();
        std::fs::write(base.join("d/b"), "45").unwrap();
        assert_eq!(dir_size(&base), 5);
        let _ = std::fs::remove_dir_all(&base);
    }
}
