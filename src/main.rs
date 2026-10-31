mod layout;
mod project;
mod rex;
mod state;
mod style;

use anstream::{eprint, eprintln, print, println};
use anyhow::{Context, Result, anyhow, bail};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

use project::{LOCAL_DIR, LOCAL_NAMES, Project};
use rex::{CreatedWindow, Rex};
use style::{BOLD, CYAN, Color, DIM, GREEN, RED, RESET, YELLOW};

/// Start Rex sessions from project files, like tmuxinator.
///
/// Project files live in a project's .rexinator.yml or .rexinator/NAME.yml,
/// or in ~/.config/rexinator/NAME.yml (or $REXINATOR_CONFIG). With no project
/// name, the nearest project folder's .rexinator.yml is used.
#[derive(Parser)]
#[command(name = "rexinator", version, styles = style::HELP)]
struct Cli {
    /// Rex server to use, as `rex -S` takes it. Overrides the project's `server`.
    #[arg(short = 'S', long, global = true)]
    server: Option<String>,
    /// When to colorize output
    #[arg(long, global = true, value_name = "WHEN", default_value = "auto")]
    color: Color,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args)]
struct Target {
    /// Use this session name instead of the project's
    #[arg(long)]
    name: Option<String>,
    // In `help` rather than a doc comment, so rustdoc doesn't read the brackets as links.
    #[arg(
        value_name = "ARGS",
        help = "[PROJECT] [@LAYOUT] [KEY=VALUE]... PROJECT is a name, a file, or a project folder"
    )]
    args: Vec<String>,
}

