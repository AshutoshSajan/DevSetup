//! Setup files: a shareable list of roles and tools (`devsetup.yaml`).
//! A setup file can only pick tools from DevSetup's own manifest. It cannot carry commands.
use crate::manifest::Manifest;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

const MAX_BYTES: usize = 200_000;
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Default)]
struct SetupFile {
    #[serde(default)] version: u32,
    #[serde(default)] name: String,
    #[serde(default)] roles: Vec<String>,
    #[serde(default)] tools: Vec<String>,
}

/// A setup file after checking it against the manifest. Unknown ids are reported, not applied.
#[derive(Serialize, Default, Debug, PartialEq)]
pub struct Imported {
    pub name: String,
    pub roles: Vec<String>,
    pub tools: Vec<String>,
    pub unknown_roles: Vec<String>,
    pub unknown_tools: Vec<String>,
}

pub fn to_yaml(name: &str, roles: &[String], tools: &[String], notes: &[String]) -> String {
    let f = SetupFile { version: VERSION, name: name.trim().chars().take(80).collect(), roles: roles.to_vec(), tools: tools.to_vec() };
    let mut text = format!("# DevSetup setup file. Open DevSetup and choose Load setup to apply it.\n{}", serde_yaml::to_string(&f).unwrap_or_default());
    if !notes.is_empty() {
        text.push_str("\n# Also found on the source machine. Not in DevSetup's list, so this file does not restore them:\n");
        for n in notes.iter().take(400) {
            // Control characters are dropped so a note can never break out of its comment line.
            let clean: String = n.chars().filter(|c| !c.is_control()).take(120).collect();
            text.push_str(&format!("#   {clean}\n"));
        }
    }
    text
}

pub fn parse(text: &str, m: &Manifest) -> Result<Imported, String> {
    if text.len() > MAX_BYTES { return Err("This file is too large to be a setup file.".into()); }
    let f: SetupFile = serde_yaml::from_str(text).map_err(|e| format!("This is not a valid setup file: {e}"))?;
    if f.version > VERSION { return Err("This setup was made by a newer DevSetup. Update DevSetup to load it.".into()); }
    if f.roles.is_empty() && f.tools.is_empty() { return Err("The setup file lists no roles or tools.".into()); }
    let role_ids: HashSet<&str> = m.roles.iter().map(|r| r.id.as_str()).collect();
    let tool_ids: HashSet<&str> = m.tools.iter().map(|t| t.id.as_str()).collect();
    let mut out = Imported { name: f.name.trim().chars().take(80).collect(), ..Default::default() };
    let mut seen: HashSet<String> = HashSet::new();
    for r in f.roles.into_iter().take(500) {
        if !seen.insert(format!("r:{r}")) { continue; }
        if role_ids.contains(r.as_str()) { out.roles.push(r); } else { out.unknown_roles.push(r.chars().take(40).collect()); }
    }
    for t in f.tools.into_iter().take(500) {
        if !seen.insert(format!("t:{t}")) { continue; }
        if tool_ids.contains(t.as_str()) { out.tools.push(t); } else { out.unknown_tools.push(t.chars().take(40).collect()); }
    }
    Ok(out)
}

/// Turns a GitHub link or `owner/repo` into a raw-file URL. Only GitHub hosts are accepted.
pub fn github_raw_url(input: &str) -> Result<String, String> {
    let s = input.trim();
    let only_github = || "Only GitHub links are supported: github.com, gist.github.com, raw.githubusercontent.com or owner/repo.".to_string();
    if s.is_empty() { return Err("Enter a GitHub link or owner/repo.".into()); }
    let name_ok = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    let repo_root = |o: &str, r: &str| format!("https://raw.githubusercontent.com/{o}/{r}/HEAD/devsetup.yaml");

    if !s.contains("://") {
        let parts: Vec<&str> = s.trim_start_matches("github.com/").split('/').collect();
        if !s.contains(' ') && parts.len() == 2 && parts.iter().all(|p| name_ok(p)) { return Ok(repo_root(parts[0], parts[1])); }
        return Err(only_github());
    }
    let rest = s.strip_prefix("https://").ok_or_else(|| "Links must start with https://".to_string())?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let path = path.split(|c| c == '?' || c == '#').next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if !segs.iter().all(|p| *p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.~%".contains(c))) { return Err(only_github()); }
    match host {
        "raw.githubusercontent.com" if segs.len() >= 4 => Ok(format!("https://{host}/{}", segs.join("/"))),
        "gist.githubusercontent.com" if segs.len() >= 3 => Ok(format!("https://{host}/{}", segs.join("/"))),
        "gist.github.com" if segs.len() == 2 => Ok(format!("https://gist.githubusercontent.com/{}/{}/raw", segs[0], segs[1])),
        "github.com" if segs.len() == 2 => Ok(repo_root(segs[0], segs[1])),
        "github.com" if segs.len() >= 5 && (segs[2] == "blob" || segs[2] == "raw") => {
            Ok(format!("https://raw.githubusercontent.com/{}/{}/{}/{}", segs[0], segs[1], segs[3], segs[4..].join("/")))
        }
        _ => Err(only_github()),
    }
}

/// Downloads a setup file from GitHub with curl (same tool and flags style as the rest of DevSetup).
pub fn fetch(input: &str) -> Result<String, String> {
    let url = github_raw_url(input)?;
    let out = Command::new("curl")
        .args(["--proto", "=https", "--tlsv1.2", "-fsSL", "--max-time", "15", "--max-filesize", "200000", url.as_str()])
        .output()
        .map_err(|e| format!("Could not run curl: {e}"))?;
    if !out.status.success() { return Err(format!("Could not download {url}. Check the link and that the file is public.")); }
    String::from_utf8(out.stdout).map_err(|_| "The download is not a text file.".to_string())
}

fn yaml_path(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    match p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("yaml") | Some("yml") => Ok(p),
        _ => Err("Setup files must end in .yaml or .yml".into()),
    }
}

