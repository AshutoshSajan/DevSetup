//! Packaging contract: the .deb installs the polkit policy where polkit reads it, the policy names the binary
//! the .deb installs, and it never allows anything without an administrator password. Runs from src-tauri/.
use serde_json::Value;
use std::path::Path;

const POLICY_DEST: &str = "/usr/share/polkit-1/actions/dev.devsetup.helper.policy";

fn conf() -> Value {
    serde_json::from_str(&std::fs::read_to_string("tauri.conf.json").expect("src-tauri/tauri.conf.json")).expect("tauri.conf.json is valid JSON")
}

fn policy() -> String { std::fs::read_to_string("../packaging/dev.devsetup.helper.policy").expect("packaging/dev.devsetup.helper.policy") }

/// Text between `<name>` and `</name>`, first occurrence.
fn tag(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find(&format!("</{name}>"))?;
    Some(xml[start..end].trim().to_string())
}

#[test]
fn the_main_binary_is_named_like_the_cargo_package() {
    assert_eq!(conf()["mainBinaryName"].as_str(), Some(env!("CARGO_PKG_NAME")), "mainBinaryName must match the Cargo binary so the .deb installs /usr/bin/{}", env!("CARGO_PKG_NAME"));
}

#[test]
fn the_policy_names_the_binary_the_deb_installs() {
    let bin = conf()["mainBinaryName"].as_str().expect("mainBinaryName").to_string();
    let want = format!("<annotate key=\"org.freedesktop.policykit.exec.path\">/usr/bin/{bin}</annotate>");
    assert!(policy().contains(&want), "the policy must annotate exec.path with /usr/bin/{bin}");
}

#[test]
fn the_deb_installs_the_policy_where_polkit_reads_it() {
    let c = conf();
    assert!(c["bundle"]["targets"].as_array().map_or(false, |t| t.iter().any(|x| x == "deb")), "the deb target must stay enabled");
    let files = c["bundle"]["linux"]["deb"]["files"].as_object().expect("bundle.linux.deb.files");
    let src = files.get(POLICY_DEST).and_then(|v| v.as_str()).unwrap_or_else(|| panic!("deb.files must map {POLICY_DEST}"));
    assert!(Path::new(src).is_file(), "{src} does not exist (paths are relative to src-tauri)");
    assert!(src.ends_with("dev.devsetup.helper.policy"));
}

#[test]
fn the_policy_asks_for_an_administrator_password_every_time() {
    let p = policy();
    assert!(p.contains("<action id=\"dev.devsetup.helper\">"), "action id");
    for key in ["allow_any", "allow_inactive", "allow_active"] {
        assert_eq!(tag(&p, key).as_deref(), Some("auth_admin"), "{key} must be auth_admin: no unauthenticated and no cached approval");
    }
    assert!(!p.contains("auth_admin_keep") && !p.contains(">yes<") && !p.contains("auth_self"));
}

#[test]
fn the_deb_depends_on_polkit_and_its_icons_exist() {
    let c = conf();
    let deps = c["bundle"]["linux"]["deb"]["depends"].as_array().expect("deb.depends");
    assert!(deps.iter().any(|d| d.as_str().map_or(false, |s| s.contains("pkexec"))), "the root helper needs pkexec");
    for icon in c["bundle"]["icon"].as_array().expect("bundle.icon") {
        let path = icon.as_str().expect("icon path");
        assert!(Path::new(path).is_file(), "bundle icon {path} is missing");
    }
}
