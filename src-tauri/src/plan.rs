use crate::{manifest::Manifest, state, system::{apt_installed, is_installed}};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Default, Serialize)]
pub struct Plan {
    pub install: Vec<String>,
    pub remove: Vec<String>,
    /// Installed and unticked, but not removed: not installed by DevSetup, or another tool still needs it.
    pub left_alone: Vec<String>,
    pub keep: Vec<String>,
    pub warnings: Vec<String>,
}

/// Plans against this machine: reads what is installed and what DevSetup installed itself.
pub fn build(m: &Manifest, selected: &[String]) -> Plan {
    let apt = apt_installed();
    let managed = state::load().managed;
    let installed: HashSet<&str> = m.tools.iter().filter(|t| is_installed(t, &apt)).map(|t| t.id.as_str()).collect();
    build_with(m, selected, &installed, &managed)
}

/// The planning rules. The machine's state is passed in, so they can be tested without touching the system.
pub(crate) fn build_with(m: &Manifest, selected: &[String], installed: &HashSet<&str>, managed: &[String]) -> Plan {
    // Unticked tools that DevSetup installed itself are the removal candidates.
    let removing: HashSet<&str> = m.tools.iter()
        .filter(|t| installed.contains(t.id.as_str()) && !selected.contains(&t.id) && managed.contains(&t.id))
        .map(|t| t.id.as_str()).collect();
    let mut p = Plan::default();
    for t in &m.tools {
        let sel = selected.contains(&t.id);
        match (sel, installed.contains(t.id.as_str())) {
            (true, false) => p.install.push(t.id.clone()),
            (true, true) => p.keep.push(t.id.clone()),
            (false, true) if removing.contains(t.id.as_str()) => {
                let needed_by: Vec<&str> = m.tools.iter()
                    .filter(|d| d.requires.contains(&t.id)
                        && (selected.contains(&d.id) || (installed.contains(d.id.as_str()) && !removing.contains(d.id.as_str()))))
                    .map(|d| d.name.as_str()).collect();
                if needed_by.is_empty() { p.remove.push(t.id.clone()); }
                else {
                    p.left_alone.push(t.id.clone());
                    p.warnings.push(format!("{} stays installed: {} needs it", t.name, needed_by.join(", ")));
                }
            }
            (false, true) => p.left_alone.push(t.id.clone()),
            _ => {}
        }
        if sel {
            for r in &t.requires {
                if !selected.contains(r) { p.warnings.push(format!("{} needs {}", t.name, r)); }
            }
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Method, Tool};

    fn tool(id: &str, requires: &[&str]) -> Tool {
        Tool { id: id.into(), name: id.into(), group: "g".into(), check: String::new(), roles: vec![],
               requires: requires.iter().map(|r| r.to_string()).collect(), method: Method::Apt { package: id.into() } }
    }
    fn manifest() -> Manifest {
        // b needs a; c and d stand alone.
        Manifest { roles: vec![], tools: vec![tool("a", &[]), tool("b", &["a"]), tool("c", &[]), tool("d", &[])] }
    }
    fn v(ids: &[&str]) -> Vec<String> { ids.iter().map(|s| s.to_string()).collect() }
    fn set<'a>(ids: &[&'a str]) -> HashSet<&'a str> { ids.iter().copied().collect() }

    #[test]
    fn ticked_tools_are_installed_or_kept() {
        let p = build_with(&manifest(), &v(&["c", "d"]), &set(&["d"]), &[]);
        assert_eq!(p.install, v(&["c"]));
        assert_eq!(p.keep, v(&["d"]));
        assert!(p.remove.is_empty() && p.left_alone.is_empty() && p.warnings.is_empty());
    }

    #[test]
    fn only_tools_devsetup_installed_can_be_removed() {
        let p = build_with(&manifest(), &[], &set(&["c", "d"]), &v(&["c"]));
        assert_eq!(p.remove, v(&["c"]));
        assert_eq!(p.left_alone, v(&["d"]), "installed by the user, so it is left alone");
    }

    #[test]
    fn a_tool_that_another_tool_still_needs_is_kept_with_a_warning() {
        let p = build_with(&manifest(), &v(&["b"]), &set(&["a", "b"]), &v(&["a", "b"]));
        assert!(p.remove.is_empty());
        assert_eq!(p.left_alone, v(&["a"]));
        assert_eq!(p.keep, v(&["b"]));
        assert!(p.warnings.contains(&"a stays installed: b needs it".to_string()), "{:?}", p.warnings);
    }

    #[test]
    fn a_tool_and_what_needs_it_can_be_removed_together() {
        let p = build_with(&manifest(), &[], &set(&["a", "b"]), &v(&["a", "b"]));
        assert_eq!(p.remove, v(&["a", "b"]));
        assert!(p.left_alone.is_empty() && p.warnings.is_empty());
    }

    #[test]
    fn a_ticked_tool_without_its_requirement_gets_a_warning() {
        let p = build_with(&manifest(), &v(&["b"]), &set(&[]), &[]);
        assert_eq!(p.install, v(&["b"]));
        assert_eq!(p.warnings, v(&["b needs a"]));
    }

    #[test]
    fn untouched_tools_appear_nowhere() {
        let p = build_with(&manifest(), &[], &set(&[]), &[]);
        assert!(p.install.is_empty() && p.keep.is_empty() && p.remove.is_empty() && p.left_alone.is_empty() && p.warnings.is_empty());
    }

    #[test]
    fn a_tool_ticked_for_install_blocks_removal_of_what_it_needs() {
        // a is installed by DevSetup and unticked, b (needs a) is newly ticked: a must stay.
        let p = build_with(&manifest(), &v(&["b"]), &set(&["a"]), &v(&["a"]));
        assert_eq!(p.install, v(&["b"]));
        assert_eq!(p.left_alone, v(&["a"]));
        assert!(p.remove.is_empty());
    }
}
