//! The outside world rexinator starts from: working directory, home folder
//! and environment variables, read once and passed around as a value.

use anyhow::Result;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Env {
    pub cwd: PathBuf,
    pub home: PathBuf,
    /// Where named project files live.
    pub config_dir: PathBuf,
    /// The record of sessions rexinator started.
    pub state_file: PathBuf,
    /// The Rex session this terminal is in, if any.
    pub rex_session: Option<String>,
    /// The `rex` program to run.
    pub rex_bin: String,
    pub editor: String,
    /// Every environment variable, the last place `${NAME}` is looked up.
    pub vars: HashMap<String, String>,
}

impl Env {
    /// Read the process's environment. The only place rexinator does.
    pub fn from_process() -> Result<Self> {
        Ok(Self::new(
            std::env::current_dir()?,
            std::env::vars().collect(),
        ))
    }

    /// Derive everything else from a working directory and variables.
    pub fn new(cwd: PathBuf, vars: HashMap<String, String>) -> Self {
        let get = |name: &str| vars.get(name).filter(|v| !v.is_empty()).cloned();
        let home = get("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
        let config_dir = get("REXINATOR_CONFIG")
            .map(PathBuf::from)
            .or_else(|| get("XDG_CONFIG_HOME").map(|d| PathBuf::from(d).join("rexinator")))
            .unwrap_or_else(|| home.join(".config/rexinator"));
        let state_file = get("XDG_STATE_HOME")
            .map_or_else(|| home.join(".local/state"), PathBuf::from)
            .join("rexinator/sessions.json");
        let rex_session = get("REX_SESSION");
        let rex_bin = get("REXINATOR_REX").unwrap_or_else(|| "rex".into());
        let editor = get("VISUAL")
            .or_else(|| get("EDITOR"))
            .unwrap_or_else(|| "vi".into());
        Self {
            config_dir: absolute(&cwd, &config_dir),
            state_file: absolute(&cwd, &state_file),
            rex_session,
            rex_bin,
            editor,
            home,
            cwd,
            vars,
        }
    }
}

/// `path` resolved against `base`, with `.` and `..` worked out from the
/// text alone, so `/git/app/src/../x` is `/git/app/x`.
pub fn absolute(base: &Path, path: &Path) -> PathBuf {
    base.join(path)
        .components()
        .fold(PathBuf::new(), |acc, part| match part {
            Component::CurDir => acc,
            Component::ParentDir => match acc.parent() {
                Some(up) => up.to_path_buf(),
                None => acc,
            },
            part => acc.join(part),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Env {
        let vars = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Env::new(PathBuf::from("/work"), vars)
    }

    #[test]
    fn derives_folders_and_programs() {
        let e = env(&[("HOME", "/home/me")]);
        assert_eq!(e.config_dir, PathBuf::from("/home/me/.config/rexinator"));
        assert_eq!(
            e.state_file,
            PathBuf::from("/home/me/.local/state/rexinator/sessions.json")
        );
        assert_eq!((e.rex_bin.as_str(), e.editor.as_str()), ("rex", "vi"));
        assert_eq!(e.rex_session, None);

        let e = env(&[
            ("HOME", "/home/me"),
            ("XDG_CONFIG_HOME", "/xdg"),
            ("XDG_STATE_HOME", "/state"),
            ("EDITOR", "nvim"),
            ("REX_SESSION", "session:1"),
        ]);
        assert_eq!(e.config_dir, PathBuf::from("/xdg/rexinator"));
        assert_eq!(
            e.state_file,
            PathBuf::from("/state/rexinator/sessions.json")
        );
        assert_eq!(e.editor, "nvim");
        assert_eq!(e.rex_session.as_deref(), Some("session:1"));
    }

    #[test]
    fn relative_config_folder_is_resolved_against_cwd() {
        let e = env(&[("HOME", "/h"), ("REXINATOR_CONFIG", "./projects")]);
        assert_eq!(e.config_dir, PathBuf::from("/work/projects"));
    }

    #[test]
    fn absolute_paths() {
        let abs = |base: &str, p: &str| absolute(Path::new(base), Path::new(p));
        assert_eq!(abs("/a/b", "../c/./d"), PathBuf::from("/a/c/d"));
        assert_eq!(abs("/a", "/x/../y"), PathBuf::from("/y"));
        assert_eq!(abs("/", "../.."), PathBuf::from("/"));
    }
}
