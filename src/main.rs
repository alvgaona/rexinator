//! rexinator: start Rex sessions from project files, like tmuxinator.
//!
//! Built as a functional core with an imperative shell:
//!
//! - Data: [`model`] (projects, windows, panes), [`env::Env`] (the
//!   environment, read once), [`state::Started`] (sessions it started).
//! - Pure core: [`yaml`], [`vars`], [`when`], [`project`] and [`layout`]
//!   turn files into a [`model::Project`] and Rex requests; [`discovery`]
//!   finds project files; [`plan`] decides what each command does; [`report`]
//!   renders output; [`complete`] answers shell completion. They read files
//!   only through the [`fs::Fs`] they're given.
//! - Shell: [`commands`] and [`rex`] run programs, write files and print.

mod cli;
mod commands;
mod complete;
mod discovery;
mod env;
mod fs;
mod layout;
mod model;
mod plan;
mod project;
mod report;
mod rex;
mod state;
mod style;
mod template;
mod vars;
mod when;
mod yaml;

use anstream::{eprintln, print};
use anyhow::Result;
use clap::{CommandFactory, FromArgMatches};

use cli::{Cli, Cmd};
use commands::World;
use style::Color;

fn main() {
    // Answers the shell's completion requests (`COMPLETE=zsh rexinator ...`) and exits.
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    // Resolved from the raw arguments so help and parse errors are already
    // colorized (or not) by the time clap renders them.
    let color = Color::from_raw_args();
    color.apply();
    let cli = Cli::from_arg_matches(&Cli::command().color(color.into()).get_matches())
        .unwrap_or_else(|e| e.exit());

    if let Err(e) = run(cli) {
        eprintln!("{}", report::error(&format!("{e:#}")));
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let world = World {
        env: env::Env::from_process()?,
        fs: fs::Fs::real(),
        server: cli.server,
    };
    match cli.cmd {
        Cmd::Start {
            here: true, target, ..
        } => commands::start_here(&world, &commands::load(&world, &target)?.project),
        Cmd::Start {
            no_show, target, ..
        } => commands::start(&world, &commands::load(&world, &target)?, !no_show),
        Cmd::Restart { no_show, target } => {
            commands::restart(&world, &commands::load(&world, &target)?, !no_show)
        }
        Cmd::Check { target } => commands::check(&world, &target),
        Cmd::Stop { all: true, target } => {
            if !target.args.is_empty() || target.name.is_some() {
                anyhow::bail!("--all doesn't take a project");
            }
            commands::stop_all(&world)
        }
        Cmd::Stop { all: false, target } => {
            commands::stop_one(&world, &commands::load(&world, &target)?.project)
        }
        Cmd::Debug { target } => commands::debug(&commands::load(&world, &target)?),
        Cmd::List => commands::list(&world),
        Cmd::Edit { name, here } => commands::edit(&world, name.as_deref(), here),
        Cmd::Copy { from, to } => commands::copy(&world, &from, &to),
        Cmd::Delete { name, yes } => commands::delete(&world, &name, yes),
        Cmd::Completions { shell } => {
            print!("{}", complete::registration(shell.name())?);
            Ok(())
        }
    }
}
