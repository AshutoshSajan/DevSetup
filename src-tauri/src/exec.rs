//! Runs a plan: one pkexec call for all apt work, then user-level steps one by one.
//! Cancel stops user-level steps (their whole process group). The apt step runs as root
//! and cannot be interrupted safely, so cancel takes effect after it finishes.
use crate::manifest::{self, Manifest, Method, Tool};
use crate::{state, system};
use serde::Serialize;
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use tauri::{AppHandle, Emitter};

pub(crate) static RUNNING: AtomicBool = AtomicBool::new(false);
pub(crate) static CANCEL: AtomicBool = AtomicBool::new(false);
/// Process group of the user-level step that is running now (0 = none).
static CHILD_PGID: AtomicI32 = AtomicI32::new(0);

#[derive(Clone, Serialize)] struct Log { id: String, stream: &'static str, line: String }
#[derive(Clone, Serialize)] struct Step { id: String, state: &'static str, message: String }
#[derive(Clone, Serialize)] pub(crate) struct Done { pub ok: bool, pub failed: usize, pub cancelled: bool }

pub(crate) fn cancelled() -> bool { CANCEL.load(Ordering::SeqCst) }

pub(crate) fn log(app: &AppHandle, id: &str, stream: &'static str, line: &str) {
    let _ = app.emit("run-log", Log { id: id.into(), stream, line: line.into() });
}
pub(crate) fn step(app: &AppHandle, id: &str, state: &'static str, message: &str) {
    let _ = app.emit("run-step", Step { id: id.into(), state, message: message.into() });
}

fn kill_group(pg: i32, sig: i32) {
    // SAFETY: plain syscall; a negative pid addresses the whole process group.
    unsafe { libc::kill(-pg, sig); }
}

/// Asks the running install to stop. Safe to call at any time.
pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
    let pg = CHILD_PGID.load(Ordering::SeqCst);
    if pg > 0 {
        kill_group(pg, libc::SIGTERM);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            if CHILD_PGID.load(Ordering::SeqCst) == pg { kill_group(pg, libc::SIGKILL); }
        });
    }
}

fn pump<R: Read + Send + 'static>(app: &AppHandle, id: &str, stream: &'static str, r: R) -> std::thread::JoinHandle<()> {
    let (app, id) = (app.clone(), id.to_string());
    std::thread::spawn(move || {
        let mut rd = BufReader::new(r);
        let mut buf = Vec::new();
        while rd.read_until(b'\n', &mut buf).unwrap_or(0) > 0 {
            log(&app, &id, stream, String::from_utf8_lossy(&buf).trim_end());
            buf.clear();
        }
    })
}

/// Runs a command, streaming its output to the UI. Returns true on exit code 0.
/// `group` = run in its own process group so cancel can stop it (user-level steps only).
pub(crate) fn stream(app: &AppHandle, id: &str, mut cmd: Command, group: bool) -> bool {
    if group && cancelled() { return false; }
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if group { cmd.process_group(0); }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => { log(app, id, "stderr", &format!("could not start: {e}")); return false; }
    };
    let pg = child.id() as i32;
    if group {
        CHILD_PGID.store(pg, Ordering::SeqCst);
        if cancelled() { kill_group(pg, libc::SIGTERM); } // cancel arrived just before the store
    }
    let mut hs = vec![];
    if let Some(o) = child.stdout.take() { hs.push(pump(app, id, "stdout", o)); }
    if let Some(e) = child.stderr.take() { hs.push(pump(app, id, "stderr", e)); }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    if group { CHILD_PGID.store(0, Ordering::SeqCst); }
    for h in hs { let _ = h.join(); }
    ok
}

fn child_path() -> String {
    let mut dirs: Vec<String> = vec![];
    if let Some(h) = system::home() {
        for d in [".local/bin", ".local/share/mise/shims", ".cargo/bin", ".bun/bin"] { dirs.push(h.join(d).to_string_lossy().into_owned()); }
    }
    if let Ok(p) = std::env::var("PATH") { dirs.push(p); }
    dirs.join(":")
}

/// `sh -c <script> sh <args...>`: values reach the script as "$1", "$2", never spliced into the text.
pub(crate) fn sh(script: &str, args: &[String]) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(script).arg("sh").args(args).env("PATH", child_path());
    c
}

