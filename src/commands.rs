//! The imperative shell: each command gathers facts (files, Rex sessions,
//! the state file), asks the pure core what to do, and does it. This is the
//! only module that runs hooks, writes files or prints.

use anstream::{eprint, eprintln, print, println};
use anyhow::{Context as _, Result, anyhow, bail};
use std::collections::{BTreeSet, HashMap};
use std::io::IsTerminal;
use std::path::Path;
use std::process::Command;

use crate::cli::Target;
use crate::discovery;
use crate::env::Env;
use crate::fs::Fs;
use crate::layout;
use crate::model::Project;
use crate::plan::{self, Focus, Loaded, Start};
use crate::project::{self, Request};
use crate::report;
use crate::rex::{Rex, Session};
use crate::state;
use crate::style::{BOLD, DIM, GREEN, RESET, YELLOW};
use crate::template;

/// Everything a command works from.
pub struct World {
    pub env: Env,
    pub fs: Fs,
    /// `--server` from the command line.
    pub server: Option<String>,
}

impl World {
    fn rex(&self, server: Option<String>) -> Rex {
        Rex::new(&self.env.rex_bin, server)
    }

    fn load(&self, path: &Path, request: &Request) -> Result<Project> {
        project::load(&self.fs, &self.env, path, request)
    }

    /// Running sessions on each of these servers, fetched once per server.
    /// A server that can't be reached has none.
    fn live(&self, servers: BTreeSet<Option<String>>) -> HashMap<Option<String>, Vec<Session>> {
        servers
            .into_iter()
            .map(|s| (s.clone(), self.rex(s).sessions().unwrap_or_default()))
            .collect()
    }

    fn update_state(
        &self,
        change: impl Fn(&[state::Started]) -> Vec<state::Started>,
    ) -> Result<()> {
        state::update(&self.fs, &self.env.state_file, change)
    }
}

/// Find the project file a target names, and load it.
pub fn load(world: &World, target: &Target) -> Result<Loaded> {
    let (name, request) = crate::cli::parse_target(&target.args)?;
    let path = discovery::find_project(&world.fs, &world.env, name.as_deref())?;
    let project = world.load(&path, &request)?;
    Ok(Loaded {
        project: plan::with_overrides(project, target.name.as_deref(), world.server.as_deref()),
        path,
        request,
    })
}

pub fn start(world: &World, loaded: &Loaded, show: bool) -> Result<()> {
    let project = &loaded.project;
    let rex = world.rex(project.server.clone());
    run_hooks(&project.hooks.on_start, &project.root, "on_start")?;

    match plan::start(project, &rex.sessions()?) {
        Start::Switch(id) => {
            println!("{}", report::already_running(&project.name));
            run_hooks(&project.hooks.on_restart, &project.root, "on_restart")?;
            if show {
                show_session(&rex, &id);
            }
        }
        Start::Create(request) => {
            if rex.is_local() {
                check_dirs(world, project)?;
            }
            run_hooks(
                &project.hooks.on_first_start,
                &project.root,
                "on_first_start",
            )?;
            let created = rex.create_session(&request)?;
            apply_focus(
                &rex,
                &created.session_id,
                plan::focus(project, &created.initial_windows),
            );
            let record = plan::record(loaded, &created.session_id);
            if let Err(e) = world.update_state(|all| state::with(all, record.clone())) {
                warn(&format!(
                    "couldn't record the session for `list` and `stop --all`: {e:#}"
                ));
            }
            println!("{}", report::started(&project.name, project.windows.len()));
            if show {
                show_session(&rex, &created.session_id);
            }
        }
    }
    Ok(())
}

/// Add the project's windows to the Rex session this terminal is in.
pub fn start_here(world: &World, project: &Project) -> Result<()> {
    if project.server.is_some() {
        bail!("--here adds to the session you're in, so it can't use another server");
    }
    let session =
        world.env.rex_session.as_deref().ok_or_else(|| {
            anyhow!("--here only works in a Rex terminal (REX_SESSION isn't set)")
        })?;
    let rex = world.rex(None);
    check_dirs(world, project)?;
    run_hooks(&project.hooks.on_start, &project.root, "on_start")?;
    run_hooks(
        &project.hooks.on_first_start,
        &project.root,
        "on_first_start",
    )?;

    let created = project
        .windows
        .iter()
        .map(|w| {
            rex.new_window(session, &layout::window_request(w))
                .with_context(|| format!("couldn't add window `{}`", w.name))
        })
        .collect::<Result<Vec<_>>>()?;
    apply_focus(&rex, session, plan::focus(project, &created));
    println!("{}", report::added(&project.name, created.len()));
    Ok(())
}

fn check_dirs(world: &World, project: &Project) -> Result<()> {
    let missing = plan::missing_dirs(project, |d| (world.fs.is_dir)(d));
    if missing.is_empty() {
        Ok(())
    } else {
        bail!("these directories don't exist: {}", missing.join(", "))
    }
}

