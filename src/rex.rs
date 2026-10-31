//! Talking to Rex by running the `rex` command.

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Stdio};

pub struct Rex {
    bin: String,
    /// Passed as `rex -S`. None uses the server `rex` finds by itself.
    server: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Session {
    pub session_id: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Deserialize)]
pub struct Created {
    pub session_id: String,
    #[serde(default)]
    pub initial_windows: Vec<CreatedWindow>,
}

#[derive(Debug, Deserialize)]
pub struct CreatedWindow {
    pub window_id: String,
    /// In layout order, the same order as `Node::panes`.
    #[serde(default)]
    pub block_ids: Vec<String>,
}

impl Rex {
    /// Uses `rex` from PATH, or the program named by `REXINATOR_REX`.
    pub fn new(server: Option<String>) -> Self {
        Rex {
            bin: std::env::var("REXINATOR_REX").unwrap_or_else(|_| "rex".into()),
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
        #[derive(Deserialize)]
        struct List {
            #[serde(default)]
            sessions: Vec<Session>,
        }
        let out = self.run(&["ls", "--json"], None)?;
        Ok(serde_json::from_str::<List>(&out)
            .context("unexpected output from `rex ls`")?
            .sessions)
    }

    pub fn find_session(&self, label: &str) -> Result<Option<String>> {
        Ok(self
            .sessions()?
            .into_iter()
            .find(|s| s.label == label)
            .map(|s| s.session_id))
    }

    pub fn create_session(&self, request: &Value) -> Result<Created> {
        let out = self.run(
            &["api", "call", "session.create", "-"],
            Some(&request.to_string()),
        )?;
        serde_json::from_str(&out)
            .map_err(|e| anyhow!("unexpected output from session.create: {e}"))
    }

    /// Add a window to an existing session without switching to it.
    pub fn new_window(&self, session: &str, window: &Value) -> Result<CreatedWindow> {
        let mut request = window.clone();
        request["focus"] = json!(false);
        let out = self.run(
            &["api", "call", "-s", session, "session.new_window", "-"],
            Some(&request.to_string()),
        )?;
        serde_json::from_str(&out)
            .map_err(|e| anyhow!("unexpected output from session.new_window: {e}"))
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