/// Downloads a script, verifies its SHA-256 when a pin is given, and only then runs it.
/// Arguments: url, sha256 (empty = not pinned), then arguments for the script.
const SCRIPT_RUNNER: &str = r#"url="$1"; sum="$2"; shift 2
tmp=$(mktemp) || exit 1
rc=1
if curl --proto '=https' --tlsv1.2 -fsSL "$url" -o "$tmp"; then
  if [ -n "$sum" ] && ! echo "$sum  $tmp" | sha256sum -c - >/dev/null 2>&1; then
    echo "Checksum mismatch for $url" >&2
    echo "  expected: $sum" >&2
    echo "  actual:   $(sha256sum "$tmp" | cut -d' ' -f1)" >&2
    echo "Refusing to run it. The script may have been updated: run 'npm run pin' and review the change." >&2
  else
    sh "$tmp" "$@"; rc=$?
  fi
fi
rm -f "$tmp"
exit $rc"#;

pub(crate) fn install_cmd(t: &Tool) -> Option<Command> {
    Some(match &t.method {
        Method::Apt { .. } => return None,
        Method::Mise { tool, version } => {
            let v = version.clone().unwrap_or_else(|| if tool.as_str() == "node" { "lts".to_string() } else { "latest".to_string() });
            sh("mise use --global \"$1\"", &[format!("{tool}@{v}")])
        }
        Method::Npm { package, version } => {
            let spec = match version { Some(v) => format!("{package}@{v}"), None => package.clone() };
            sh("npm install -g \"$1\"", &[spec])
        }
        Method::Pipx { package, version } => {
            let spec = match version { Some(v) => format!("{package}=={v}"), None => package.clone() };
            sh("pipx install \"$1\"", &[spec])
        }
        Method::Flatpak { app_id } => sh(
            "flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo && flatpak install --user -y --noninteractive flathub \"$1\"",
            &[app_id.clone()],
        ),
        Method::Script { url, sha256, args, .. } => {
            let mut a = vec![url.clone(), sha256.clone().unwrap_or_default()];
            a.extend(args.iter().cloned());
            sh(SCRIPT_RUNNER, &a)
        }
    })
}

/// Removes a mise tool: drop it from the global config, delete every installed version, refresh shims,
/// then fail if mise still reports the tool as installed.
const MISE_REMOVE: &str = r#"mise unuse --global "$1" || true
mise uninstall --all "$1" || true
mise reshim
if mise ls --installed "$1" 2>/dev/null | grep -q .; then echo "$1 is still installed" >&2; exit 1; fi"#;

pub(crate) fn remove_cmd(t: &Tool) -> Option<Command> {
    Some(match &t.method {
        Method::Apt { .. } => return None, // handled by the root helper
        Method::Flatpak { app_id } => sh("flatpak uninstall --user -y --noninteractive \"$1\"", &[app_id.clone()]),
        Method::Mise { tool, .. } => sh(MISE_REMOVE, &[tool.clone()]),
        Method::Npm { package, .. } => sh("npm uninstall -g \"$1\"", &[package.clone()]),
        Method::Pipx { package, .. } => sh("pipx uninstall \"$1\"", &[package.clone()]),
        Method::Script { uninstall: Some(u), .. } => sh(u, &[]),
        Method::Script { .. } => return None, // no known uninstall command: remove manually
    })
}

/// Splits removal candidates into those that can go and those another remaining tool still needs.
fn split_removals(m: &Manifest, remove: &[Tool], to_install: &[Tool], apt: &HashSet<String>) -> (Vec<Tool>, Vec<(Tool, String)>) {
    let removing: HashSet<&str> = remove.iter().map(|t| t.id.as_str()).collect();
    let installing: HashSet<&str> = to_install.iter().map(|t| t.id.as_str()).collect();
    let (mut ok, mut blocked) = (vec![], vec![]);
    for t in remove {
        let needed_by: Vec<&str> = m.tools.iter()
            .filter(|d| d.requires.contains(&t.id)
                && ((system::is_installed(d, apt) && !removing.contains(d.id.as_str())) || installing.contains(d.id.as_str())))
            .map(|d| d.name.as_str()).collect();
        if needed_by.is_empty() { ok.push(t.clone()); } else { blocked.push((t.clone(), needed_by.join(", "))); }
    }
    (ok, blocked)
}

