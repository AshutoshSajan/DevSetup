use crate::system;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Tools DevSetup installed itself. Only these may be removed by DevSetup.
#[derive(Default, Serialize, Deserialize)]
pub struct State { #[serde(default)] pub managed: Vec<String> }

fn base() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| system::home().map(|h| h.join(".local/share")))
}
fn path() -> Option<PathBuf> { Some(base()?.join("devsetup/state.json")) }
/// Location used while the app was still called FlashSetup; read once so earlier installs stay managed.
fn legacy_path() -> Option<PathBuf> { Some(base()?.join("flashsetup/state.json")) }

fn read(p: Option<PathBuf>) -> Option<State> {
    serde_json::from_str(&std::fs::read_to_string(p?).ok()?).ok()
}

pub fn load() -> State { read(path()).or_else(|| read(legacy_path())).unwrap_or_default() }

pub fn save(s: &State) {
    if let Some(p) = path() {
        if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
        if let Ok(j) = serde_json::to_string_pretty(s) { let _ = std::fs::write(p, j); }
    }
}
