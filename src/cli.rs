//! The command line: what clap parses, and splitting `[PROJECT] [@LAYOUT]
//! [KEY=VALUE]...` into a request.

use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::ArgValueCandidates;

use crate::complete;
use crate::project::Request;
use crate::style::{self, Color};

/// Start Rex sessions from project files, like tmuxinator.
///
/// Project files live in a project's .rexinator.yml or .rexinator/NAME.yml,
/// or in ~/.config/rexinator/NAME.yml (or $REXINATOR_CONFIG). With no project
/// name, the nearest project folder's .rexinator.yml is used.
#[derive(Parser)]
#[command(name = "rexinator", version, styles = style::HELP)]
pub struct Cli {
    /// Rex server to use, as `rex -S` takes it. Overrides the project's `server`.
    #[arg(short = 'S', long, global = true)]
    pub server: Option<String>,
    /// When to colorize output
    #[arg(long, global = true, value_name = "WHEN", default_value = "auto")]
    pub color: Color,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(clap::Args)]
pub struct Target {
    /// Use this session name instead of the project's
    #[arg(long)]
    pub name: Option<String>,
    // In `help` rather than a doc comment, so rustdoc doesn't read the brackets as links.
    #[arg(
        value_name = "ARGS",
        help = "[PROJECT] [@LAYOUT] [KEY=VALUE]... PROJECT is a name, a file, or a project folder",
        add = ArgValueCandidates::new(complete::targets)
    )]
    pub args: Vec<String>,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Start a project's session, or switch to it if it's already running
    #[command(alias = "s")]
    Start {
        /// Don't switch the Rex app to the session
        #[arg(long)]
        no_show: bool,
        /// Add the project's windows to the Rex session you're in, instead of a new session
        #[arg(long)]
        here: bool,
        #[command(flatten)]
        target: Target,
    },
    /// Stop a project's session if it's running, then start it again
    ///
    /// Picks up changes to the project file. Runs on_stop, then the start hooks.
    Restart {
        /// Don't switch the Rex app to the session
        #[arg(long)]
        no_show: bool,
        #[command(flatten)]
        target: Target,
    },
    /// Close a project's session and run its on_stop commands
    Stop {
        /// Stop every project that's running
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        target: Target,
    },
    /// Check project files for mistakes, without starting anything
    ///
    /// With no project, checks every project here and in the config folder.
    /// Each layout is checked unless you pick one with @LAYOUT. Exits with 1
    /// if any file has errors; missing folders are only warnings.
    Check {
        #[command(flatten)]
        target: Target,
    },
    /// Print the request `start` would send to Rex, without sending it
    Debug {
        #[command(flatten)]
        target: Target,
    },
    /// List projects and whether they're running
    #[command(alias = "ls")]
    List,
    /// Open a project file in $EDITOR, creating it from a template first if needed
    ///
    /// With no NAME, uses this project's .rexinator.yml. A NAME is looked up in
    /// this project's .rexinator/ folder, then the config folder; a new one
    /// goes in the config folder unless you pass --here.
    #[command(alias = "new", alias = "open")]
    Edit {
        #[arg(add = ArgValueCandidates::new(complete::projects))]
        name: Option<String>,
        /// Create it in this project: ./.rexinator.yml, or .rexinator/NAME.yml with a NAME
        #[arg(long)]
        here: bool,
    },
    /// Copy a project file to a new name
    #[command(alias = "cp")]
    Copy {
        #[arg(add = ArgValueCandidates::new(complete::projects))]
        from: String,
        to: String,
    },
    /// Delete a project file
    #[command(alias = "rm")]
    Delete {
        #[arg(add = ArgValueCandidates::new(complete::projects))]
        name: String,
        /// Don't ask first
        #[arg(short, long)]
        yes: bool,
    },
    /// Print a shell completion script, e.g. `eval "$(rexinator completions zsh)"`
    ///
    /// Load it when the shell starts rather than saving it to a file, so it
    /// stays in step with rexinator after upgrades.
    Completions { shell: Shell },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Elvish,
    Powershell,
}

impl Shell {
    pub fn name(self) -> &'static str {
        match self {
            Shell::Bash => "bash",
            Shell::Zsh => "zsh",
            Shell::Fish => "fish",
            Shell::Elvish => "elvish",
            Shell::Powershell => "powershell",
        }
    }
}

/// The PROJECT argument, and the layout and variables asked for.
pub fn parse_target(args: &[String]) -> Result<(Option<String>, Request)> {
    args.iter()
        .try_fold((None, Request::default()), |(project, request), arg| {
            if let Some(layout) = arg.strip_prefix('@') {
                if layout.is_empty() {
                    bail!("`@` needs a layout name, like @focus");
                }
                if request.layout.is_some() {
                    bail!("give only one @LAYOUT");
                }
                let request = Request {
                    layout: Some(layout.to_string()),
                    ..request
                };
                return Ok((project, request));
            }
            match arg.split_once('=') {
                Some((k, v)) if !k.is_empty() && !k.contains('/') => {
                    let vars = request
                        .vars
                        .into_iter()
                        .chain([(k.to_string(), v.to_string())])
                        .collect();
                    Ok((project, Request { vars, ..request }))
                }
                _ if project.is_none() => Ok((Some(arg.clone()), request)),
                _ => bail!("expected KEY=VALUE or @LAYOUT, got `{arg}`"),
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn parse(args: &[&str]) -> Result<(Option<String>, Request)> {
        parse_target(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn splits_project_layout_and_vars() {
        let (project, request) = parse(&["shop", "env=prod", "@focus", "url=a=b"]).unwrap();
        assert_eq!(project.as_deref(), Some("shop"));
        assert_eq!(request.layout.as_deref(), Some("focus"));
        assert_eq!(
            request.vars,
            HashMap::from([("env".into(), "prod".into()), ("url".into(), "a=b".into())])
        );
        // A path with `=` in it is a project, not a variable.
        let (project, _) = parse(&["./a=b/x.yml"]).unwrap();
        assert_eq!(project.as_deref(), Some("./a=b/x.yml"));
        assert_eq!(parse(&[]).unwrap(), (None, Request::default()));
    }

    #[test]
    fn rejects_bad_arguments() {
        assert!(parse(&["@"]).is_err());
        assert!(parse(&["@a", "@b"]).is_err());
        assert!(parse(&["one", "two"]).is_err());
    }

    #[test]
    fn cli_is_well_formed() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
