//! Pre-flight checks shown in the review screen before anything is installed.
use crate::{exec, manifest::{self, Method, Tool}, system};
use serde::Serialize;
use std::collections::BTreeSet;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Serialize, Clone)]
pub struct Check { pub id: String, pub label: String, pub status: &'static str, pub detail: String }

fn check(id: &str, label: &str, status: &'static str, detail: impl Into<String>) -> Check {
    Check { id: id.into(), label: label.into(), status, detail: detail.into() }
}

fn host_of(url: &str) -> Option<String> {
    url.strip_prefix("https://")?.split('/').next().map(|s| s.to_string())
}

fn reachable(host: &str) -> bool {
    (host, 443u16).to_socket_addrs().ok()
        .and_then(|mut it| it.find_map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(4)).ok()))
        .is_some()
}

fn fmt(b: u64) -> String {
    if b >= 1_000_000_000 { format!("{:.1} GB", b as f64 / 1e9) } else { format!("{:.0} MB", b as f64 / 1e6) }
}

/// Reads sizes from apt output such as "After this operation, 4,129 kB of additional disk space will be used."
fn parse_after(text: &str, prefix: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.contains(prefix))?;
    let rest = &line[line.find(prefix)? + prefix.len()..];
    if rest.contains("freed") { return Some(0); }
    let mut it = rest.split_whitespace();
    let num: f64 = it.next()?.replace(',', "").parse().ok()?;
    let unit: String = it.next()?.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let mult = match unit.as_str() { "B" => 1.0, "kB" => 1e3, "MB" => 1e6, "GB" => 1e9, _ => return None };
    Some((num * mult).round() as u64)
}

struct Sim { download: Option<u64>, disk: Option<u64>, missing: Vec<String> }

/// `apt-get -s install` needs no root. Missing packages are removed and the simulation is retried once.
fn simulate(pkgs: &[String]) -> Sim {
    let mut pk = pkgs.to_vec();
    let mut missing: Vec<String> = vec![];
    for _ in 0..2 {
        let Ok(out) = Command::new("apt-get").args(["-s", "install", "-y"]).args(&pk).env("LC_ALL", "C").output() else { break };
        let (o, e) = (String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string());
        let mut miss: Vec<String> = vec![];
        for l in e.lines() {
            if let Some(p) = l.strip_prefix("E: Unable to locate package ") { miss.push(p.trim().to_string()); }
            else if let Some(p) = l.strip_prefix("E: Package '") { if let Some(n) = p.split('\'').next() { miss.push(n.to_string()); } }
        }
        if !miss.is_empty() {
            pk.retain(|p| !miss.contains(p));
            missing.extend(miss);
            if pk.is_empty() { break; }
            continue;
        }
        if out.status.success() {
            return Sim { download: parse_after(&o, "Need to get "), disk: parse_after(&o, "After this operation, "), missing };
        }
        break;
    }
    Sim { download: None, disk: None, missing }
}

/// Returns (mount point, available kilobytes) for the filesystem holding `path`.
pub(crate) fn free_space(path: &str) -> Option<(String, u64)> {
    let out = Command::new("df").args(["-Pk", path]).env("LC_ALL", "C").output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let f: Vec<&str> = s.lines().nth(1)?.split_whitespace().collect();
    Some((f.get(5)?.to_string(), f.get(3)?.parse().ok()?))
}

