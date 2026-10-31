//! Reading project files: variables, layouts, windows, panes and split trees.

use anyhow::{Context, Result, anyhow, bail};
use serde_yaml::{Mapping, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::layout;

/// File names for a project file kept inside a project's own folder.
pub const LOCAL_NAMES: &[&str] = &[".rexinator.yml", ".rexinator.yaml"];
/// A folder in a project that holds several project files.
pub const LOCAL_DIR: &str = ".rexinator";

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// Session label. Also how `start` finds a session that's already running.
    pub name: String,
    pub root: PathBuf,
    /// The Rex server to use, as `rex -S` takes it. None means the local one.
    pub server: Option<String>,
    /// Runs on every `start`, before anything else.
    pub on_start: Vec<String>,
    /// Runs only when the session is created.
    pub on_first_start: Vec<String>,
    /// Runs only when the session was already running.
    pub on_restart: Vec<String>,
    pub on_stop: Vec<String>,
    pub windows: Vec<Window>,
    /// Index of the window to show first.
    pub startup_window: usize,
    /// Index of the pane to focus in the startup window, in layout order.
    pub startup_pane: Option<usize>,
    /// The layout used, if the file has `layouts`.
    pub layout: Option<String>,
    /// Every layout the file offers, in order.
    pub layouts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub name: String,
    pub layout: Node,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Pane(Pane),
    Split {
        direction: Direction,
        /// Share of the space that `before` gets, between 0 and 1.
        ratio: f64,
        before: Box<Node>,
        after: Box<Node>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Side by side: `before` on the left.
    Horizontal,
    /// Stacked: `before` on top.
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Pane {
    pub cwd: PathBuf,
    pub label: Option<String>,
    /// Commands typed into the pane's shell, one per line.
    pub typed: Vec<String>,
    /// A program to run as the pane itself, instead of a shell.
    pub run: Option<Vec<String>>,
    pub keep_open: bool,
}

impl Node {
    /// Panes in layout order: each split's `before` side first. Rex returns
    /// block IDs in the same order.
    pub fn panes(&self) -> Vec<&Pane> {
        match self {
            Node::Pane(p) => vec![p],
            Node::Split { before, after, .. } => {
                let mut v = before.panes();
                v.extend(after.panes());
                v
            }
        }
    }

    fn panes_mut(&mut self) -> Vec<&mut Pane> {
        match self {
            Node::Pane(p) => vec![p],
            Node::Split { before, after, .. } => {
                let mut v = before.panes_mut();
                v.extend(after.panes_mut());
                v
            }
        }
    }
}

/// Values used when a project file leaves something out.
pub struct Defaults {
    pub name: String,
    pub root: PathBuf,
    /// Relative `root` paths are resolved against this.
    pub base: PathBuf,
}

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

/// Default name and folders for a project file.
pub fn defaults_for(path: &Path) -> Result<Defaults> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string();
    Ok(match project_dir_of(path) {
        // A project file in a project's folder belongs to that folder: it
        // names the session after it and starts panes in it.
        Some(dir) => {
            let folder = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("project")
                .to_string();
            let name = if LOCAL_NAMES.iter().any(|n| path.ends_with(n)) || stem == "default" {
                folder
            } else {
                format!("{folder}-{stem}")
            };
            Defaults {
                name,
                root: dir.clone(),
                base: dir,
            }
        }
        None => Defaults {
            name: stem,
            root: std::env::current_dir()?,
            base: path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("/")),
        },
    })
}

/// The personal overrides file next to a project file:
/// `.rexinator.local.yml` for `.rexinator.yml`, `shop.local.yml` for `shop.yml`.
pub fn overlay_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("yml");
    path.with_file_name(format!("{stem}.local.{ext}"))
}

/// Load a project file, the files it `extends`, and its `.local` overrides
/// file. `overrides` are the KEY=VALUE pairs from the command line, and
/// `layout` is the layout asked for, if any.
pub fn load(
    path: &Path,
    overrides: &HashMap<String, String>,
    layout: Option<&str>,
) -> Result<Project> {
    let path = std::path::absolute(path)?;
    let defaults = defaults_for(&path)?;
    let doc = read_doc(&path, 0)?;

    let overlay_file = overlay_path(&path);
    let overlay = if overlay_file.is_file() {
        let text = std::fs::read_to_string(&overlay_file)?;
        Some(
            serde_yaml::from_str(&text)
                .with_context(|| format!("in {}", overlay_file.display()))?,
        )
    } else {
        None
    };

    build(doc, overlay, overrides, &defaults, layout)
        .with_context(|| format!("in {}", path.display()))
}

