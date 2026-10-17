//! Git and GitHub wizard: identity, SSH key, GitHub sign-in, signed commits.
//! Only public keys are ever read. Values from the UI are validated, passed as arguments (never through a shell),
//! and key paths must be public keys directly inside ~/.ssh.
use crate::exec::{self, cancelled, step, stream};
use crate::system;
use serde::Serialize;
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Default, Clone)]
pub struct KeyInfo { pub path: String, pub fingerprint: String, pub public: String }

#[derive(Serialize, Default)]
pub struct GitState {
    pub git_installed: bool,
    pub ssh_installed: bool,
    pub gh_installed: bool,
    pub git_version: String,
    pub name: String,
    pub email: String,
    pub default_branch: String,
    pub gh_user: Option<String>,
    pub keys: Vec<KeyInfo>,
    pub signing: bool,
    pub signing_key: String,
}

#[derive(Serialize, Default)]
pub struct SshTest { pub ok: bool, pub output: String }

// ---------- validation and parsing ----------

fn clean_text(s: &str, max: usize) -> Result<String, String> {
    let t = s.trim();
    if t.is_empty() || t.chars().count() > max || t.chars().any(|c| c.is_control()) { return Err("Enter a value without line breaks.".into()); }
    Ok(t.to_string())
}

fn valid_email(s: &str) -> bool {
    let parts: Vec<&str> = s.split('@').collect();
    s.len() <= 120 && !s.starts_with('-') && !s.chars().any(|c| c.is_whitespace() || c.is_control())
        && parts.len() == 2 && !parts[0].is_empty() && parts[1].contains('.') && !parts[1].starts_with('.') && !parts[1].ends_with('.')
}

fn git_version_at_least(text: &str, major: u32, minor: u32) -> bool {
    let v: Vec<u32> = text.split_whitespace().nth(2).unwrap_or("").split('.').filter_map(|p| p.parse().ok()).collect();
    match (v.first(), v.get(1)) { (Some(a), Some(b)) => (*a, *b) >= (major, minor), _ => false }
}

/// "Logged in to github.com account octocat (keyring)" or the older "... as octocat".
fn parse_gh_user(text: &str) -> Option<String> {
    for l in text.lines().filter(|l| l.contains("Logged in to github.com")) {
        for marker in [" account ", " as "] {
            if let Some(i) = l.find(marker) {
                if let Some(w) = l[i + marker.len()..].split_whitespace().next() { return Some(w.to_string()); }
            }
        }
    }
    None
}

/// Second field of `ssh-keygen -l` output: "256 SHA256:abc comment (ED25519)".
fn parse_fingerprint(text: &str) -> String { text.split_whitespace().nth(1).unwrap_or("").to_string() }

/// `path` must be a public key (.pub) directly inside `ssh_dir`.
fn pub_key_in(ssh_dir: &Path, path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    let ok = p.parent() == Some(ssh_dir) && p.extension().map_or(false, |e| e == "pub")
        && std::fs::symlink_metadata(&p).map(|m| m.is_file()).unwrap_or(false);
    if ok { Ok(p) } else { Err("Choose a public key (.pub file) from your ~/.ssh folder.".into()) }
}

/// One allowed_signers line: `email namespaces="git" keytype base64`. The key comment is dropped.
fn signers_line(email: &str, public: &str) -> Option<String> {
    let mut f = public.split_whitespace();
    let (kind, blob) = (f.next()?, f.next()?);
    if !kind.starts_with("ssh-") && !kind.starts_with("ecdsa-") && !kind.starts_with("sk-") { return None; }
    Some(format!("{email} namespaces=\"git\" {kind} {blob}"))
}

// ---------- helpers ----------

fn ssh_dir() -> Result<PathBuf, String> { system::home().map(|h| h.join(".ssh")).ok_or_else(|| "no home folder".to_string()) }

