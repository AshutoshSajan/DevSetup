//! VS Code extension packs per role. Extensions are installed through the editor's own command line,
//! one at a time. The UI only sends extension ids; each one must be listed in manifest/extensions.yaml.
use crate::exec::{self, cancelled, step, stream};
use crate::system;
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter};

#[derive(Deserialize, Serialize, Clone)]
pub struct Ext { pub id: String, pub name: String }

#[derive(Deserialize, Serialize, Clone)]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)] pub roles: Vec<String>,
    pub extensions: Vec<Ext>,
}

#[derive(Deserialize)]
struct PackFile { packs: Vec<Pack> }

pub fn packs() -> Vec<Pack> {
    serde_yaml::from_str::<PackFile>(include_str!("../../manifest/extensions.yaml")).expect("manifest/extensions.yaml is invalid").packs
}

// ---------- editors ----------

enum Kind {
    /// A command on PATH, for example `code`.
    Cli(&'static str),
    /// A Flatpak app id, run with `flatpak run`.
    Flatpak(&'static str),
}

struct EditorDef { id: &'static str, name: &'static str, kind: Kind }

static EDITORS: [EditorDef; 4] = [
    EditorDef { id: "code", name: "VS Code", kind: Kind::Cli("code") },
    EditorDef { id: "code-flatpak", name: "VS Code (Flatpak)", kind: Kind::Flatpak("com.visualstudio.code") },
    EditorDef { id: "codium", name: "VSCodium", kind: Kind::Cli("codium") },
    EditorDef { id: "codium-flatpak", name: "VSCodium (Flatpak)", kind: Kind::Flatpak("com.vscodium.codium") },
];

fn available(d: &EditorDef) -> bool {
    match d.kind { Kind::Cli(cmd) => system::find_in_path(cmd).is_some(), Kind::Flatpak(app) => system::flatpak_has(app) }
}

/// Program and arguments that run the editor's command line with `extra` after them.
fn editor_args(d: &EditorDef, extra: &[&str]) -> (String, Vec<String>) {
    match d.kind {
        Kind::Cli(cmd) => (cmd.to_string(), extra.iter().map(|s| s.to_string()).collect()),
        Kind::Flatpak(app) => ("flatpak".to_string(), ["run", app].iter().chain(extra.iter()).map(|s| s.to_string()).collect()),
    }
}

/// `publisher.name`: letters, digits, dots and dashes only, so an id can never be mistaken for an option.
fn valid_ext_id(s: &str) -> bool {
    let Some((publisher, name)) = s.split_once('.') else { return false };
    s.len() <= 120
        && !publisher.is_empty() && publisher.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !name.is_empty() && !name.starts_with('.') && !name.starts_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// Finds an extension by id, ignoring case (the editor reports ids in lower case).
fn find_ext<'a>(packs: &'a [Pack], id: &str) -> Option<&'a Ext> {
    packs.iter().flat_map(|p| p.extensions.iter()).find(|x| x.id.eq_ignore_ascii_case(id))
}

fn installed_extensions(d: &EditorDef) -> Vec<String> {
    let (program, args) = editor_args(d, &["--list-extensions"]);
    let out = Command::new("timeout").arg("30").arg(program).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| valid_ext_id(l)).collect()).unwrap_or_default()
}

// ---------- state ----------

#[derive(Serialize)]
pub struct EditorInfo { pub id: String, pub name: String, pub installed: Vec<String> }

#[derive(Serialize, Default)]
pub struct VsState { pub editors: Vec<EditorInfo>, pub packs: Vec<Pack> }

pub fn state() -> VsState {
    let editors = EDITORS.iter().filter(|d| available(d))
        .map(|d| EditorInfo { id: d.id.to_string(), name: d.name.to_string(), installed: installed_extensions(d) }).collect();
    VsState { editors, packs: packs() }
}

// ---------- install ----------