#[derive(Subcommand)]
enum Cmd {
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
    /// Close a project's session and run its on_stop commands
    Stop {
        /// Stop every project that's running
        #[arg(long)]
        all: bool,
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
    List {
        /// Print only project names, for shell completion
        #[arg(long, hide = true)]
        names: bool,
    },
    /// Open a project file in $EDITOR, creating it from a template first if needed
    ///
    /// With no NAME, uses this project's .rexinator.yml. A NAME is looked up in
    /// this project's .rexinator/ folder, then the config folder; a new one
    /// goes in the config folder unless you pass --here.
    #[command(alias = "new", alias = "open")]
    Edit {
        name: Option<String>,
        /// Create it in this project: ./.rexinator.yml, or .rexinator/NAME.yml with a NAME
        #[arg(long)]
        here: bool,
    },
    /// Copy a project file to a new name
    #[command(alias = "cp")]
    Copy { from: String, to: String },
    /// Delete a project file
    #[command(alias = "rm")]
    Delete {
        name: String,
        /// Don't ask first
        #[arg(short, long)]
        yes: bool,
    },
    /// Print a shell completion script, e.g. `eval "$(rexinator completions zsh)"`
    Completions { shell: Shell },
}

#[derive(Clone, Copy, ValueEnum)]
enum Shell {
    Bash,
    Zsh,
    Fish,
}

fn main() {
    // Resolved from the raw arguments so help and parse errors are already
    // colorized (or not) by the time clap renders them.
    let color = Color::from_raw_args();
    color.apply();
    let cli = Cli::from_arg_matches(&Cli::command().color(color.into()).get_matches())
        .unwrap_or_else(|e| e.exit());

    if let Err(e) = run(cli) {
        eprintln!("{BOLD}{RED}error:{RESET} {e:#}");
        std::process::exit(1);
    }
}

/// Print a warning that doesn't stop the command.
fn warn(msg: impl std::fmt::Display) {
    eprintln!("{BOLD}{YELLOW}warning:{RESET} {msg}");
}

fn run(cli: Cli) -> Result<()> {
    let server = cli.server;
    match cli.cmd {
        Cmd::Start {
            no_show,
            here,
            target,
        } => {
            let loaded = load(&target, &server)?;
            if here {
                if loaded.project.server.is_some() {
                    bail!("--here adds to the session you're in, so it can't use another server");
                }
                start_here(&Rex::new(None), &loaded.project)
            } else {
                start(&Rex::new(loaded.project.server.clone()), &loaded, !no_show)
            }
        }
        Cmd::Stop { all: true, target } => {
            if !target.args.is_empty() || target.name.is_some() {
                bail!("--all doesn't take a project");
            }
            stop_all(&server)
        }
        Cmd::Stop { all: false, target } => {
            let project = load(&target, &server)?.project;
            if !stop(&Rex::new(project.server.clone()), &project)? {
                println!("{BOLD}{}{RESET} isn't running", project.name);
            }
            Ok(())
        }
        Cmd::Debug { target } => {
            let project = load(&target, &server)?.project;
            println!(
                "{}",
                serde_json::to_string_pretty(&layout::session_request(&project))?
            );
            Ok(())
        }
        Cmd::List { names: true } => {
            for name in project_names() {
                println!("{name}");
            }
            Ok(())
        }
        Cmd::List { names: false } => list(&server),
        Cmd::Edit { name, here } => edit(name.as_deref(), here),
        Cmd::Copy { from, to } => copy(&from, &to),
        Cmd::Delete { name, yes } => delete(&name, yes),
        Cmd::Completions { shell } => {
            print!("{}", completions(shell));
            Ok(())
        }
    }
}

fn start(rex: &Rex, loaded: &Loaded, show: bool) -> Result<()> {
    let project = &loaded.project;
    run_hooks(&project.on_start, &project.root, "on_start")?;

    if let Some(id) = rex.find_session(&project.name)? {
        println!("{BOLD}{}{RESET} is already running", project.name);
        run_hooks(&project.on_restart, &project.root, "on_restart")?;
        if show {
            show_session(rex, &id);
        }
        return Ok(());
    }

    if rex.is_local() {
        check_dirs(project)?;
    }
    run_hooks(&project.on_first_start, &project.root, "on_first_start")?;
    let created = rex.create_session(&layout::session_request(project))?;
    focus_startup(rex, &created.session_id, project, &created.initial_windows);
    let record = state::Started {
        session_id: created.session_id.clone(),
        label: project.name.clone(),
        project: loaded.path.clone(),
        vars: loaded.vars.clone(),
        server: project.server.clone(),
        layout: project.layout.clone(),
    };
    if let Err(e) = state::record(record) {
        warn(format!(
            "couldn't record the session for `list` and `stop --all`: {e:#}"
        ));
    }

    let n = project.windows.len();
    println!(
        "{GREEN}Started{RESET} {BOLD}{}{RESET} {DIM}({n} window{}){RESET}",
        project.name,
        if n == 1 { "" } else { "s" }
    );
    if show {
        show_session(rex, &created.session_id);
    }
    Ok(())
}

/// Add the project's windows to the Rex session this terminal is in.
fn start_here(rex: &Rex, project: &Project) -> Result<()> {
    let session = std::env::var("REX_SESSION")
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("--here only works in a Rex terminal (REX_SESSION isn't set)"))?;
    check_dirs(project)?;
    run_hooks(&project.on_start, &project.root, "on_start")?;
    run_hooks(&project.on_first_start, &project.root, "on_first_start")?;

    let mut created = Vec::new();
    for w in &project.windows {
        created.push(
            rex.new_window(&session, &layout::window_request(w))
                .with_context(|| format!("couldn't add window `{}`", w.name))?,
        );
    }
    focus_startup(rex, &session, project, &created);
    let n = created.len();
    println!(
        "{GREEN}Added{RESET} {n} window{} from {BOLD}{}{RESET} to this session",
        if n == 1 { "" } else { "s" },
        project.name
    );
    Ok(())
}

/// Switch to the startup window, and to the startup pane if there is one.
fn focus_startup(rex: &Rex, session: &str, project: &Project, windows: &[CreatedWindow]) {
    let Some(w) = windows.get(project.startup_window) else {
        return;
    };
    let result = match project.startup_pane.and_then(|i| w.block_ids.get(i)) {
        Some(block) => rex.focus_block(session, block),
        None => rex.focus_window(session, &w.window_id),
    };
    if let Err(e) = result {
        warn(format!("couldn't switch to the startup window: {e:#}"));
    }
}

