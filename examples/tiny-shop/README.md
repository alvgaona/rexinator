# tiny-shop

A small example project for rexinator: a web server, a background worker, their logs and some
tests, all opened together by [`.rexinator.yml`](.rexinator.yml). It only needs Python 3.

```sh
cd examples/tiny-shop
rexinator start                          # tiny-shop-dev on port 8000
rexinator start env=staging port=8001    # a second copy, side by side
rexinator start @focus                   # a smaller layout
rexinator list
rexinator stop --all
```

It has two layouts. **full** (the default) opens four tabs:

- **code**: your `$EDITOR` on `shop.py`.
- **dev**: the server, with the worker and a `curl` of the page beside it. The server pane has focus.
- **logs**: the request log and the order log, side by side. `watch_logs=false` leaves it out.
- **tests**: the unit tests.

**focus** (`rexinator start @focus`) is for a small screen: the editor with the server beside it,
and the tests. It runs on port 8002, so it can run next to the full layout.

`pre_window` passes `env` and `port` to every pane. Each environment and layout logs to its own
folder under `logs/`, which `rexinator stop` deletes. For your own settings, copy
`.rexinator.local.yml.example` to `.rexinator.local.yml`.