/// Installs the chosen extensions with the chosen editor, one step per extension.
pub fn start_install(app: AppHandle, editor: String, ids: Vec<String>) -> Result<(), String> {
    let def = EDITORS.iter().find(|d| d.id == editor.as_str() && available(d)).ok_or_else(|| "That editor was not found.".to_string())?;
    let all = packs();
    let mut chosen: Vec<Ext> = vec![];
    for id in &ids {
        let ext = find_ext(&all, id).ok_or_else(|| format!("{id} is not in DevSetup's extension packs."))?;
        if !chosen.iter().any(|c| c.id == ext.id) { chosen.push(ext.clone()); }
    }
    if chosen.is_empty() { return Err("Nothing selected.".into()); }
    if exec::RUNNING.swap(true, Ordering::SeqCst) { return Err("a run is already in progress".into()); }
    exec::CANCEL.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        for x in &chosen { step(&app, &x.id, "queued", ""); }
        let mut failed = 0;
        for x in &chosen {
            if cancelled() { step(&app, &x.id, "cancelled", ""); continue; }
            step(&app, &x.id, "running", "");
            let (program, args) = editor_args(def, &["--install-extension", x.id.as_str()]);
            let mut cmd = Command::new(program);
            cmd.args(args);
            if stream(&app, &x.id, cmd, true) { step(&app, &x.id, "done", ""); }
            else if cancelled() { step(&app, &x.id, "cancelled", "stopped"); }
            else { step(&app, &x.id, "failed", "see log"); failed += 1; }
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
    use std::collections::HashSet;

    #[test]
    fn extension_ids_must_be_publisher_dot_name() {
        for ok in ["ms-python.python", "Vue.volar", "mtxr.sqltools-driver-pg", "Dart-Code.flutter", "a.b"] { assert!(valid_ext_id(ok), "{ok}"); }
        let too_long = format!("a.{}", "b".repeat(130));
        for bad in ["", "python", ".python", "ms-python.", "--install-extension", "a b.c", "a.b;rm", "a.-b", "a./b", too_long.as_str()] { assert!(!valid_ext_id(bad), "{bad:?}"); }
    }

    #[test]
    fn every_pack_in_the_manifest_is_well_formed() {
        let ps = packs();
        assert!(ps.len() >= 2);
        let ids: HashSet<&str> = ps.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.len(), ps.len(), "duplicate pack ids");
        let tools = crate::manifest::load();
        let roles: HashSet<&str> = tools.roles.iter().map(|r| r.id.as_str()).collect();
        for p in &ps {
            assert!(!p.name.is_empty() && !p.description.is_empty() && !p.extensions.is_empty(), "{} is incomplete", p.id);
            for r in &p.roles { assert!(r == "all" || roles.contains(r.as_str()), "{} names unknown role {r}", p.id); }
            let mut seen = HashSet::new();
            for x in &p.extensions {
                assert!(valid_ext_id(&x.id), "{} has a malformed extension id {}", p.id, x.id);
                assert!(!x.name.trim().is_empty(), "{} has an unnamed extension", x.id);
                assert!(seen.insert(x.id.to_ascii_lowercase()), "{} lists {} twice", p.id, x.id);
            }
        }
    }

    #[test]
    fn every_role_has_a_pack_and_everyone_gets_the_essentials() {
        let ps = packs();
        for r in crate::manifest::load().roles { assert!(ps.iter().any(|p| p.roles.contains(&r.id)), "no pack for role {}", r.id); }
        assert!(ps.iter().any(|p| p.roles.iter().any(|r| r == "all")));
    }

    #[test]
    fn commands_go_through_the_editors_own_cli() {
        let code = &EDITORS[0];
        assert_eq!(editor_args(code, &["--install-extension", "a.b"]), ("code".to_string(), vec!["--install-extension".to_string(), "a.b".to_string()]));
        let fp = &EDITORS[1];
        assert_eq!(editor_args(fp, &["--list-extensions"]), ("flatpak".to_string(), vec!["run".to_string(), "com.visualstudio.code".to_string(), "--list-extensions".to_string()]));
    }

    #[test]
    fn extensions_are_found_whatever_the_case_and_unknown_ones_are_not() {
        let ps = packs();
        assert_eq!(find_ext(&ps, "ms-python.PYTHON").map(|x| x.id.as_str()), Some("ms-python.python"));
        assert!(find_ext(&ps, "evil.extension").is_none());
        assert!(find_ext(&ps, "").is_none());
    }
}