/// Rex silently falls back to $HOME for a missing directory, so check first.
fn check_dirs(project: &Project) -> Result<()> {
    let mut missing: Vec<String> = std::iter::once(project.root.as_path())
        .chain(
            project
                .windows
                .iter()
                .flat_map(|w| w.layout.panes())
                .map(|p| p.cwd.as_path()),
        )
        .filter(|d| !d.is_dir())
        .map(|d| d.display().to_string())
        .collect();
    missing.sort();
    missing.dedup();
    if !missing.is_empty() {
        bail!("these directories don't exist: {}", missing.join(", "));
    }
    Ok(())
}

/// Returns whether the project was running.
fn stop(rex: &Rex, project: &Project) -> Result<bool> {
    let Some(id) = rex.find_session(&project.name)? else {
        return Ok(false);
    };
    rex.kill(&id)?;
    let _ = state::forget(&id);
    run_hooks(&project.on_stop, &project.root, "on_stop")?;
    println!("{YELLOW}Stopped{RESET} {BOLD}{}{RESET}", project.name);
    Ok(true)
}

fn stop_all(server: &Option<String>) -> Result<()> {
    let mut stopped = 0;

    // Sessions rexinator started, each with the variables it was started with.
    for s in state::load() {
        let rex = Rex::new(server.clone().or(s.server.clone()));
        let alive = rex
            .sessions()
            .is_ok_and(|all| all.iter().any(|x| x.session_id == s.session_id));
        let _ = state::forget(&s.session_id);
        if !alive {
            continue;
        }
        rex.kill(&s.session_id)?;
        match project::load(&s.project, &s.vars, s.layout.as_deref()) {
            Ok(p) => run_hooks(&p.on_stop, &p.root, "on_stop")?,
            Err(e) => warn(format!("{} has no on_stop commands to run: {e:#}", s.label)),
        }
        println!("{YELLOW}Stopped{RESET} {BOLD}{}{RESET}", s.label);
        stopped += 1;
    }

    // Then any project whose default session is running, such as one started
    // by hand or before rexinator kept records.
    for (name, path) in all_projects() {
        let project = match project::load(&path, &HashMap::new(), None) {
            Ok(mut p) => {
                p.server = server.clone().or(p.server);
                p
            }
            Err(e) => {
                warn(format!("skipping {name}: {e:#}"));
                continue;
            }
        };
        if stop(&Rex::new(project.server.clone()), &project)? {
            stopped += 1;
        }
    }
    if stopped == 0 {
        println!("{DIM}No projects are running{RESET}");
    }
    Ok(())
}

fn show_session(rex: &Rex, id: &str) {
    if let Err(e) = rex.show(id) {
        warn(format!("couldn't switch the Rex app to the session: {e:#}"));
        eprintln!(
            "  {DIM}(with several Rex apps connected, set REX_CLIENT to the one to use){RESET}"
        );
    }
}

fn run_hooks(cmds: &[String], root: &Path, what: &str) -> Result<()> {
    for cmd in cmds {
        let status = Command::new("sh")
            .args(["-c", cmd])
            .current_dir(root)
            .status()
            .with_context(|| format!("couldn't run {what} command `{cmd}`"))?;
        if !status.success() {
            bail!("{what} command `{cmd}` failed ({status})");
        }
    }
    Ok(())
}

struct Loaded {
    project: Project,
    /// The project file, as an absolute path.
    path: PathBuf,
    /// The KEY=VALUE pairs from the command line.
    vars: HashMap<String, String>,
}

