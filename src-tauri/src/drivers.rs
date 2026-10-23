//! Driver helper: read-only GPU and Secure Boot diagnostics with steps to follow yourself.
//! Nothing is installed or changed here: a failed graphics driver install can leave a machine without a desktop.
use crate::system;
use serde::Serialize;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Serialize, Default, Clone, PartialEq, Debug)]
pub struct Gpu {
    /// "NVIDIA", "AMD", "Intel" or "Other"
    pub vendor: String,
    /// PCI id such as "10de:25a2"
    pub id: String,
    pub name: String,
    /// Kernel driver in use, empty when none is bound
    pub driver: String,
}

#[derive(Serialize, Default, Clone)]
pub struct Advice {
    /// "ok", "info" or "warn"
    pub level: &'static str,
    pub title: String,
    pub detail: String,
    pub steps: Vec<String>,
    pub commands: Vec<String>,
}

#[derive(Serialize, Default, Clone)]
pub struct Report {
    pub gpus: Vec<Gpu>,
    /// "enabled", "disabled", "unsupported" (legacy BIOS boot) or "unknown"
    pub secure_boot: String,
    /// Version reported by nvidia-smi when the NVIDIA driver works
    pub nvidia_driver: Option<String>,
    pub nonfree: bool,
    pub nonfree_firmware: bool,
    pub nvidia_candidate: bool,
    pub distro_id: String,
    pub distro: String,
    pub advice: Vec<Advice>,
}

// ---------- parsing (pure) ----------

fn parse_hex_id(s: &str) -> Option<u16> { u16::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok() }

fn vendor_name(vendor: u16) -> &'static str {
    match vendor { 0x10de => "NVIDIA", 0x1002 => "AMD", 0x8086 => "Intel", _ => "Other" }
}

/// (pci id, name) for each display controller in `lspci -nn` output.
pub(crate) fn parse_lspci(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|l| l.contains("VGA compatible controller") || l.contains("3D controller") || l.contains("Display controller"))
        .filter_map(|l| {
            let open = l.rfind('[')?;
            let close = open + l[open..].find(']')?;
            let id = l[open + 1..close].to_ascii_lowercase();
            let hex = |p: &str| p.len() == 4 && p.chars().all(|c| c.is_ascii_hexdigit());
            let (v, d) = id.split_once(':')?;
            if !hex(v) || !hex(d) { return None; }
            let start = l.find("]: ")? + 3;
            if start > open { return None; }
            Some((id.clone(), l[start..open].trim().to_string()))
        })
        .collect()
}

/// The efivars file holds 4 attribute bytes followed by the value, 1 when Secure Boot is on.
fn parse_secure_boot(bytes: &[u8]) -> Option<bool> {
    if bytes.len() < 5 { None } else { bytes.last().map(|b| *b == 1) }
}

/// Driver version from `nvidia-smi --query-gpu=name,driver_version --format=csv,noheader`.
fn parse_smi(text: &str) -> Option<String> {
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let version = line.rsplit(',').next()?.trim();
    if !version.is_empty() && version.chars().all(|c| c.is_ascii_digit() || c == '.') { Some(version.to_string()) } else { None }
}

fn has_component(sources: &str, component: &str) -> bool {
    sources.lines().filter(|l| !l.trim_start().starts_with('#')).any(|l| l.split_whitespace().any(|t| t == component))
}

fn parse_candidate(policy: &str) -> bool {
    policy.lines().find_map(|l| l.trim().strip_prefix("Candidate:")).map_or(false, |c| c.trim() != "(none)" && !c.trim().is_empty())
}

// ---------- detection ----------

