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