fn apply_focus(rex: &Rex, session: &str, focus: Option<Focus>) {
    let result = match focus {
        None => return,
        Some(Focus::Block(block)) => rex.focus_block(session, &block),
        Some(Focus::Window(window)) => rex.focus_window(session, &window),
    };
    if let Err(e) = result {
        warn(&format!("couldn't switch to the startup window: {e:#}"));
    }
}

pub fn stop_one(world: &World, project: &Project) -> Result<()> {
    if !stop(world, project)? {
        println!("{}", report::not_running(&project.name));
    }
    Ok(())
}

/// Stop the project's session. Returns whether it was running.
fn stop(world: &World, project: &Project) -> Result<bool> {
    let rex = world.rex(project.server.clone());
    let Some(session) = plan::session_for(&rex.sessions()?, &project.name).cloned() else {
        return Ok(false);
    };
    rex.kill(&session.session_id)?;
    let _ = world.update_state(|all| state::without(all, &session.session_id));
    run_hooks(&project.hooks.on_stop, &project.root, "on_stop")?;
    println!("{}", report::stopped(&project.name));
    Ok(true)
}

pub fn stop_all(world: &World) -> Result<()> {
    let cli = world.server.as_deref();
    let server_of = |r: &state::Started| plan::server(cli, r.server.as_deref());

    // Sessions rexinator started, each with the variables it was started with.
    let records = state::load(&world.fs, &world.env.state_file);
    let live = world.live(records.iter().map(server_of).collect());
    let alive: Vec<state::Started> = plan::recorded_alive(&records, |r| {
        live.get(&server_of(r)).map_or(&[][..], Vec::as_slice)
    })
    .into_iter()
    .cloned()
    .collect();
    let _ = world.update_state(|_| alive.clone());
    let recorded = alive.iter().try_fold(0, |n, s| -> Result<usize> {
        world.rex(server_of(s)).kill(&s.session_id)?;
        let _ = world.update_state(|all| state::without(all, &s.session_id));
        let request = Request {
            vars: s.vars.clone(),
            layout: s.layout.clone(),
        };
        match world.load(&s.project, &request) {
            Ok(p) => run_hooks(&p.hooks.on_stop, &p.root, "on_stop")?,
            Err(e) => warn(&format!(
                "{} has no on_stop commands to run: {e:#}",
                s.label
            )),
        }
        println!("{}", report::stopped(&s.label));
        Ok(n + 1)
    })?;

    // Then any project whose default session is running, such as one started
    // by hand or before rexinator kept records.
    let defaults = discovery::all_projects(&world.fs, &world.env)
        .into_iter()
        .try_fold(0, |n, (name, path)| -> Result<usize> {
            match world.load(&path, &Request::default()) {
                Ok(p) => {
                    let p = plan::with_overrides(p, None, cli);
                    Ok(n + usize::from(stop(world, &p)?))
                }
                Err(e) => {
                    warn(&format!("skipping {name}: {e:#}"));
                    Ok(n)
                }
            }
        })?;

    if recorded + defaults == 0 {
        println!("{}", report::nothing_running());
    }
    Ok(())
}

/// Stop the session if it's running, then start it.
pub fn restart(world: &World, loaded: &Loaded, show: bool) -> Result<()> {
    stop(world, &loaded.project)?;
    start(world, loaded, show)
}

/// Check one project (with the target's variables and layout), or every
/// project when the target names none.
pub fn check(world: &World, target: &Target) -> Result<()> {
    let (name, request) = crate::cli::parse_target(&target.args)?;
    let files = match &name {
        None if request == Request::default() => discovery::all_projects(&world.fs, &world.env),
        _ => {
            let path = discovery::find_project(&world.fs, &world.env, name.as_deref())?;
            vec![(name.unwrap_or_else(|| "(here)".to_string()), path)]
        }
    };
    if files.is_empty() {
        let dir = world.env.config_dir.display().to_string();
        println!("{}", report::no_projects(&dir));
        return Ok(());
    }
    let results: Vec<plan::Checked> = files
        .iter()
        .map(|(name, path)| check_file(world, name, path, &request))
        .collect();
    results
        .iter()
        .for_each(|c| print!("{}", report::checked(c)));
    match results.iter().filter(|c| !c.errors.is_empty()).count() {
        0 => {
            println!("{}", report::check_summary(results.len()));
            Ok(())
        }
        failed => bail!("{} of {} have errors", failed, results.len()),
    }
}

/// Load a project file once per layout and judge the results.
fn check_file(world: &World, name: &str, path: &Path, request: &Request) -> plan::Checked {
    let first = world.load(path, request);
    let layouts: Vec<Option<String>> = match (&request.layout, &first) {
        (Some(l), _) => vec![Some(l.clone())],
        (None, Ok(p)) if !p.layouts.is_empty() => p.layouts.iter().cloned().map(Some).collect(),
        (None, _) => vec![None],
    };
    let loads: Vec<_> = layouts
        .into_iter()
        .map(|layout| {
            let request = Request {
                layout: layout.clone(),
                ..request.clone()
            };
            let result = world.load(path, &request).map_err(|e| format!("{e:#}"));
            (layout, result)
        })
        .collect();
    plan::check(name, &loads, |d| (world.fs.is_dir)(d))
}