/// Split `[PROJECT] [@LAYOUT] [KEY=VALUE]...`, load the project, and apply --name and --server.
fn load(target: &Target, server: &Option<String>) -> Result<Loaded> {
    let mut project = None;
    let mut layout = None;
    let mut vars = HashMap::new();
    for a in &target.args {
        if let Some(l) = a.strip_prefix('@') {
            if l.is_empty() {
                bail!("`@` needs a layout name, like @focus");
            }
            if layout.replace(l.to_string()).is_some() {
                bail!("give only one @LAYOUT");
            }
            continue;
        }
        match a.split_once('=') {
            Some((k, v)) if !k.is_empty() && !k.contains('/') => {
                vars.insert(k.to_string(), v.to_string());
            }
            _ if project.is_none() => project = Some(a.as_str()),
            _ => bail!("expected KEY=VALUE or @LAYOUT, got `{a}`"),
        }
    }
    let path = std::path::absolute(find_project(project)?)?;
    let mut p = project::load(&path, &vars, layout.as_deref())?;
    if let Some(name) = &target.name {
        p.name = name.clone();
    }
    if server.is_some() {
        p.server = server.clone();
    }
    Ok(Loaded {
        project: p,
        path,
        vars,
    })
}

fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("REXINATOR_CONFIG") {
        return d.into();
    }
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d).join("rexinator"),
        _ => project::home().join(".config/rexinator"),
    }
}

/// The .rexinator.yml in `dir`, if it has one.
fn local_file_in(dir: &Path) -> Option<PathBuf> {
    LOCAL_NAMES
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}

/// `dir/NAME.yml` or `dir/NAME.yaml`, if it exists.
fn file_named(dir: &Path, name: &str) -> Option<PathBuf> {
    ["yml", "yaml"]
        .iter()
        .map(|ext| dir.join(format!("{name}.{ext}")))
        .find(|p| p.is_file())
}

/// A project file in the config folder, if there is one by that name.
fn config_file(name: &str) -> Option<PathBuf> {
    file_named(&config_dir(), name)
}

