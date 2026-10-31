//! The decisions behind each command, as pure functions: given what Rex and
//! the state file report, what should happen next. The commands carry them out.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::layout;
use crate::model::Project;
use crate::project::Request;
use crate::rex::{CreatedWindow, Session};
use crate::state::Started;

/// A project ready to start, with what it was loaded from.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub project: Project,
    /// The project file, as an absolute path.
    pub path: PathBuf,
    pub request: Request,
}

/// What `start` does.
#[derive(Debug, Clone, PartialEq)]
pub enum Start {
    /// The session is already running: switch to it.
    Switch(String),
    /// Create the session with this `session.create` request.
    Create(Value),
}

pub fn start(project: &Project, live: &[Session]) -> Start {
    match session_for(live, &project.name) {
        Some(s) => Start::Switch(s.session_id.clone()),
        None => Start::Create(layout::session_request(project)),
    }
}

/// The running session with this label.
pub fn session_for<'a>(live: &'a [Session], label: &str) -> Option<&'a Session> {
    live.iter().find(|s| s.label == label)
}

fn is_live(live: &[Session], session_id: &str) -> bool {
    live.iter().any(|s| s.session_id == session_id)
}

/// Where to put the focus once the windows exist.
#[derive(Debug, Clone, PartialEq)]
pub enum Focus {
    Block(String),
    Window(String),
}

/// The startup pane if there is one, else the startup window.
pub fn focus(project: &Project, windows: &[CreatedWindow]) -> Option<Focus> {
    let window = windows.get(project.startup_window)?;
    Some(
        match project.startup_pane.and_then(|i| window.block_ids.get(i)) {
            Some(block) => Focus::Block(block.clone()),
            None => Focus::Window(window.window_id.clone()),
        },
    )
}

/// Folders the project needs that don't exist, sorted. Rex silently falls
/// back to $HOME for a missing folder, so `start` checks first.
pub fn missing_dirs(project: &Project, is_dir: impl Fn(&Path) -> bool) -> Vec<String> {
    std::iter::once(project.root.as_path())
        .chain(
            project
                .windows
                .iter()
                .flat_map(|w| w.layout.panes())
                .map(|p| p.cwd.as_path()),
        )
        .filter(|d| !is_dir(d))
        .map(|d| d.display().to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The state record for a session `start` just created.
pub fn record(loaded: &Loaded, session_id: &str) -> Started {
    Started {
        session_id: session_id.to_string(),
        label: loaded.project.name.clone(),
        project: loaded.path.clone(),
        vars: loaded.request.vars.clone(),
        server: loaded.project.server.clone(),
        layout: loaded.project.layout.clone(),
    }
}

/// The project with the command line's --name and --server applied.
pub fn with_overrides(project: Project, name: Option<&str>, server: Option<&str>) -> Project {
    Project {
        name: name.map_or(project.name, String::from),
        server: server.map(String::from).or(project.server),
        ..project
    }
}

/// The server to use: the command line's, else the project's.
pub fn server(cli: Option<&str>, own: Option<&str>) -> Option<String> {
    cli.or(own).map(String::from)
}

/// Recorded sessions that are still running, given each one's live sessions.
pub fn recorded_alive<'a>(
    records: &'a [Started],
    live_for: impl Fn(&Started) -> &'a [Session],
) -> Vec<&'a Started> {
    records
        .iter()
        .filter(|r| is_live(live_for(r), &r.session_id))
        .collect()
}

/// Labels of a project's running sessions: its default one, then any
/// recorded ones started with other variables or layouts.
pub fn running<'a>(
    project: &'a Project,
    path: &Path,
    live: &[Session],
    records: &'a [Started],
) -> Vec<&'a str> {
    let default = session_for(live, &project.name).map(|_| project.name.as_str());
    let recorded = records
        .iter()
        .filter(|r| r.project == path && is_live(live, &r.session_id))
        .map(|r| r.label.as_str());
    default
        .into_iter()
        .chain(recorded)
        .fold(Vec::new(), |seen, label| {
            if seen.contains(&label) {
                seen
            } else {
                seen.into_iter().chain([label]).collect()
            }
        })
}

/// The STATUS cell for `list`.
pub fn status(project: &Project, running: &[&str]) -> String {
    let state = match running {
        [] => String::new(),
        [only] if *only == project.name => "running".to_string(),
        many => format!("running: {}", many.join(", ")),
    };
    match &project.server {
        Some(s) => format!("{state} (on {s})").trim().to_string(),
        None => state,
    }
}

