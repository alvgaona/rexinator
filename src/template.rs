//! The starting text for new project files.

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind<'a> {
    /// ~/.config/rexinator/NAME.yml
    Config(&'a str),
    /// .rexinator/NAME.yml in a project
    Folder(&'a str),
    /// .rexinator.yml in a project
    Local,
}

/// The text of a new project file.
pub fn template(kind: Kind) -> String {
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
