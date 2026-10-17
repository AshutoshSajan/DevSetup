use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Role { pub id: String, pub name: String, pub description: String }

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Method {
    Apt { package: String },
    Flatpak { app_id: String },
    Mise { tool: String, #[serde(default)] version: Option<String> },
    Npm { package: String, #[serde(default)] version: Option<String> },
    Pipx { package: String, #[serde(default)] version: Option<String> },
    Script { url: String, #[serde(default)] sha256: Option<String>, #[serde(default)] args: Vec<String>, #[serde(default)] uninstall: Option<String>, #[serde(default)] update: Option<String> },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tool {
    pub id: String,
    pub name: String,
    pub group: String,
    /// Command on PATH that proves the tool is installed (optional for apt/flatpak tools).
    #[serde(default)] pub check: String,
    /// Role ids, or "all" (always shown, pre-ticked) / "any" (shown once any role is picked).
    #[serde(default)] pub roles: Vec<String>,
    #[serde(default)] pub requires: Vec<String>,
    pub method: Method,
}

impl Tool {
    /// True for install scripts that carry a SHA-256 pin.
    pub fn script_is_pinned(&self) -> bool {
        matches!(&self.method, Method::Script { sha256: Some(s), .. } if !s.is_empty())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest { pub roles: Vec<Role>, pub tools: Vec<Tool> }

pub fn load() -> Manifest {
    serde_yaml::from_str(include_str!("../../manifest/tools.yaml")).expect("manifest/tools.yaml is invalid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn manifest_is_consistent() {
        let m = load();
        let ids: HashSet<&str> = m.tools.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids.len(), m.tools.len(), "duplicate tool ids");
        let roles: HashSet<&str> = m.roles.iter().map(|r| r.id.as_str()).collect();
        for t in &m.tools {
            for r in &t.requires { assert!(ids.contains(r.as_str()), "{} requires unknown tool {}", t.id, r); }
            if let Method::Script { sha256: Some(h), .. } = &t.method {
                assert!(h.len() == 64 && h.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)), "{} has a malformed sha256", t.id);
            }
            for r in &t.roles { assert!(r == "all" || r == "any" || roles.contains(r.as_str()), "{} has unknown role {}", t.id, r); }
        }
    }
}
