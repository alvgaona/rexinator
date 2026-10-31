# rexinator

Start Rex sessions from project files, like tmuxinator does for tmux.

```sh
cargo install --path .
```

## Usage

```sh
rexinator new             # create .rexinator.yml in this folder and open it
rexinator start           # build the session, or switch to it if it's running
rexinator stop            # close it
rexinator list            # projects, and which are running
```

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

See [examples/tiny-shop](examples/tiny-shop) for a fuller example, and `rexinator --help` for
every command.

See [CONTRIBUTING.md](CONTRIBUTING.md) to work on rexinator.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT),
at your option.