/// Tools that depend on others come first, so nothing is removed while something still needs it.
pub(crate) fn removal_order(set: &[Tool]) -> Vec<Tool> {
    fn visit(t: &Tool, set: &[Tool], seen: &mut HashSet<String>, out: &mut Vec<Tool>) {
        if !seen.insert(t.id.clone()) { return; }
        for r in &t.requires { if let Some(d) = set.iter().find(|x| &x.id == r) { visit(d, set, seen, out); } }
        out.push(t.clone());
    }
    let (mut seen, mut out) = (HashSet::new(), vec![]);
    for t in set { visit(t, set, &mut seen, &mut out); }
    out.reverse();
    out
}

/// Adds missing requirements and orders everything so requirements come first.
pub(crate) fn resolve(m: &Manifest, ids: &[String], apt: &HashSet<String>) -> Vec<Tool> {
    fn visit(m: &Manifest, id: &str, apt: &HashSet<String>, seen: &mut HashSet<String>, out: &mut Vec<Tool>) {
        if !seen.insert(id.to_string()) { return; }
        let Some(t) = m.tools.iter().find(|t| t.id == id) else { return; };
        for r in &t.requires { visit(m, r, apt, seen, out); }
        if !system::is_installed(t, apt) { out.push(t.clone()); }
    }
    let (mut seen, mut out) = (HashSet::new(), vec![]);
    for id in ids { visit(m, id, apt, &mut seen, &mut out); }
    out
}

/// Quotes an argument for display, the way a shell would need it.
fn quote(a: &str) -> String {
    if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "@._/:=+-".contains(c)) { a.to_string() }
    else { format!("'{}'", a.replace('\'', "'\\''")) }
}

/// Human-readable form of a command, built from the very Command that will run, so the review cannot drift
/// from what executes. For `sh -c script sh args...` the positional arguments are substituted into the script.
pub(crate) fn render(cmd: &Command) -> String {
    let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    if cmd.get_program() == "sh" && args.len() >= 3 && args[0] == "-c" {
        let mut script = args[1].clone();
        for (i, a) in args[3..].iter().enumerate() { script = script.replace(&format!("\"${}\"", i + 1), &quote(a)); }
        return script;
    }
    let mut parts = vec![cmd.get_program().to_string_lossy().into_owned()];
    parts.extend(args.iter().map(|a| quote(a)));
    parts.join(" ")
}

/// Everything a run will do, decided in one place so the review screen and the executor agree.
pub(crate) struct Planned {
    pub to_install: Vec<Tool>,
    pub candidates: Vec<Tool>,
    pub to_remove: Vec<Tool>,
    pub blocked: Vec<(Tool, String)>,
}

pub(crate) fn plan_steps(m: &Manifest, apt: &HashSet<String>, managed: &HashSet<String>, install: &[String], remove: &[String]) -> Planned {
    let to_install = resolve(m, install, apt);
    // Safety: only tools DevSetup installed itself can be removed.
    let candidates: Vec<Tool> = remove.iter().filter(|id| managed.contains(id.as_str())).filter_map(|id| m.tools.iter().find(|t| &t.id == id).cloned()).collect();
    // A tool that another remaining tool needs is kept.
    let (to_remove, blocked) = split_removals(m, &candidates, &to_install, apt);
    Planned { to_install, candidates, to_remove, blocked }
}