pub fn write_file(path: &str, content: &str) -> Result<(), String> {
    if content.len() > MAX_BYTES { return Err("The setup is too large to save.".into()); }
    let p = yaml_path(path)?;
    std::fs::write(&p, content).map_err(|e| format!("Could not write {}: {e}", p.display()))
}

pub fn read_file(path: &str) -> Result<String, String> {
    let p = yaml_path(path)?;
    let len = std::fs::metadata(&p).map_err(|e| format!("Could not open {}: {e}", p.display()))?.len();
    if len > MAX_BYTES as u64 { return Err("This file is too large to be a setup file.".into()); }
    std::fs::read_to_string(&p).map_err(|e| format!("Could not read {}: {e}", p.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Method, Role, Tool};

    fn manifest() -> Manifest {
        let t = |id: &str| Tool { id: id.into(), name: id.into(), group: "g".into(), check: String::new(), roles: vec![], requires: vec![], method: Method::Apt { package: id.into() } };
        Manifest { roles: vec![Role { id: "web".into(), name: "Web".into(), description: String::new() }], tools: vec![t("git"), t("vim")] }
    }
    fn v(a: &[&str]) -> Vec<String> { a.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn export_then_import_round_trips() {
        let text = to_yaml("My setup", &v(&["web"]), &v(&["git", "vim"]), &[]);
        let got = parse(&text, &manifest()).unwrap();
        assert_eq!(got, Imported { name: "My setup".into(), roles: v(&["web"]), tools: v(&["git", "vim"]), unknown_roles: vec![], unknown_tools: vec![] });
    }

    #[test]
    fn unknown_and_duplicate_entries_are_reported_not_applied() {
        let got = parse("version: 1\nroles: [web, nope]\ntools: [git, git, evil, vim]\n", &manifest()).unwrap();
        assert_eq!(got.tools, v(&["git", "vim"]));
        assert_eq!(got.unknown_tools, v(&["evil"]));
        assert_eq!(got.unknown_roles, v(&["nope"]));
    }

    #[test]
    fn bad_files_are_rejected() {
        let m = manifest();
        assert!(parse("version: 2\ntools: [git]\n", &m).is_err());
        assert!(parse("version: 1\n", &m).is_err());
        assert!(parse("<html>not a setup</html>", &m).is_err());
        assert!(parse(&"a".repeat(MAX_BYTES + 1), &m).is_err());
    }

    #[test]
    fn github_links_become_raw_urls() {
        let raw = "https://raw.githubusercontent.com/me/dots/HEAD/devsetup.yaml";
        assert_eq!(github_raw_url("me/dots").unwrap(), raw);
        assert_eq!(github_raw_url("github.com/me/dots").unwrap(), raw);
        assert_eq!(github_raw_url("https://github.com/me/dots").unwrap(), raw);
        assert_eq!(github_raw_url("https://github.com/me/dots/blob/main/team/web.yaml").unwrap(), "https://raw.githubusercontent.com/me/dots/main/team/web.yaml");
        assert_eq!(github_raw_url("https://raw.githubusercontent.com/me/dots/main/a.yaml").unwrap(), "https://raw.githubusercontent.com/me/dots/main/a.yaml");
        assert_eq!(github_raw_url("https://gist.github.com/me/abc123").unwrap(), "https://gist.githubusercontent.com/me/abc123/raw");
    }

    #[test]
    fn only_github_https_links_are_accepted() {
        for bad in ["", "https://evil.example/me/dots", "http://github.com/me/dots", "ftp://github.com/me/dots", "me/dots/extra", "a b/c", "https://github.com/me/../x/blob/m/f.yaml"] {
            assert!(github_raw_url(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn only_yaml_paths_are_read_or_written() {
        assert!(read_file("/etc/passwd").is_err());
        assert!(write_file("/tmp/x.txt", "a").is_err());
    }

    #[test]
    fn notes_stay_comments_and_cannot_inject_entries() {
        let notes = v(&["flatpak: org.example.App", "evil\ntools: [vim]"]);
        let text = to_yaml("n", &v(&["web"]), &v(&["git"]), &notes);
        assert!(text.contains("#   flatpak: org.example.App"));
        let got = parse(&text, &manifest()).unwrap();
        assert_eq!(got.tools, v(&["git"]));
    }
}