fn run(program: &str, args: &[&str]) -> String {
    Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

fn read_trim(p: &Path) -> Option<String> { std::fs::read_to_string(p).ok().map(|s| s.trim().to_string()) }

fn detect_gpus() -> Vec<Gpu> {
    let names = parse_lspci(&run("lspci", &["-nn"]));
    let mut out: Vec<Gpu> = vec![];
    if let Ok(rd) = std::fs::read_dir("/sys/class/drm") {
        let mut cards: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("card") && n.len() > 4 && n[4..].chars().all(|c| c.is_ascii_digit())).collect();
        cards.sort();
        for c in cards {
            let dev = Path::new("/sys/class/drm").join(&c).join("device");
            let (Some(v), Some(d)) = (read_trim(&dev.join("vendor")).and_then(|s| parse_hex_id(&s)), read_trim(&dev.join("device")).and_then(|s| parse_hex_id(&s))) else { continue };
            let id = format!("{v:04x}:{d:04x}");
            let driver = std::fs::read_link(dev.join("driver")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
            let name = names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone()).unwrap_or_else(|| format!("GPU {id}"));
            out.push(Gpu { vendor: vendor_name(v).to_string(), id, name, driver });
        }
    }
    if out.is_empty() {
        for (id, name) in names {
            let v = u16::from_str_radix(&id[..4], 16).unwrap_or(0);
            out.push(Gpu { vendor: vendor_name(v).to_string(), id, name, driver: String::new() });
        }
    }
    out
}

const SECURE_BOOT_VAR: &str = "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";

fn secure_boot() -> &'static str {
    if !Path::new("/sys/firmware/efi").exists() { return "unsupported"; }
    match std::fs::read(SECURE_BOOT_VAR).ok().and_then(|b| parse_secure_boot(&b)) { Some(true) => "enabled", Some(false) => "disabled", None => "unknown" }
}

fn read_sources() -> String {
    let mut s = std::fs::read_to_string("/etc/apt/sources.list").unwrap_or_default();
    if let Ok(rd) = std::fs::read_dir("/etc/apt/sources.list.d") {
        for e in rd.flatten() {
            let p = e.path();
            if matches!(p.extension().and_then(|x| x.to_str()), Some("list") | Some("sources")) {
                s.push('\n');
                s.push_str(&std::fs::read_to_string(&p).unwrap_or_default());
            }
        }
    }
    s
}

// ---------- advice (pure) ----------

fn adv(level: &'static str, title: &str, detail: &str, steps: &[&str], commands: &[&str]) -> Advice {
    Advice { level, title: title.to_string(), detail: detail.to_string(), steps: steps.iter().map(|s| s.to_string()).collect(), commands: commands.iter().map(|s| s.to_string()).collect() }
}

