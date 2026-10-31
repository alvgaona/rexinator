//! Remembers the sessions rexinator started, with the variables they were
//! started with, so `list` and `stop --all` can find sessions whose names
//! came from variables.
//!
//! The record is a list of [`Started`] values. Changes are pure functions
//! from one list to the next; [`load`] and [`save`] are the only I/O.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::fs::Fs;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Started {
    pub session_id: String,
    pub label: String,
    /// The project file, as an absolute path.
    pub project: PathBuf,
    #[serde(default)]
    pub vars: HashMap<String, String>,
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub layout: Option<String>,
}

/// Records from the file's text. Unreadable text means none.
pub fn parse(text: &str) -> Vec<Started> {
    serde_json::from_str(text).unwrap_or_default()
}

/// The records with `entry` added, replacing any for the same session.
pub fn with(all: &[Started], entry: Started) -> Vec<Started> {
    without(all, &entry.session_id)
        .into_iter()
        .chain([entry])
        .collect()
}

/// The records without the one for `session_id`.
pub fn without(all: &[Started], session_id: &str) -> Vec<Started> {
    all.iter()
        .filter(|s| s.session_id != session_id)
        .cloned()
        .collect()
}

/// The records in `path`. A missing file means none.
pub fn load(fs: &Fs, path: &Path) -> Vec<Started> {
    (fs.read)(path).map(|t| parse(&t)).unwrap_or_default()
}

pub fn save(path: &Path, all: &[Started]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(all)?)?;
    Ok(())
}

/// Apply `change` to the records in `path` and write them back, if it changed anything.
pub fn update(fs: &Fs, path: &Path, change: impl Fn(&[Started]) -> Vec<Started>) -> Result<()> {
    let before = load(fs, path);
    let after = change(&before);
    if after == before {
        Ok(())
    } else {
        save(path, &after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(id: &str, label: &str) -> Started {
        Started {
            session_id: id.into(),
            label: label.into(),
            project: "/p.yml".into(),
            vars: HashMap::new(),
            server: None,
            layout: None,
        }
    }

    #[test]
    fn records_replace_and_forget() {
        let all = with(&[], started("1", "a"));
        let all = with(&all, started("2", "b"));
        let all = with(&all, started("1", "a2"));
        let labels: Vec<&str> = all.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["b", "a2"]);
        assert_eq!(without(&all, "2"), vec![started("1", "a2")]);
        assert_eq!(without(&all, "nope"), all);
    }

    #[test]
    fn parses_text_and_tolerates_junk() {
        let text = serde_json::to_string(&[started("1", "a")]).unwrap();
        assert_eq!(parse(&text), vec![started("1", "a")]);
        assert_eq!(parse("not json"), vec![]);
        // Fields added later are optional.
        let old = r#"[{"session_id": "1", "label": "a", "project": "/p.yml"}]"#;
        assert_eq!(parse(old), vec![started("1", "a")]);
    }
}