pub fn run(install: Vec<String>, remove: Vec<String>, strict: bool) -> Vec<Check> {
    let m = manifest::load();
    let apt = system::apt_installed();
    let to_install = exec::resolve(&m, &install, &apt);
    let is_apt = |t: &Tool| matches!(t.method, Method::Apt { .. });
    let apt_in: Vec<&Tool> = to_install.iter().filter(|t| is_apt(t)).collect();
    let removes_apt = remove.iter().filter_map(|id| m.tools.iter().find(|t| &t.id == id)).any(|t| is_apt(t));
    let uses_apt = !apt_in.is_empty() || removes_apt;
    let mut out: Vec<Check> = vec![];

    if uses_apt {
        let d = system::distro();
        if Path::new("/var/lib/dpkg/status").exists() && system::on_path("apt-get") {
            out.push(check("system", "System", "ok", d.name));
        } else {
            out.push(check("system", "System", "fail", format!("{} is not supported yet: DevSetup installs system packages with apt (Debian, Ubuntu and derivatives)", d.name)));
        }
        if Path::new("/usr/bin/pkexec").exists() || system::on_path("pkexec") {
            out.push(check("admin", "Administrator access", "ok", "pkexec found, you will be asked for your password"));
        } else {
            out.push(check("admin", "Administrator access", "fail", "pkexec not found: install the pkexec (polkit) package"));
        }
    }

    if to_install.iter().any(|t| matches!(t.method, Method::Script { .. })) {
        let ok = system::on_path("curl") || to_install.iter().any(|t| t.id == "curl");
        out.push(if ok { check("curl", "curl", "ok", "available for install scripts") }
                 else { check("curl", "curl", "fail", "needed by install scripts: tick curl or install it first") });
    }

    let scripts: Vec<&Tool> = to_install.iter().filter(|t| matches!(t.method, Method::Script { .. })).collect();
    if !scripts.is_empty() {
        let unpinned: Vec<&str> = scripts.iter().filter(|t| !t.script_is_pinned()).map(|t| t.name.as_str()).collect();
        out.push(if unpinned.is_empty() {
            check("scripts", "Install scripts", "ok", format!("{} verified against pinned checksums", scripts.len()))
        } else if strict {
            check("scripts", "Install scripts", "fail", format!("not pinned: {}. Strict mode only runs pinned scripts", unpinned.join(", ")))
        } else {
            check("scripts", "Install scripts", "warn", format!("not pinned: {}. They run whatever the server returns today", unpinned.join(", ")))
        });
    }

    if !to_install.is_empty() {
        let distro = system::distro();
        let mut hosts: BTreeSet<String> = BTreeSet::new();
        for t in &to_install {
            match &t.method {
                Method::Apt { .. } => {
                    let h = if distro.id == "ubuntu" { "archive.ubuntu.com" } else { "deb.debian.org" };
                    hosts.insert(h.to_string());
                }
                Method::Flatpak { .. } => { hosts.insert("dl.flathub.org".to_string()); }
                Method::Mise { .. } => { hosts.insert("github.com".to_string()); }
                Method::Npm { .. } => { hosts.insert("registry.npmjs.org".to_string()); }
                Method::Pipx { .. } => { hosts.insert("pypi.org".to_string()); }
                Method::Script { url, .. } => { if let Some(h) = host_of(url) { hosts.insert(h); } }
            }
        }
        let handles: Vec<_> = hosts.into_iter().map(|h| std::thread::spawn(move || { let ok = reachable(&h); (h, ok) })).collect();
        let results: Vec<(String, bool)> = handles.into_iter().filter_map(|h| h.join().ok()).collect();
        let down: Vec<&str> = results.iter().filter(|(_, ok)| !ok).map(|(h, _)| h.as_str()).collect();
        out.push(if down.is_empty() { check("network", "Internet", "ok", format!("{} hosts reachable", results.len())) }
                 else if down.len() == results.len() { check("network", "Internet", "fail", "no connection: check your network") }
                 else { check("network", "Internet", "warn", format!("cannot reach {}, so those tools will fail", down.join(", "))) });
    }

    let mut est_disk: Option<u64> = None;
    if !apt_in.is_empty() && system::on_path("apt-get") {
        let pkgs: Vec<String> = apt_in.iter().filter_map(|t| if let Method::Apt { package } = &t.method { Some(package.clone()) } else { None }).collect();
        let sim = simulate(&pkgs);
        est_disk = sim.disk;
        if !sim.missing.is_empty() {
            out.push(check("packages", "Packages", "warn", format!("not found in your apt sources: {}. One missing package fails the whole apt step, so untick it or fix your sources", sim.missing.join(", "))));
        } else if let (Some(dl), Some(dk)) = (sim.download, sim.disk) {
            out.push(check("packages", "Packages", "ok", format!("about {} to download, {} on disk", fmt(dl), fmt(dk))));
        }
    }

    if !to_install.is_empty() {
        let mut paths = vec![("/".to_string(), est_disk.unwrap_or(0) + GIB)];
        if let Some(h) = system::home() { paths.push((h.to_string_lossy().into_owned(), GIB)); }
        let mut seen = BTreeSet::new();
        for (path, need) in paths {
            let Some((mount, kb)) = free_space(&path) else { continue };
            if !seen.insert(mount.clone()) { continue; }
            let avail = kb * 1024;
            let st = if avail < need { "fail" } else if avail < need + 4 * GIB { "warn" } else { "ok" };
            out.push(check("disk", &format!("Disk space on {mount}"), st, format!("{} free", fmt(avail))));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_apt_sizes() {
        assert_eq!(parse_after("After this operation, 4,129 kB of additional disk space will be used.", "After this operation, "), Some(4_129_000));
        assert_eq!(parse_after("Need to get 12.3 MB/45.6 MB of archives.", "Need to get "), Some(12_300_000));
        assert_eq!(parse_after("After this operation, 1,024 B disk space will be freed.", "After this operation, "), Some(0));
        assert_eq!(parse_after("nothing here", "Need to get "), None);
    }
    #[test]
    fn extracts_hosts() {
        assert_eq!(host_of("https://mise.run"), Some("mise.run".into()));
        assert_eq!(host_of("https://bun.sh/install"), Some("bun.sh".into()));
        assert_eq!(host_of("http://x.y"), None);
    }
}
