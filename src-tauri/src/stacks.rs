//! Stacks: disposable Docker services per project, described by manifest/stacks.yaml.
//! A stack is a generated compose file under ~/.local/share/devsetup/stacks/<name>/. Ports are bound to 127.0.0.1 only.
//! The UI sends a stack name, service ids and an action; names are validated and nothing reaches a shell.
use crate::exec::{self, cancelled, log, step, stream};
use crate::system;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::net::TcpListener;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter};

// ---------- the catalog (data) ----------

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct GuiDef {
    pub name: String,
    pub image: String,
    pub container_port: u16,
    pub host_port: u16,
    #[serde(default)] pub env: BTreeMap<String, String>,
    #[serde(default)] pub url: String,
    #[serde(default)] pub note: String,
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct ExtraPort {
    pub label: String,
    pub container_port: u16,
    pub host_port: u16,
    #[serde(default)] pub url: String,
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct ServiceDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub image: String,
    pub container_port: u16,
    pub host_port: u16,
    #[serde(default)] pub data_dir: String,
    #[serde(default)] pub user: String,
    #[serde(default)] pub db: String,
    #[serde(default)] pub env: BTreeMap<String, String>,
    #[serde(default)] pub command: Vec<String>,
    pub connection: String,
    #[serde(default)] pub env_name: String,
    #[serde(default)] pub note: String,
    #[serde(default)] pub extra_ports: Vec<ExtraPort>,
    #[serde(default)] pub gui: Option<GuiDef>,
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct Combo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub services: Vec<String>,
    #[serde(default)] pub guis: Vec<String>,
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct Catalog {
    pub services: Vec<ServiceDef>,
    #[serde(default)] pub combos: Vec<Combo>,
}

pub fn catalog() -> Catalog {
    serde_yaml::from_str(include_str!("../../manifest/stacks.yaml")).expect("manifest/stacks.yaml is invalid")
}

// ---------- what a stack remembers ----------

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Conn { pub service: String, pub label: String, pub value: String }

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Link { pub label: String, pub url: String }

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Meta {
    pub name: String,
    #[serde(default)] pub created: u64,
    #[serde(default)] pub services: Vec<String>,
    #[serde(default)] pub guis: Vec<String>,
    #[serde(default)] pub connections: Vec<Conn>,
    #[serde(default)] pub links: Vec<Link>,
    #[serde(default)] pub env: Vec<(String, String)>,
    #[serde(default)] pub notes: Vec<String>,
    #[serde(default)] pub ports: Vec<u16>,
}

// ---------- building a stack (pure) ----------

/// Lower-case letters, digits and dashes; starts with a letter or digit; at most 30 characters.
pub fn valid_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 30
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        && !s.ends_with('-')
}

fn fill(template: &str, vars: &[(&str, String)]) -> String {
    let mut s = template.to_string();
    for (k, v) in vars { s = s.replace(&format!("{{{k}}}"), v); }
    s
}

/// The first port at or above `preferred` that is neither taken by a stack nor busy right now.
fn alloc(preferred: u16, used: &mut HashSet<u16>, free: &dyn Fn(u16) -> bool) -> Result<u16, String> {
    let mut p = preferred;
    for _ in 0..200 {
        if !used.contains(&p) && free(p) { used.insert(p); return Ok(p); }
        p = p.checked_add(1).ok_or_else(|| "No free port found.".to_string())?;
    }
    Err(format!("No free port found near {preferred}."))
}

pub struct Built { pub compose: String, pub meta: Meta }

/// Compose file and metadata for a stack. `taken` holds ports used by other stacks, `free` tests a port right now.
pub fn build(stack: &str, cat: &Catalog, services: &[String], guis: &[String], password: &str, taken: &HashSet<u16>, free: &dyn Fn(u16) -> bool) -> Result<Built, String> {
    if !valid_name(stack) { return Err("Use lowercase letters, digits and dashes (up to 30), starting with a letter or digit.".into()); }
    if services.is_empty() { return Err("Pick at least one service.".into()); }
    let mut used = taken.clone();
    let (mut svc_map, mut vol_map) = (Map::new(), Map::new());
    let mut meta = Meta { name: stack.to_string(), ..Default::default() };
    meta.notes.push(format!("User dev, password {password} (the same for every service in this stack)."));
    let mut seen: HashSet<&str> = HashSet::new();
    for id in services {
        if !seen.insert(id.as_str()) { continue; }
        let def = cat.services.iter().find(|s| &s.id == id).ok_or_else(|| format!("Unknown service {id}."))?;
        let user = if def.user.is_empty() { "dev" } else { def.user.as_str() };
        let db = if def.db.is_empty() { "app" } else { def.db.as_str() };
        let port = alloc(def.host_port, &mut used, free)?;
        let mut vars: Vec<(&str, String)> = vec![
            ("port", port.to_string()), ("container_port", def.container_port.to_string()), ("user", user.to_string()),
            ("password", password.to_string()), ("db", db.to_string()), ("service", def.id.clone()),
        ];
        let mut ports = vec![format!("127.0.0.1:{port}:{}", def.container_port)];
        meta.ports.push(port);
        for e in &def.extra_ports {
            let hp = alloc(e.host_port, &mut used, free)?;
            ports.push(format!("127.0.0.1:{hp}:{}", e.container_port));
            meta.ports.push(hp);
            if !e.url.is_empty() { meta.links.push(Link { label: format!("{} {}", def.name, e.label), url: fill(&e.url, &[("port", hp.to_string())]) }); }
        }
        let env: BTreeMap<String, String> = def.env.iter().map(|(k, v)| (k.clone(), fill(v, &vars))).collect();
        let mut svc = Map::new();
        svc.insert("image".into(), json!(def.image));
        svc.insert("ports".into(), json!(ports));
        if !env.is_empty() { svc.insert("environment".into(), json!(env)); }
        if !def.command.is_empty() { svc.insert("command".into(), json!(def.command)); }
        if !def.data_dir.is_empty() {
            let vol = format!("{}-data", def.id);
            svc.insert("volumes".into(), json!([format!("{vol}:{}", def.data_dir)]));
            vol_map.insert(vol, json!({}));
        }
        svc.insert("labels".into(), json!({ "com.devsetup.stack": stack, "com.devsetup.service": def.id }));
        svc_map.insert(def.id.clone(), Value::Object(svc));
        meta.services.push(def.id.clone());

        let value = fill(&def.connection, &vars);
        meta.connections.push(Conn { service: def.name.clone(), label: "Connection string".into(), value: value.clone() });
        if !def.env_name.is_empty() { meta.env.push((def.env_name.clone(), value)); }
        if !def.note.is_empty() { meta.notes.push(format!("{}: {}", def.name, fill(&def.note, &vars))); }

        if guis.iter().any(|g| g == &def.id) {
            if let Some(g) = &def.gui {
                let gp = alloc(g.host_port, &mut used, free)?;
                vars.push(("gui_port", gp.to_string()));
                let genv: BTreeMap<String, String> = g.env.iter().map(|(k, v)| (k.clone(), fill(v, &vars))).collect();
                let gid = format!("{}-gui", def.id);
                let mut gs = Map::new();
                gs.insert("image".into(), json!(g.image));
                gs.insert("ports".into(), json!([format!("127.0.0.1:{gp}:{}", g.container_port)]));
                if !genv.is_empty() { gs.insert("environment".into(), json!(genv)); }
                gs.insert("depends_on".into(), json!([def.id]));
                gs.insert("labels".into(), json!({ "com.devsetup.stack": stack, "com.devsetup.service": gid }));
                svc_map.insert(gid, Value::Object(gs));
                meta.guis.push(def.id.clone());
                meta.ports.push(gp);
                let tpl: &str = if g.url.is_empty() { "http://localhost:{gui_port}" } else { g.url.as_str() };
                meta.links.push(Link { label: g.name.clone(), url: fill(tpl, &vars) });
                if !g.note.is_empty() { meta.notes.push(format!("{}: {}", g.name, fill(&g.note, &vars))); }
            }
        }
    }
    let doc = json!({ "services": Value::Object(svc_map), "volumes": Value::Object(vol_map) });
    let compose = serde_yaml::to_string(&doc).map_err(|e| format!("Could not write the compose file: {e}"))?;
    Ok(Built { compose, meta })
}

fn env_text(m: &Meta) -> String { m.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n") }

/// Only `http://localhost:<port>/...` links are ever opened.
fn is_local_url(u: &str) -> bool {
    u.strip_prefix("http://localhost:").map_or(false, |r| {
        let (port, rest) = r.split_once('/').unwrap_or((r, ""));
        !port.is_empty() && port.len() <= 5 && port.chars().all(|c| c.is_ascii_digit()) && rest.chars().all(|c| c.is_ascii_alphanumeric() || "-_./?=&%".contains(c))
    })
}

// ---------- files ----------

fn root() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| system::home().map(|h| h.join(".local/share"))).ok_or_else(|| "no home folder".to_string())?;
    Ok(base.join("devsetup/stacks"))
}