/// Read a project file, applying it on top of the file it `extends`.
fn read_doc(path: &Path, depth: usize) -> Result<Mapping> {
    if depth > 10 {
        bail!("`extends` goes more than 10 files deep; is there a loop?");
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("can't read {}", path.display()))?;
    let mut doc =
        to_mapping(serde_yaml::from_str(&text).with_context(|| format!("in {}", path.display()))?)
            .with_context(|| format!("in {}", path.display()))?;
    let Some(parent) = doc.remove("extends") else {
        return Ok(doc);
    };

    let parent = scalar(&parent, "extends")?;
    let dir = path.parent().unwrap_or(Path::new("/"));
    let mut parent_path = join(dir, &parent);
    if parent_path.extension().is_none() {
        parent_path.set_extension("yml");
    }
    let mut base = read_doc(&parent_path, depth + 1)
        .with_context(|| format!("in the file {} extends", path.display()))?;
    merge(&mut base, doc);
    Ok(base)
}

fn to_mapping(v: Value) -> Result<Mapping> {
    match v {
        Value::Mapping(m) => Ok(m),
        _ => bail!("a project file must be a mapping with `windows` or `layouts`"),
    }
}

/// Parse project YAML, with optional overrides from a `.local` file.
#[cfg(test)]
pub fn parse(
    text: &str,
    overlay: Option<Value>,
    overrides: &HashMap<String, String>,
    defaults: &Defaults,
    layout: Option<&str>,
) -> Result<Project> {
    build(
        to_mapping(serde_yaml::from_str(text)?)?,
        overlay,
        overrides,
        defaults,
        layout,
    )
}

