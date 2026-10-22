//! Doctor: read-only health checks with suggested fixes. Fixes are fixed operations chosen by id.
use crate::exec::{self, cancelled, log, step, stream};
use crate::manifest::{self, Manifest, Method, Tool};
use crate::{preflight, state, system};
use serde::Serialize;
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter};

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Serialize, Default, Clone)]
pub struct FixInfo {
    pub id: String,
    pub label: String,
    /// "run" (as you), "root" (password prompt) or "copy" (text to paste yourself)
    pub kind: &'static str,
    pub command: String,
    pub warning: String,
}

#[derive(Serialize, Default, Clone)]
pub struct Finding {
    pub id: String,
    pub title: String,
    /// "ok", "warn", "fail" or "info"
    pub status: &'static str,
    pub detail: String,
    pub advice: String,
    pub fix: Option<FixInfo>,
}

fn f(id: &str, title: &str, status: &'static str, detail: impl Into<String>, advice: impl Into<String>) -> Finding {
    Finding { id: id.into(), title: title.into(), status, detail: detail.into(), advice: advice.into(), fix: None }
}

fn with_fix(mut x: Finding, id: &str, label: &str, kind: &'static str, command: &str, warning: &str) -> Finding {
    x.fix = Some(FixInfo { id: id.into(), label: label.into(), kind, command: command.into(), warning: warning.into() });
    x
}