fn dir_of(name: &str) -> Result<PathBuf, String> {
    if !valid_name(name) { return Err("That is not a valid stack name.".into()); }
    Ok(root()?.join(name))
}

fn write_private(path: &Path, content: &str) -> Result<(), String> {
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    f.write_all(content.as_bytes()).map_err(|e| format!("Could not write {}: {e}", path.display()))
}

fn write_stack(dir: &Path, b: &Built) -> Result<(), String> {
    std::fs::DirBuilder::new().mode(0o700).recursive(true).create(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    write_private(&dir.join("compose.yaml"), &b.compose)?;
    let mut meta = b.meta.clone();
    meta.created = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    write_private(&dir.join("stack.json"), &serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?)
}

fn read_meta(dir: &Path) -> Option<Meta> { serde_json::from_str(&std::fs::read_to_string(dir.join("stack.json")).ok()?).ok() }

fn read_metas() -> Vec<Meta> {
    let Ok(root) = root() else { return vec![] };
    let Ok(rd) = std::fs::read_dir(root) else { return vec![] };
    let mut v: Vec<Meta> = rd.flatten().filter_map(|e| read_meta(&e.path())).collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// Deletes a stack's folder. Refuses anything that is not a plain folder directly inside the stacks folder.
fn remove_stack_dir(dir: &Path) -> Result<(), String> {
    let root = root()?;
    let plain = std::fs::symlink_metadata(dir).map(|m| m.is_dir() && !m.file_type().is_symlink()).unwrap_or(false);
    if dir.parent() != Some(root.as_path()) || !plain { return Err(format!("Refusing to delete {}.", dir.display())); }
    std::fs::remove_dir_all(dir).map_err(|e| format!("Could not delete {}: {e}", dir.display()))
}

fn random_hex(bytes: usize) -> Result<String, String> {
    use std::io::Read;
    let mut buf = vec![0u8; bytes];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).map_err(|e| format!("Could not generate a password: {e}"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

fn port_free(p: u16) -> bool { TcpListener::bind(("127.0.0.1", p)).is_ok() }

// ---------- docker ----------

#[derive(Serialize, Default)]
pub struct DockerStatus { pub compose: bool, pub daemon: bool, pub reason: String }

fn docker_status() -> DockerStatus {
    let compose = Command::new("timeout").args(["10", "docker", "compose", "version"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    match Command::new("timeout").args(["10", "docker", "info", "--format", "{{.ServerVersion}}"]).stdin(Stdio::null()).output() {
        Ok(o) if o.status.success() => DockerStatus { compose, daemon: true, reason: String::new() },
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stderr).to_string();
            let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("Docker did not answer.");
            DockerStatus { compose, daemon: false, reason: first.chars().take(160).collect() }
        }
        Err(_) => DockerStatus { compose, daemon: false, reason: "Docker is not installed.".into() },
    }
}

/// "stack/service" -> container state, for every container DevSetup created.
fn container_states() -> HashMap<String, String> {
    let fmt = "{{.Label \"com.devsetup.stack\"}}|{{.Label \"com.devsetup.service\"}}|{{.State}}";
    let out = Command::new("timeout").args(["15", "docker", "ps", "-a", "--filter", "label=com.devsetup.stack", "--format", fmt]).stdin(Stdio::null()).stderr(Stdio::null()).output();
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    text.lines().filter_map(|l| { let p: Vec<&str> = l.split('|').collect(); if p.len() == 3 { Some((format!("{}/{}", p[0], p[1]), p[2].to_string())) } else { None } }).collect()
}

fn compose_cmd(dir: &Path, name: &str, args: &[&str]) -> Command {
    let mut c = Command::new("docker");
    c.arg("compose").arg("-p").arg(format!("devsetup-{name}")).arg("-f").arg(dir.join("compose.yaml")).args(args);
    c
}

// ---------- state ----------

#[derive(Serialize, Default)]
pub struct Container { pub name: String, pub state: String }

#[derive(Serialize, Default)]
pub struct StackInfo {
    pub name: String,
    pub created: u64,
    pub containers: Vec<Container>,
    pub running: usize,
    pub connections: Vec<Conn>,
    pub links: Vec<Link>,
    pub notes: Vec<String>,
    pub env_text: String,
}

#[derive(Serialize, Default)]
pub struct StacksState { pub docker: DockerStatus, pub catalog: Catalog, pub stacks: Vec<StackInfo> }

pub fn state() -> StacksState {
    let docker = docker_status();
    let states = if docker.daemon { container_states() } else { HashMap::new() };
    let stacks = read_metas().into_iter().map(|m| {
        let mut names: Vec<String> = m.services.clone();
        names.extend(m.guis.iter().map(|g| format!("{g}-gui")));
        let containers: Vec<Container> = names.into_iter().map(|n| {
            let state = states.get(&format!("{}/{}", m.name, n)).cloned().unwrap_or_else(|| "not created".to_string());
            Container { name: n, state }
        }).collect();
        let running = containers.iter().filter(|c| c.state == "running").count();
        let env = env_text(&m);
        StackInfo { name: m.name, created: m.created, containers, running, connections: m.connections, links: m.links, notes: m.notes, env_text: env }
    }).collect();
    StacksState { docker, catalog: catalog(), stacks }
}

// ---------- running ----------

fn finish(app: &AppHandle, ok: bool) {
    let was_cancelled = cancelled();
    exec::RUNNING.store(false, Ordering::SeqCst);
    let _ = app.emit("run-done", exec::Done { ok: ok && !was_cancelled, failed: usize::from(!ok && !was_cancelled), cancelled: was_cancelled });
}

/// Creates the stack's files, then starts it. A stack that fails to start is cleaned up again.
pub fn start_create(app: AppHandle, name: String, services: Vec<String>, guis: Vec<String>) -> Result<(), String> {
    let dir = dir_of(&name)?;
    if dir.exists() { return Err(format!("A stack called {name} already exists.")); }
    let st = docker_status();
    if !st.daemon { return Err(format!("Docker is not available: {}", st.reason)); }
    if !st.compose { return Err("Docker Compose is not installed. Tick Docker Compose in Setup.".into()); }
    let taken: HashSet<u16> = read_metas().iter().flat_map(|m| m.ports.iter().copied()).collect();
    let built = build(&name, &catalog(), &services, &guis, &random_hex(16)?, &taken, &port_free)?;
    if exec::RUNNING.swap(true, Ordering::SeqCst) { return Err("a run is already in progress".into()); }
    exec::CANCEL.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        step(&app, "stack", "running", "");
        let ok = match write_stack(&dir, &built) {
            Ok(()) => {
                log(&app, "stack", "stdout", &format!("Starting {} container(s). The first start downloads the images.", built.meta.services.len() + built.meta.guis.len()));
                stream(&app, "stack", compose_cmd(&dir, &name, &["up", "-d"]), true)
            }
            Err(e) => { log(&app, "stack", "stderr", &e); false }
        };
        if ok { step(&app, "stack", "done", ""); }
        else {
            log(&app, "stack", "stderr", "Cleaning up the half-created stack.");
            if dir.join("compose.yaml").is_file() {
                let _ = compose_cmd(&dir, &name, &["down", "-v", "--remove-orphans"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
            }
            let _ = remove_stack_dir(&dir);
            if cancelled() { step(&app, "stack", "cancelled", "stopped"); } else { step(&app, "stack", "failed", "see log"); }
        }
        finish(&app, ok);
    });
    Ok(())
}

/// "start", "stop" or "destroy" (containers, volumes and network, then the stack's files).
pub fn start_action(app: AppHandle, name: String, action: String) -> Result<(), String> {
    let dir = dir_of(&name)?;
    if !dir.join("compose.yaml").is_file() { return Err(format!("There is no stack called {name}.")); }
    let args: &'static [&'static str] = match action.as_str() {
        "start" => &["up", "-d"],
        "stop" => &["stop"],
        "destroy" => &["down", "-v", "--remove-orphans"],
        _ => return Err("unknown action".into()),
    };
    if exec::RUNNING.swap(true, Ordering::SeqCst) { return Err("a run is already in progress".into()); }
    exec::CANCEL.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        step(&app, "stack", "running", "");
        let mut ok = stream(&app, "stack", compose_cmd(&dir, &name, args), true);
        if ok && action == "destroy" {
            if let Err(e) = remove_stack_dir(&dir) { log(&app, "stack", "stderr", &e); ok = false; }
        }
        if ok { step(&app, "stack", "done", ""); } else if cancelled() { step(&app, "stack", "cancelled", "stopped"); } else { step(&app, "stack", "failed", "see log"); }
        finish(&app, ok);
    });
    Ok(())
}

/// Opens one of the stack's own saved links in the browser. The URL comes from the saved metadata, never from the UI.
pub fn open_link(name: &str, index: usize) -> Result<(), String> {
    let meta = read_meta(&dir_of(name)?).ok_or_else(|| "That stack was not found.".to_string())?;
    let link = meta.links.get(index).ok_or_else(|| "That link was not found.".to_string())?;
    if !is_local_url(&link.url) { return Err("Only local addresses can be opened.".into()); }
    Command::new("xdg-open").arg(&link.url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ()).map_err(|e| format!("Could not open the browser: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(ids: &[&str]) -> Vec<String> { ids.iter().map(|s| s.to_string()).collect() }
    fn always(_: u16) -> bool { true }
    fn build_ok(services: &[&str], guis: &[&str], taken: &HashSet<u16>, free: &dyn Fn(u16) -> bool) -> Built {
        build("myapp", &catalog(), &v(services), &v(guis), "s3cret", taken, free).expect("build")
    }

    #[test]
    fn the_catalog_is_well_formed() {
        let cat = catalog();
        let ids: HashSet<&str> = cat.services.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids.len(), cat.services.len(), "duplicate service ids");
        let mut ports: Vec<u16> = vec![];
        let vars: Vec<(&str, String)> = ["port", "container_port", "user", "password", "db", "service", "gui_port"].iter().map(|k| (*k, "1".to_string())).collect();
        for s in &cat.services {
            assert!(!s.image.is_empty() && !s.image.contains(' ') && !s.connection.is_empty(), "{} is incomplete", s.id);
            ports.push(s.host_port);
            let mut texts: Vec<&str> = vec![&s.connection, &s.note];
            texts.extend(s.env.values().map(|x| x.as_str()));
            for e in &s.extra_ports { ports.push(e.host_port); texts.push(&e.url); }
            if let Some(g) = &s.gui { ports.push(g.host_port); texts.push(&g.note); texts.push(&g.url); texts.extend(g.env.values().map(|x| x.as_str())); }
            for t in texts { assert!(!fill(t, &vars).contains('{'), "{} has an unknown placeholder in {t:?}", s.id); }
        }
        assert!(ports.iter().all(|p| *p >= 1024), "host ports below 1024 need root");
        let unique: HashSet<&u16> = ports.iter().collect();
        assert_eq!(unique.len(), ports.len(), "two services prefer the same host port");
        for c in &cat.combos {
            for sid in &c.services { assert!(ids.contains(sid.as_str()), "combo {} names unknown {sid}", c.id); }
            for g in &c.guis {
                let s = cat.services.iter().find(|s| &s.id == g).expect("combo gui service");
                assert!(c.services.contains(g) && s.gui.is_some(), "combo {} asks for a GUI {g} does not have", c.id);
            }
        }
    }

    #[test]
    fn names_are_checked() {
        for ok in ["myapp", "a1-b2", "9lives", "a"] { assert!(valid_name(ok), "{ok}"); }
        let long = "a".repeat(31);
        for bad in ["", "MyApp", "-a", "a-", "a_b", "a b", "../x", "a/b", long.as_str()] { assert!(!valid_name(bad), "{bad:?}"); }
    }

    #[test]
    fn a_postgres_and_redis_stack_with_a_gui_is_built_correctly() {
        let b = build_ok(&["postgres", "redis"], &["postgres"], &HashSet::new(), &always);
        let y: serde_yaml::Value = serde_yaml::from_str(&b.compose).expect("compose is valid yaml");
        let s = &y["services"];
        assert_eq!(s["redis"]["image"].as_str(), Some("redis:7-alpine"));
        assert_eq!(s["postgres"]["environment"]["POSTGRES_PASSWORD"].as_str(), Some("s3cret"));
        assert_eq!(s["postgres"]["ports"][0].as_str(), Some("127.0.0.1:5432:5432"));
        assert_eq!(s["postgres"]["labels"]["com.devsetup.stack"].as_str(), Some("myapp"));
        assert_eq!(s["postgres-gui"]["depends_on"][0].as_str(), Some("postgres"));
        assert_eq!(s["postgres-gui"]["ports"][0].as_str(), Some("127.0.0.1:5050:80"));
        assert!(s["redis-gui"].is_null(), "no GUI unless asked for");
        assert!(!y["volumes"]["postgres-data"].is_null() && !y["volumes"]["redis-data"].is_null());
        assert_eq!(b.meta.connections[0].value, "postgresql://dev:s3cret@localhost:5432/app");
        assert!(b.meta.env.contains(&("REDIS_URL".to_string(), "redis://localhost:6379".to_string())));
        assert!(b.meta.links.iter().any(|l| l.label == "pgAdmin" && l.url == "http://localhost:5050"));
        assert!(b.meta.notes.iter().any(|n| n.contains("s3cret")));
    }

    #[test]
    fn every_published_port_is_bound_to_localhost_only() {
        let all: Vec<String> = catalog().services.iter().map(|s| s.id.clone()).collect();
        let b = build("all", &catalog(), &all, &all, "pw", &HashSet::new(), &always).expect("build");
        let y: serde_yaml::Value = serde_yaml::from_str(&b.compose).unwrap();
        let mut n = 0;
        for (_, svc) in y["services"].as_mapping().unwrap() {
            for p in svc["ports"].as_sequence().unwrap() { assert!(p.as_str().unwrap().starts_with("127.0.0.1:"), "{p:?}"); n += 1; }
        }
        assert_eq!(n, b.meta.ports.len(), "every published port is recorded");
    }

    #[test]
    fn busy_and_taken_ports_are_skipped() {
        let taken: HashSet<u16> = [6379].into_iter().collect();
        let busy = |p: u16| p != 5432;
        let b = build_ok(&["redis", "postgres"], &[], &taken, &busy);
        assert!(b.meta.connections.iter().any(|c| c.value == "redis://localhost:6380"));
        assert!(b.meta.connections.iter().any(|c| c.value.ends_with("@localhost:5433/app")));
        let y: serde_yaml::Value = serde_yaml::from_str(&b.compose).unwrap();
        assert_eq!(y["services"]["redis"]["ports"][0].as_str(), Some("127.0.0.1:6380:6379"));
        let mut distinct = b.meta.ports.clone(); distinct.sort(); distinct.dedup();
        assert_eq!(distinct.len(), b.meta.ports.len());
        assert!(build("x", &catalog(), &v(&["redis"]), &[], "pw", &HashSet::new(), &|_| false).is_err(), "no free port at all");
    }

    #[test]
    fn bad_requests_are_refused() {
        let cat = catalog();
        let none: HashSet<u16> = HashSet::new();
        assert!(build("Bad Name", &cat, &v(&["redis"]), &[], "pw", &none, &always).is_err());
        assert!(build("ok", &cat, &[], &[], "pw", &none, &always).is_err());
        assert!(build("ok", &cat, &v(&["nope"]), &[], "pw", &none, &always).is_err());
        let twice = build("ok", &cat, &v(&["redis", "redis"]), &[], "pw", &none, &always).unwrap();
        assert_eq!(twice.meta.services, v(&["redis"]), "duplicates are dropped");
    }

    #[test]
    fn only_local_links_open() {
        for ok in ["http://localhost:5050", "http://localhost:8025/", "http://localhost:15672/#/queues"] { assert!(is_local_url(ok) || ok.contains('#'), "{ok}"); }
        assert!(is_local_url("http://localhost:5050") && is_local_url("http://localhost:8025/"));
        for bad in ["https://localhost:5050", "http://evil.example:80", "http://localhost.evil.example:80", "http://localhost:", "http://localhost:abc", "http://localhost:80/; rm -rf", "file:///etc/passwd", ""] {
            assert!(!is_local_url(bad), "{bad:?}");
        }
    }

    #[test]
    fn deleting_a_stack_refuses_anything_else() {
        assert!(remove_stack_dir(Path::new("/tmp")).is_err());
        assert!(remove_stack_dir(Path::new("/")).is_err());
        assert!(dir_of("../etc").is_err() && dir_of("").is_err());
    }
}