pub fn debug(loaded: &Loaded) -> Result<()> {
    let request = layout::session_request(&loaded.project);
    println!("{}", serde_json::to_string_pretty(&request)?);
    Ok(())
}

pub fn list(world: &World) -> Result<()> {
    let projects: Vec<_> = discovery::all_projects(&world.fs, &world.env)
        .into_iter()
        .map(|(name, path)| {
            let project = world.load(&path, &Request::default()).ok();
            (name, path, project)
        })
        .collect();
    if projects.is_empty() {
        let dir = world.env.config_dir.display().to_string();
        println!("{}", report::no_projects(&dir));
        return Ok(());
    }

    let cli = world.server.as_deref();
    let server_of = |p: &Project| plan::server(cli, p.server.as_deref());
    let live = world.live(
        projects
            .iter()
            .filter_map(|(_, _, p)| p.as_ref().map(server_of))
            .collect(),
    );
    let records = state::load(&world.fs, &world.env.state_file);
    let rows: Vec<[String; 4]> = projects
        .iter()
        .map(|(name, path, project)| match project {
            Some(p) => {
                let on_server = live.get(&server_of(p)).map_or(&[][..], Vec::as_slice);
                let running = plan::running(p, path, on_server, &records);
                plan::list_row(name, Some((p, &running)))
            }
            None => plan::list_row(name, None),
        })
        .collect();
    print!("{}", report::table(&rows));
    Ok(())
}

/// Open a project file in the editor, creating it from a template first if needed.
pub fn edit(world: &World, name: Option<&str>, here: bool) -> Result<()> {
    let (path, kind) = discovery::edit_target(&world.fs, &world.env, name, here);
    if !(world.fs.is_file)(&path) {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("can't create {}", dir.display()))?;
        }
        std::fs::write(&path, template::template(kind))?;
        println!("{GREEN}Created{RESET} {}", path.display());
    }
    let editor = &world.env.editor;
    // Through sh so an editor setting with flags, such as `code -w`, works.
    let status = Command::new("sh")
        .args(["-c", &format!("{editor} \"$1\""), "sh"])
        .arg(&path)
        .status()
        .with_context(|| format!("couldn't start editor `{editor}`"))?;
    if !status.success() {
        bail!("editor exited with {status}");
    }
    Ok(())
}

pub fn copy(world: &World, from: &str, to: &str) -> Result<()> {
    let src = discovery::find_project(&world.fs, &world.env, Some(from))?;
    if discovery::config_file(&world.fs, &world.env, to).is_some() {
        bail!("project `{to}` already exists");
    }
    let dir = &world.env.config_dir;
    let dst = dir.join(format!("{to}.yml"));
    std::fs::create_dir_all(dir)?;
    std::fs::copy(&src, &dst).with_context(|| format!("can't copy to {}", dst.display()))?;
    println!(
        "{GREEN}Copied{RESET} {} to {}",
        src.display(),
        dst.display()
    );
    Ok(())
}

pub fn delete(world: &World, name: &str, yes: bool) -> Result<()> {
    let path = discovery::config_file(&world.fs, &world.env, name)
        .ok_or_else(|| anyhow!("no project `{name}` in {}", world.env.config_dir.display()))?;
    if !yes && !confirm(&format!("Delete {BOLD}{}{RESET}?", path.display()))? {
        println!("Kept it");
        return Ok(());
    }
    std::fs::remove_file(&path)?;
    println!("{YELLOW}Deleted{RESET} {}", path.display());
    let overlay = project::overlay_path(&path);
    if (world.fs.is_file)(&overlay) {
        println!(
            "{DIM}(left its overrides file in place: {}){RESET}",
            overlay.display()
        );
    }
    Ok(())
}

/// Ask a yes/no question on the terminal. No terminal is an error, not a no.
fn confirm(question: &str) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("pass --yes to delete without asking");
    }
    eprint!("{question} [y/N] ");
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

fn show_session(rex: &Rex, id: &str) {
    if let Err(e) = rex.show(id) {
        warn(&format!(
            "couldn't switch the Rex app to the session: {e:#}"
        ));
        eprintln!(
            "  {DIM}(with several Rex apps connected, set REX_CLIENT to the one to use){RESET}"
        );
    }
}

fn run_hooks(cmds: &[String], root: &Path, what: &str) -> Result<()> {
    cmds.iter().try_for_each(|cmd| {
        let status = Command::new("sh")
            .args(["-c", cmd])
            .current_dir(root)
            .status()
            .with_context(|| format!("couldn't run {what} command `{cmd}`"))?;
        if status.success() {
            Ok(())
        } else {
            bail!("{what} command `{cmd}` failed ({status})")
        }
    })
}

/// Print a warning that doesn't stop the command.
fn warn(msg: &str) {
    eprintln!("{}", report::warning(msg));
}