fn run(app: &AppHandle, install: Vec<String>, remove: Vec<String>, strict: bool) -> usize {
    let m = manifest::load();
    let apt = system::apt_installed();
    let mut st = state::load();
    let mut failed = 0;

    // Safety: only tools DevSetup installed itself can be removed.
    let managed: HashSet<String> = st.managed.iter().cloned().collect();
    let Planned { to_install, candidates, to_remove, blocked } = plan_steps(&m, &apt, &managed, &install, &remove);

    for t in to_install.iter().chain(candidates.iter()) { step(app, &t.id, "queued", ""); }
    for (t, by) in &blocked {
        log(app, &t.id, "stderr", &format!("{} was not removed because {} needs it.", t.name, by));
        step(app, &t.id, "skipped", &format!("needed by {by}"));
    }

    // Phase 0: user-level removals, dependents first. Runs before apt so tools like Flatpak still exist.
    let user_rm: Vec<Tool> = to_remove.iter().filter(|t| !matches!(t.method, Method::Apt { .. })).cloned().collect();
    for t in removal_order(&user_rm) {
        if cancelled() { step(app, &t.id, "cancelled", ""); continue; }
        let Some(cmd) = remove_cmd(&t) else { step(app, &t.id, "skipped", "remove manually"); continue; };
        step(app, &t.id, "running", "");
        if stream(app, &t.id, cmd, true) {
            step(app, &t.id, "done", "removed");
            st.managed.retain(|x| x != &t.id);
            state::save(&st);
        } else if cancelled() { step(app, &t.id, "cancelled", "stopped"); }
        else { step(app, &t.id, "failed", "see log"); failed += 1; }
    }

    // Phase 1: all apt work in a single pkexec call (one password prompt). Not cancellable once started.
    let apt_in: Vec<&Tool> = to_install.iter().filter(|t| matches!(t.method, Method::Apt { .. })).collect();
    let apt_rm: Vec<&Tool> = to_remove.iter().filter(|t| matches!(t.method, Method::Apt { .. })).collect();
    if !apt_in.is_empty() || !apt_rm.is_empty() {
        if cancelled() {
            for t in apt_in.iter().chain(apt_rm.iter()) { step(app, &t.id, "cancelled", ""); }
        } else {
            for t in apt_in.iter().chain(apt_rm.iter()) { step(app, &t.id, "running", ""); }
            let mut cmd = Command::new("pkexec");
            match std::env::current_exe() {
                Ok(exe) => { cmd.arg(exe).arg("--helper").arg("apt"); }
                Err(e) => log(app, "apt", "stderr", &format!("cannot find own executable: {e}")),
            }
            for t in &apt_in { cmd.arg(format!("+{}", t.id)); }
            for t in &apt_rm { cmd.arg(format!("-{}", t.id)); }
            let ok = stream(app, "apt", cmd, false);
            for t in &apt_in {
                if ok { step(app, &t.id, "done", ""); if !st.managed.contains(&t.id) { st.managed.push(t.id.clone()); } }
                else { step(app, &t.id, "failed", "see log"); failed += 1; }
            }
            for t in &apt_rm {
                if ok { step(app, &t.id, "done", "removed"); st.managed.retain(|x| x != &t.id); }
                else { step(app, &t.id, "failed", "see log"); failed += 1; }
            }
            state::save(&st);
        }
    }

    // Phase 2: user-level installs, in dependency order.
    for t in to_install.iter().filter(|t| !matches!(t.method, Method::Apt { .. })) {
        if cancelled() { step(app, &t.id, "cancelled", ""); continue; }
        if strict && matches!(t.method, Method::Script { .. }) && !t.script_is_pinned() {
            log(app, &t.id, "stderr", "Strict mode: this install script has no pinned checksum, so it was not run.");
            step(app, &t.id, "failed", "not pinned");
            failed += 1;
            continue;
        }
        step(app, &t.id, "running", "");
        let ok = install_cmd(t).map(|c| stream(app, &t.id, c, true)).unwrap_or(false);
        if ok {
            step(app, &t.id, "done", "");
            if !st.managed.contains(&t.id) { st.managed.push(t.id.clone()); }
            state::save(&st);
        } else if cancelled() {
            log(app, &t.id, "stderr", "Cancelled. This step may have left partial files.");
            step(app, &t.id, "cancelled", "stopped");
        } else { step(app, &t.id, "failed", "see log"); failed += 1; }
    }
    failed
}