fn build(
    mut doc: Mapping,
    overlay: Option<Value>,
    overrides: &HashMap<String, String>,
    defaults: &Defaults,
    layout: Option<&str>,
) -> Result<Project> {
    let mut overlay = overlay
        .map(|o| to_mapping(o).context("a .local file must be a mapping, like the project file"))
        .transpose()?;

    // Pick the layout. A .local file can add layouts or change the default.
    let mut layouts = take_mapping(&mut doc, "layouts")?;
    let mut default = doc.remove("default");
    if let Some(ov) = &mut overlay {
        if let Some(extra) = take_mapping(ov, "layouts")? {
            let all = layouts.get_or_insert_with(Mapping::new);
            for (k, v) in extra {
                all.insert(k, v);
            }
        }
        if let Some(d) = ov.remove("default") {
            default = Some(d);
        }
    }
    let mut layout_names_session = false;
    let (chosen, names, is_default) = match layouts {
        None => {
            if let Some(l) = layout {
                bail!("there's no layout `{l}`: this project has no `layouts`");
            }
            if default.is_some() {
                bail!("`default` names a layout, but this project has no `layouts`");
            }
            (None, vec![], true)
        }
        Some(all) => {
            if doc.contains_key("windows") {
                bail!("use either `windows` or `layouts`, not both");
            }
            let names = all
                .keys()
                .map(|k| scalar(k, "layout name"))
                .collect::<Result<Vec<_>>>()?;
            let Some(first) = names.first() else {
                bail!("`layouts` is empty")
            };
            let default = match default {
                Some(d) => scalar(&d, "default")?,
                None => first.clone(),
            };
            if !names.contains(&default) {
                bail!(
                    "`default` is `{default}`, but there's no such layout (there are: {})",
                    names.join(", ")
                );
            }
            let want = layout.map(String::from).unwrap_or_else(|| default.clone());
            let body = all.get(want.as_str()).ok_or_else(|| {
                anyhow!(
                    "there's no layout `{want}` (this project has: {})",
                    names.join(", ")
                )
            })?;
            let body = to_mapping(body.clone())
                .with_context(|| format!("layout `{want}` must be a mapping"))?;
            if body.contains_key("layouts") || body.contains_key("default") {
                bail!("layout `{want}` can't contain `layouts` or `default`");
            }
            layout_names_session = body.contains_key("name");
            merge(&mut doc, body);
            let is_default = want == default;
            (Some(want), names, is_default)
        }
    };
    if let Some(ov) = overlay {
        merge(&mut doc, ov);
    }
    let raw_name = doc.get("name").and_then(Value::as_str).map(String::from);

    // `${layout}` is the chosen layout's name.
    let mut overrides = overrides.clone();
    if let Some(l) = &chosen {
        overrides
            .entry("layout".into())
            .or_insert_with(|| l.clone());
    }
    let overrides = &overrides;

    let vars = resolve_vars(doc.remove("vars").as_ref(), overrides)?;
    let lookup = |name: &str| -> Option<String> {
        overrides
            .get(name)
            .or_else(|| vars.get(name))
            .cloned()
            .or_else(|| std::env::var(name).ok())
    };
    let doc = to_mapping(subst_value(Value::Mapping(doc), &lookup)?)?;

    check_keys(
        &doc,
        &[
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
        ],
        "the project",
    )?;

    let mut name = match doc.get("name") {
        Some(v) => scalar(v, "name")?,
        None => defaults.name.clone(),
    };
    // Another layout gets its own session, unless the name already says which.
    if let Some(l) = &chosen
        && !is_default
        && !layout_names_session
        && !raw_name.is_some_and(|n| n.contains("${layout"))
    {
        name = format!("{name}-{l}");
    }
    let root = match doc.get("root") {
        Some(v) => join(&defaults.base, &scalar(v, "root")?),
        None => defaults.root.clone(),
    };
    let pre_window = commands(doc.get("pre_window"), "pre_window")?;

    let mut hidden = Vec::new();
    let mut windows = Vec::new();
    match doc.get("windows") {
        Some(Value::Sequence(items)) if !items.is_empty() => {
            for (i, item) in items.iter().enumerate() {
                match parse_window(item, i, &root, &pre_window)? {
                    Ok(w) => windows.push(w),
                    Err(hidden_name) => hidden.push(hidden_name),
                }
            }
        }
        Some(_) => bail!("`windows` must be a non-empty list"),
        None => bail!("the project has no `windows`"),
    }
    if windows.is_empty() {
        bail!("every window is switched off by `when`");
    }

    let startup_window = match doc.get("startup_window") {
        None => 0,
        Some(v) => {
            let want = scalar(v, "startup_window")?;
            match windows.iter().position(|w| w.name == want) {
                Some(i) => i,
                // Switched off by `when`: show the first window instead.
                None if hidden.contains(&want) => 0,
                None => number(&want, windows.len(), "startup_window", || {
                    "doesn't match any window name".to_string()
                })?,
            }
        }
    };

    let startup_pane = match doc.get("startup_pane") {
        None => None,
        Some(v) => {
            let want = scalar(v, "startup_pane")?;
            let window = &windows[startup_window];
            let panes = window.layout.panes();
            Some(
                match panes
                    .iter()
                    .position(|p| p.label.as_deref() == Some(want.as_str()))
                {
                    Some(i) => i,
                    None => number(&want, panes.len(), "startup_pane", || {
                        format!("doesn't match a pane label in window `{}`", window.name)
                    })?,
                },
            )
        }
    };

    Ok(Project {
        name,
        root,
        server: doc.get("server").map(|v| scalar(v, "server")).transpose()?,
        on_start: commands(doc.get("on_start"), "on_start")?,
        on_first_start: commands(doc.get("on_first_start"), "on_first_start")?,
        on_restart: commands(doc.get("on_restart"), "on_restart")?,
        on_stop: commands(doc.get("on_stop"), "on_stop")?,
        windows,
        startup_window,
        startup_pane,
        layout: chosen,
        layouts: names,
    })
}

fn take_mapping(m: &mut Mapping, key: &str) -> Result<Option<Mapping>> {
    match m.remove(key) {
        None => Ok(None),
        Some(Value::Mapping(v)) => Ok(Some(v)),
        Some(_) => bail!("`{key}` must be a mapping"),
    }
}

/// A 1-based position, as an index.
fn number(want: &str, len: usize, what: &str, no_match: impl Fn() -> String) -> Result<usize> {
    match want.parse::<usize>() {
        Ok(n) if n >= 1 && n <= len => Ok(n - 1),
        Ok(n) => bail!("{what} {n} is out of range (there are {len}, numbered from 1)"),
        Err(_) => bail!("{what} `{want}` {}", no_match()),
    }
}