fn out(cmd: &mut Command) -> String {
    cmd.stdin(Stdio::null()).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}
fn run_text(cmd: &mut Command) -> Result<String, String> {
    let o = cmd.stdin(Stdio::null()).output().map_err(|e| format!("Could not start the command: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().to_string();
    if o.status.success() { Ok(text) } else { Err(if text.is_empty() { "The command failed.".to_string() } else { text }) }
}
fn git_get(key: &str) -> String { out(Command::new("git").args(["config", "--global", "--get", key])) }
fn git_set(key: &str, val: &str) -> Result<(), String> { run_text(Command::new("git").args(["config", "--global", key, val])).map(|_| ()) }

fn list_keys() -> Vec<KeyInfo> {
    let Ok(dir) = ssh_dir() else { return vec![] };
    let Ok(rd) = std::fs::read_dir(&dir) else { return vec![] };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |e| e == "pub")).collect();
    paths.sort();
    paths.truncate(10);
    paths.into_iter().filter_map(|p| {
        let public = std::fs::read_to_string(&p).ok()?.lines().next()?.trim().to_string();
        if public.is_empty() || public.len() > 2000 { return None; }
        let fingerprint = parse_fingerprint(&out(Command::new("ssh-keygen").arg("-lf").arg(&p)));
        Some(KeyInfo { path: p.to_string_lossy().into_owned(), fingerprint, public })
    }).collect()
}

// ---------- read ----------

pub fn state() -> GitState {
    let mut s = GitState { git_installed: system::on_path("git"), ssh_installed: system::on_path("ssh-keygen"), gh_installed: system::on_path("gh"), ..Default::default() };
    if s.git_installed {
        s.git_version = out(Command::new("git").arg("--version"));
        s.name = git_get("user.name");
        s.email = git_get("user.email");
        s.default_branch = git_get("init.defaultBranch");
        s.signing = git_get("commit.gpgsign") == "true" && git_get("gpg.format") == "ssh";
        s.signing_key = git_get("user.signingkey");
    }
    if s.gh_installed {
        // `gh auth status` exits non-zero when signed out and prints to stdout or stderr depending on the version.
        let text = match run_text(Command::new("timeout").args(["10", "gh", "auth", "status"])) { Ok(t) | Err(t) => t };
        s.gh_user = parse_gh_user(&text);
    }
    if s.ssh_installed { s.keys = list_keys(); }
    s
}

// ---------- actions ----------

pub fn save_identity(name: &str, email: &str, default_main: bool) -> Result<(), String> {
    let name = clean_text(name, 100)?;
    let email = email.trim();
    if !valid_email(email) { return Err("Enter a valid email address.".into()); }
    git_set("user.name", &name)?;
    git_set("user.email", email)?;
    if default_main { git_set("init.defaultBranch", "main")?; }
    Ok(())
}

