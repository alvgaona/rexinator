//! The data a project file describes: plain values, no behavior beyond
//! walking and transforming the pane tree.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// Session label. Also how `start` finds a session that's already running.
    pub name: String,
    pub root: PathBuf,
    /// The Rex server to use, as `rex -S` takes it. None means the local one.
    pub server: Option<String>,
    pub hooks: Hooks,
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

/// Shell commands run in the project's root around a session's life.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hooks {
    /// Runs on every `start`, before anything else.
    pub on_start: Vec<String>,
    /// Runs only when the session is created.
    pub on_first_start: Vec<String>,
    /// Runs only when the session was already running.
    pub on_restart: Vec<String>,
    pub on_stop: Vec<String>,
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

/// A list with at least one item, so code that takes one never has to
/// handle the empty case.
#[derive(Debug, Clone, PartialEq)]
pub struct NonEmpty<T> {
    pub head: T,
    pub tail: Vec<T>,
}

impl<T> NonEmpty<T> {
    pub fn one(head: T) -> Self {
        NonEmpty { head, tail: vec![] }
    }

    /// None for an empty list.
    pub fn from_vec(items: Vec<T>) -> Option<Self> {
        let (first, tail) = split_at(items, 1);
        first.into_iter().next().map(|head| NonEmpty { head, tail })
    }

    pub fn len(&self) -> usize {
        1 + self.tail.len()
    }

    pub fn map<U>(self, f: impl Fn(T) -> U) -> NonEmpty<U> {
        NonEmpty {
            head: f(self.head),
            tail: self.tail.into_iter().map(f).collect(),
        }
    }

    /// The items in groups of `size` (at least 1), in order. Only the last
    /// group can be smaller.
    pub fn chunks(self, size: usize) -> NonEmpty<NonEmpty<T>> {
        let (now, later) = split_at(self.tail, size.max(1) - 1);
        let first = NonEmpty {
            head: self.head,
            tail: now,
        };
        match NonEmpty::from_vec(later) {
            None => NonEmpty::one(first),
            Some(rest) => {
                let rest = rest.chunks(size);
                NonEmpty {
                    head: first,
                    tail: std::iter::once(rest.head).chain(rest.tail).collect(),
                }
            }
        }
    }
}

/// The first `n` items, and the rest.
fn split_at<T>(items: Vec<T>, n: usize) -> (Vec<T>, Vec<T>) {
    let (front, back): (Vec<_>, Vec<_>) = items.into_iter().enumerate().partition(|(i, _)| *i < n);
    let drop_index = |part: Vec<(usize, T)>| part.into_iter().map(|(_, item)| item).collect();
    (drop_index(front), drop_index(back))
}

impl Node {
    pub fn split(direction: Direction, ratio: f64, before: Node, after: Node) -> Node {
        Node::Split {
            direction,
            ratio,
            before: Box::new(before),
            after: Box::new(after),
        }
    }

    /// Panes in layout order: each split's `before` side first. Rex returns
    /// block IDs in the same order.
    pub fn panes(&self) -> Vec<&Pane> {
        match self {
            Node::Pane(p) => vec![p],
            Node::Split { before, after, .. } => {
                before.panes().into_iter().chain(after.panes()).collect()
            }
        }
    }

    /// The same tree with `f` applied to every pane.
    pub fn map_panes(self, f: &impl Fn(Pane) -> Pane) -> Node {
        match self {
            Node::Pane(p) => Node::Pane(f(p)),
            Node::Split {
                direction,
                ratio,
                before,
                after,
            } => Node::split(direction, ratio, before.map_panes(f), after.map_panes(f)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_empty_lists() {
        assert_eq!(NonEmpty::<i32>::from_vec(vec![]), None);
        let list = NonEmpty::from_vec(vec![1, 2, 3, 4, 5]).unwrap();
        assert_eq!(list.len(), 5);
        assert_eq!(list.clone().map(|n| n * 10).tail, [20, 30, 40, 50]);
        let groups: Vec<Vec<i32>> = {
            let c = list.chunks(2);
            std::iter::once(c.head)
                .chain(c.tail)
                .map(|g| std::iter::once(g.head).chain(g.tail).collect())
                .collect()
        };
        assert_eq!(groups, [vec![1, 2], vec![3, 4], vec![5]]);
        assert_eq!(NonEmpty::one(7).chunks(0).head, NonEmpty::one(7));
    }
}
