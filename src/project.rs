//! Turning project files into a [`Project`]: layouts, variables, windows,
//! panes and split trees. Everything here is pure except [`load`], which
//! reads files through the [`Fs`] it's given.

use anyhow::{Context as _, Result, anyhow, bail};
use serde_yaml::{Mapping, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::env::{self, Env};
use crate::fs::Fs;
use crate::layout;
use crate::model::{Direction, Hooks, Node, NonEmpty, Pane, Project, Window};
use crate::vars::{self, Lookup};
use crate::when::truthy;
use crate::yaml::{self, check_keys, commands, scalar};

/// File names for a project file kept inside a project's own folder.
pub const LOCAL_NAMES: &[&str] = &[".rexinator.yml", ".rexinator.yaml"];
/// A folder in a project that holds several project files.
pub const LOCAL_DIR: &str = ".rexinator";

const PROJECT_KEYS: &[&str] = &[
    "name",
    "root",
    "server",
    "pre_window",
    "on_start",
    "on_first_start",
    "on_restart",
    "on_stop",
    "startup_window",
    "startup_pane",
    "windows",
];

/// Values used when a project file leaves something out.
#[derive(Debug, Clone, PartialEq)]
pub struct Defaults {
    pub name: String,
    pub root: PathBuf,
    /// Relative `root` paths are resolved against this.
    pub base: PathBuf,
}

/// What building a project needs from outside its files.
pub struct Context<'a> {
    /// What `~` stands for.
    pub home: &'a Path,
    /// Environment variables, the last place `${NAME}` is looked up.
    pub env: &'a Lookup<'a>,
}

/// What the command line asks of a project file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    /// KEY=VALUE pairs.
    pub vars: HashMap<String, String>,
    /// `@LAYOUT`.
    pub layout: Option<String>,
}

// ---------------------------------------------------------------------------
// Reading files: the only part that touches the file system, through `fs`.

/// Load a project file, the files it `extends`, and its `.local` overrides file.
pub fn load(fs: &Fs, env: &Env, path: &Path, request: &Request) -> Result<Project> {
    let path = env::absolute(&env.cwd, path);
    let doc = read_doc(fs, &env.home, &path, 0)?;
    let overlay_file = overlay_path(&path);
    let overlay = (fs.is_file)(&overlay_file)
        .then(|| read_yaml(fs, &overlay_file))
        .transpose()?;
    let lookup = |name: &str| env.vars.get(name).cloned();
    let cx = Context {
        home: &env.home,
        env: &lookup,
    };
    build(doc, overlay, request, &defaults_for(&path, &env.cwd), &cx)
        .with_context(|| format!("in {}", path.display()))
}

fn read_yaml(fs: &Fs, path: &Path) -> Result<Value> {
    let text = (fs.read)(path).with_context(|| format!("can't read {}", path.display()))?;
    serde_yaml::from_str(&text).with_context(|| format!("in {}", path.display()))
}

/// Read a project file, applied on top of the file it `extends`.
fn read_doc(fs: &Fs, home: &Path, path: &Path, depth: usize) -> Result<Mapping> {
    if depth > 10 {
        bail!("`extends` goes more than 10 files deep; is there a loop?");
    }
    let doc = yaml::mapping(read_yaml(fs, path)?, "a project file")
        .with_context(|| format!("in {}", path.display()))?;
    let (doc, parent) = yaml::split(&doc, "extends");
    let Some(parent) = parent else {
        return Ok(doc);
    };
    let parent = extends_path(home, path, &scalar(&parent, "extends")?);
    let base = read_doc(fs, home, &parent, depth + 1)
        .with_context(|| format!("in the file {} extends", path.display()))?;
    Ok(yaml::merge(&base, &doc))
}

/// `extends: NAME` is relative to the file, and `.yml` is optional.
fn extends_path(home: &Path, file: &Path, name: &str) -> PathBuf {
    let path = join(home, file.parent().unwrap_or(Path::new("/")), name);
    match path.extension() {
        Some(_) => path,
        None => path.with_extension("yml"),
    }
}

// ---------------------------------------------------------------------------
// Where a project file is, and what that implies.

