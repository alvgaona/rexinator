//! Preset layouts, and turning a project into a Rex `session.create` request.

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

use crate::model::{Direction, Node, NonEmpty, Pane, Project, Window};

const TERMINAL: &str = "com.superlogical.terminal.shell";

pub const PRESETS: &[&str] = &[
    "even-horizontal",
    "even-vertical",
    "main-left",
    "main-top",
    "tiled",
];

/// Build a split tree from a preset name, such as `main-left` or `main-left 70`.
pub fn preset(spec: &str, panes: NonEmpty<Node>) -> Result<Node> {
    let words: Vec<&str> = spec.split_whitespace().collect();
    let (name, size) = match words.as_slice() {
        [] => ("", None),
        [name] => (*name, None),
        [name, size] => (*name, Some(main_size(spec, size)?)),
        _ => bail!("layout `{spec}` has too many words"),
    };
    if size.is_some() && !name.starts_with("main-") {
        bail!("layout `{name}` doesn't take a size; only main-left and main-top do");
    }
    Ok(match name {
        "even-horizontal" => even(panes, Direction::Horizontal),
        "even-vertical" => even(panes, Direction::Vertical),
        "main-left" => main(panes, Direction::Horizontal, size.unwrap_or(0.6)),
        "main-top" => main(panes, Direction::Vertical, size.unwrap_or(0.6)),
        "tiled" => tiled(panes),
        other => bail!(
            "unknown layout `{other}` (expected one of: {})",
            PRESETS.join(", ")
        ),
    })
}

/// The main pane's share, from a percentage such as `70` or `70%`.
fn main_size(spec: &str, word: &str) -> Result<f64> {
    match word.trim_end_matches('%').parse::<f64>() {
        Ok(n) if n > 0.0 && n < 100.0 => Ok(n / 100.0),
        _ => bail!("layout `{spec}`: the size must be a percentage between 1 and 99"),
    }
}

/// The first pane takes `ratio` of the space; the rest share what's left,
/// laid out across the other direction.
fn main(panes: NonEmpty<Node>, direction: Direction, ratio: f64) -> Node {
    match NonEmpty::from_vec(panes.tail) {
        None => panes.head,
        Some(rest) => Node::split(direction, ratio, panes.head, even(rest, direction.other())),
    }
}

/// Rows of equal panes, as close to a square grid as the count allows.
fn tiled(panes: NonEmpty<Node>) -> Node {
    let cols = (panes.len() as f64).sqrt().ceil() as usize;
    let rows = panes
        .chunks(cols)
        .map(|row| even(row, Direction::Horizontal));
    even(rows, Direction::Vertical)
}

/// Equal-sized panes in a row or column: the first of n panes gets 1/n of
/// the space, and the rest share what's left the same way.
fn even(nodes: NonEmpty<Node>, direction: Direction) -> Node {
    match NonEmpty::from_vec(nodes.tail) {
        None => nodes.head,
        Some(rest) => {
            let ratio = 1.0 / (rest.len() + 1) as f64;
            Node::split(direction, ratio, nodes.head, even(rest, direction))
        }
    }
}

impl Direction {
    fn other(self) -> Direction {
        match self {
            Direction::Horizontal => Direction::Vertical,
            Direction::Vertical => Direction::Horizontal,
        }
    }
}

/// The JSON payload for Rex's `session.create` method.
pub fn session_request(p: &Project) -> Value {
    json!({
        "label": p.name,
        "initial_windows": p.windows.iter().map(window_request).collect::<Vec<_>>(),
    })
}

/// One window: a `session.create` entry, or the payload for `session.new_window`.
pub fn window_request(w: &Window) -> Value {
    json!({ "window_label": w.name, "layout": node_json(&w.layout) })
}

fn node_json(node: &Node) -> Value {
    match node {
        Node::Pane(p) => json!({ "block": block_json(p) }),
        Node::Split {
            direction,
            ratio,
            before,
            after,
        } => json!({
            "split": {
                "direction": match direction {
                    Direction::Horizontal => "horizontal",
                    Direction::Vertical => "vertical",
                },
                "ratio": (ratio * 10000.0).round() / 10000.0,
                "before": node_json(before),
                "after": node_json(after),
            }
        }),
    }
}

/// A terminal block: only the options a pane sets are included.
fn block_json(p: &Pane) -> Value {
    let options: Map<String, Value> = [
        Some(("cwd", json!(p.cwd))),
        (!p.typed.is_empty()).then(|| {
            let input: String = p.typed.iter().map(|c| format!("{c}\n")).collect();
            ("initial_input", json!(input))
        }),
        p.run.as_ref().map(|cmd| ("command", json!(cmd))),
        p.keep_open
            .then(|| ("exit", json!({ "on_completion": false }))),
    ]
    .into_iter()
    .flatten()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let block: Map<String, Value> = [
        Some(("flavor", json!(TERMINAL))),
        Some(("options", Value::Object(options))),
        p.label.as_ref().map(|l| ("label", json!(l))),
    ]
    .into_iter()
    .flatten()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    Value::Object(block)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panes(n: usize) -> NonEmpty<Node> {
        let nodes = (0..n)
            .map(|i| {
                Node::Pane(Pane {
                    typed: vec![format!("p{i}")],
                    ..Default::default()
                })
            })
            .collect();
        NonEmpty::from_vec(nodes).unwrap()
    }

    /// Render a tree compactly, e.g. `H(p0,V(p1,p2))`.
    fn shape(n: &Node) -> String {
        match n {
            Node::Pane(p) => p.typed[0].clone(),
            Node::Split {
                direction,
                before,
                after,
                ..
            } => {
                let d = if *direction == Direction::Horizontal {
                    "H"
                } else {
                    "V"
                };
                format!("{d}({},{})", shape(before), shape(after))
            }
        }
    }

    #[test]
    fn presets_have_the_expected_shape() {
        assert_eq!(
            shape(&preset("even-horizontal", panes(3)).unwrap()),
            "H(p0,H(p1,p2))"
        );
        assert_eq!(
            shape(&preset("main-left", panes(3)).unwrap()),
            "H(p0,V(p1,p2))"
        );
        assert_eq!(
            shape(&preset("main-top", panes(3)).unwrap()),
            "V(p0,H(p1,p2))"
        );
        assert_eq!(
            shape(&preset("tiled", panes(4)).unwrap()),
            "V(H(p0,p1),H(p2,p3))"
        );
        assert_eq!(
            shape(&preset("tiled", panes(5)).unwrap()),
            "V(H(p0,H(p1,p2)),H(p3,p4))"
        );
        assert_eq!(shape(&preset("main-left", panes(1)).unwrap()), "p0");
    }

    #[test]
    fn even_splits_give_equal_space() {
        // First pane gets 1/3, then the rest split 1/2: each ends up with a third.
        let Node::Split { ratio, after, .. } = preset("even-vertical", panes(3)).unwrap() else {
            panic!()
        };
        assert!((ratio - 1.0 / 3.0).abs() < 1e-9);
        let Node::Split { ratio, .. } = *after else {
            panic!()
        };
        assert!((ratio - 0.5).abs() < 1e-9);
    }

    #[test]
    fn main_size_and_errors() {
        let Node::Split { ratio, .. } = preset("main-left 70", panes(2)).unwrap() else {
            panic!()
        };
        assert!((ratio - 0.7).abs() < 1e-9);
        assert!(preset("tiled 50", panes(2)).is_err());
        assert!(preset("main-left 0", panes(2)).is_err());
        assert!(
            preset("grid", panes(2))
                .unwrap_err()
                .to_string()
                .contains("unknown layout")
        );
    }
}