fn run_sh(script: &str) -> String {
    exec::sh(script, &[]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}
fn sh_ok(script: &str) -> bool {
    exec::sh(script, &[]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

// ---------- commands respond ----------

/// Only command-line tools from these groups are run with --version. GUI apps are never started.
const RUN_GROUPS: [&str; 9] = ["Essentials", "Languages and runtimes", "Systems and debugging", "Containers and cloud", "Databases", "AI harnesses", "Data and ML", "Git and hosting", "Security"];
const RUN_SKIP: [&str; 3] = ["meld", "scrcpy", "sqlitebrowser"];

/// Why `cmd --version` shows the command is broken, or None when it responds normally.
/// A normal non-zero exit (many tools reject --version) is not a failure.
fn broken_reason(cmd: &str) -> Option<String> {
    let out = exec::sh("timeout 5 \"$1\" --version 2>&1 </dev/null", &[cmd.to_string()]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    match out.status.code() {
        Some(126) | Some(127) => return Some("cannot be run".to_string()),
        Some(132) | Some(134) | Some(135) | Some(139) => return Some("crashes".to_string()),
        _ => {}
    }
    ["not installed", "no such file or directory", "cannot execute binary", "error while loading shared libraries"]
        .iter().find(|p| text.contains(*p)).map(|p| format!("reports \"{p}\""))
}

fn check_runs(m: &Manifest, apt: &HashSet<String>) -> Option<Finding> {
    let cands: Vec<&Tool> = m.tools.iter().filter(|t| {
        !t.check.is_empty() && RUN_GROUPS.contains(&t.group.as_str()) && !RUN_SKIP.contains(&t.id.as_str())
            && system::is_installed(t, apt) && system::on_path(&t.check)
    }).collect();
    if cands.is_empty() { return None; }
    let broken: Vec<String> = cands.iter().filter_map(|t| broken_reason(&t.check).map(|why| format!("{} ({why})", t.name))).collect();
    Some(if broken.is_empty() {
        f("runs", "Installed commands run", "ok", format!("{} commands respond normally", cands.len()), "")
    } else {
        f("runs", "Some installed commands are broken", "fail", broken.join(", "), "Install them again from Setup (untick, apply, then tick and install), or fix the folder they live in.")
    })
}

// ---------- terminal PATH ----------

const PATH_SCRIPT_POSIX: &str = r#"printf '__DEVSETUP_OK__\n'; for c in "$@"; do command -v "$c" >/dev/null 2>&1 || printf '__MISSING__%s\n' "$c"; done"#;
const PATH_SCRIPT_FISH: &str = r#"echo __DEVSETUP_OK__; for c in $argv; command -v $c >/dev/null 2>&1; or echo __MISSING__$c; end"#;

/// The rc file and the lines to add so the given folders are on PATH in `shell`.
fn path_snippet(shell: &str, home: &Path, dirs: &[PathBuf]) -> (String, String) {
    let (rc, fish) = match shell { "zsh" => ("~/.zshrc", false), "fish" => ("~/.config/fish/config.fish", true), "bash" => ("~/.bashrc", false), _ => ("~/.profile", false) };
    let mut lines: Vec<String> = vec![];
    for d in dirs {
        if d.ends_with(".local/share/mise/shims") {
            lines.push(if fish { "mise activate fish | source".to_string() } else { format!("eval \"$(mise activate {})\"", if shell == "zsh" { "zsh" } else { "bash" }) });
            continue;
        }
        let rel = d.strip_prefix(home).map(|r| r.to_string_lossy().into_owned()).unwrap_or_else(|_| d.to_string_lossy().into_owned());
        lines.push(if fish { format!("fish_add_path $HOME/{rel}") } else { format!("export PATH=\"$HOME/{rel}:$PATH\"") });
    }
    lines.dedup();
    (rc.to_string(), lines.join("\n"))
}

fn check_terminal_path(m: &Manifest, apt: &HashSet<String>) -> Option<Finding> {
    let home = system::home()?;
    // Tools that live in the user's home are the ones that depend on shell configuration.
    let found: Vec<(&Tool, PathBuf)> = m.tools.iter()
        .filter(|t| !t.check.is_empty() && !matches!(t.method, Method::Flatpak { .. }) && system::is_installed(t, apt))
        .filter_map(|t| system::find_in_path(&t.check).map(|p| (t, p)))
        .filter(|(_, p)| p.starts_with(&home)).collect();
    if found.is_empty() { return None; }
    let shell_path = std::env::var("SHELL").unwrap_or_default();
    let shell = Path::new(&shell_path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if !Path::new(&shell_path).is_file() || !["bash", "zsh", "sh", "dash", "fish"].contains(&shell.as_str()) {
        return Some(f("terminal-path", "Terminal PATH", "info", format!("Could not check: unsupported shell ({})", if shell.is_empty() { "unknown" } else { &shell }), ""));
    }
    let script = if shell == "fish" { PATH_SCRIPT_FISH } else { PATH_SCRIPT_POSIX };
    let mut cmd = Command::new("timeout");
    cmd.args(["20", shell_path.as_str(), "-ic", script]);
    if shell != "fish" { cmd.arg("devsetup"); }
    cmd.args(found.iter().map(|(t, _)| t.check.as_str())).stdin(Stdio::null()).stderr(Stdio::null());
    let out = String::from_utf8_lossy(&cmd.output().ok()?.stdout).into_owned();
    if !out.contains("__DEVSETUP_OK__") {
        return Some(f("terminal-path", "Terminal PATH", "info", format!("Could not check: {shell} did not start cleanly"), ""));
    }
    let missing: HashSet<&str> = out.lines().filter_map(|l| l.strip_prefix("__MISSING__")).collect();
    if missing.is_empty() { return Some(f("terminal-path", "Terminal PATH", "ok", format!("{} tools in your home folder are found from a new {shell} terminal", found.len()), "")); }
    let lost: Vec<&(&Tool, PathBuf)> = found.iter().filter(|(t, _)| missing.contains(t.check.as_str())).collect();
    let dirs: BTreeSet<PathBuf> = lost.iter().filter_map(|(_, p)| p.parent().map(|d| d.to_path_buf())).collect();
    let (rc, text) = path_snippet(&shell, &home, &dirs.into_iter().collect::<Vec<_>>());
    let names = lost.iter().map(|(t, _)| t.name.as_str()).collect::<Vec<_>>().join(", ");
    let x = f("terminal-path", "Some tools are not on your terminal PATH", "warn", format!("Found by DevSetup but not from a new {shell} terminal: {names}"),
              format!("Add these lines to {rc}, then open a new terminal."));
    Some(with_fix(x, "path-snippet", &format!("Copy the lines for {rc}"), "copy", &text, ""))
}

// ---------- Docker ----------

fn check_docker(m: &Manifest, apt: &HashSet<String>) -> Vec<Finding> {
    let mut v = vec![];
    let installed = m.tools.iter().find(|t| t.id == "docker").map_or(false, |t| system::is_installed(t, apt)) || system::on_path("docker");
    if !installed { return v; }
    if system::on_path("systemctl") {
        let active = run_sh("systemctl is-active docker 2>/dev/null").trim() == "active";
        let enabled = run_sh("systemctl is-enabled docker 2>/dev/null").trim() == "enabled";
        v.push(if active && enabled { f("docker-service", "Docker service", "ok", "Running and starts at boot", "") } else {
            with_fix(f("docker-service", "Docker service", "warn", format!("Running: {}. Starts at boot: {}", if active { "yes" } else { "no" }, if enabled { "yes" } else { "no" }), "Docker only works while its service runs."),
                     "docker-service", "Start Docker and enable it at boot", "root", "systemctl enable --now docker", "")
        });
    }
    let user = run_sh("id -un").trim().to_string();
    let in_session = run_sh("id -nG").split_whitespace().any(|g| g == "docker");
    let assigned = run_sh("getent group docker 2>/dev/null").trim().rsplit(':').next().unwrap_or("").split(',').any(|g| g == user);
    v.push(if in_session { f("docker-group", "Docker group", "ok", "You are in the docker group, so docker works without sudo", "") }
    else if assigned { f("docker-group", "Docker group", "warn", "You were added to the docker group, but this session started before that", "Log out and back in, then run these checks again.") }
    else {
        with_fix(f("docker-group", "Docker group", "warn", "You are not in the docker group, so docker needs sudo", "Adding yourself lets you run docker directly. Log out and back in afterwards."),
                 "docker-group", "Add me to the docker group", "root", "usermod -aG docker <you>",
                 "Members of the docker group can gain full root access on this machine. Only do this on a computer you trust.\n\nContinue?")
    });
    if in_session && !sh_ok("timeout 10 docker info") {
        v.push(f("docker-daemon", "Docker daemon", "fail", "docker info failed", "Check that the Docker service is running (see above)."));
    }
    v
}

// ---------- smaller checks ----------

fn check_managed(m: &Manifest, apt: &HashSet<String>) -> Option<Finding> {
    let managed = state::load().managed;
    if managed.is_empty() { return None; }
    let gone: Vec<String> = managed.iter().filter_map(|id| m.tools.iter().find(|t| &t.id == id)).filter(|t| !system::is_installed(t, apt)).map(|t| t.name.clone()).collect();
    Some(if gone.is_empty() { f("managed", "Tools DevSetup installed", "ok", format!("All {} are still present", managed.len()), "") }
    else { f("managed", "Some tools DevSetup installed are gone", "warn", gone.join(", "), "They were removed or broken outside DevSetup. Tick them in Setup to install them again. After a Node upgrade with mise, global npm tools need this.") })
}

fn check_git() -> Option<Finding> {
    if !system::on_path("git") { return None; }
    let get = |k: &str| run_sh(&format!("git config --global {k} 2>/dev/null")).trim().to_string();
    let (name, email) = (get("user.name"), get("user.email"));
    Some(if !name.is_empty() && !email.is_empty() { f("git", "Git identity", "ok", format!("{name} <{email}>"), "") } else {
        with_fix(f("git", "Git identity is not set", "warn", "Commits need a name and an email address", "Run these two commands with your own details."),
                 "git-identity", "Copy the commands", "copy", "git config --global user.name \"Your Name\"\ngit config --global user.email \"you@example.com\"", "")
    })
}

fn check_flatpak() -> Option<Finding> {
    if !system::on_path("flatpak") { return None; }
    let has = run_sh("flatpak remotes --columns=name 2>/dev/null").lines().any(|l| l.trim() == "flathub");
    Some(if has { f("flathub", "Flathub", "ok", "The Flathub remote is set up", "") } else {
        with_fix(f("flathub", "Flathub is not set up", "warn", "Flatpak apps cannot be installed without a remote", "This adds Flathub for your user only."),
                 "flathub", "Add Flathub", "run", "flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo", "")
    })
}

fn check_apt() -> Option<Finding> {
    if !system::on_path("apt-get") || !Path::new("/var/lib/dpkg/status").exists() { return None; }
    let o = Command::new("apt-get").arg("check").env("LC_ALL", "C").output().ok()?;
    Some(if o.status.success() { f("apt", "Package database", "ok", "apt reports no broken packages", "") } else {
        let first = String::from_utf8_lossy(&o.stderr).lines().find(|l| l.starts_with("E:")).unwrap_or("apt check failed").to_string();
        f("apt", "Package database has problems", "fail", first, "Run `sudo apt --fix-broken install` in a terminal.")
    })
}

fn check_disk() -> Option<Finding> {
    let (mount, kb) = preflight::free_space("/")?;
    let avail = kb * 1024;
    let gb = format!("{:.1} GB free on {mount}", avail as f64 / 1e9);
    Some(if avail < 2 * GIB { f("disk", "Disk space", "fail", gb, "Open Cleaner to free space.") }
    else if avail < 5 * GIB { f("disk", "Disk space", "warn", gb, "Open Cleaner to free space.") }
    else { f("disk", "Disk space", "ok", gb, "") })
}

fn check_reboot() -> Option<Finding> {
    Path::new("/var/run/reboot-required").exists().then(|| f("reboot", "A restart is needed", "info", "Updates were installed that need a restart", "Restart when convenient."))
}

const POLICY_PATH: &str = "/usr/share/polkit-1/actions/dev.devsetup.helper.policy";

/// Whether the root helper can start from where DevSetup runs, and whether polkit shows DevSetup's own prompt.
fn root_helper_finding(appimage: Option<&str>, exe: &str, policy_installed: bool) -> Finding {
    if appimage.is_some() || exe.contains("/.mount_") {
        return f("root-helper", "System steps cannot run from an AppImage", "warn",
            "DevSetup is running as an AppImage. It is mounted where only your user can read it, so pkexec cannot start the root helper from there.",
            "Install the .deb package instead. Steps that run as you (Flatpak, mise, npm, install scripts) still work.");
    }
    let installed = exe == "/usr/bin/devsetup";
    if installed && policy_installed {
        return f("root-helper", "Root helper", "ok", "Installed in /usr/bin with its polkit policy, so the password prompt names DevSetup", "");
    }
    if installed {
        return f("root-helper", "Root helper", "info", "The polkit policy is not installed, so the password prompt uses polkit's generic wording. System steps still work.", "Reinstall the .deb to restore the policy.");
    }
    f("root-helper", "Root helper", "info", format!("Running from {exe}, not from the .deb. System steps still work, but the password prompt uses polkit's generic wording."), "")
}

pub fn scan() -> Vec<Finding> {
    let m = manifest::load();
    let apt = system::apt_installed();
    let mut v: Vec<Finding> = vec![];
    v.extend(check_managed(&m, &apt));
    v.extend(check_runs(&m, &apt));
    v.extend(check_terminal_path(&m, &apt));
    v.extend(check_docker(&m, &apt));
    v.extend(check_git());
    v.extend(check_flatpak());
    v.extend(check_apt());
    v.extend(check_disk());
    v.extend(check_reboot());
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    v.push(root_helper_finding(std::env::var("APPIMAGE").ok().as_deref(), &exe, Path::new(POLICY_PATH).exists()));
    v.sort_by_key(|x| match x.status { "fail" => 0, "warn" => 1, "info" => 2, _ => 3 });
    v
}

// ---------- fixes ----------

enum FixAction { Root(&'static str), Sh(&'static str) }

fn fix_action(id: &str) -> Option<FixAction> {
    match id {
        "docker-group" => Some(FixAction::Root("docker-group")),
        "docker-service" => Some(FixAction::Root("docker-service")),
        "flathub" => Some(FixAction::Sh("flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo")),
        _ => None,
    }
}

pub fn start_fix(app: AppHandle, id: String) -> Result<(), String> {
    let action = fix_action(&id).ok_or_else(|| "unknown fix".to_string())?;
    if exec::RUNNING.swap(true, Ordering::SeqCst) { return Err("a run is already in progress".into()); }
    exec::CANCEL.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        step(&app, &id, "running", "");
        let ok = match action {
            FixAction::Root(op) => {
                let mut cmd = Command::new("pkexec");
                match std::env::current_exe() {
                    Ok(exe) => { cmd.arg(exe).arg("--helper").arg("fix").arg(op); }
                    Err(e) => log(&app, &id, "stderr", &format!("cannot find own executable: {e}")),
                }
                stream(&app, &id, cmd, false)
            }
            FixAction::Sh(script) => stream(&app, &id, exec::sh(script, &[]), true),
        };
        if ok { step(&app, &id, "done", ""); } else if cancelled() { step(&app, &id, "cancelled", "stopped"); } else { step(&app, &id, "failed", "see log"); }
        let was_cancelled = cancelled();
        exec::RUNNING.store(false, Ordering::SeqCst);
        let _ = app.emit("run-done", exec::Done { ok: ok && !was_cancelled, failed: usize::from(!ok && !was_cancelled), cancelled: was_cancelled });
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_broken_and_working_commands() {
        assert!(broken_reason("definitely-not-a-command-xyz").is_some());
        assert!(broken_reason("true").is_none());
    }

    #[test]
    fn path_snippets_match_the_shell() {
        let home = Path::new("/home/u");
        let dirs = vec![PathBuf::from("/home/u/.local/bin"), PathBuf::from("/home/u/.local/share/mise/shims")];
        assert_eq!(path_snippet("bash", home, &dirs), ("~/.bashrc".to_string(), "export PATH=\"$HOME/.local/bin:$PATH\"\neval \"$(mise activate bash)\"".to_string()));
        assert_eq!(path_snippet("zsh", home, &dirs[..1]), ("~/.zshrc".to_string(), "export PATH=\"$HOME/.local/bin:$PATH\"".to_string()));
        assert_eq!(path_snippet("fish", home, &dirs), ("~/.config/fish/config.fish".to_string(), "fish_add_path $HOME/.local/bin\nmise activate fish | source".to_string()));
    }

    #[test]
    fn only_known_fixes_exist() {
        assert!(fix_action("docker-group").is_some() && fix_action("flathub").is_some());
        assert!(fix_action("rm -rf /").is_none() && fix_action("path-snippet").is_none());
    }

    #[test]
    fn findings_sort_failures_first() {
        let mut v = vec![f("a", "a", "ok", "", ""), f("b", "b", "fail", "", ""), f("c", "c", "warn", "", "")];
        v.sort_by_key(|x| match x.status { "fail" => 0, "warn" => 1, "info" => 2, _ => 3 });
        assert_eq!(v.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["b", "c", "a"]);
    }

    #[test]
    fn the_root_helper_finding_covers_appimage_deb_and_source_runs() {
        let appimage = root_helper_finding(Some("/home/u/DevSetup.AppImage"), "/tmp/.mount_DevSetXyz/usr/bin/devsetup", false);
        assert_eq!(appimage.status, "warn");
        assert_eq!(root_helper_finding(None, "/tmp/.mount_DevSetXyz/usr/bin/devsetup", true).status, "warn", "a mount path alone is enough");
        assert_eq!(root_helper_finding(None, "/usr/bin/devsetup", true).status, "ok");
        let no_policy = root_helper_finding(None, "/usr/bin/devsetup", false);
        assert_eq!(no_policy.status, "info");
        assert!(no_policy.detail.contains("policy"));
        let source = root_helper_finding(None, "/home/u/DevSetup/src-tauri/target/debug/devsetup", false);
        assert_eq!(source.status, "info");
        assert!(source.detail.contains("target/debug/devsetup"));
    }
}
