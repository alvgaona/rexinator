# Contributing

## Setup

Tasks run through [pixi](https://pixi.sh). Install the git hooks once per clone:

```sh
pixi run install-hooks
```

On every commit, the hooks check the message with commitlint, and check staged Rust files with
`rustfmt` and `cargo clippy`.

## Tasks

```sh
pixi run build        # cargo build
pixi run test         # cargo test
pixi run fmt          # cargo fmt
pixi run install      # install rexinator from this checkout
pixi run lint         # rustfmt and clippy on every file
pixi run lint-commit  # check the last commit's message
```

`pixi task list` shows them all.

## Commits

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org), for example
`feat: add layouts` or `fix(list): show running variants`.

## Code

The code is split into a pure core and a thin shell. Parsing, discovery and decisions are pure
functions, and only `commands.rs` and `rex.rs` run programs, write files or print. Keep new logic
in the core, and test it there.

## License

Unless you say otherwise, any contribution you submit for inclusion in rexinator, as defined in the
Apache-2.0 license, is dual licensed as MIT OR Apache-2.0, without any additional terms or
conditions.
