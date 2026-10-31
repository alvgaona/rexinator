# rexinator

Start Rex sessions from project files, like tmuxinator does for tmux. Each project file describes a
session's tabs and panes. `rexinator start` builds the whole session in one call to Rex, or switches
to it if it's already running.

```sh
cargo install --path .

rexinator new shop                      # create ~/.config/rexinator/shop.yml and open it
rexinator start shop env=staging        # build the session (or switch to it)
rexinator stop shop env=staging         # close it and run on_stop
rexinator list                          # projects, and which sessions are running
rexinator debug shop                    # print the request sent to Rex
```

## A project file in your project

Commit a `.rexinator.yml` to a repository and run `rexinator` from anywhere inside it:

```sh
cd ~/git/myapp && rexinator new         # create ./.rexinator.yml from a template
cd src/deep/folder && rexinator start   # finds ../../../.rexinator.yml
rexinator start ~/git/myapp             # or point at the folder from elsewhere
```

A project file in a project's folder names the session after the folder and starts panes there,
unless it sets `name` or `root`. Personal changes go in `.rexinator.local.yml` next to it (add it
to `.gitignore`). Its settings replace the project's, and its `vars` are merged in one by one:

```yaml
# .rexinator.local.yml
vars:
  port: 4000
```

Project files in `~/.config/rexinator` can have a `.local.yml` file the same way, for example
`shop.local.yml`. The folder is `$REXINATOR_CONFIG`, else `$XDG_CONFIG_HOME/rexinator`, else
`~/.config/rexinator`.

### Several configs in one project

For separate parts of a repository, such as a monorepo's services, put one file per part in a
`.rexinator/` folder:

```text
myapp/
  .rexinator/
    _base.yml      # shared settings; files starting with _ aren't projects
    api.yml        # rexinator start api     -> session myapp-api
    web.yml        # rexinator start web     -> session myapp-web
```

`rexinator start NAME` looks in the nearest `.rexinator/` folder before `~/.config/rexinator`. With
no name, it uses `.rexinator.yml`, else `.rexinator/default.yml`, else the only file in the
folder. Panes start in the project folder (`myapp`). `rexinator new api --here` creates
`.rexinator/api.yml`.

`extends` starts a file from another one. Its keys replace the other file's, and `vars` and
`layouts` are merged one entry at a time:

```yaml
# .rexinator/api.yml
extends: _base            # relative to this file; .yml is added if there's no extension
vars: { port: 4000 }
windows:
  - api: { cwd: services/api, type: make dev }
```

`extends` works in any project file, including ones in `~/.config/rexinator`.

## Project files

```yaml
name: shop-${env}          # the session label; how `start` finds a running session
root: ~/git/shop           # where panes start; relative paths are relative to this file

vars:                      # defaults; override with KEY=VALUE on the command line
  env: dev
  port: 3000
  url: http://localhost:${port}   # can use earlier vars and environment variables

pre_window: source .venv/bin/activate   # runs first in every pane (string or list)

on_start: git fetch                     # runs on every `start`
on_first_start: docker compose up -d    # runs only when the session is created
on_restart: echo welcome back           # runs only when it was already running
on_stop: docker compose down            # runs after `rexinator stop`

startup_window: dev        # window to show first: name or number (default: the first)
startup_pane: logs         # pane to focus in it: label or number, in layout order
server: devbox             # open on another Rex host, as `rex -S` takes it

windows:
  - editor: nvim .                       # one pane; the command is typed into a shell
  - shell:                               # empty: just a shell

  - dev:                                 # a preset layout
      root: ./frontend                   # optional, relative to the project root
      pre: export DEBUG=1                # runs first in this window's panes, after pre_window
      layout: main-left 65               # even-horizontal, even-vertical, main-left [N%],
      panes:                             # main-top [N%], tiled. Default: even-vertical
        - PORT=${port} npm run dev
        - [git fetch, git status]        # several commands, one after another
        - { cwd: ./logs, type: tail -f app.log, label: logs }

  - monitor:                             # an explicit split tree
      split: right                       # right = side by side, below = stacked
      ratio: 70                          # percent for `before` (default 50)
      before: { run: htop }              # `run`: the program is the pane; it closes on exit
      after:
        split: below
        before: { run: [k9s, -n, "${env}"], keep_open: true }
        after: cargo watch -x check
```

**Variables.** `${name}` and `${name:-default}` work anywhere: names, paths and commands. Values
come from the command line, then the `.local` file, then `vars`, then the environment. A bare
`$NAME` is left for the shell, and `$$` is a literal `$`.

**Quoting.** This is plain YAML, so quote a value that contains `: ` (common in `echo` commands), or
one inside `{ ... }` that contains `${...}` or a comma:

```yaml
on_stop: 'echo "done: cleaned up"'
before: { label: logs, run: "tail -F ${logs}/app.log" }
```

### Layouts