/// Apply `top` over `base`: its keys replace the base's, except `vars` and
/// `layouts`, which are merged one entry at a time.
fn merge(base: &mut Mapping, top: Mapping) {
    for (k, v) in top {
        if let Some(key @ ("vars" | "layouts")) = k.as_str()
            && let (Some(Value::Mapping(into)), Value::Mapping(extra)) = (base.get_mut(key), &v)
        {
            for (ek, ev) in extra {
                into.insert(ek.clone(), ev.clone());
            }
            continue;
        }
        base.insert(k, v);
    }
}

/// Whether a `when` value switches something on. Off: false, no, off, 0,
/// empty. A leading `!` flips it.
fn truthy(v: &Value) -> Result<bool> {
    let s = match v {
        Value::Bool(b) => return Ok(*b),
        Value::Null => return Ok(false),
        v => scalar(v, "when")?,
    };
    let s = s.trim();
    let (negate, s) = match s.strip_prefix('!') {
        Some(rest) => (true, rest.trim()),
        None => (false, s),
    };
    let on = !matches!(
        s.to_ascii_lowercase().as_str(),
        "" | "false" | "no" | "off" | "0"
    );
    Ok(on != negate)
}

/// Take `key` out of a mapping value, if it's a mapping that has it.
fn take_key(v: &Value, key: &str) -> (Value, Option<Value>) {
    match v {
        Value::Mapping(m) if m.contains_key(key) => {
            let mut m = m.clone();
            let taken = m.remove(key);
            (Value::Mapping(m), taken)
        }
        v => (v.clone(), None),
    }
}

/// A window, or `Err(name)` if `when` switched it (or all its panes) off.
fn parse_window(
    item: &Value,
    index: usize,
    root: &Path,
    pre_window: &[String],
) -> Result<Result<Window, String>> {
    let Some(m) = item.as_mapping().filter(|m| m.len() == 1) else {
        bail!(
            "window {} must be written as `- NAME: COMMAND` or `- NAME: {{ ... }}`",
            index + 1
        );
    };
    let (key, value) = m.iter().next().unwrap();
    let name = scalar(key, "window name")?;

    // `pre` and `when` belong to the window, so take them out before reading the layout.
    let (value, pre) = take_key(value, "pre");
    let pre = commands(pre.as_ref(), "pre")?;
    let (value, when) = take_key(&value, "when");
    if let Some(w) = when
        && !truthy(&w).with_context(|| format!("in window `{name}`"))?
    {
        return Ok(Err(name));
    }

    let Some(mut layout) =
        parse_node(&value, root).with_context(|| format!("in window `{name}`"))?
    else {
        return Ok(Err(name));
    };
    let pre: Vec<String> = pre_window.iter().chain(&pre).cloned().collect();
    if !pre.is_empty() {
        for pane in layout.panes_mut() {
            apply_pre(pane, &pre);
        }
    }
    Ok(Ok(Window { name, layout }))
}

/// Run `pre` before the pane's own commands. A `run` pane has no shell to
/// type into, so it's wrapped in one that runs `pre` and then the program.
fn apply_pre(pane: &mut Pane, pre: &[String]) {
    match pane.run.take() {
        None => {
            let mut typed = pre.to_vec();
            typed.append(&mut pane.typed);
            pane.typed = typed;
        }
        Some(argv) => {
            let mut cmd = vec![
                "sh".to_string(),
                "-c".to_string(),
                format!("{}\nexec \"$@\"", pre.join("\n")),
                "sh".to_string(),
            ];
            cmd.extend(argv);
            pane.run = Some(cmd);
        }
    }
}

