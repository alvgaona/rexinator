# rexinator

[![CI](https://github.com/alvgaona/rexinator/actions/workflows/ci.yaml/badge.svg)](https://github.com/alvgaona/rexinator/actions/workflows/ci.yaml)
[![Crates.io](https://img.shields.io/crates/v/rexinator.svg)](https://crates.io/crates/rexinator)
[![Downloads](https://img.shields.io/crates/d/rexinator.svg)](https://crates.io/crates/rexinator)
[![Rust](https://img.shields.io/badge/rust-2024_edition-orange.svg?logo=rust)](https://doc.rust-lang.org/edition-guide/rust-2024/)
[![Pixi](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/prefix-dev/pixi/main/assets/badge/v0.json)](https://pixi.sh)
[![License](https://img.shields.io/crates/l/rexinator.svg)](#license)

Start Rex sessions from project files, like tmuxinator does for tmux.

## Installation

```sh
cargo install rexinator
```

Or from source:

```sh
cargo install --git https://github.com/alvgaona/rexinator
```

## Usage

```sh
rexinator new             # create .rexinator.yml in this folder and open it
rexinator start           # build the session, or switch to it if it's running
rexinator stop            # close it
rexinator list            # projects, and which are running
```

`rexinator --help` lists every command.

For tab completion of commands, project names and layouts, add this to your shell's startup file:

```sh
eval "$(rexinator completions zsh)"          # ~/.zshrc (use bash in ~/.bashrc)
rexinator completions fish | source          # ~/.config/fish/config.fish
```

### Example

```yaml
# .rexinator.yml
vars:
  port: 3000

windows:
  - editor: nvim .
  - dev:
      layout: main-left 65
      panes:
        - PORT=${port} npm run dev
        - git status
```

Override variables on the command line: `rexinator start port=4000`.

See [examples/tiny-shop](examples/tiny-shop) for a fuller example.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT),
at your option.