/// Project files in a folder, by name. `.local` override files and files
/// starting with `_` (bases for `extends`) aren't projects.
fn projects_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "yml" || e == "yaml"))
                .filter_map(|p| {
                    let stem = p.file_stem()?.to_str()?.to_string();
                    (!stem.ends_with(".local") && !stem.starts_with('_')).then_some((stem, p))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// The nearest folder, this one or a parent, with a .rexinator.yml or a .rexinator/ folder.
fn find_project_dir() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if local_file_in(&dir).is_some() || dir.join(LOCAL_DIR).is_dir() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// A folder's project: its .rexinator.yml, else .rexinator/default.yml, else
/// the only file in .rexinator/.
fn folder_project(dir: &Path) -> Result<PathBuf> {
    if let Some(p) = local_file_in(dir) {
        return Ok(p);
    }
    let files = projects_in(&dir.join(LOCAL_DIR));
    if let Some((_, p)) = files.iter().find(|(n, _)| n == "default") {
        return Ok(p.clone());
    }
    match files.as_slice() {
        [(_, only)] => Ok(only.clone()),
        [] => bail!("there's no .rexinator.yml in {}", dir.display()),
        many => {
            let names: Vec<&str> = many.iter().map(|(n, _)| n.as_str()).collect();
            bail!(
                "this project has several configs; pick one: {}",
                names.join(", ")
            )
        }
    }
}

/// A project file by name: this project's .rexinator/ folder first, then the config folder.
fn named_file(name: &str) -> Option<PathBuf> {
    find_project_dir()
        .and_then(|d| file_named(&d.join(LOCAL_DIR), name))
        .or_else(|| config_file(name))
}

fn find_project(name: Option<&str>) -> Result<PathBuf> {
    let Some(name) = name else {
        let dir = find_project_dir().ok_or_else(|| {
            anyhow!("no project given, and there's no .rexinator.yml or .rexinator/ folder here or in a parent folder")
        })?;
        return folder_project(&dir);
    };
    if name == "." || name.contains('/') || name.ends_with(".yml") || name.ends_with(".yaml") {
        let path = PathBuf::from(name);
        if path.is_dir() {
            return folder_project(&path);
        }
        return Ok(path);
    }
    named_file(name).ok_or_else(|| {
        anyhow!(
            "no project `{name}` in this project's .rexinator/ folder or in {} (create it with `rexinator new {name}`)",
            config_dir().display()
        )
    })
}

/// This project's files, then the config folder's, as `list` names them.
fn all_projects() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    if let Some(dir) = find_project_dir() {
        if let Some(p) = local_file_in(&dir) {
            out.push(("(here)".to_string(), p));
        }
        for (n, p) in projects_in(&dir.join(LOCAL_DIR)) {
            out.push((format!("{n} (here)"), p));
        }
    }
    out.extend(projects_in(&config_dir()));
    out
}

/// The names `start NAME` accepts, for shell completion.
fn project_names() -> Vec<String> {
    let mut names: Vec<String> = find_project_dir()
        .map(|d| projects_in(&d.join(LOCAL_DIR)))
        .unwrap_or_default()
        .into_iter()
        .chain(projects_in(&config_dir()))
        .map(|(n, _)| n)
        .collect();
    names.sort();
    names.dedup();
    names
}

fn list(server: &Option<String>) -> Result<()> {
    let projects = all_projects();
    if projects.is_empty() {
        println!(
            "No projects in {}. Create one with {BOLD}rexinator new NAME{RESET}.",
            config_dir().display()
        );
        return Ok(());
    }

    // Sessions, fetched once per server.
    let mut sessions: HashMap<Option<String>, Vec<rex::Session>> = HashMap::new();
    let records = state::load();
    let rows: Vec<[String; 4]> = projects
        .into_iter()
        .map(
            |(name, path)| match project::load(&path, &HashMap::new(), None) {
                Ok(p) => {
                    let srv = server.clone().or(p.server.clone());
                    let live = sessions
                        .entry(srv.clone())
                        .or_insert_with(|| Rex::new(srv.clone()).sessions().unwrap_or_default());

                    // The default session, plus any started with other variables or layouts.
                    let mut running: Vec<&str> = Vec::new();
                    if live.iter().any(|s| s.label == p.name) {
                        running.push(&p.name);
                    }
                    for r in state::for_project(&records, &path) {
                        if live.iter().any(|s| s.session_id == r.session_id)
                            && !running.contains(&r.label.as_str())
                        {
                            running.push(&r.label);
                        }
                    }
                    let mut status = match running.as_slice() {
                        [] => String::new(),
                        [only] if *only == p.name => "running".to_string(),
                        many => format!("running: {}", many.join(", ")),
                    };
                    if let Some(s) = &p.server {
                        status = format!("{status} (on {s})").trim().to_string();
                    }
                    // The default layout is marked with *.
                    let layouts: Vec<String> = p
                        .layouts
                        .iter()
                        .map(|l| {
                            if Some(l) == p.layout.as_ref() {
                                format!("{l}*")
                            } else {
                                l.clone()
                            }
                        })
                        .collect();
                    [name, p.name, layouts.join(" "), status]
                }
                Err(_) => [
                    name,
                    "-".into(),
                    String::new(),
                    "invalid (see `rexinator debug`)".into(),
                ],
            },
        )
        .collect();
    let header = ["PROJECT", "SESSION", "LAYOUTS", "STATUS"];
    let widths: Vec<usize> = (0..3)
        .map(|i| {
            rows.iter()
                .map(|r| r[i].len())
                .max()
                .unwrap_or(0)
                .max(header[i].len())
        })
        .collect();
    // Pad first, then color, so escape codes don't count toward the widths.
    let cell = |color: &str, text: &str, width: usize| format!("{color}{text:width$}{RESET}");
    println!(
        "{}  {}  {}  {BOLD}STATUS{RESET}",
        cell(BOLD, header[0], widths[0]),
        cell(BOLD, header[1], widths[1]),
        cell(BOLD, header[2], widths[2]),
    );
    for r in &rows {
        let status_color = if r[3].starts_with("running") {
            GREEN
        } else if r[3].starts_with("invalid") {
            RED
        } else {
            DIM
        };
        let colors = [CYAN, "", DIM, status_color];
        // Leave out empty cells at the end, so lines don't end in spaces.
        let used = r.iter().rposition(|c| !c.is_empty()).map_or(0, |i| i + 1);
        let cells: Vec<String> = (0..used)
            .map(|i| {
                let width = if i + 1 == used { 0 } else { widths[i] };
                cell(colors[i], &r[i], width)
            })
            .collect();
        println!("{}", cells.join("  "));
    }
    Ok(())
}

const TEMPLATE_BODY: &str = r#"
vars:                 # defaults, overridden with KEY=VALUE on the command line
  port: 3000

# pre_window: source .venv/bin/activate   # runs first in every pane
# on_start: git fetch                     # runs on every `start`
# on_first_start: docker compose up -d    # runs only when the session is created
# on_restart: echo welcome back           # runs only when it was already running
# on_stop: docker compose down            # runs after `rexinator stop`
# startup_window: editor                  # window to show first (name or number)
# startup_pane: 1                         # pane to focus in it (label or number)
# server: devbox                          # open on another Rex host (see `rex hosts`)

windows:
  # A window with one pane. The command is typed into a shell, which stays open.
  - editor: nvim .

  # Several panes arranged by a preset:
  # even-horizontal, even-vertical, main-left [SIZE%], main-top [SIZE%], tiled
  - dev:
      layout: main-left 65
      panes:
        - PORT=${port} npm run dev
        - git status
        -                          # an empty pane is just a shell

  # Or build the split tree yourself:
  # - monitor:
  #     pre: export LOG_LEVEL=debug  # runs first in this window's panes
  #     split: right                 # right = side by side, below = stacked
  #     ratio: 70                    # percent of the space for `before`
  #     before: { cwd: ./logs, type: tail -f app.log, label: logs }
  #     after: { run: htop }         # `run` makes the program the pane itself

  # Switch a window or pane on with a variable: rexinator start with_db=true
  # - db: { when: '${with_db}', type: psql }    # needs `with_db: false` in vars

# Several layouts in one file: replace `windows:` with
# default: full
# layouts:
#   full:
#     windows: [ ... ]
#   focus:                         # rexinator start @focus
#     windows:
#       - editor: nvim .
"#;

/// Where a new project file goes, which decides its template.
enum Kind<'a> {
    /// ~/.config/rexinator/NAME.yml
    Config(&'a str),
    /// .rexinator/NAME.yml in a project
    Folder(&'a str),
    /// .rexinator.yml in a project
    Local,
}

fn template(kind: Kind) -> String {
    const LOCAL_NOTES: &str = "# root: .            # where panes start (default: the project folder)\n\
         # Personal changes, such as other vars, go in a .local.yml file next to this one.\n\
         # extends: _base     # start from another file, such as .rexinator/_base.yml\n";
    let header = match kind {
        Kind::Config(n) => format!(
            "# rexinator project. Run it with: rexinator start {n} [KEY=VALUE]...\n\
             name: {n}            # the Rex session's label; can use ${{variables}}\n\
             root: ~/git/{n}      # where every pane starts, unless it says otherwise\n"
        ),
        Kind::Folder(n) => format!(
            "# rexinator project `{n}`. Run it with `rexinator start {n}` anywhere in this project.\n\
             # name: myapp-{n}    # the session label (default: the project folder's name, then -{n})\n\
             {LOCAL_NOTES}"
        ),
        Kind::Local => format!(
            "# rexinator project for this folder. Run `rexinator start` here or in any subfolder.\n\
             # name: myapp        # the session label (default: this folder's name)\n\
             {LOCAL_NOTES}"
        ),
    };
    header + TEMPLATE_BODY
}

/// Find or create the project file to edit. See `Cmd::Edit`.
fn edit(name: Option<&str>, here: bool) -> Result<()> {
    let (path, kind) = match (name, here) {
        // In this project's .rexinator/ folder, or this folder's if it isn't in a project yet.
        (Some(n), true) => {
            let dir = find_project_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(LOCAL_DIR);
            let path = file_named(&dir, n).unwrap_or_else(|| dir.join(format!("{n}.yml")));
            (path, Kind::Folder(n))
        }
        (Some(n), false) => {
            let path = named_file(n).unwrap_or_else(|| config_dir().join(format!("{n}.yml")));
            (path, Kind::Config(n))
        }
        (None, true) => {
            let path =
                local_file_in(Path::new(".")).unwrap_or_else(|| PathBuf::from(LOCAL_NAMES[0]));
            (path, Kind::Local)
        }
        (None, false) => {
            let path = find_project_dir()
                .and_then(|d| local_file_in(&d))
                .unwrap_or_else(|| PathBuf::from(LOCAL_NAMES[0]));
            (path, Kind::Local)
        }
    };
    if !path.exists() {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("can't create {}", dir.display()))?;
        }
        std::fs::write(&path, template(kind))?;
        println!("{GREEN}Created{RESET} {}", path.display());
    }
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
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

fn copy(from: &str, to: &str) -> Result<()> {
    let src = find_project(Some(from))?;
    if config_file(to).is_some() {
        bail!("project `{to}` already exists");
    }
    let dst = config_dir().join(format!("{to}.yml"));
    std::fs::create_dir_all(config_dir())?;
    std::fs::copy(&src, &dst).with_context(|| format!("can't copy to {}", dst.display()))?;
    println!(
        "{GREEN}Copied{RESET} {} to {}",
        src.display(),
        dst.display()
    );
    Ok(())
}

fn delete(name: &str, yes: bool) -> Result<()> {
    let path = config_file(name)
        .ok_or_else(|| anyhow!("no project `{name}` in {}", config_dir().display()))?;
    if !yes {
        if !std::io::stdin().is_terminal() {
            bail!("pass --yes to delete without asking");
        }
        eprint!("Delete {BOLD}{}{RESET}? [y/N] ", path.display());
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("Kept it");
            return Ok(());
        }
    }
    std::fs::remove_file(&path)?;
    println!("{YELLOW}Deleted{RESET} {}", path.display());
    let overlay = project::overlay_path(&path);
    if overlay.is_file() {
        println!(
            "{DIM}(left its overrides file in place: {}){RESET}",
            overlay.display()
        );
    }
    Ok(())
}