/// A node is a pane, a split (`split:`), or a preset layout (`panes:`).
/// None when `when` switched it off.
fn parse_node(v: &Value, base: &Path) -> Result<Option<Node>> {
    let m = match v {
        Value::Null | Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Sequence(_) => {
            return Ok(Some(Node::Pane(Pane {
                cwd: base.to_path_buf(),
                typed: commands(Some(v), "pane")?,
                ..Default::default()
            })));
        }
        Value::Mapping(m) => m,
        Value::Tagged(_) => bail!("YAML tags aren't supported"),
    };
    if let Some(w) = m.get("when")
        && !truthy(w)?
    {
        return Ok(None);
    }

    if m.contains_key("split") {
        check_keys(
            m,
            &["split", "ratio", "before", "after", "root", "when"],
            "a split",
        )?;
        let base = sub_root(m, base)?;
        let direction = match scalar(&m["split"], "split")?.as_str() {
            "horizontal" | "h" | "right" => Direction::Horizontal,
            "vertical" | "v" | "below" | "down" => Direction::Vertical,
            other => {
                bail!("unknown split `{other}`: use `right` (side by side) or `below` (stacked)")
            }
        };
        let ratio = match m.get("ratio") {
            Some(r) => parse_ratio(r)?,
            None => 0.5,
        };
        let side = |k: &str| -> Result<Option<Node>> {
            let v = m
                .get(k)
                .ok_or_else(|| anyhow!("a split needs both `before` and `after`"))?;
            parse_node(v, &base).with_context(|| format!("in `{k}`"))
        };
        // With one side switched off, the other side takes the whole space.
        return Ok(match (side("before")?, side("after")?) {
            (Some(before), Some(after)) => Some(Node::Split {
                direction,
                ratio,
                before: Box::new(before),
                after: Box::new(after),
            }),
            (one, other) => one.or(other),
        });
    }

    if m.contains_key("panes") {
        check_keys(m, &["panes", "layout", "root", "when"], "a pane layout")?;
        let base = sub_root(m, base)?;
        let Some(Value::Sequence(items)) = m
            .get("panes")
            .filter(|p| p.as_sequence().is_some_and(|s| !s.is_empty()))
        else {
            bail!("`panes` must be a non-empty list");
        };
        let mut panes = Vec::new();
        for (i, p) in items.iter().enumerate() {
            if let Some(node) =
                parse_node(p, &base).with_context(|| format!("in pane {}", i + 1))?
            {
                panes.push(node);
            }
        }
        if panes.is_empty() {
            return Ok(None);
        }
        let spec = match m.get("layout") {
            Some(l) => scalar(l, "layout")?,
            None => "even-vertical".to_string(),
        };
        return layout::preset(&spec, panes).map(Some);
    }

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
    let keep_open = match m.get("keep_open") {
        None => false,
        Some(v) => truthy(v)?,
    };
    Ok(Some(Node::Pane(Pane {
        cwd: match m.get("cwd") {
            Some(c) => join(base, &scalar(c, "cwd")?),
            None => base.to_path_buf(),
        },
        label: m.get("label").map(|l| scalar(l, "label")).transpose()?,
        typed: commands(m.get("type"), "type")?,
        run,
        keep_open,
    })))
}

fn sub_root(m: &Mapping, base: &Path) -> Result<PathBuf> {
    Ok(match m.get("root") {
        Some(r) => join(base, &scalar(r, "root")?),
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

/// Expand `~` and resolve `path` against `base`.
pub fn join(base: &Path, path: &str) -> PathBuf {
    let p = if path == "~" {
        home()
    } else if let Some(rest) = path.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(path)
    };
    let p = if p.is_absolute() { p } else { base.join(p) };
    // Drops `.` components, so `/a/./b` becomes `/a/b`.
    p.components().collect()
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn scalar(v: &Value, what: &str) -> Result<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => bail!("`{what}` must be a single value"),
    }
}

/// Null → none; a value → one command; a list → several.
fn commands(v: Option<&Value>, what: &str) -> Result<Vec<String>> {
    match v {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Sequence(items)) => items.iter().map(|i| scalar(i, what)).collect(),
        Some(v) => Ok(vec![scalar(v, what)?]),
    }
}

fn check_keys(m: &Mapping, allowed: &[&str], what: &str) -> Result<()> {
    for k in m.keys() {
        let k = scalar(k, "key")?;
        if !allowed.contains(&k.as_str()) {
            bail!(
                "unknown key `{k}` in {what} (expected one of: {})",
                allowed.join(", ")
            );
        }
    }
    Ok(())
}

