//! Preset layouts, and turning a project into a Rex `session.create` request.

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::project::{Direction, Node, Project, Window};

const TERMINAL: &str = "com.superlogical.terminal.shell";

pub const PRESETS: &[&str] = &[
    "even-horizontal",
    "even-vertical",
    "main-left",
    "main-top",
    "tiled",
];

/// Build a split tree from a preset name, such as `main-left` or `main-left 70`.
pub fn preset(spec: &str, mut panes: Vec<Node>) -> Result<Node> {
    let mut words = spec.split_whitespace();
    let name = words.next().unwrap_or("");
    let main_pct = match words.next() {
        None => None,
        Some(w) => match w.trim_end_matches('%').parse::<f64>() {
            Ok(n) if n > 0.0 && n < 100.0 => Some(n / 100.0),
            _ => bail!("layout `{spec}`: the size must be a percentage between 1 and 99"),
        },
    };
    if words.next().is_some() {
        bail!("layout `{spec}` has too many words");
    }
    if main_pct.is_some() && !name.starts_with("main-") {
        bail!("layout `{name}` doesn't take a size; only main-left and main-top do");
    }
    Ok(match name {
        "even-horizontal" => even(panes, Direction::Horizontal),
        "even-vertical" => even(panes, Direction::Vertical),
        "main-left" | "main-top" => {
            let (dir, rest_dir) = if name == "main-left" {
                (Direction::Horizontal, Direction::Vertical)
            } else {
                (Direction::Vertical, Direction::Horizontal)
            };
            let main = panes.remove(0);
            if panes.is_empty() {
                main
            } else {
                Node::Split {
                    direction: dir,
                    ratio: main_pct.unwrap_or(0.6),
                    before: Box::new(main),
                    after: Box::new(even(panes, rest_dir)),
                }
            }
        }
        "tiled" => {
            let cols = (panes.len() as f64).sqrt().ceil() as usize;
            let mut rows = Vec::new();
            while !panes.is_empty() {
                let row: Vec<Node> = panes.drain(..cols.min(panes.len())).collect();
                rows.push(even(row, Direction::Horizontal));
            }
            even(rows, Direction::Vertical)
        }
        other => bail!(
            "unknown layout `{other}` (expected one of: {})",
            PRESETS.join(", ")
        ),
    })
}

/// Equal-sized panes in a row or column.
fn even(mut nodes: Vec<Node>, direction: Direction) -> Node {
    let n = nodes.len();
    let first = nodes.remove(0);
    if n == 1 {
        return first;
    }
    Node::Split {
        direction,
        ratio: 1.0 / n as f64,
        before: Box::new(first),
        after: Box::new(even(nodes, direction)),
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
        Node::Pane(p) => {
            let mut options = json!({ "cwd": p.cwd });
            if !p.typed.is_empty() {
                options["initial_input"] =
                    json!(p.typed.iter().map(|c| format!("{c}\n")).collect::<String>());
            }
            if let Some(cmd) = &p.run {
                options["command"] = json!(cmd);
            }
            if p.keep_open {
                options["exit"] = json!({ "on_completion": false });
            }
            let mut block = json!({ "flavor": TERMINAL, "options": options });
            if let Some(label) = &p.label {
                block["label"] = json!(label);
            }
            json!({ "block": block })
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Pane;

    fn panes(n: usize) -> Vec<Node> {
        (0..n)
            .map(|i| {
                Node::Pane(Pane {
                    typed: vec![format!("p{i}")],
                    ..Default::default()
                })
            })
            .collect()
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
