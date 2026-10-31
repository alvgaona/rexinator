//! Remembers the sessions rexinator started, with the variables they were
//! started with, so `list` and `stop --all` can find sessions whose names
//! came from variables.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Started {
    pub session_id: String,
    pub label: String,
    pub project: PathBuf,
    #[serde(default)]
    pub vars: HashMap<String, String>,
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub layout: Option<String>,
}

fn file() -> PathBuf {
    let dir = match std::env::var_os("XDG_STATE_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => crate::project::home().join(".local/state"),
    };
    dir.join("rexinator/sessions.json")
}

/// The recorded sessions. A missing or unreadable file means none.
pub fn load() -> Vec<Started> {
    std::fs::read_to_string(file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(all: &[Started]) -> Result<()> {
    let path = file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(all)?)?;
    Ok(())
}

pub fn record(entry: Started) -> Result<()> {
    let mut all = load();
    all.retain(|s| s.session_id != entry.session_id);
    all.push(entry);
    save(&all)
}

pub fn forget(session_id: &str) -> Result<()> {
    let mut all = load();
    let before = all.len();
    all.retain(|s| s.session_id != session_id);
    if all.len() != before {
        save(&all)
    } else {
        Ok(())
    }
}

/// Recorded sessions for one project file.
pub fn for_project<'a>(all: &'a [Started], project: &Path) -> impl Iterator<Item = &'a Started> {
    let project = std::path::absolute(project).unwrap_or_else(|_| project.to_path_buf());
    all.iter().filter(move |s| s.project == project)
}