/// Defaults from `vars:`, overridden on the command line. A default can use
/// earlier variables and environment variables.
fn resolve_vars(
    vars: Option<&Value>,
    overrides: &HashMap<String, String>,
) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    let Some(vars) = vars else { return Ok(out) };
    let Value::Mapping(vars) = vars else {
        bail!("`vars` must be a mapping of names to defaults")
    };
    for (k, v) in vars {
        let name = scalar(k, "variable name")?;
        let value = match overrides.get(&name) {
            Some(o) => o.clone(),
            None => {
                let raw = if v.is_null() {
                    String::new()
                } else {
                    scalar(v, &name)?
                };
                let lookup = |n: &str| {
                    overrides
                        .get(n)
                        .or_else(|| out.get(n))
                        .cloned()
                        .or_else(|| std::env::var(n).ok())
                };
                substitute(&raw, &lookup).with_context(|| format!("in variable `{name}`"))?
            }
        };
        out.insert(name, value);
    }
    Ok(out)
}

fn subst_value(v: Value, lookup: &dyn Fn(&str) -> Option<String>) -> Result<Value> {
    Ok(match v {
        Value::String(s) => Value::String(substitute(&s, lookup)?),
        Value::Sequence(items) => Value::Sequence(
            items
                .into_iter()
                .map(|i| subst_value(i, lookup))
                .collect::<Result<_>>()?,
        ),
        Value::Mapping(m) => {
            let mut out = Mapping::new();
            for (k, v) in m {
                out.insert(subst_value(k, lookup)?, subst_value(v, lookup)?);
            }
            Value::Mapping(out)
        }
        other => other,
    })
}