fn completions(shell: Shell) -> &'static str {
    match shell {
        Shell::Zsh => {
            r#"#compdef rexinator
_rexinator() {
  local -a cmds
  cmds=(
    'start:Start a project, or switch to it'
    'stop:Stop a project'
    'debug:Print the request sent to Rex'
    'list:List projects'
    'edit:Edit a project file'
    'new:Create a project file'
    'copy:Copy a project file'
    'delete:Delete a project file'
    'completions:Print a shell completion script'
  )
  if (( CURRENT == 2 )); then
    _describe 'command' cmds
    return
  fi
  case $words[2] in
    start|s|stop|debug|edit|new|open|copy|cp|delete|rm)
      [[ $PREFIX == -* ]] || compadd -- ${(f)"$(rexinator list --names 2>/dev/null)"} ;;
    completions) compadd bash zsh fish ;;
  esac
}
compdef _rexinator rexinator
"#
        }
        Shell::Bash => {
            r#"_rexinator() {
  local cur=${COMP_WORDS[COMP_CWORD]}
  if [ "$COMP_CWORD" -eq 1 ]; then
    COMPREPLY=($(compgen -W "start stop debug list edit new copy delete completions" -- "$cur"))
    return
  fi
  case ${COMP_WORDS[1]} in
    start|s|stop|debug|edit|new|open|copy|cp|delete|rm)
      [[ $cur == -* ]] || COMPREPLY=($(compgen -W "$(rexinator list --names 2>/dev/null)" -- "$cur")) ;;
    completions) COMPREPLY=($(compgen -W "bash zsh fish" -- "$cur")) ;;
  esac
}
complete -F _rexinator rexinator
"#
        }
        Shell::Fish => {
            r#"set -l cmds start stop debug list edit new copy delete completions
complete -c rexinator -f
complete -c rexinator -n "not __fish_seen_subcommand_from $cmds s ls open cp rm" -a "$cmds"
complete -c rexinator -n "__fish_seen_subcommand_from start s stop debug edit new open copy cp delete rm" -a "(rexinator list --names 2>/dev/null)"
complete -c rexinator -n "__fish_seen_subcommand_from completions" -a "bash zsh fish"
"#
        }
    }
}
