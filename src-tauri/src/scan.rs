//! Read-only scan of this machine: which tools from DevSetup's list are installed, which roles they suggest,
//! and some other software DevSetup does not manage (recorded only as comments in a setup file).
use crate::manifest::{self, Manifest, Method};
use crate::{exec, state, system};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Serialize, Default)]
pub struct Found { pub id: String, pub name: String, pub group: String, pub source: String, pub managed: bool }

#[derive(Serialize, Default)]
pub struct RoleHit { pub id: String, pub name: String, pub installed: usize, pub total: usize, pub suggested: bool }

#[derive(Serialize, Default)]
pub struct Extras { pub flatpak: Vec<String>, pub npm: Vec<String>, pub pipx: Vec<String> }

#[derive(Serialize, Default)]
pub struct Scan { pub system: String, pub found: Vec<Found>, pub roles: Vec<RoleHit>, pub extras: Extras }

/// A role is suggested when at least this many of its own tools are installed.
const SUGGEST_AT: usize = 3;

fn source(m: &Method) -> &'static str {
    match m {
        Method::Apt { .. } => "apt",
        Method::Flatpak { .. } => "flatpak",
        Method::Mise { .. } => "mise",
        Method::Npm { .. } => "npm",
        Method::Pipx { .. } => "pipx",
        Method::Script { .. } => "script",
    }
}

/// Counts installed tools per role. Only tools named for a role count, not the shared "all" and "any" ones.
fn role_hits(m: &Manifest, installed: &HashSet<&str>) -> Vec<RoleHit> {
    m.roles.iter().map(|r| {
        let own: Vec<&str> = m.tools.iter().filter(|t| t.roles.contains(&r.id)).map(|t| t.id.as_str()).collect();
        let n = own.iter().filter(|id| installed.contains(**id)).count();
        RoleHit { id: r.id.clone(), name: r.name.clone(), installed: n, total: own.len(), suggested: n >= SUGGEST_AT }
    }).collect()
}

fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 120 && s.chars().all(|c| c.is_ascii_alphanumeric() || "@._/-+".contains(c))
}

fn clean(mut v: Vec<String>) -> Vec<String> {
    v.retain(|s| valid_name(s));
    v.sort();
    v.dedup();
    v.truncate(200);
    v
}

fn parse_flatpak_list(text: &str) -> Vec<String> { clean(text.lines().map(|l| l.trim().to_string()).collect()) }

fn parse_pipx_short(text: &str) -> Vec<String> {
    clean(text.lines().filter_map(|l| l.split_whitespace().next().map(|s| s.to_string())).collect())
}

fn parse_npm_json(text: &str) -> Vec<String> {
    let names: Vec<String> = serde_json::from_str::<serde_json::Value>(text).ok()
        .and_then(|v| v.get("dependencies").and_then(|d| d.as_object()).map(|o| o.keys().cloned().collect()))
        .unwrap_or_default();
    clean(names.into_iter().filter(|n| n != "npm" && n != "corepack").collect())
}

fn run_sh(script: &str) -> String {
    exec::sh(script, &[]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

pub fn scan() -> Scan {
    let m = manifest::load();
    let apt = system::apt_installed();
    let managed: HashSet<String> = state::load().managed.into_iter().collect();

    let found: Vec<Found> = m.tools.iter().filter(|t| system::is_installed(t, &apt)).map(|t| Found {
        id: t.id.clone(), name: t.name.clone(), group: t.group.clone(), source: source(&t.method).to_string(), managed: managed.contains(&t.id),
    }).collect();
    let installed: HashSet<&str> = found.iter().map(|f| f.id.as_str()).collect();
    let roles = role_hits(&m, &installed);

    // Software DevSetup does not manage. Anything already in the manifest is left out.
    let known = |pick: &dyn Fn(&Method) -> Option<String>| -> HashSet<String> { m.tools.iter().filter_map(|t| pick(&t.method)).collect() };
    let known_flatpak = known(&|x| if let Method::Flatpak { app_id } = x { Some(app_id.clone()) } else { None });
    let known_npm = known(&|x| if let Method::Npm { package, .. } = x { Some(package.clone()) } else { None });
    let known_pipx = known(&|x| if let Method::Pipx { package, .. } = x { Some(package.clone()) } else { None });
    let extras = Extras {
        flatpak: parse_flatpak_list(&run_sh("flatpak list --app --columns=application 2>/dev/null")).into_iter().filter(|s| !known_flatpak.contains(s)).collect(),
        npm: parse_npm_json(&run_sh("npm ls -g --depth=0 --json 2>/dev/null")).into_iter().filter(|s| !known_npm.contains(s)).collect(),
        pipx: parse_pipx_short(&run_sh("pipx list --short 2>/dev/null")).into_iter().filter(|s| !known_pipx.contains(s)).collect(),
    };
    Scan { system: system::distro().name, found, roles, extras }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Role, Tool};

    fn tool(id: &str, roles: &[&str]) -> Tool {
        Tool { id: id.into(), name: id.into(), group: "g".into(), check: String::new(), roles: roles.iter().map(|r| r.to_string()).collect(), requires: vec![], method: Method::Apt { package: id.into() } }
    }

    #[test]
    fn roles_are_suggested_from_their_own_tools_only() {
        let m = Manifest {
            roles: vec![Role { id: "web".into(), name: "Web".into(), description: String::new() }, Role { id: "ops".into(), name: "Ops".into(), description: String::new() }],
            tools: vec![tool("a", &["web"]), tool("b", &["web"]), tool("c", &["web"]), tool("d", &["web"]), tool("e", &["ops"]), tool("f", &["ops"]), tool("g", &["all"]), tool("h", &["any"])],
        };
        let installed: HashSet<&str> = ["a", "b", "c", "e", "f", "g", "h"].into_iter().collect();
        let hits = role_hits(&m, &installed);
        assert_eq!((hits[0].installed, hits[0].total, hits[0].suggested), (3, 4, true));
        assert_eq!((hits[1].installed, hits[1].total, hits[1].suggested), (2, 2, false));
    }

    #[test]
    fn parses_flatpak_pipx_and_npm_output() {
        assert_eq!(parse_flatpak_list("org.mozilla.firefox\n\ncom.spotify.Client\n; rm -rf /\norg.mozilla.firefox\n"), vec!["com.spotify.Client", "org.mozilla.firefox"]);
        assert_eq!(parse_pipx_short("aider-chat 0.50.1\nhttpie 3.2.2\n"), vec!["aider-chat", "httpie"]);
        let npm = r#"{"name":"lib","dependencies":{"npm":{"version":"10"},"corepack":{"version":"0.1"},"@openai/codex":{"version":"1"},"docx":{"version":"9"}}}"#;
        assert_eq!(parse_npm_json(npm), vec!["@openai/codex", "docx"]);
        assert!(parse_npm_json("not json").is_empty());
    }
}
