//! Root helper, started as `pkexec devsetup --helper apt +id -id ...`.
//! It never accepts commands or package names from the UI: it only accepts tool ids,
//! looks each one up in the embedded manifest and requires an apt method.
use crate::manifest::{self, Method};
use std::path::Path;
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

/// Cleaner operations that need root. Only these fixed operations exist; arguments are never passed on to apt.
fn clean(ops: &[String]) -> i32 {
    if ops.iter().any(|o| o != "autoremove" && o != "apt-cache") {
        eprintln!("unknown clean operation");
        return 2;
    }
    let mut rc = 0;
    for op in ops {
        let ok = if op == "autoremove" { apt(&["autoremove", "-y"]) } else { apt(&["clean"]) };
        if !ok { rc = 1; }
    }
    rc
}

/// Name of the user who started pkexec, taken from PKEXEC_UID. Refuses root and system accounts.
fn pkexec_user(uid: Option<&str>, passwd: &str) -> Result<String, String> {
    let uid: u32 = uid.ok_or("not started through pkexec".to_string())?.parse().map_err(|_| "bad user id".to_string())?;
    if !(1000..60000).contains(&uid) { return Err("refusing to change a system account".to_string()); }
    let name = passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        if f.len() > 2 && f[2].parse::<u32>().ok() == Some(uid) { Some(f[0].to_string()) } else { None }
    }).ok_or("user not found".to_string())?;
    if name.is_empty() || name.starts_with('-') || !name.chars().all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c)) { return Err("unsafe user name".to_string()); }
    Ok(name)
}

fn run_cmd(program: &str, args: &[&str]) -> bool {
    Command::new(program).args(args).env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin").status().map(|s| s.success()).unwrap_or(false)
}

/// First candidate that exists and is listed in /etc/shells.
fn pick_shell(candidates: &[&str], etc_shells: &str, exists: &dyn Fn(&str) -> bool) -> Option<String> {
    candidates.iter().find(|c| exists(c) && etc_shells.lines().any(|l| l.trim() == **c)).map(|c| c.to_string())
}

/// Changes the login shell of the user who started pkexec. Only zsh and bash exist as choices.
fn set_login_shell(ops: &[String]) -> i32 {
    if ops.len() != 1 { eprintln!("shell takes exactly one name"); return 2; }
    let cands: &[&str] = match ops[0].as_str() {
        "zsh" => &["/usr/bin/zsh", "/bin/zsh"],
        "bash" => &["/usr/bin/bash", "/bin/bash"],
        _ => { eprintln!("unknown shell"); return 2; }
    };
    let etc = std::fs::read_to_string("/etc/shells").unwrap_or_default();
    let Some(path) = pick_shell(cands, &etc, &|p| Path::new(p).exists()) else {
        eprintln!("{} is not installed or not listed in /etc/shells", ops[0]);
        return 1;
    };
    let uid = std::env::var("PKEXEC_UID").ok();
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    match pkexec_user(uid.as_deref(), &passwd) {
        Ok(name) => {
            if run_cmd("usermod", &["-s", path.as_str(), name.as_str()]) { println!("Login shell changed to {path} for {name}. Log out and back in for it to take effect."); 0 } else { 1 }
        }
        Err(e) => { eprintln!("{e}"); 2 }
    }
}

/// Doctor fixes that need root. Only these fixed operations exist.
fn fix(ops: &[String]) -> i32 {
    if ops.len() != 1 { eprintln!("fix takes exactly one operation"); return 2; }
    match ops[0].as_str() {
        "docker-service" => if run_cmd("systemctl", &["enable", "--now", "docker"]) { 0 } else { 1 },
        "docker-group" => {
            let uid = std::env::var("PKEXEC_UID").ok();
            let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
            match pkexec_user(uid.as_deref(), &passwd) {
                Ok(name) => {
                    if !run_cmd("getent", &["group", "docker"]) { eprintln!("The docker group does not exist. Is Docker installed?"); return 1; }
                    if run_cmd("usermod", &["-aG", "docker", &name]) { println!("Added {name} to the docker group. Log out and back in for it to take effect."); 0 } else { 1 }
                }
                Err(e) => { eprintln!("{e}"); 2 }
            }
        }
        _ => { eprintln!("unknown fix"); 2 }
    }
}

pub fn run(args: &[String]) -> i32 {
    if args.first().map(String::as_str) == Some("clean") { return clean(&args[1..]); }
    if args.first().map(String::as_str) == Some("fix") { return fix(&args[1..]); }
    if args.first().map(String::as_str) == Some("shell") { return set_login_shell(&args[1..]); }
    if args.first().map(String::as_str) == Some("upgrade") {
        if args.len() != 1 { eprintln!("upgrade takes no arguments"); return 2; }
        return if apt(&["update"]) && apt(&["upgrade", "-y", "-o", "Dpkg::Options::=--force-confold"]) { 0 } else { 1 };
    }
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
        assert_eq!(run(&["clean".into(), "autoremove".into(), "rm".into()]), 2);
        assert_eq!(run(&["upgrade".into(), "-y".into()]), 2); // upgrade accepts no arguments
        assert_eq!(run(&["fix".into()]), 2);
        assert_eq!(run(&["shell".into()]), 2);
        assert_eq!(run(&["shell".into(), "fish".into()]), 2);
        assert_eq!(run(&["shell".into(), "zsh".into(), "extra".into()]), 2);
    }

    #[test]
    fn picks_only_listed_existing_shells() {
        let etc = "# shells\n/bin/sh\n/usr/bin/zsh\n";
        assert_eq!(pick_shell(&["/usr/bin/zsh", "/bin/zsh"], etc, &|_| true), Some("/usr/bin/zsh".to_string()));
        assert_eq!(pick_shell(&["/usr/bin/zsh"], etc, &|_| false), None);
        assert_eq!(pick_shell(&["/usr/bin/fish"], etc, &|_| true), None);
        assert_eq!(run(&["fix".into(), "rm".into()]), 2);
        assert_eq!(run(&["fix".into(), "docker-group".into(), "extra".into()]), 2);
    }

    #[test]
    fn pkexec_user_is_the_pkexec_user_only() {
        let passwd = "root:x:0:0::/root:/bin/bash\nme:x:1000:1000::/home/me:/bin/bash\nsvc:x:998:998::/:/usr/sbin/nologin\nbad name:x:1001:1001::/h:/bin/sh\n";
        assert_eq!(pkexec_user(Some("1000"), passwd), Ok("me".to_string()));
        assert!(pkexec_user(None, passwd).is_err());
        assert!(pkexec_user(Some("0"), passwd).is_err());
        assert!(pkexec_user(Some("998"), passwd).is_err());
        assert!(pkexec_user(Some("1001"), passwd).is_err()); // unsafe name
        assert!(pkexec_user(Some("1234"), passwd).is_err()); // not in passwd
        assert!(pkexec_user(Some("abc"), passwd).is_err()); // nothing runs if any operation is unknown
    }
}
