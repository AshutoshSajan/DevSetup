//! Root helper, started as `pkexec devsetup --helper apt +id -id ...`.
//! It never accepts commands or package names from the UI: it only accepts tool ids,
//! looks each one up in the embedded manifest and requires an apt method.
use crate::manifest::{self, Method};
use std::process::Command;

fn ok_pkg(p: &str) -> bool {
    !p.is_empty()
        && p.len() < 100
        && p.chars().next().map_or(false, |c| c.is_ascii_alphanumeric())
        && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn apt(args: &[&str]) -> bool {
    Command::new("apt-get")
        .args(args)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn run(args: &[String]) -> i32 {
    if args.first().map(String::as_str) != Some("apt") { eprintln!("unknown helper command"); return 2; }
    let m = manifest::load();
    let (mut inst, mut rem): (Vec<String>, Vec<String>) = (vec![], vec![]);
    for a in &args[1..] {
        let Some(sign) = a.chars().next() else { eprintln!("empty argument"); return 2; };
        let id = &a[sign.len_utf8()..];
        let Some(t) = m.tools.iter().find(|t| t.id == id) else { eprintln!("unknown tool: {id}"); return 2; };
        let Method::Apt { package } = &t.method else { eprintln!("{id} is not an apt tool"); return 2; };
        if !ok_pkg(package) { eprintln!("bad package name for {id}"); return 2; }
        match sign { '+' => inst.push(package.clone()), '-' => rem.push(package.clone()), _ => { eprintln!("bad argument: {a}"); return 2; } }
    }
    if !inst.is_empty() {
        println!("Updating package lists...");
        if !apt(&["update"]) { return 1; }
        let mut a = vec!["install", "-y"];
        a.extend(inst.iter().map(String::as_str));
        if !apt(&a) { return 1; }
    }
    if !rem.is_empty() {
        let mut a = vec!["remove", "-y"];
        a.extend(rem.iter().map(String::as_str));
        if !apt(&a) { return 1; }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_names_are_checked() {
        assert!(ok_pkg("build-essential") && ok_pkg("g++") && ok_pkg("docker.io"));
        assert!(!ok_pkg("-y") && !ok_pkg("a b") && !ok_pkg("a;b") && !ok_pkg(""));
    }
    #[test]
    fn helper_rejects_unknown_and_non_apt_tools() {
        assert_eq!(run(&["apt".into(), "+no-such-tool".into()]), 2);
        assert_eq!(run(&["apt".into(), "+vscode".into()]), 2); // flatpak tool
        assert_eq!(run(&["rm".into()]), 2);
    }
}
