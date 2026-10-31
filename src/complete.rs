//! Shell completion. The shell calls back into rexinator (`COMPLETE=zsh
//! rexinator -- ...`) as you type, so commands and flags always match this
//! version, and project names and layouts come from your files.

use anyhow::{Result, anyhow};
use clap_complete::CompletionCandidate;
use clap_complete::env::Shells;

use crate::discovery;
use crate::env::Env;
use crate::fs::Fs;
use crate::project::{self, Request};

/// The script that hooks rexinator's completions into `shell`.
pub fn registration(shell: &str) -> Result<String> {
    let shells = Shells::builtins();
    let completer = shells
        .completer(shell)
        .ok_or_else(|| anyhow!("unknown shell `{shell}`"))?;
    let mut script = Vec::new();
    completer.write_registration(
        "COMPLETE",
        "rexinator",
        "rexinator",
        "rexinator",
        &mut script,
    )?;
    Ok(String::from_utf8(script)?)
}

/// Project names, for commands that take one.
pub fn projects() -> Vec<CompletionCandidate> {
    with_world(|fs, env| {
        discovery::project_names(fs, env)
            .into_iter()
            .map(CompletionCandidate::new)
            .collect()
    })
}

/// Project names, plus `@LAYOUT` for the layouts of the project you're in.
pub fn targets() -> Vec<CompletionCandidate> {
    with_world(|fs, env| {
        let layouts = layouts_here(fs, env)
            .into_iter()
            .map(|l| CompletionCandidate::new(format!("@{l}")).help(Some("layout".into())));
        projects().into_iter().chain(layouts).collect()
    })
}

/// The layouts of the project in this folder or a parent. None if there's no
/// project here, or it doesn't load.
fn layouts_here(fs: &Fs, env: &Env) -> Vec<String> {
    discovery::find_project(fs, env, None)
        .and_then(|path| project::load(fs, env, &path, &Request::default()))
        .map(|p| p.layouts)
        .unwrap_or_default()
}

/// Completion must never fail loudly: without an environment, offer nothing.
fn with_world(f: impl Fn(&Fs, &Env) -> Vec<CompletionCandidate>) -> Vec<CompletionCandidate> {
    Env::from_process()
        .map(|env| f(&Fs::real(), &env))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_every_supported_shell() {
        for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
            let script = registration(shell).unwrap();
            assert!(script.contains("rexinator"), "{shell}: {script}");
        }
        assert!(registration("tcsh").is_err());
    }
}