pub(crate) fn advise(r: &Report) -> Vec<Advice> {
    let mut v: Vec<Advice> = vec![];
    v.push(match r.secure_boot.as_str() {
        "enabled" => adv("info", "Secure Boot is on", "The kernel only loads signed modules. Drivers that come from outside the kernel (the proprietary NVIDIA driver, VirtualBox) must be signed with a key you enroll, or they will not load.", &[], &[]),
        "disabled" => adv("info", "Secure Boot is off", "Third-party kernel modules load without being signed.", &[], &[]),
        "unsupported" => adv("info", "Legacy BIOS boot", "This machine does not boot through UEFI, so Secure Boot does not apply.", &[], &[]),
        _ => adv("info", "Secure Boot state unknown", "It could not be read. Run mokutil --sb-state in a terminal to check.", &[], &["mokutil --sb-state"]),
    });
    if r.gpus.is_empty() {
        v.push(adv("info", "No graphics device found", "The kernel lists no GPU. This is normal inside a virtual machine or container.", &[], &[]));
    }
    let hybrid = r.gpus.len() > 1;
    let (debian, ubuntu) = (r.distro_id == "debian", r.distro_id == "ubuntu");
    let mut nvidia_working = false;
    for g in &r.gpus {
        let driver = if g.driver.is_empty() { "no driver".to_string() } else { g.driver.clone() };
        match g.vendor.as_str() {
            "NVIDIA" => {
                if let Some(ver) = &r.nvidia_driver {
                    nvidia_working = true;
                    let mut a = adv("ok", &format!("NVIDIA driver {ver} is working"), &g.name, &[], &[]);
                    if hybrid {
                        a.steps.push("This machine has more than one GPU. To run one program on the NVIDIA GPU, start it with these variables set.".to_string());
                        a.commands.push("__NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia <program>".to_string());
                    }
                    v.push(a);
                } else if g.driver == "nvidia" {
                    let a = if debian { adv("warn", "The NVIDIA module is loaded but nvidia-smi is missing", &g.name, &["nvidia-smi comes in its own package."], &["sudo apt install nvidia-smi"]) }
                            else { adv("warn", "The NVIDIA module is loaded but nvidia-smi is missing", &g.name, &["Install the nvidia-utils package that matches your driver version."], &[]) };
                    v.push(a);
                } else {
                    let mut a = adv("warn", "The NVIDIA GPU is not using the NVIDIA driver", &format!("{} currently uses {driver}.", g.name),
                        &["DevSetup does not install graphics drivers for you: a failed driver install can leave you without a desktop. These are steps to follow yourself."], &[]);
                    if ubuntu {
                        a.commands.push("sudo ubuntu-drivers install".to_string());
                    } else if debian {
                        if !r.nonfree {
                            a.steps.push("Add the non-free and non-free-firmware components after main contrib in /etc/apt/sources.list or /etc/apt/sources.list.d/debian.sources.".to_string());
                        }
                        a.steps.push("Update the package lists, then install the driver. The kernel module is built for your kernel through DKMS.".to_string());
                        a.commands.push("sudo apt update".to_string());
                        a.commands.push("sudo apt install nvidia-driver firmware-misc-nonfree".to_string());
                    } else {
                        a.steps.push("See your distribution's documentation for installing the NVIDIA driver.".to_string());
                    }
                    if r.secure_boot == "enabled" {
                        a.steps.push("Secure Boot is on. When the installer asks for a password, choose a one-time password. On the next boot choose Enroll MOK and enter it, otherwise the driver will not load. You can also turn Secure Boot off in your firmware settings.".to_string());
                    }
                    a.steps.push("Restart, then check that the driver works.".to_string());
                    a.commands.push("nvidia-smi".to_string());
                    v.push(a);
                }
            }
            "AMD" => {
                if g.driver == "amdgpu" {
                    let mut a = adv("ok", "The AMD GPU uses the amdgpu driver", &g.name, &[], &[]);
                    if debian && !r.nonfree_firmware {
                        a.steps.push("Graphics firmware comes from the non-free-firmware component. Add it to your apt sources, then install it if you see display problems.".to_string());
                        a.commands.push("sudo apt install firmware-amd-graphics".to_string());
                    }
                    v.push(a);
                } else {
                    v.push(adv("warn", "The AMD GPU is not using the amdgpu driver", &format!("{} currently uses {driver}.", g.name), &["Check that your kernel and firmware packages are up to date, then restart."], &[]));
                }
            }
            "Intel" => {
                if g.driver == "i915" || g.driver == "xe" { v.push(adv("ok", &format!("The Intel GPU uses the {} driver", g.driver), &g.name, &[], &[])); }
                else { v.push(adv("warn", "The Intel GPU has no kernel driver", &format!("{} currently uses {driver}.", g.name), &["Check that your kernel is up to date, then restart."], &[])); }
            }
            _ => v.push(adv("info", &format!("{} uses {driver}", g.name), "DevSetup has no advice for this graphics device.", &[], &[])),
        }
    }
    if nvidia_working {
        v.push(adv("info", "CUDA and GPU containers", "Optional, for machine learning and GPU work.",
            &["The CUDA toolkit is a large package from Debian's non-free section.",
              "GPU access inside Docker needs the NVIDIA Container Toolkit. It comes from NVIDIA's own package repository, so follow NVIDIA's install guide: https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/install-guide.html"],
            &["sudo apt install nvidia-cuda-toolkit"]));
    }
    v.push(adv("info", "Android phones over USB (adb)", "If adb lists a device as \"no permissions\", the udev rules are missing. You can also tick Android udev rules under Mobile in Setup.",
        &["Install the rules, then unplug and replug the phone and accept the prompt on its screen."],
        &["sudo apt install android-sdk-platform-tools-common", "adb devices"]));
    v
}