/// The LAYOUTS cell for `list`: the default one is marked with `*`.
pub fn layouts(project: &Project) -> String {
    project
        .layouts
        .iter()
        .map(|l| {
            if project.layout.as_ref() == Some(l) {
                format!("{l}*")
            } else {
                l.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One `list` row: PROJECT, SESSION, LAYOUTS, STATUS.
pub fn list_row(name: &str, project: Option<(&Project, &[&str])>) -> [String; 4] {
    match project {
        Some((p, running)) => [
            name.to_string(),
            p.name.clone(),
            layouts(p),
            status(p, running),
        ],
        None => [
            name.to_string(),
            "-".into(),
            String::new(),
            "invalid (see `rexinator debug`)".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Hooks, Node, Pane, Window};
    use std::collections::HashMap;

    fn project(name: &str) -> Project {
        Project {
            name: name.into(),
            root: "/p".into(),
            server: None,
            hooks: Hooks::default(),
            windows: vec![
                Window {
                    name: "a".into(),
                    layout: Node::Pane(Pane {
                        cwd: "/p".into(),
                        ..Default::default()
                    }),
                },
                Window {
                    name: "b".into(),
                    layout: Node::Pane(Pane {
                        cwd: "/p/logs".into(),
                        ..Default::default()
                    }),
                },
            ],
            startup_window: 1,
            startup_pane: None,
            layout: None,
            layouts: vec![],
        }
    }

    fn session(id: &str, label: &str) -> Session {
        Session {
            session_id: id.into(),
            label: label.into(),
        }
    }

    fn record(id: &str, label: &str, path: &str) -> Started {
        Started {
            session_id: id.into(),
            label: label.into(),
            project: path.into(),
            vars: HashMap::new(),
            server: None,
            layout: None,
        }
    }

    #[test]
    fn start_switches_to_a_running_session_or_creates_one() {
        let p = project("shop");
        assert_eq!(
            start(&p, &[session("s1", "shop")]),
            Start::Switch("s1".into())
        );
        let Start::Create(request) = start(&p, &[session("s1", "other")]) else {
            panic!()
        };
        assert_eq!(request["label"], "shop");
    }

    #[test]
    fn focus_prefers_the_startup_pane() {
        let windows = vec![
            CreatedWindow {
                window_id: "w0".into(),
                block_ids: vec!["b0".into()],
            },
            CreatedWindow {
                window_id: "w1".into(),
                block_ids: vec!["b1".into(), "b2".into()],
            },
        ];
        let p = project("x");
        assert_eq!(focus(&p, &windows), Some(Focus::Window("w1".into())));
        let p = Project {
            startup_pane: Some(1),
            ..p
        };
        assert_eq!(focus(&p, &windows), Some(Focus::Block("b2".into())));
        assert_eq!(focus(&p, &[]), None);
    }

    #[test]
    fn missing_dirs_are_sorted_and_unique() {
        let p = project("x");
        assert_eq!(missing_dirs(&p, |_| true), Vec::<String>::new());
        assert_eq!(missing_dirs(&p, |_| false), ["/p", "/p/logs"]);
    }

    #[test]
    fn overrides_replace_name_and_server() {
        let p = with_overrides(project("x"), Some("y"), Some("box"));
        assert_eq!((p.name.as_str(), p.server.as_deref()), ("y", Some("box")));
        let p = with_overrides(p, None, None);
        assert_eq!((p.name.as_str(), p.server.as_deref()), ("y", Some("box")));
    }

    #[test]
    fn running_and_status() {
        let p = project("shop");
        let path = Path::new("/c/shop.yml");
        let live = [session("s1", "shop"), session("s2", "shop-prod")];
        let records = [
            record("s1", "shop", "/c/shop.yml"),
            record("s2", "shop-prod", "/c/shop.yml"),
            record("s3", "shop-gone", "/c/shop.yml"),
            record("s4", "other", "/c/other.yml"),
        ];
        let r = running(&p, path, &live, &records);
        assert_eq!(r, ["shop", "shop-prod"]);
        assert_eq!(status(&p, &r), "running: shop, shop-prod");
        assert_eq!(status(&p, &["shop"]), "running");
        assert_eq!(status(&p, &[]), "");
        let remote = Project {
            server: Some("box".into()),
            ..p
        };
        assert_eq!(status(&remote, &[]), "(on box)");
        assert_eq!(
            recorded_alive(&records, |_| &live)
                .iter()
                .map(|r| r.session_id.as_str())
                .collect::<Vec<_>>(),
            ["s1", "s2"]
        );
    }

    #[test]
    fn list_rows() {
        let p = Project {
            layouts: vec!["full".into(), "focus".into()],
            layout: Some("full".into()),
            ..project("shop")
        };
        assert_eq!(
            list_row("shop", Some((&p, &["shop"]))),
            ["shop", "shop", "full* focus", "running"]
        );
        assert_eq!(list_row("bad", None)[3], "invalid (see `rexinator debug`)");
    }
}
