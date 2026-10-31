//! ANSI styling for command output, the clap help palette, and `--color`.
//!
//! Output that uses these codes goes through `anstream`, which strips them
//! when the stream isn't a terminal or `--color never` is given.

use clap::builder::styling::{AnsiColor, Effects, Style, Styles};

pub(crate) const BOLD: &str = "\x1b[1m";
pub(crate) const DIM: &str = "\x1b[2m";
pub(crate) const RESET: &str = "\x1b[0m";
pub(crate) const CYAN: &str = "\x1b[36m";
pub(crate) const GREEN: &str = "\x1b[32m";
pub(crate) const YELLOW: &str = "\x1b[33m";
pub(crate) const RED: &str = "\x1b[31m";

pub(crate) const HELP: Styles = Styles::styled()
    .header(
        AnsiColor::Blue
            .on_default()
            .effects(Effects::BOLD.insert(Effects::UNDERLINE)),
    )
    .usage(
        AnsiColor::Blue
            .on_default()
            .effects(Effects::BOLD.insert(Effects::UNDERLINE)),
    )
    .literal(Style::new().effects(Effects::BOLD))
    .placeholder(Style::new())
    .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
    .valid(AnsiColor::Blue.on_default())
    .invalid(AnsiColor::Yellow.on_default().effects(Effects::BOLD));

/// When to colorize output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub(crate) enum Color {
    /// Colorize when stderr and stdout are terminals
    #[default]
    Auto,
    /// Always emit ANSI escapes
    Always,
    /// Never emit ANSI escapes
    Never,
}

impl Color {
    /// Read `--color` straight off the raw arguments.
    ///
    /// clap needs the choice before it renders help or an error, which is
    /// before it has parsed anything, so this can't wait for the parse.
    pub(crate) fn from_raw_args() -> Self {
        Self::from_args(std::env::args().skip(1))
    }

    fn from_args(args: impl Iterator<Item = String>) -> Self {
        let args: Vec<String> = args.collect();
        args.iter()
            .enumerate()
            .find_map(|(i, arg)| match arg.split_once('=') {
                Some(("--color", value)) => Some(value),
                _ if arg == "--color" => args.get(i + 1).map(String::as_str),
                _ => None,
            })
            .and_then(|value| match value {
                "always" => Some(Self::Always),
                "never" => Some(Self::Never),
                "auto" => Some(Self::Auto),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Apply the choice to every stream rexinator writes styled output through.
    ///
    /// clap is told separately, via [`clap::Command::color`], because it owns
    /// its own stream for help and errors.
    pub(crate) fn apply(self) {
        anstream::ColorChoice::write_global(self.into());
    }
}

impl From<Color> for anstream::ColorChoice {
    fn from(color: Color) -> Self {
        match color {
            Color::Auto => Self::Auto,
            Color::Always => Self::Always,
            Color::Never => Self::Never,
        }
    }
}

impl From<Color> for clap::ColorChoice {
    fn from(color: Color) -> Self {
        match color {
            Color::Auto => Self::Auto,
            Color::Always => Self::Always,
            Color::Never => Self::Never,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(args: &[&str]) -> Color {
        Color::from_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn reads_color_from_raw_args() {
        assert_eq!(color(&["start", "--color", "never"]), Color::Never);
        assert_eq!(color(&["--color=always", "list"]), Color::Always);
        assert_eq!(color(&["list"]), Color::Auto);
        assert_eq!(color(&["--color", "rainbow"]), Color::Auto);
    }
}