One project can offer several arrangements, such as a full layout and a smaller one for a laptop.
Replace `windows:` with `layouts:`. Everything outside `layouts` is shared, and each layout can set
any project key, such as `windows`, `startup_window` or its own `vars`:

```yaml
name: shop
vars: { port: 3000 }
pre_window: source .venv/bin/activate

default: full                    # used by plain `rexinator start` (default: the first)
layouts:
  full:
    windows: [ ... ]
  focus:                         # rexinator start @focus
    vars: { port: 3001 }
    windows:
      - editor: nvim .
```

A layout other than the default gets its own session, `shop-focus`, so the two can run side by
side. To name sessions yourself, use `${layout}` in `name`, or set `name` in the layout.
`${layout}` also works anywhere else in the file. A `.local` file can add layouts or change
`default`.

### Switching windows and panes on and off

`when:` on a window or a pane includes it only when its value is on. `false`, `no`, `off`, `0`
and an empty value are off; anything else is on, and a leading `!` flips it. `A == B` and
`A != B` compare text instead, after variables are filled in; quotes around either side are
optional. `&&` and `||` combine these, with `&&` binding tighter (there are no parentheses):

```yaml
vars:
  with_db: false
windows:
  - db: { when: '${with_db}', type: psql }        # rexinator start with_db=true
  - dev:
      panes:
        - npm run dev
        - { when: '!${with_db}', type: echo "no database" }
  - deploy:
      when: ${env} == prod                         # rexinator start env=prod
      panes: [./deploy.sh --dry-run]
  - seed:
      when: ${with_db} && ${env} != prod           # never seed production
      panes: [./seed.sh]
```

Conditions are checked after variables are filled in, so a variable whose value contains `==`,
`!=`, `&&` or `||` changes the condition's meaning.

When one side of a split is switched off, the other side fills the space. If `startup_window`
names a window that's switched off, the first window is shown instead.

**Panes.** A pane is a string or list (commands typed into a shell, which stays open after you stop
them), or a mapping with `type`, `run`, `cwd`, `label` and `keep_open`. Anywhere a pane goes, you can
also put a `split:` tree or a `panes:` layout. `pre_window` and `pre` also apply to `run` panes: the
program is started from a shell that runs them first.

## Commands

| Command | What it does |
| --- | --- |
| `start [PROJECT] [@LAYOUT] [KEY=VALUE]...` | Build the session, or switch to it. `--no-show` leaves the Rex app alone. `--name` overrides the session name. |
| `start --here [PROJECT]` | Add the project's windows to the Rex session you're in. |
| `stop [PROJECT] [@LAYOUT] [KEY=VALUE]...` | Close the session and run `on_stop`. |
| `stop --all` | Stop every session rexinator started, whatever variables or layout it used. |
| `list` | Projects, their layouts (the default marked `*`), and the sessions running for each. |
| `debug [PROJECT] [KEY=VALUE]...` | Print the request `start` would send. |
| `new`/`edit [NAME]` | Open a project file in `$EDITOR`, creating it first. No name: this project's `.rexinator.yml`. With `--here`, a new NAME goes in `.rexinator/`. |
| `copy FROM TO`, `delete NAME` | Copy or delete a project file. |
| `completions bash\|zsh\|fish` | Shell completion, e.g. `eval "$(rexinator completions zsh)"`. |

`PROJECT` is a name (from this project's `.rexinator/` folder or the config folder), a file, or a
project folder. With no `PROJECT`, the nearest project folder's config is used. `-S SERVER` works with every command and overrides
the project's `server`. `--color auto|always|never` controls colored output (default: `auto`,
which colors only when writing to a terminal).

## Example

[`examples/tiny-shop`](examples/tiny-shop) is a small Python project with a `.rexinator.yml`
that uses most of these features, including two layouts and a `when` switch. Run
`rexinator start` inside it.

## How it works

It runs the `rex` command (`rex api call session.create`, `rex ls`, `rex kill`, `rex do
session.select`). Set `REXINATOR_REX` to use a different `rex` binary, and `REX_CLIENT` to pick
which Rex app to switch when several are connected.

Sessions it starts are recorded in `~/.local/state/rexinator/sessions.json` (or under
`$XDG_STATE_HOME`), so `list` and `stop --all` find sessions whose names came from variables or layouts.

On another server (`server:` or `-S`), use absolute paths: `~` is expanded on this machine, and
folders aren't checked before the session is created. Hooks always run on this machine.

## Contributing

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org), such as
`feat: add layouts` or `fix(list): show running variants`. Install the git hooks once per clone:

```sh
pixi run install-hooks
```

On every commit this checks the message with commitlint, and checks staged Rust files with
`rustfmt` and `cargo clippy`. Other tasks: `pixi run lint` checks every file, `pixi run
lint-commit` checks the last commit's message, and `build`, `test`, `fmt` and `install` run the
matching cargo commands. `pixi task list` shows them all.