/// The project folder a project file belongs to: the folder holding a
/// `.rexinator.yml`, or the folder above a `.rexinator/` folder. None for a
/// file in the config folder.
pub fn project_dir_of(path: &Path) -> Option<PathBuf> {
    let dir = path.parent()?;
    let file = path.file_name()?.to_str()?;
    if LOCAL_NAMES.contains(&file) {
        Some(dir.to_path_buf())
    } else if dir.file_name().is_some_and(|n| n == LOCAL_DIR) {
        dir.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

/// Default name and folders for a project file. A file in a project's
/// folder names the session after that folder and starts panes in it; one
/// in the config folder uses its own name and the current folder.
pub fn defaults_for(path: &Path, cwd: &Path) -> Defaults {
    let stem = file_stem(path).unwrap_or("project").to_string();
    match project_dir_of(path) {
        Some(dir) => {
            let folder = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("project");
            let is_main = LOCAL_NAMES.iter().any(|n| path.ends_with(n)) || stem == "default";
            Defaults {
                name: if is_main {
                    folder.to_string()
                } else {
                    format!("{folder}-{stem}")
                },
                root: dir.clone(),
                base: dir,
            }
        }
        None => Defaults {
            name: stem,
            root: cwd.to_path_buf(),
            base: path
                .parent()
                .map_or_else(|| PathBuf::from("/"), Path::to_path_buf),
        },
    }
}

/// The personal overrides file next to a project file:
/// `.rexinator.local.yml` for `.rexinator.yml`, `shop.local.yml` for `shop.yml`.
pub fn overlay_path(path: &Path) -> PathBuf {
    let stem = file_stem(path).unwrap_or("");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("yml");
    path.with_file_name(format!("{stem}.local.{ext}"))
}

fn file_stem(path: &Path) -> Option<&str> {
    path.file_stem().and_then(|s| s.to_str())
}

// ---------------------------------------------------------------------------
// Building a project from its documents.

/// A project from its file (with `extends` already applied) and an optional
/// `.local` overrides file.
pub fn build(
    doc: Mapping,
    overlay: Option<Value>,
    request: &Request,
    defaults: &Defaults,
    cx: &Context,
) -> Result<Project> {
    let overlay = overlay
        .map(|o| yaml::mapping(o, "a .local file, like the project file,"))
        .transpose()?
        .unwrap_or_default();
    let chosen = choose_layout(&doc, &overlay, request.layout.as_deref())?;
    let (overlay, _) = yaml::split(&overlay, "layouts");
    let (overlay, _) = yaml::split(&overlay, "default");
    let doc = yaml::merge(&chosen.doc, &overlay);
    let raw_name = doc.get("name").and_then(Value::as_str).map(String::from);

    // `${layout}` is the chosen layout's name, unless the command line sets it.
    let overrides: HashMap<String, String> = chosen
        .layout
        .iter()
        .map(|l| ("layout".to_string(), l.clone()))
        .chain(request.vars.clone())
        .collect();
    let (doc, var_defs) = yaml::split(&doc, "vars");
    let vars = vars::resolve(var_defs.as_ref(), &overrides, cx.env)?;
    let lookup = |name: &str| {
        overrides
            .get(name)
            .or_else(|| vars.get(name))
            .cloned()
            .or_else(|| (cx.env)(name))
    };
    let doc = yaml::mapping(
        vars::substitute_value(Value::Mapping(doc), &lookup)?,
        "a project file",
    )?;
    check_keys(&doc, PROJECT_KEYS, "the project")?;

    let root = match doc.get("root") {
        Some(v) => join(cx.home, &defaults.base, &scalar(v, "root")?),
        None => defaults.root.clone(),
    };
    let pre_window = commands(doc.get("pre_window"), "pre_window")?;
    let (windows, hidden) = windows(doc.get("windows"), &root, &pre_window, cx.home)?;
    let startup_window = startup_window(doc.get("startup_window"), &windows, &hidden)?;
    let startup_pane = startup_pane(doc.get("startup_pane"), &windows[startup_window])?;

    Ok(Project {
        name: session_name(&doc, defaults, &chosen, raw_name.as_deref())?,
        root,
        server: doc.get("server").map(|v| scalar(v, "server")).transpose()?,
        hooks: Hooks {
            on_start: commands(doc.get("on_start"), "on_start")?,
            on_first_start: commands(doc.get("on_first_start"), "on_first_start")?,
            on_restart: commands(doc.get("on_restart"), "on_restart")?,
            on_stop: commands(doc.get("on_stop"), "on_stop")?,
        },
        windows,
        startup_window,
        startup_pane,
        layout: chosen.layout,
        layouts: chosen.names,
    })
}

/// The project document with the chosen layout applied.
struct Chosen {
    doc: Mapping,
    layout: Option<String>,
    /// Every layout name, in order.
    names: Vec<String>,
    is_default: bool,
    /// The layout sets `name` itself.
    names_session: bool,
}

/// Apply the layout asked for, or the default one. A `.local` file can add
/// layouts or change the default.
fn choose_layout(doc: &Mapping, overlay: &Mapping, want: Option<&str>) -> Result<Chosen> {
    let (doc, layouts) = yaml::split(doc, "layouts");
    let (doc, default) = yaml::split(&doc, "default");
    let default = overlay.get("default").cloned().or(default);
    let layouts = match (layouts, overlay.get("layouts").cloned()) {
        (None, None) => None,
        (base, extra) => Some(yaml::overlay(
            &yaml::optional_mapping(base, "`layouts`")?,
            &yaml::optional_mapping(extra, "`layouts`")?,
        )),
    };

    let Some(all) = layouts else {
        if let Some(l) = want {
            bail!("there's no layout `{l}`: this project has no `layouts`");
        }
        if default.is_some() {
            bail!("`default` names a layout, but this project has no `layouts`");
        }
        return Ok(Chosen {
            doc,
            layout: None,
            names: vec![],
            is_default: true,
            names_session: false,
        });
    };

    if doc.contains_key("windows") {
        bail!("use either `windows` or `layouts`, not both");
    }
    let names = all
        .keys()
        .map(|k| scalar(k, "layout name"))
        .collect::<Result<Vec<_>>>()?;
    let default = match (default, names.first()) {
        (Some(d), _) => scalar(&d, "default")?,
        (None, Some(first)) => first.clone(),
        (None, None) => bail!("`layouts` is empty"),
    };
    if !names.contains(&default) {
        bail!(
            "`default` is `{default}`, but there's no such layout (there are: {})",
            names.join(", ")
        );
    }
    let want = want.map_or_else(|| default.clone(), String::from);
    let body = all.get(want.as_str()).cloned().ok_or_else(|| {
        anyhow!(
            "there's no layout `{want}` (this project has: {})",
            names.join(", ")
        )
    })?;
    let body = yaml::mapping(body, &format!("layout `{want}`"))?;
    if body.contains_key("layouts") || body.contains_key("default") {
        bail!("layout `{want}` can't contain `layouts` or `default`");
    }
    Ok(Chosen {
        doc: yaml::merge(&doc, &body),
        is_default: want == default,
        names_session: body.contains_key("name"),
        layout: Some(want),
        names,
    })
}

/// The session label. A layout other than the default gets its own session,
/// unless the name already says which layout it is.
fn session_name(
    doc: &Mapping,
    defaults: &Defaults,
    chosen: &Chosen,
    raw_name: Option<&str>,
) -> Result<String> {
    let name = match doc.get("name") {
        Some(v) => scalar(v, "name")?,
        None => defaults.name.clone(),
    };
    let names_layout = chosen.names_session || raw_name.is_some_and(|n| n.contains("${layout"));
    Ok(match &chosen.layout {
        Some(l) if !chosen.is_default && !names_layout => format!("{name}-{l}"),
        _ => name,
    })
}

/// A window, or the name of one that `when` switched off.
enum Parsed {
    On(Window),
    Off(String),
}

/// The windows that are on, and the names of those switched off.
fn windows(
    v: Option<&Value>,
    root: &Path,
    pre_window: &[String],
    home: &Path,
) -> Result<(Vec<Window>, Vec<String>)> {
    let items = match v {
        Some(Value::Sequence(items)) if !items.is_empty() => items,
        Some(_) => bail!("`windows` must be a non-empty list"),
        None => bail!("the project has no `windows`"),
    };
    let parsed = items
        .iter()
        .enumerate()
        .map(|(i, item)| parse_window(item, i, root, pre_window, home))
        .collect::<Result<Vec<_>>>()?;
    let on: Vec<Window> = parsed
        .iter()
        .filter_map(|p| match p {
            Parsed::On(w) => Some(w.clone()),
            Parsed::Off(_) => None,
        })
        .collect();
    let off: Vec<String> = parsed
        .into_iter()
        .filter_map(|p| match p {
            Parsed::Off(name) => Some(name),
            Parsed::On(_) => None,
        })
        .collect();
    if on.is_empty() {
        bail!("every window is switched off by `when`");
    }
    Ok((on, off))
}

/// The window to show first, by name or number. One that `when` switched off
/// falls back to the first window.
fn startup_window(v: Option<&Value>, windows: &[Window], hidden: &[String]) -> Result<usize> {
    let Some(v) = v else { return Ok(0) };
    let want = scalar(v, "startup_window")?;
    match windows.iter().position(|w| w.name == want) {
        Some(i) => Ok(i),
        None if hidden.contains(&want) => Ok(0),
        None => number(&want, windows.len(), "startup_window", || {
            "doesn't match any window name".to_string()
        }),
    }
}

/// The pane to focus in the startup window, by label or number.
fn startup_pane(v: Option<&Value>, window: &Window) -> Result<Option<usize>> {
    let Some(v) = v else { return Ok(None) };
    let want = scalar(v, "startup_pane")?;
    let panes = window.layout.panes();
    panes
        .iter()
        .position(|p| p.label.as_deref() == Some(want.as_str()))
        .map_or_else(
            || {
                number(&want, panes.len(), "startup_pane", || {
                    format!("doesn't match a pane label in window `{}`", window.name)
                })
            },
            Ok,
        )
        .map(Some)
}

/// A 1-based position, as an index.
fn number(want: &str, len: usize, what: &str, no_match: impl Fn() -> String) -> Result<usize> {
    match want.parse::<usize>() {
        Ok(n) if n >= 1 && n <= len => Ok(n - 1),
        Ok(n) => bail!("{what} {n} is out of range (there are {len}, numbered from 1)"),
        Err(_) => bail!("{what} `{want}` {}", no_match()),
    }
}

fn parse_window(
    item: &Value,
    index: usize,
    root: &Path,
    pre_window: &[String],
    home: &Path,
) -> Result<Parsed> {
    let Some((key, value)) = item
        .as_mapping()
        .filter(|m| m.len() == 1)
        .and_then(|m| m.iter().next())
    else {
        bail!(
            "window {} must be written as `- NAME: COMMAND` or `- NAME: {{ ... }}`",
            index + 1
        );
    };
    let name = scalar(key, "window name")?;

    // `pre` and `when` belong to the window, so take them out before reading the layout.
    let (value, pre) = yaml::split_value(value, "pre");
    let (value, when) = yaml::split_value(&value, "when");
    let on = match &when {
        Some(w) => truthy(w).with_context(|| format!("in window `{name}`"))?,
        None => true,
    };
    if !on {
        return Ok(Parsed::Off(name));
    }
    let pre: Vec<String> = pre_window
        .iter()
        .cloned()
        .chain(commands(pre.as_ref(), "pre")?)
        .collect();
    Ok(
        match parse_node(&value, root, home).with_context(|| format!("in window `{name}`"))? {
            None => Parsed::Off(name),
            Some(layout) if pre.is_empty() => Parsed::On(Window { name, layout }),
            Some(layout) => Parsed::On(Window {
                name,
                layout: layout.map_panes(&|pane| with_pre(pane, &pre)),
            }),
        },
    )
}

/// The pane with `pre` run before its own commands. A `run` pane has no
/// shell to type into, so it's wrapped in one that runs `pre` and then the program.
fn with_pre(pane: Pane, pre: &[String]) -> Pane {
    match pane.run {
        None => Pane {
            typed: pre.iter().cloned().chain(pane.typed).collect(),
            ..pane
        },
        Some(argv) => Pane {
            run: Some(
                [
                    "sh".to_string(),
                    "-c".to_string(),
                    format!("{}\nexec \"$@\"", pre.join("\n")),
                    "sh".to_string(),
                ]
                .into_iter()
                .chain(argv)
                .collect(),
            ),
            ..pane
        },
    }
}

/// A node is a pane, a split (`split:`), or a preset layout (`panes:`).
/// None when `when` switched it off.
fn parse_node(v: &Value, base: &Path, home: &Path) -> Result<Option<Node>> {
    let m = match v {
        Value::Mapping(m) => m,
        Value::Tagged(_) => bail!("YAML tags aren't supported"),
        // A command, a list of commands, or nothing: a shell pane.
        _ => {
            return Ok(Some(Node::Pane(Pane {
                cwd: base.to_path_buf(),
                typed: commands(Some(v), "pane")?,
                ..Default::default()
            })));
        }
    };
    if let Some(w) = m.get("when")
        && !truthy(w)?
    {
        return Ok(None);
    }
    if m.contains_key("split") {
        parse_split(m, base, home)
    } else if m.contains_key("panes") {
        parse_panes(m, base, home)
    } else {
        parse_pane(m, base, home).map(|p| Some(Node::Pane(p)))
    }
}

fn parse_split(m: &Mapping, base: &Path, home: &Path) -> Result<Option<Node>> {
    check_keys(
        m,
        &["split", "ratio", "before", "after", "root", "when"],
        "a split",
    )?;
    let base = sub_root(m, base, home)?;
    let direction = match scalar(&m["split"], "split")?.as_str() {
        "horizontal" | "h" | "right" => Direction::Horizontal,
        "vertical" | "v" | "below" | "down" => Direction::Vertical,
        other => {
            bail!("unknown split `{other}`: use `right` (side by side) or `below` (stacked)")
        }
    };
    let ratio = m.get("ratio").map(parse_ratio).transpose()?.unwrap_or(0.5);
    let side = |k: &str| -> Result<Option<Node>> {
        let v = m
            .get(k)
            .ok_or_else(|| anyhow!("a split needs both `before` and `after`"))?;
        parse_node(v, &base, home).with_context(|| format!("in `{k}`"))
    };
    // With one side switched off, the other side takes the whole space.
    Ok(match (side("before")?, side("after")?) {
        (Some(before), Some(after)) => Some(Node::split(direction, ratio, before, after)),
        (one, other) => one.or(other),
    })
}

fn parse_panes(m: &Mapping, base: &Path, home: &Path) -> Result<Option<Node>> {
    check_keys(m, &["panes", "layout", "root", "when"], "a pane layout")?;
    let base = sub_root(m, base, home)?;
    let Some(items) = m
        .get("panes")
        .and_then(Value::as_sequence)
        .filter(|s| !s.is_empty())
    else {
        bail!("`panes` must be a non-empty list");
    };
    let panes: Vec<Node> = items
        .iter()
        .enumerate()
        .map(|(i, p)| parse_node(p, &base, home).with_context(|| format!("in pane {}", i + 1)))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    let Some(panes) = NonEmpty::from_vec(panes) else {
        return Ok(None);
    };
    let spec = match m.get("layout") {
        Some(l) => scalar(l, "layout")?,
        None => "even-vertical".to_string(),
    };
    layout::preset(&spec, panes).map(Some)
}

fn parse_pane(m: &Mapping, base: &Path, home: &Path) -> Result<Pane> {
    check_keys(
        m,
        &["type", "run", "cwd", "label", "keep_open", "when"],
        "a pane",
    )?;
    let run = match m.get("run") {
        None | Some(Value::Null) => None,
        Some(Value::Sequence(_)) => Some(commands(m.get("run"), "run")?),
        Some(v) => Some(vec!["sh".into(), "-c".into(), scalar(v, "run")?]),
    };
    Ok(Pane {
        cwd: match m.get("cwd") {
            Some(c) => join(home, base, &scalar(c, "cwd")?),
            None => base.to_path_buf(),
        },
        label: m.get("label").map(|l| scalar(l, "label")).transpose()?,
        typed: commands(m.get("type"), "type")?,
        run,
        keep_open: m.get("keep_open").map(truthy).transpose()?.unwrap_or(false),
    })
}

fn sub_root(m: &Mapping, base: &Path, home: &Path) -> Result<PathBuf> {
    Ok(match m.get("root") {
        Some(r) => join(home, base, &scalar(r, "root")?),
        None => base.to_path_buf(),
    })
}

/// Accepts a percentage (60) or a fraction (0.6).
fn parse_ratio(v: &Value) -> Result<f64> {
    let s = scalar(v, "ratio")?;
    let n: f64 = s
        .trim_end_matches('%')
        .parse()
        .map_err(|_| anyhow!("ratio `{s}` isn't a number"))?;
    let r = if n >= 1.0 { n / 100.0 } else { n };
    if !(r > 0.0 && r < 1.0) {
        bail!("ratio `{s}` must be between 1 and 99 (percent)");
    }
    Ok(r)
}

/// `path` with `~` expanded, resolved against `base`.
pub fn join(home: &Path, base: &Path, path: &str) -> PathBuf {
    let path = match path.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with('/') => home.join(&rest[1..]),
        _ => PathBuf::from(path),
    };
    env::absolute(base, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn defaults() -> Defaults {
        Defaults {
            name: "dflt".into(),
            root: "/cwd".into(),
            base: "/".into(),
        }
    }

    fn parse(text: &str, overlay: Option<&str>, request: &Request) -> Result<Project> {
        let no_env = |_: &str| None;
        let cx = Context {
            home: Path::new("/home/me"),
            env: &no_env,
        };
        let doc = yaml::mapping(serde_yaml::from_str(text)?, "a project file")?;
        let overlay = overlay.map(serde_yaml::from_str).transpose()?;
        build(doc, overlay, request, &defaults(), &cx)
    }

    fn p(src: &str) -> Result<Project> {
        parse(src, None, &Request::default())
    }

    fn pv(src: &str, pairs: &[(&str, &str)], layout: Option<&str>) -> Result<Project> {
        let request = Request {
            vars: vars(pairs),
            layout: layout.map(String::from),
        };
        parse(src, None, &request)
    }

    fn pane(n: &Node) -> &Pane {
        let Node::Pane(p) = n else {
            panic!("not a pane: {n:?}")
        };
        p
    }

    fn window_names(p: &Project) -> Vec<&str> {
        p.windows.iter().map(|w| w.name.as_str()).collect()
    }

    const SHOP: &str = r#"
name: shop-${env}
root: /srv/shop
vars:
  env: dev
  port: 3000
  url: http://localhost:${port}
on_first_start: docker compose up -d
windows:
  - editor: nvim .
  - server: PORT=${port} NODE_ENV=${env} npm run dev
  - shell:
"#;

    #[test]
    fn parses_simple_project_with_vars() {
        let p = pv(SHOP, &[("env", "staging")], None).unwrap();
        assert_eq!(p.name, "shop-staging");
        assert_eq!(p.root, PathBuf::from("/srv/shop"));
        assert_eq!(p.hooks.on_first_start, vec!["docker compose up -d"]);
        assert!(p.hooks.on_start.is_empty());
        assert_eq!(p.windows.len(), 3);
        let server = pane(&p.windows[1].layout);
        assert_eq!(server.typed, vec!["PORT=3000 NODE_ENV=staging npm run dev"]);
        assert_eq!(server.cwd, PathBuf::from("/srv/shop"));
        assert!(pane(&p.windows[2].layout).typed.is_empty());
        assert_eq!(p.layout, None);
    }

    #[test]
    fn defaults_apply_when_name_and_root_are_missing() {
        let p = p("windows: [{a: x}]").unwrap();
        assert_eq!(p.name, "dflt");
        assert_eq!(p.root, PathBuf::from("/cwd"));
        assert_eq!(pane(&p.windows[0].layout).cwd, PathBuf::from("/cwd"));
    }

    #[test]
    fn home_and_relative_paths() {
        let p = p("root: ~/code\nwindows: [{a: {cwd: ./sub, type: x}}, {b: {cwd: '~', type: y}}]")
            .unwrap();
        assert_eq!(p.root, PathBuf::from("/home/me/code"));
        assert_eq!(
            pane(&p.windows[0].layout).cwd,
            PathBuf::from("/home/me/code/sub")
        );
        assert_eq!(pane(&p.windows[1].layout).cwd, PathBuf::from("/home/me"));
    }

    #[test]
    fn env_vars_are_the_last_lookup() {
        let env = |n: &str| (n == "USER").then(|| "ana".to_string());
        let cx = Context {
            home: Path::new("/h"),
            env: &env,
        };
        let doc =
            serde_yaml::from_str("vars: {who: '${USER}'}\nwindows: [{w: 'hi ${who} ${USER}'}]")
                .unwrap();
        let p = build(doc, None, &Request::default(), &defaults(), &cx).unwrap();
        assert_eq!(pane(&p.windows[0].layout).typed, vec!["hi ana ana"]);
    }

    #[test]
    fn local_file_overrides_keys_and_merges_vars() {
        let src = "vars: {a: 1, b: 2}\nwindows: [{w: '${a} ${b}'}]";
        let p = parse(src, Some("vars: {b: 20}\nname: mine"), &Request::default()).unwrap();
        assert_eq!(p.name, "mine");
        assert_eq!(pane(&p.windows[0].layout).typed, vec!["1 20"]);
        // The command line still wins over the .local file.
        let request = Request {
            vars: vars(&[("b", "cli")]),
            layout: None,
        };
        let p = parse(src, Some("vars: {b: 20}"), &request).unwrap();
        assert_eq!(pane(&p.windows[0].layout).typed, vec!["1 cli"]);
    }

    #[test]
    fn parses_split_tree() {
        let src = r#"
root: /p
windows:
  - run:
      split: right
      ratio: 60
      before: cargo run
      after:
        split: below
        before: { run: htop, keep_open: true }
        after:  { cwd: ./logs, type: [cd ., tail -f app.log] }
"#;
        let p = p(src).unwrap();
        let Node::Split {
            direction,
            ratio,
            after,
            ..
        } = &p.windows[0].layout
        else {
            panic!()
        };
        assert_eq!(*direction, Direction::Horizontal);
        assert!((ratio - 0.6).abs() < 1e-9);
        let Node::Split {
            direction,
            before,
            after,
            ..
        } = after.as_ref()
        else {
            panic!()
        };
        assert_eq!(*direction, Direction::Vertical);
        let htop = pane(before);
        assert_eq!(
            htop.run.as_deref(),
            Some(&["sh".into(), "-c".into(), "htop".into()][..])
        );
        assert!(htop.keep_open);
        let logs = pane(after);
        assert_eq!(logs.cwd, PathBuf::from("/p/logs"));
        assert_eq!(logs.typed.len(), 2);
    }

    #[test]
    fn pre_window_and_window_pre_run_first() {
        let src = r#"
pre_window: source .venv/bin/activate
windows:
  - a: pytest
  - b:
      pre: export DEBUG=1
      panes: [serve, { run: [htop, -d, "5"] }]
  - c: { pre: echo hi }
"#;
        let p = p(src).unwrap();
        assert_eq!(
            pane(&p.windows[0].layout).typed,
            vec!["source .venv/bin/activate", "pytest"]
        );
        let b = p.windows[1].layout.panes();
        assert_eq!(
            b[0].typed,
            vec!["source .venv/bin/activate", "export DEBUG=1", "serve"]
        );
        assert_eq!(
            b[1].run.as_ref().unwrap(),
            &[
                "sh",
                "-c",
                "source .venv/bin/activate\nexport DEBUG=1\nexec \"$@\"",
                "sh",
                "htop",
                "-d",
                "5"
            ]
        );
        assert_eq!(
            pane(&p.windows[2].layout).typed,
            vec!["source .venv/bin/activate", "echo hi"]
        );
    }

    #[test]
    fn reports_typos() {
        let err = p("windows: [{w: {split: right, before: a, afetr: b}}]").unwrap_err();
        assert!(
            format!("{err:#}").contains("unknown key `afetr`"),
            "{err:#}"
        );
        let err = p("windws: []").unwrap_err();
        assert!(err.to_string().contains("unknown key `windws`"));
    }

    #[test]
    fn startup_window_and_pane() {
        let first = |src: &str| p(src).unwrap().startup_window;
        assert_eq!(first("startup_window: b\nwindows: [{a: x}, {b: y}]"), 1);
        assert_eq!(first("startup_window: 1\nwindows: [{a: x}, {b: y}]"), 0);
        assert!(p("startup_window: 3\nwindows: [{a: x}, {b: y}]").is_err());

        let src = "startup_window: b\nstartup_pane: PANE\nwindows: [{a: x}, {b: {panes: [x, {label: logs}, z]}}]";
        let pane_of = |which: &str| p(&src.replace("PANE", which)).map(|p| p.startup_pane);
        assert_eq!(pane_of("logs").unwrap(), Some(1));
        assert_eq!(pane_of("3").unwrap(), Some(2));
        let err = pane_of("nope").unwrap_err();
        assert!(err.to_string().contains("window `b`"), "{err}");
    }

    #[test]
    fn overlay_file_names() {
        assert_eq!(
            overlay_path(Path::new("/p/.rexinator.yml")),
            PathBuf::from("/p/.rexinator.local.yml")
        );
        assert_eq!(
            overlay_path(Path::new("/c/shop.yaml")),
            PathBuf::from("/c/shop.local.yaml")
        );
    }

    #[test]
    fn defaults_for_project_files() {
        let cwd = Path::new("/here");
        let d = defaults_for(Path::new("/git/myapp/.rexinator.yml"), cwd);
        assert_eq!(
            (d.name.as_str(), d.root.as_path()),
            ("myapp", Path::new("/git/myapp"))
        );
        let d = defaults_for(Path::new("/git/myapp/.rexinator/backend.yml"), cwd);
        assert_eq!(
            (d.name.as_str(), d.root.as_path()),
            ("myapp-backend", Path::new("/git/myapp"))
        );
        let d = defaults_for(Path::new("/git/myapp/.rexinator/default.yml"), cwd);
        assert_eq!(d.name, "myapp");
        let d = defaults_for(Path::new("/home/me/.config/rexinator/shop.yml"), cwd);
        assert_eq!((d.name.as_str(), d.root.as_path()), ("shop", cwd));
        assert_eq!(
            project_dir_of(Path::new("/home/me/.config/rexinator/shop.yml")),
            None
        );
    }

    const LAYOUTS: &str = r#"
name: shop
vars: {port: 1}
pre_window: echo pre
default: full
layouts:
  focus:
    windows: [{code: nvim}]
  full:
    vars: {port: 2}
    startup_window: server
    windows: [{code: nvim}, {server: 'serve ${port}'}]
"#;

    #[test]
    fn layouts_pick_default_or_named() {
        let p = pv(LAYOUTS, &[], None).unwrap();
        assert_eq!(p.layout.as_deref(), Some("full"));
        assert_eq!(p.layouts, vec!["focus", "full"]);
        assert_eq!(p.name, "shop");
        assert_eq!(window_names(&p), ["code", "server"]);
        assert_eq!(p.startup_window, 1);
        // The layout's vars apply on top of the project's, and pre_window is shared.
        assert_eq!(
            pane(&p.windows[1].layout).typed,
            vec!["echo pre", "serve 2"]
        );

        let p = pv(LAYOUTS, &[], Some("focus")).unwrap();
        assert_eq!(p.name, "shop-focus");
        assert_eq!(window_names(&p), ["code"]);
        assert_eq!(pane(&p.windows[0].layout).typed, vec!["echo pre", "nvim"]);

        let err = pv(LAYOUTS, &[], Some("nope")).unwrap_err();
        assert!(err.to_string().contains("has: focus, full"), "{err}");
    }

    #[test]
    fn layout_name_can_be_used_in_the_session_name() {
        let src = "name: s-${layout}\nlayouts: {a: {windows: [{w: x}]}, b: {windows: [{w: y}]}}";
        assert_eq!(pv(src, &[], None).unwrap().name, "s-a");
        assert_eq!(pv(src, &[], Some("b")).unwrap().name, "s-b");
        // A layout that sets its own name keeps it as is.
        let src = "name: s\nlayouts: {a: {windows: [{w: x}]}, b: {name: other, windows: [{w: y}]}}";
        assert_eq!(pv(src, &[], Some("b")).unwrap().name, "other");
        // Without `layouts`, `${layout}` is just an unknown variable.
        assert!(p("name: ${layout}\nwindows: [{w: x}]").is_err());
    }

    #[test]
    fn layout_errors() {
        assert!(
            p("windows: [{w: x}]\nlayouts: {a: {windows: [{w: x}]}}")
                .unwrap_err()
                .to_string()
                .contains("not both")
        );
        assert!(pv("windows: [{w: x}]", &[], Some("a")).is_err());
        assert!(p("default: c\nlayouts: {a: {windows: [{w: x}]}}").is_err());
        assert!(p("layouts: {}").is_err());
    }

    #[test]
    fn local_file_can_add_a_layout_and_change_the_default() {
        let overlay = "default: mine\nlayouts: {mine: {windows: [{me: x}]}}";
        let p = parse(LAYOUTS, Some(overlay), &Request::default()).unwrap();
        assert_eq!(p.layout.as_deref(), Some("mine"));
        assert_eq!(p.layouts, vec!["focus", "full", "mine"]);
        assert_eq!(p.name, "shop");
    }

    #[test]
    fn when_switches_windows_and_panes() {
        let src = r#"
vars: {db: false, logs: true}
startup_window: db
windows:
  - code: nvim
  - db: { when: '${db}', type: psql }
  - dev:
      panes:
        - serve
        - { when: '${logs}', type: tail -f log }
        - { when: '!${logs}', type: echo no logs }
  - side:
      split: right
      before: { when: '${db}', type: a }
      after: b
"#;
        let first_commands = |w: &Window| -> Vec<String> {
            w.layout
                .panes()
                .iter()
                .map(|p| p.typed[0].clone())
                .collect()
        };
        let p = pv(src, &[], None).unwrap();
        assert_eq!(window_names(&p), ["code", "dev", "side"]);
        // The startup window is switched off, so the first one shows.
        assert_eq!(p.startup_window, 0);
        assert_eq!(first_commands(&p.windows[1]), ["serve", "tail -f log"]);
        // One side of the split is off: the other side fills the window.
        assert_eq!(pane(&p.windows[2].layout).typed, vec!["b"]);

        let p = pv(src, &[("db", "yes"), ("logs", "0")], None).unwrap();
        assert_eq!(window_names(&p), ["code", "db", "dev", "side"]);
        assert_eq!(p.startup_window, 1);
        assert_eq!(first_commands(&p.windows[2]), ["serve", "echo no logs"]);
        assert!(matches!(p.windows[3].layout, Node::Split { .. }));
    }

    #[test]
    fn when_compares_and_combines_values() {
        let src = r#"
vars: {env: dev, with_db: false}
windows:
  - always: x
  - prod: { when: '${env} == prod', type: careful }
  - not-prod:
      when: ${env} != prod
      panes: [y]
  - db: { when: '${with_db} && ${env} != prod', type: psql }
  - ci: { when: '${env} == ci || ${env} == staging', type: run-ci }
"#;
        let names = |pairs: &[(&str, &str)]| -> Vec<String> {
            window_names(&pv(src, pairs, None).unwrap())
                .into_iter()
                .map(String::from)
                .collect()
        };
        assert_eq!(names(&[]), ["always", "not-prod"]);
        assert_eq!(names(&[("env", "prod")]), ["always", "prod"]);
        assert_eq!(
            names(&[("with_db", "yes"), ("env", "staging")]),
            ["always", "not-prod", "db", "ci"]
        );
        assert_eq!(
            names(&[("with_db", "yes"), ("env", "prod")]),
            ["always", "prod"]
        );
    }

    #[test]
    fn when_on_every_window_is_an_error() {
        assert!(
            p("windows: [{a: {when: false, type: x}}]")
                .unwrap_err()
                .to_string()
                .contains("switched off")
        );
    }

    #[test]
    fn load_applies_extends_and_local_overrides() {
        let fs = Fs::memory(&[
            (
                "/git/app/.rexinator/_base.yml",
                "vars: {a: 1, b: 2}\npre_window: echo base\nwindows: [{w: base}]",
            ),
            (
                "/git/app/.rexinator/api.yml",
                "extends: _base\nvars: {b: 3, c: x}\nwindows: [{api: 'serve ${a} ${b} ${c}'}]",
            ),
            ("/git/app/.rexinator/api.local.yml", "vars: {c: mine}"),
        ]);
        let env = Env::new(PathBuf::from("/git/app/src"), HashMap::new());
        let p = load(
            &fs,
            &env,
            Path::new("../.rexinator/api.yml"),
            &Request::default(),
        );
        let p = p.unwrap();
        assert_eq!(p.name, "app-api");
        assert_eq!(p.root, PathBuf::from("/git/app"));
        assert_eq!(window_names(&p), ["api"]);
        assert_eq!(
            pane(&p.windows[0].layout).typed,
            vec!["echo base", "serve 1 3 mine"]
        );
    }

    #[test]
    fn extends_loops_are_caught() {
        let fs = Fs::memory(&[("/c/a.yml", "extends: b"), ("/c/b.yml", "extends: a")]);
        let env = Env::new(PathBuf::from("/"), HashMap::new());
        let err = load(&fs, &env, Path::new("/c/a.yml"), &Request::default()).unwrap_err();
        assert!(format!("{err:#}").contains("loop"), "{err:#}");
    }
}
