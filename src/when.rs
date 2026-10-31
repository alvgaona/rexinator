//! `when:` conditions. Pure.

use anyhow::Result;
use serde_yaml::Value;

use crate::yaml::scalar;

/// Whether a `when` value switches something on. `||` and `&&` combine
/// terms, with `&&` binding tighter. Each term is a comparison (`A == B`,
/// `A != B`) or a value, which is off for false, no, off, 0 and empty; a
/// leading `!` flips a value.
pub fn truthy(v: &Value) -> Result<bool> {
    Ok(match v {
        Value::Bool(b) => *b,
        Value::Null => false,
        v => scalar(v, "when")?
            .split("||")
            .any(|any| any.split("&&").all(term_on)),
    })
}

/// One term of a `when` value: a comparison, or a value with an optional `!`.
fn term_on(term: &str) -> bool {
    compare(term).unwrap_or_else(|| {
        let term = term.trim();
        let (negate, term) = term
            .strip_prefix('!')
            .map_or((false, term), |rest| (true, rest.trim()));
        let on = !matches!(
            term.to_ascii_lowercase().as_str(),
            "" | "false" | "no" | "off" | "0"
        );
        on != negate
    })
}

/// `A == B` or `A != B`, if `s` is a comparison. The first operator wins, and
/// each side is trimmed and may be quoted, so `${env} == "staging"` works.
fn compare(s: &str) -> Option<bool> {
    let (at, equal) = match (s.find("=="), s.find("!=")) {
        (Some(eq), Some(ne)) if ne < eq => (ne, false),
        (Some(eq), _) => (eq, true),
        (None, Some(ne)) => (ne, false),
        (None, None) => return None,
    };
    Some((unquote(&s[..at]) == unquote(&s[at + 2..])) == equal)
}

fn unquote(side: &str) -> &str {
    let side = side.trim();
    ['"', '\'']
        .iter()
        .find_map(|q| side.strip_prefix(*q).and_then(|r| r.strip_suffix(*q)))
        .unwrap_or(side)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(s: &str) -> bool {
        truthy(&Value::String(s.into())).unwrap()
    }

    #[test]
    fn values() {
        assert!(on("yes") && on("1") && on("anything"));
        assert!(!on("false") && !on("No") && !on("off") && !on("0") && !on(""));
        assert!(on("!false") && !on("!true"));
        assert!(truthy(&Value::Bool(true)).unwrap());
        assert!(!truthy(&Value::Null).unwrap());
    }

    #[test]
    fn comparisons() {
        assert!(on("staging == staging"));
        assert!(on("staging == \"staging\""));
        assert!(on("dev != prod"));
        assert!(!on("dev == prod"));
        assert!(!on("prod != prod"));
        assert!(on(" == ''"));
        // The first operator decides: this compares `a` with `b == c`.
        assert!(on("a != b == c"));
    }

    #[test]
    fn and_or() {
        assert!(on("true && yes"));
        assert!(!on("true && false"));
        assert!(on("false || 1"));
        assert!(!on("no || off"));
        assert!(on("dev == dev && !false"));
        assert!(on("a == b || c != d"));
        // && binds tighter: false || (true && true).
        assert!(on("false || true && true"));
        // (true && false) || true.
        assert!(on("true && false || true"));
        assert!(!on("true && false || false"));
    }
}
