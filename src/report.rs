//! The text rexinator prints. Pure: every function returns a string.

use crate::style::{BOLD, CYAN, DIM, GREEN, RED, RESET, YELLOW};

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

pub fn started(name: &str, windows: usize) -> String {
    format!(
        "{GREEN}Started{RESET} {BOLD}{name}{RESET} {DIM}({}){RESET}",
        plural(windows, "window")
    )
}

pub fn added(name: &str, windows: usize) -> String {
    format!(
        "{GREEN}Added{RESET} {} from {BOLD}{name}{RESET} to this session",
        plural(windows, "window")
    )
}

pub fn already_running(name: &str) -> String {
    format!("{BOLD}{name}{RESET} is already running")
}

pub fn not_running(name: &str) -> String {
    format!("{BOLD}{name}{RESET} isn't running")
}

pub fn stopped(name: &str) -> String {
    format!("{YELLOW}Stopped{RESET} {BOLD}{name}{RESET}")
}

pub fn nothing_running() -> String {
    format!("{DIM}No projects are running{RESET}")
}

pub fn no_projects(config_dir: &str) -> String {
    format!("No projects in {config_dir}. Create one with {BOLD}rexinator new NAME{RESET}.")
}

/// `check`'s lines for one project: ok, or each error and warning.
pub fn checked(c: &crate::plan::Checked) -> String {
    let name = format!("{BOLD}{}{RESET}", c.name);
    let ok = c.errors.is_empty().then(|| {
        let layouts = match c.layouts {
            0 => String::new(),
            n => format!(" {DIM}({}){RESET}", plural(n, "layout")),
        };
        format!("{GREEN}ok{RESET}      {name}{layouts}\n")
    });
    let errors = c
        .errors
        .iter()
        .map(|e| format!("{RED}error{RESET}   {name}: {e}\n"));
    let warnings = c
        .warnings
        .iter()
        .map(|w| format!("{YELLOW}warning{RESET} {name}: {w}\n"));
    ok.into_iter().chain(errors).chain(warnings).collect()
}

/// The last line of a `check` that found no errors.
pub fn check_summary(checked: usize) -> String {
    format!("{DIM}Checked {}{RESET}", plural(checked, "project"))
}

pub fn warning(msg: &str) -> String {
    format!("{BOLD}{YELLOW}warning:{RESET} {msg}")
}

pub fn error(msg: &str) -> String {
    format!("{BOLD}{RED}error:{RESET} {msg}")
}

/// The `list` table. Cells are padded before they're colored, so escape
/// codes don't count toward the widths, and empty cells at the end of a row
/// are left out, so lines don't end in spaces.
pub fn table(rows: &[[String; 4]]) -> String {
    const HEADER: [&str; 4] = ["PROJECT", "SESSION", "LAYOUTS", "STATUS"];
    let widths: Vec<usize> = (0..3)
        .map(|i| {
            rows.iter()
                .map(|r| r[i].len())
                .chain([HEADER[i].len()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: &[&str], colors: [&str; 4]| -> String {
        let used = cells
            .iter()
            .rposition(|c| !c.is_empty())
            .map_or(0, |i| i + 1);
        (0..used)
            .map(|i| {
                let width = if i + 1 == used { 0 } else { widths[i] };
                format!("{}{:width$}{RESET}", colors[i], cells[i])
            })
            .collect::<Vec<_>>()
            .join("  ")
    };
    std::iter::once(line(&HEADER, [BOLD; 4]))
        .chain(rows.iter().map(|r| {
            let cells: Vec<&str> = r.iter().map(String::as_str).collect();
            line(&cells, [CYAN, "", DIM, status_color(&r[3])])
        }))
        .map(|l| l + "\n")
        .collect()
}

fn status_color(status: &str) -> &'static str {
    if status.starts_with("running") {
        GREEN
    } else if status.starts_with("invalid") {
        RED
    } else {
        DIM
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without color codes.
    fn plain(s: &str) -> String {
        let re = |s: &str| {
            s.split('\x1b')
                .enumerate()
                .map(|(i, part)| {
                    if i == 0 {
                        part
                    } else {
                        part.split_once('m').map_or(part, |(_, rest)| rest)
                    }
                })
                .collect::<String>()
        };
        re(s)
    }

    #[test]
    fn messages() {
        assert_eq!(plain(&started("shop", 1)), "Started shop (1 window)");
        assert_eq!(
            plain(&added("shop", 3)),
            "Added 3 windows from shop to this session"
        );
    }

    #[test]
    fn check_lines() {
        let c = crate::plan::Checked {
            name: "shop".into(),
            layouts: 2,
            errors: vec![],
            warnings: vec!["missing".into()],
        };
        assert_eq!(
            plain(&checked(&c)),
            "ok      shop (2 layouts)\nwarning shop: missing\n"
        );
        let c = crate::plan::Checked {
            errors: vec!["bad".into()],
            warnings: vec![],
            ..c
        };
        assert_eq!(plain(&checked(&c)), "error   shop: bad\n");
        assert_eq!(plain(&check_summary(3)), "Checked 3 projects");
    }

    #[test]
    fn table_aligns_columns_and_trims_rows() {
        let rows = [
            [
                "shop".to_string(),
                "shop".into(),
                "full*".into(),
                "running".into(),
            ],
            [
                "longer-name".to_string(),
                "x".into(),
                String::new(),
                String::new(),
            ],
        ];
        assert_eq!(
            plain(&table(&rows)),
            "PROJECT      SESSION  LAYOUTS  STATUS\n\
             shop         shop     full*    running\n\
             longer-name  x\n"
        );
    }
}