/// Replace `${name}` and `${name:-default}`. `$$` is a literal `$`. A bare
/// `$NAME` is left alone so the shell can expand it.
pub fn substitute(s: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Result<String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(r) = after.strip_prefix('$') {
            out.push('$');
            rest = r;
        } else if let Some(r) = after.strip_prefix('{') {
            let end = r
                .find('}')
                .ok_or_else(|| anyhow!("unclosed `${{` in {s:?}"))?;
            let expr = &r[..end];
            let (name, default) = match expr.split_once(":-") {
                Some((n, d)) => (n, Some(d)),
                None => (expr, None),
            };
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                bail!("bad variable name `{name}` in {s:?}");
            }
            let value = lookup(name)
                .or_else(|| default.map(String::from))
                .ok_or_else(|| {
                    anyhow!("unknown variable `{name}`: add it to `vars`, or pass {name}=VALUE")
                })?;
            out.push_str(&value);
            rest = &r[end + 1..];
        } else {
            out.push('$');
            rest = after;
        }
    }
    out.push_str(rest);
    Ok(out)
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

    fn p(src: &str) -> Result<Project> {
        parse(src, None, &HashMap::new(), &defaults(), None)
    }

    fn pv(src: &str, pairs: &[(&str, &str)], layout: Option<&str>) -> Result<Project> {
        parse(src, None, &vars(pairs), &defaults(), layout)
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

    fn sub(s: &str, pairs: &[(&str, &str)]) -> Result<String> {
        let m = vars(pairs);
        substitute(s, &|n| m.get(n).cloned())
    }

    #[test]
    fn substitutes_variables() {
        assert_eq!(sub("a-${x}-b", &[("x", "1")]).unwrap(), "a-1-b");
        assert_eq!(sub("${y:-dflt}", &[]).unwrap(), "dflt");
        assert_eq!(sub("$$HOME and $PATH", &[]).unwrap(), "$HOME and $PATH");
        assert!(
            sub("${nope}", &[])
                .unwrap_err()
                .to_string()
                .contains("unknown variable `nope`")
        );
        assert!(sub("${open", &[]).is_err());
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
        assert_eq!(p.on_first_start, vec!["docker compose up -d"]);
        assert!(p.on_start.is_empty());
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
    fn vars_can_use_earlier_vars_and_overrides() {
        let src = "vars: {port: 3000, url: 'http://x:${port}'}\nwindows: [{w: '${url}'}]";
        let p = pv(src, &[("port", "9")], None).unwrap();
        assert_eq!(pane(&p.windows[0].layout).typed, vec!["http://x:9"]);
    }

    #[test]
    fn local_file_overrides_keys_and_merges_vars() {
        let src = "vars: {a: 1, b: 2}\nwindows: [{w: '${a} ${b}'}]";
        let overlay = serde_yaml::from_str("vars: {b: 20}\nname: mine").unwrap();
        let p = parse(src, Some(overlay), &HashMap::new(), &defaults(), None).unwrap();
        assert_eq!(p.name, "mine");
        assert_eq!(pane(&p.windows[0].layout).typed, vec!["1 20"]);
        // The command line still wins over the .local file.
        let overlay = serde_yaml::from_str("vars: {b: 20}").unwrap();
        let p = parse(
            src,
            Some(overlay),
            &vars(&[("b", "cli")]),
            &defaults(),
            None,
        )
        .unwrap();
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
        assert_eq!(
            p("startup_window: b\nwindows: [{a: x}, {b: y}]")
                .unwrap()
                .startup_window,
            1
        );
        assert_eq!(
            p("startup_window: 1\nwindows: [{a: x}, {b: y}]")
                .unwrap()
                .startup_window,
            0
        );
        assert!(p("startup_window: 3\nwindows: [{a: x}, {b: y}]").is_err());

        let src = "startup_window: b\nstartup_pane: PANE\nwindows: [{a: x}, {b: {panes: [x, {label: logs}, z]}}]";
        assert_eq!(
            p(&src.replace("PANE", "logs")).unwrap().startup_pane,
            Some(1)
        );
        assert_eq!(p(&src.replace("PANE", "3")).unwrap().startup_pane, Some(2));
        let err = p(&src.replace("PANE", "nope")).unwrap_err();
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
        let d = defaults_for(Path::new("/git/myapp/.rexinator.yml")).unwrap();
        assert_eq!(
            (d.name.as_str(), d.root.as_path()),
            ("myapp", Path::new("/git/myapp"))
        );
        let d = defaults_for(Path::new("/git/myapp/.rexinator/backend.yml")).unwrap();
        assert_eq!(
            (d.name.as_str(), d.root.as_path()),
            ("myapp-backend", Path::new("/git/myapp"))
        );
        let d = defaults_for(Path::new("/git/myapp/.rexinator/default.yml")).unwrap();
        assert_eq!(d.name, "myapp");
        let d = defaults_for(Path::new("/home/me/.config/rexinator/shop.yml")).unwrap();
        assert_eq!(d.name, "shop");
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
    }

    #[test]
    fn local_file_can_add_a_layout_and_change_the_default() {
        let overlay =
            serde_yaml::from_str("default: mine\nlayouts: {mine: {windows: [{me: x}]}}").unwrap();
        let p = parse(LAYOUTS, Some(overlay), &HashMap::new(), &defaults(), None).unwrap();
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
        let p = pv(src, &[], None).unwrap();
        assert_eq!(window_names(&p), ["code", "dev", "side"]);
        // The startup window is switched off, so the first one shows.
        assert_eq!(p.startup_window, 0);
        let dev: Vec<_> = p.windows[1]
            .layout
            .panes()
            .iter()
            .map(|p| p.typed[0].clone())
            .collect();
        assert_eq!(dev, ["serve", "tail -f log"]);
        // One side of the split is off: the other side fills the window.
        assert_eq!(pane(&p.windows[2].layout).typed, vec!["b"]);

        let p = pv(src, &[("db", "yes"), ("logs", "0")], None).unwrap();
        assert_eq!(window_names(&p), ["code", "db", "dev", "side"]);
        assert_eq!(p.startup_window, 1);
        let dev: Vec<_> = p.windows[2]
            .layout
            .panes()
            .iter()
            .map(|p| p.typed[0].clone())
            .collect();
        assert_eq!(dev, ["serve", "echo no logs"]);
        assert!(matches!(p.windows[3].layout, Node::Split { .. }));
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
    fn extends_merges_files() {
        let dir = std::env::temp_dir().join(format!("rexinator-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(LOCAL_DIR)).unwrap();
        std::fs::write(
            dir.join(LOCAL_DIR).join("_base.yml"),
            "vars: {a: 1, b: 2}\npre_window: echo base\nwindows: [{w: base}]",
        )
        .unwrap();
        let file = dir.join(LOCAL_DIR).join("api.yml");
        std::fs::write(
            &file,
            "extends: _base\nvars: {b: 3}\nwindows: [{api: 'serve ${a} ${b}'}]",
        )
        .unwrap();

        let p = load(&file, &HashMap::new(), None).unwrap();
        let folder = dir.file_name().unwrap().to_str().unwrap();
        assert_eq!(p.name, format!("{folder}-api"));
        assert_eq!(p.root, dir);
        assert_eq!(window_names(&p), ["api"]);
        assert_eq!(
            pane(&p.windows[0].layout).typed,
            vec!["echo base", "serve 1 3"]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
