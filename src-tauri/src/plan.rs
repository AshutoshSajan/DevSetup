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

pub fn build(m: &Manifest, selected: &[String]) -> Plan {
    let apt = apt_installed();
    let managed = state::load().managed;
    let installed: HashSet<&str> = m.tools.iter().filter(|t| is_installed(t, &apt)).map(|t| t.id.as_str()).collect();
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