/// Creates ~/.ssh/id_ed25519. Never overwrites an existing key.
pub fn generate_key(email: &str, passphrase: &str) -> Result<String, String> {
    if !valid_email(email.trim()) { return Err("Enter a valid email address first (it becomes the key comment).".into()); }
    if passphrase.len() > 200 || passphrase.chars().any(|c| c.is_control()) { return Err("The passphrase cannot contain line breaks.".into()); }
    let dir = ssh_dir()?;
    std::fs::DirBuilder::new().mode(0o700).recursive(true).create(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let key = dir.join("id_ed25519");
    if key.exists() || key.with_extension("pub").exists() || dir.join("id_ed25519.pub").exists() {
        return Err(format!("A key already exists at {}. It was not changed.", key.display()));
    }
    // The passphrase is passed with -N, so other users on this machine could see it in the process list for a moment.
    run_text(Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-C", email.trim(), "-f"]).arg(&key).args(["-N", passphrase]))?;
    Ok(format!("Created {}", key.display()))
}

pub fn test_ssh() -> SshTest {
    let r = run_text(Command::new("timeout").args(["20", "ssh", "-o", "StrictHostKeyChecking=accept-new", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-T", "git@github.com"]));
    // GitHub closes the connection after greeting you, so ssh exits non-zero even on success.
    let text = match r { Ok(t) | Err(t) => t };
    SshTest { ok: text.contains("successfully authenticated"), output: text }
}

fn host_title() -> String {
    let h = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
    let h: String = h.trim().chars().filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c)).take(40).collect();
    format!("{} (DevSetup)", if h.is_empty() { "my computer" } else { &h })
}

/// Registers a public key with GitHub through the GitHub CLI. `kind` is "authentication" or "signing".
pub fn add_key(path: &str, kind: &str) -> Result<String, String> {
    if kind != "authentication" && kind != "signing" { return Err("unknown key type".into()); }
    let key = pub_key_in(&ssh_dir()?, path)?;
    let title = host_title();
    run_text(Command::new("gh").args(["ssh-key", "add"]).arg(&key).args(["--title", title.as_str(), "--type", kind]))
}

pub fn setup_git_credentials() -> Result<String, String> { run_text(Command::new("gh").args(["auth", "setup-git"])).map(|_| "Git will now use your GitHub sign-in for HTTPS.".to_string()) }

pub fn set_signing(enable: bool, key: &str, email: &str) -> Result<(), String> {
    if !enable {
        let _ = Command::new("git").args(["config", "--global", "--unset", "commit.gpgsign"]).status();
        return Ok(());
    }
    if !git_version_at_least(&out(Command::new("git").arg("--version")), 2, 34) { return Err("Signing with an SSH key needs Git 2.34 or newer.".into()); }
    let email = email.trim();
    if !valid_email(email) { return Err("Save your email in the identity step first.".into()); }
    let keyfile = pub_key_in(&ssh_dir()?, key)?;
    let public = std::fs::read_to_string(&keyfile).map_err(|e| e.to_string())?;
    let line = signers_line(email, public.lines().next().unwrap_or("")).ok_or_else(|| "That does not look like a public key.".to_string())?;
    let signers = system::home().ok_or("no home folder")?.join(".config/git/allowed_signers");
    if let Some(d) = signers.parent() { std::fs::create_dir_all(d).map_err(|e| e.to_string())?; }
    let existing = std::fs::read_to_string(&signers).unwrap_or_default();
    if !existing.lines().any(|l| l.trim() == line) {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&signers).map_err(|e| e.to_string())?;
        if !existing.is_empty() && !existing.ends_with('\n') { f.write_all(b"\n").map_err(|e| e.to_string())?; }
        writeln!(f, "{line}").map_err(|e| e.to_string())?;
    }
    git_set("gpg.format", "ssh")?;
    git_set("user.signingkey", &keyfile.to_string_lossy())?;
    git_set("gpg.ssh.allowedSignersFile", &signers.to_string_lossy())?;
    git_set("commit.gpgsign", "true")
}

/// Opens one of a few fixed GitHub pages in the browser.
pub fn open_url(which: &str) -> Result<(), String> {
    let url = match which {
        "github-device" => "https://github.com/login/device",
        "github-ssh" => "https://github.com/settings/ssh/new",
        _ => return Err("unknown page".into()),
    };
    Command::new("xdg-open").arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ()).map_err(|e| format!("Could not open the browser: {e}"))
}

/// Signs in with the browser (device code). The one-time code shows up in the log stream.
pub fn start_login(app: AppHandle, protocol: String) -> Result<(), String> {
    let proto = match protocol.as_str() { "ssh" => "ssh", "https" => "https", _ => return Err("protocol must be ssh or https".into()) };
    if !system::on_path("gh") { return Err("GitHub CLI is not installed. Install it from Setup.".into()); }
    if exec::RUNNING.swap(true, Ordering::SeqCst) { return Err("a run is already in progress".into()); }
    exec::CANCEL.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let mut failed = 0;
        step(&app, "gh-login", "running", "");
        let mut cmd = Command::new("gh");
        // GH_BROWSER=true stops gh from opening a browser itself; the app opens the fixed device page on request.
        cmd.args(["auth", "login", "--hostname", "github.com", "--git-protocol", proto, "--web", "--skip-ssh-key"]).env("GH_PROMPT_DISABLED", "1").env("GH_BROWSER", "true");
        let ok = stream(&app, "gh-login", cmd, true);
        if ok { step(&app, "gh-login", "done", ""); } else if cancelled() { step(&app, "gh-login", "cancelled", "stopped"); } else { step(&app, "gh-login", "failed", "see log"); failed += 1; }
        if ok && proto == "https" {
            step(&app, "gh-git", "running", "");
            let mut c = Command::new("gh");
            c.args(["auth", "setup-git"]);
            if stream(&app, "gh-git", c, true) { step(&app, "gh-git", "done", ""); } else { step(&app, "gh-git", "failed", "see log"); failed += 1; }
        }
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
    fn validates_user_input() {
        assert!(valid_email("me@example.com") && valid_email("a.b+c@sub.example.org"));
        for bad in ["", "me", "me@", "@example.com", "me@example", "me@@example.com", "a b@example.com", "-x@example.com", "me@example.com\nBcc: x", "me@.com"] { assert!(!valid_email(bad), "{bad:?}"); }
        assert_eq!(clean_text("  Ada Lovelace ", 100).unwrap(), "Ada Lovelace");
        assert!(clean_text("", 100).is_err() && clean_text("a\nb", 100).is_err() && clean_text(&"x".repeat(101), 100).is_err());
    }

    #[test]
    fn parses_tool_output() {
        assert!(git_version_at_least("git version 2.47.1", 2, 34) && git_version_at_least("git version 3.0.0", 2, 34));
        assert!(!git_version_at_least("git version 2.30.2", 2, 34) && !git_version_at_least("nonsense", 2, 34));
        assert_eq!(parse_gh_user("github.com\n  ✓ Logged in to github.com account octocat (keyring)\n"), Some("octocat".to_string()));
        assert_eq!(parse_gh_user("✓ Logged in to github.com as monalisa (oauth_token)"), Some("monalisa".to_string()));
        assert_eq!(parse_gh_user("You are not logged into any GitHub hosts."), None);
        assert_eq!(parse_fingerprint("256 SHA256:AbCd me@example.com (ED25519)"), "SHA256:AbCd");
    }

    #[test]
    fn key_paths_must_be_public_keys_directly_in_ssh_dir() {
        let base = std::env::temp_dir().join(format!("devsetup-ssh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join(".ssh");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("id_ed25519.pub"), "ssh-ed25519 AAAA me").unwrap();
        std::fs::write(dir.join("id_ed25519"), "PRIVATE").unwrap();
        std::fs::write(dir.join("sub/x.pub"), "k").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", dir.join("link.pub")).unwrap();
        assert!(pub_key_in(&dir, dir.join("id_ed25519.pub").to_str().unwrap()).is_ok());
        for bad in [dir.join("id_ed25519"), dir.join("sub/x.pub"), dir.join("link.pub"), dir.join("../outside.pub"), PathBuf::from("/etc/passwd")] {
            assert!(pub_key_in(&dir, bad.to_str().unwrap()).is_err(), "{}", bad.display());
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn allowed_signers_line_drops_the_comment_and_rejects_non_keys() {
        assert_eq!(signers_line("me@x.com", "ssh-ed25519 AAAAC3 me@laptop"), Some("me@x.com namespaces=\"git\" ssh-ed25519 AAAAC3".to_string()));
        assert_eq!(signers_line("me@x.com", "hello"), None);
        assert_eq!(signers_line("me@x.com", "rm -rf /"), None);
    }
}
