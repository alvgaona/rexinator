//! Talking to Rex by running the `rex` command. This is the I/O edge:
//! building requests and reading replies are pure functions, and [`Rex`]
//! only runs the program.

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Rex {
    bin: String,
    /// Passed as `rex -S`. None uses the server `rex` finds by itself.
    server: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Session {
    pub session_id: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Created {
    pub session_id: String,
    #[serde(default)]
    pub initial_windows: Vec<CreatedWindow>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CreatedWindow {
    pub window_id: String,
    /// In layout order, the same order as `Node::panes`.
    #[serde(default)]
    pub block_ids: Vec<String>,
}

impl Rex {
    /// `bin` is the `rex` program to run.
    pub fn new(bin: &str, server: Option<String>) -> Self {
        Rex {
            bin: bin.to_string(),
            server,
        }
    }

    pub fn is_local(&self) -> bool {
        self.server.is_none()
    }

    fn run(&self, args: &[&str], stdin: Option<&str>) -> Result<String> {
        let mut cmd = Command::new(&self.bin);
        if let Some(server) = &self.server {
            cmd.args(["-S", server]);
        }
        let mut child = cmd
            .args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!(
                    "couldn't run `{}`. Is Rex installed and on your PATH?",
                    self.bin
                )
            })?;
        if let Some(input) = stdin {
            child.stdin.take().unwrap().write_all(input.as_bytes())?;
        }
        let out = child.wait_with_output()?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let err = err.trim().strip_prefix("Error: ").unwrap_or(err.trim());
            bail!(
                "`rex {}` failed: {}",
                args.first().copied().unwrap_or(""),
                err
            );
        }
        Ok(String::from_utf8(out.stdout)?)
    }

    pub fn sessions(&self) -> Result<Vec<Session>> {
        parse_sessions(&self.run(&["ls", "--json"], None)?)
    }

    pub fn create_session(&self, request: &Value) -> Result<Created> {
        let out = self.run(
            &["api", "call", "session.create", "-"],
            Some(&request.to_string()),
        )?;
        parse_reply(&out, "session.create")
    }

    /// Add a window to an existing session without switching to it.
    pub fn new_window(&self, session: &str, window: &Value) -> Result<CreatedWindow> {
        let out = self.run(
            &["api", "call", "-s", session, "session.new_window", "-"],
            Some(&unfocused(window).to_string()),
        )?;
        parse_reply(&out, "session.new_window")
    }

    pub fn focus_window(&self, session: &str, window: &str) -> Result<()> {
        let args = json!({ "window_id": window }).to_string();
        self.run(
            &["api", "call", "-s", session, "session.focus_window", &args],
            None,
        )?;
        Ok(())
    }

    /// Focus a pane, and make its window the active one.
    pub fn focus_block(&self, session: &str, block: &str) -> Result<()> {
        let args = json!({ "block_id": block }).to_string();
        self.run(
            &["api", "call", "-s", session, "session.focus_block", &args],
            None,
        )?;
        Ok(())
    }

    pub fn kill(&self, session: &str) -> Result<()> {
        self.run(&["kill", session], None)?;
        Ok(())
    }

    /// Ask the Rex app to switch to the session. Works when exactly one app is
    /// connected, or when `REX_CLIENT` names one.
    pub fn show(&self, session: &str) -> Result<()> {
        self.run(
            &["do", "session.select", &format!("session_id={session}")],
            None,
        )?;
        Ok(())
    }
}

/// The sessions in `rex ls --json` output.
pub fn parse_sessions(out: &str) -> Result<Vec<Session>> {
    #[derive(Deserialize)]
    struct List {
        #[serde(default)]
        sessions: Vec<Session>,
    }
    Ok(serde_json::from_str::<List>(out)
        .context("unexpected output from `rex ls`")?
        .sessions)
}

fn parse_reply<T: serde::de::DeserializeOwned>(out: &str, method: &str) -> Result<T> {
    serde_json::from_str(out).map_err(|e| anyhow!("unexpected output from {method}: {e}"))
}

/// A `session.new_window` request that doesn't switch to the new window.
fn unfocused(window: &Value) -> Value {
    match window {
        Value::Object(fields) => Value::Object(
            fields
                .clone()
                .into_iter()
                .chain([("focus".to_string(), json!(false))])
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_replies() {
        let sessions = parse_sessions(
            r#"{"sessions": [{"session_id": "s1", "label": "a"}, {"session_id": "s2"}]}"#,
        )
        .unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(
            (sessions[0].label.as_str(), sessions[1].label.as_str()),
            ("a", "")
        );
        assert!(parse_sessions("nope").is_err());

        let created: Created = parse_reply(
            r#"{"session_id": "s", "initial_windows": [{"window_id": "w", "block_ids": ["b1", "b2"]}]}"#,
            "session.create",
        )
        .unwrap();
        assert_eq!(created.initial_windows[0].block_ids, ["b1", "b2"]);
    }

    #[test]
    fn new_windows_are_not_focused() {
        assert_eq!(
            unfocused(&json!({"window_label": "w"})),
            json!({"window_label": "w", "focus": false})
        );
    }
}