pub fn start(app: AppHandle, install: Vec<String>, remove: Vec<String>, strict: bool) -> Result<(), String> {
    if RUNNING.swap(true, Ordering::SeqCst) { return Err("a run is already in progress".into()); }
    CANCEL.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let failed = run(&app, install, remove, strict);
        let was_cancelled = cancelled();
        RUNNING.store(false, Ordering::SeqCst);
        let _ = app.emit("run-done", Done { ok: failed == 0 && !was_cancelled, failed, cancelled: was_cancelled });
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Runs SCRIPT_RUNNER with a stub `curl` that copies a local fixture, so no network is needed.
    fn run_runner(sum: &str) -> (bool, Option<String>) {
        let dir = std::env::temp_dir().join(format!("devsetup-runner-{}-{}", std::process::id(), sum.len()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let curl = dir.join("bin/curl");
        std::fs::write(&curl, "#!/bin/sh\nwhile [ $# -gt 0 ]; do [ \"$1\" = \"-o\" ] && out=\"$2\"; shift; done\ncp \"$FIXTURE\" \"$out\"\n").unwrap();
        std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("fixture.sh"), "echo \"ran with: $*\" > \"$MARKER\"\n").unwrap();
        let sum = if sum == "good" {
            let out = Command::new("sha256sum").arg(dir.join("fixture.sh")).output().unwrap();
            String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap().to_string()
        } else { sum.to_string() };
        let ok = Command::new("sh").arg("-c").arg(SCRIPT_RUNNER).arg("sh").arg("https://example.test/x.sh").arg(&sum).arg("-y")
            .env("PATH", format!("{}:/usr/bin:/bin", dir.join("bin").display()))
            .env("FIXTURE", dir.join("fixture.sh")).env("MARKER", dir.join("marker"))
            .output().unwrap().status.success();
        let marker = std::fs::read_to_string(dir.join("marker")).ok();
        let _ = std::fs::remove_dir_all(&dir);
        (ok, marker)
    }

    #[test]
    fn matching_checksum_runs_the_script_with_its_arguments() {
        let (ok, marker) = run_runner("good");
        assert!(ok);
        assert_eq!(marker.unwrap().trim(), "ran with: -y");
    }

    #[test]
    fn wrong_checksum_refuses_to_run_the_script() {
        let (ok, marker) = run_runner(&"0".repeat(64));
        assert!(!ok);
        assert!(marker.is_none());
    }

    #[test]
    fn unpinned_script_runs() {
        let (ok, marker) = run_runner("");
        assert!(ok && marker.is_some());
    }

    fn tool(id: &str, requires: &[&str], method: Method) -> Tool {
        Tool { id: id.into(), name: id.into(), group: "g".into(), check: String::new(), roles: vec![],
               requires: requires.iter().map(|r| r.to_string()).collect(), method }
    }

    #[test]
    fn removal_is_blocked_while_a_dependent_stays() {
        let a = tool("a", &[], Method::Apt { package: "pkg-a".into() });
        let b = tool("b", &["a"], Method::Apt { package: "pkg-b".into() });
        let m = Manifest { roles: vec![], tools: vec![a.clone(), b.clone()] };
        let installed: HashSet<String> = ["pkg-a", "pkg-b"].iter().map(|s| s.to_string()).collect();
        let (ok, blocked) = split_removals(&m, &[a.clone()], &[], &installed);
        assert!(ok.is_empty());
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].1, "b");
        let (ok, blocked) = split_removals(&m, &[a, b], &[], &installed);
        assert_eq!(ok.len(), 2);
        assert!(blocked.is_empty());
    }

    #[test]
    fn dependents_are_removed_before_what_they_need() {
        let node = tool("node", &[], Method::Mise { tool: "node".into(), version: None });
        let codex = tool("codex", &["node"], Method::Npm { package: "x".into(), version: None });
        let order: Vec<String> = removal_order(&[node, codex]).into_iter().map(|t| t.id).collect();
        assert_eq!(order, vec!["codex", "node"]);
    }

    #[test]
    fn render_substitutes_positional_arguments() {
        assert_eq!(render(&sh("mise use --global \"$1\"", &["node@lts".to_string()])), "mise use --global node@lts");
        assert_eq!(render(&sh("pipx install \"$1\"", &["a b".to_string()])), "pipx install 'a b'");
        let mut c = Command::new("pkexec");
        c.arg("--helper").arg("+git");
        assert_eq!(render(&c), "pkexec --helper +git");
    }

    #[test]
    fn plan_adds_requirements_in_install_order() {
        let a = tool("a", &[], Method::Apt { package: "pkg-a".into() });
        let b = tool("b", &["a"], Method::Apt { package: "pkg-b".into() });
        let m = Manifest { roles: vec![], tools: vec![b.clone(), a.clone()] };
        let pl = plan_steps(&m, &HashSet::new(), &HashSet::new(), &["b".to_string()], &[]);
        let ids: Vec<&str> = pl.to_install.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }
}