pub fn scan() -> Report {
    let d = system::distro();
    let sources = read_sources();
    let smi = if system::on_path("nvidia-smi") { parse_smi(&run("timeout", &["10", "nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"])) } else { None };
    let mut r = Report {
        gpus: detect_gpus(),
        secure_boot: secure_boot().to_string(),
        nvidia_driver: smi,
        nonfree: has_component(&sources, "non-free"),
        nonfree_firmware: has_component(&sources, "non-free-firmware"),
        nvidia_candidate: system::on_path("apt-cache") && parse_candidate(&run("apt-cache", &["policy", "nvidia-driver"])),
        distro_id: d.id,
        distro: d.name,
        advice: vec![],
    };
    r.advice = advise(&r);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(vendor: &str, driver: &str) -> Gpu { Gpu { vendor: vendor.into(), id: "0000:0000".into(), name: format!("{vendor} test GPU"), driver: driver.into() } }
    fn report(distro: &str, gpus: Vec<Gpu>) -> Report { Report { gpus, secure_boot: "disabled".into(), distro_id: distro.into(), ..Default::default() } }
    fn titles(v: &[Advice]) -> Vec<&str> { v.iter().map(|a| a.title.as_str()).collect() }

    #[test]
    fn parses_lspci_display_controllers_only() {
        let text = "00:02.0 VGA compatible controller [0300]: Intel Corporation TigerLake-LP GT2 [Iris Xe Graphics] [8086:9a49] (rev 01)\n\
                    00:1f.3 Audio device [0403]: Intel Corporation Tiger Lake-LP Smart Sound Technology Audio Controller [8086:a0c8] (rev 20)\n\
                    01:00.0 3D controller [0302]: NVIDIA Corporation GA107M [GeForce RTX 3050 Mobile] [10de:25a2] (rev a1)\n";
        let got = parse_lspci(text);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], ("8086:9a49".to_string(), "Intel Corporation TigerLake-LP GT2 [Iris Xe Graphics]".to_string()));
        assert_eq!(got[1], ("10de:25a2".to_string(), "NVIDIA Corporation GA107M [GeForce RTX 3050 Mobile]".to_string()));
        assert!(parse_lspci("garbage\n01:00.0 VGA compatible controller: no ids here\n").is_empty());
    }

    #[test]
    fn parses_ids_secure_boot_smi_sources_and_candidates() {
        assert_eq!((parse_hex_id("0x10de"), parse_hex_id("1002\n"), parse_hex_id("zz")), (Some(0x10de), Some(0x1002), None));
        assert_eq!((vendor_name(0x10de), vendor_name(0x1002), vendor_name(0x8086), vendor_name(0x1234)), ("NVIDIA", "AMD", "Intel", "Other"));
        assert_eq!((parse_secure_boot(&[6, 0, 0, 0, 1]), parse_secure_boot(&[6, 0, 0, 0, 0]), parse_secure_boot(&[1, 2])), (Some(true), Some(false), None));
        assert_eq!(parse_smi("NVIDIA GeForce RTX 3050 Laptop GPU, 550.54.14\n"), Some("550.54.14".to_string()));
        assert_eq!(parse_smi("NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver."), None);
        assert_eq!(parse_smi(""), None);
        let deb822 = "Types: deb\nComponents: main contrib non-free non-free-firmware\n";
        assert!(has_component(deb822, "non-free") && has_component(deb822, "non-free-firmware"));
        assert!(!has_component("Components: main non-free-firmware\n", "non-free"), "non-free-firmware is not non-free");
        assert!(!has_component("# deb http://x stable main non-free\ndeb http://x stable main\n", "non-free"), "comments do not count");
        assert!(parse_candidate("nvidia-driver:\n  Installed: (none)\n  Candidate: 550.1-1\n"));
        assert!(!parse_candidate("nvidia-driver:\n  Installed: (none)\n  Candidate: (none)\n"));
    }

    #[test]
    fn nvidia_on_nouveau_with_secure_boot_on_debian_gets_the_full_steps() {
        let mut r = report("debian", vec![gpu("NVIDIA", "nouveau")]);
        r.secure_boot = "enabled".into();
        let a = advise(&r);
        let n = a.iter().find(|x| x.title == "The NVIDIA GPU is not using the NVIDIA driver").expect("warning");
        assert_eq!(n.level, "warn");
        assert!(n.commands.contains(&"sudo apt install nvidia-driver firmware-misc-nonfree".to_string()));
        assert!(n.steps.iter().any(|s| s.contains("non-free")), "sources step when non-free is missing");
        assert!(n.steps.iter().any(|s| s.contains("Enroll MOK")), "Secure Boot step");
        assert_eq!(a[0].title, "Secure Boot is on", "Secure Boot comes first");
    }

    #[test]
    fn the_sources_step_is_dropped_once_non_free_is_enabled_and_ubuntu_uses_its_own_tool() {
        let mut r = report("debian", vec![gpu("NVIDIA", "")]);
        r.nonfree = true;
        let n = advise(&r).into_iter().find(|x| x.level == "warn").unwrap();
        assert!(!n.steps.iter().any(|s| s.contains("non-free and non-free-firmware components")));
        let u = advise(&report("ubuntu", vec![gpu("NVIDIA", "nouveau")])).into_iter().find(|x| x.level == "warn").unwrap();
        assert!(u.commands.contains(&"sudo ubuntu-drivers install".to_string()) && !u.commands.iter().any(|c| c.contains("nvidia-driver")));
    }

    #[test]
    fn a_working_nvidia_driver_on_a_hybrid_laptop_gets_offload_and_cuda_notes() {
        let mut r = report("debian", vec![gpu("AMD", "amdgpu"), gpu("NVIDIA", "nvidia")]);
        r.nvidia_driver = Some("550.54.14".into());
        r.nonfree_firmware = true;
        let a = advise(&r);
        assert!(titles(&a).contains(&"NVIDIA driver 550.54.14 is working"));
        assert!(titles(&a).contains(&"The AMD GPU uses the amdgpu driver"));
        assert!(titles(&a).contains(&"CUDA and GPU containers"));
        let n = a.iter().find(|x| x.title.starts_with("NVIDIA driver")).unwrap();
        assert!(n.commands.iter().any(|c| c.contains("__NV_PRIME_RENDER_OFFLOAD=1")));
        assert!(a.iter().all(|x| x.level != "warn"), "a healthy setup raises no warnings");
    }

    #[test]
    fn other_vendors_and_missing_gpus_are_handled() {
        assert!(titles(&advise(&report("debian", vec![]))).contains(&"No graphics device found"));
        let intel = advise(&report("debian", vec![gpu("Intel", "i915")]));
        assert!(titles(&intel).contains(&"The Intel GPU uses the i915 driver"));
        let bad = advise(&report("debian", vec![gpu("Intel", "")]));
        assert!(bad.iter().any(|x| x.level == "warn"));
        assert!(titles(&advise(&report("debian", vec![]))).contains(&"Android phones over USB (adb)"));
    }

    #[test]
    fn advice_never_runs_anything() {
        // Commands are text for the user to copy. None may be empty or contain a line break.
        let mut r = report("debian", vec![gpu("NVIDIA", "nouveau"), gpu("AMD", "")]);
        r.secure_boot = "enabled".into();
        for a in advise(&r) { for c in a.commands { assert!(!c.trim().is_empty() && !c.contains('\n'), "{c:?}"); } }
    }
}
