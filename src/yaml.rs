//! Pure helpers for reading and combining YAML values.

use anyhow::{Result, bail};
use serde_yaml::{Mapping, Value};

/// A single value as text.
pub fn scalar(v: &Value, what: &str) -> Result<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => bail!("`{what}` must be a single value"),
    }
}

/// Commands: nothing, one value, or a list.
pub fn commands(v: Option<&Value>, what: &str) -> Result<Vec<String>> {
    match v {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Sequence(items)) => items.iter().map(|i| scalar(i, what)).collect(),
        Some(v) => Ok(vec![scalar(v, what)?]),
    }
}

pub fn mapping(v: Value, what: &str) -> Result<Mapping> {
    match v {
        Value::Mapping(m) => Ok(m),
        _ => bail!("{what} must be a mapping"),
    }
}

/// A mapping that may be left out, which counts as empty.
pub fn optional_mapping(v: Option<Value>, what: &str) -> Result<Mapping> {
    v.map_or_else(|| Ok(Mapping::new()), |v| mapping(v, what))
}

pub fn check_keys(m: &Mapping, allowed: &[&str], what: &str) -> Result<()> {
    m.keys().try_for_each(|k| {
        let k = scalar(k, "key")?;
        if allowed.contains(&k.as_str()) {
            Ok(())
        } else {
            bail!(
                "unknown key `{k}` in {what} (expected one of: {})",
                allowed.join(", ")
            )
        }
    })
}

/// `m` without `key`, and the value `key` had.
pub fn split(m: &Mapping, key: &str) -> (Mapping, Option<Value>) {
    let rest = m
        .iter()
        .filter(|(k, _)| k.as_str() != Some(key))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    (rest, m.get(key).cloned())
}

/// Like [`split`], for a value that may not be a mapping.
pub fn split_value(v: &Value, key: &str) -> (Value, Option<Value>) {
    match v {
        Value::Mapping(m) if m.contains_key(key) => {
            let (rest, taken) = split(m, key);
            (Value::Mapping(rest), taken)
        }
        v => (v.clone(), None),
    }
}

/// `top` over `base`: its keys replace the base's, except `vars` and
/// `layouts`, which are merged one entry at a time.
pub fn merge(base: &Mapping, top: &Mapping) -> Mapping {
    top.iter().fold(base.clone(), |acc, (k, v)| {
        let value = match (k.as_str(), acc.get(k), v) {
            (Some("vars" | "layouts"), Some(Value::Mapping(old)), Value::Mapping(new)) => {
                Value::Mapping(overlay(old, new))
            }
            _ => v.clone(),
        };
        with(acc, k.clone(), value)
    })
}

/// Every entry of `top` over `base`, one level deep.
pub fn overlay(base: &Mapping, top: &Mapping) -> Mapping {
    top.iter()
        .fold(base.clone(), |acc, (k, v)| with(acc, k.clone(), v.clone()))
}

/// `m` with one more entry. An existing key keeps its place.
fn with(mut m: Mapping, k: Value, v: Value) -> Mapping {
    m.insert(k, v);
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn y(s: &str) -> Mapping {
        serde_yaml::from_str(s).unwrap()
    }

    #[test]
    fn merge_replaces_keys_and_merges_vars_and_layouts() {
        let base = y("name: a\nvars: {x: 1, y: 2}\nlayouts: {full: 1}\nwindows: [w]");
        let top = y("name: b\nvars: {y: 3}\nlayouts: {small: 2}\nwindows: [v]");
        let merged = merge(&base, &top);
        assert_eq!(
            merged,
            y("name: b\nvars: {x: 1, y: 3}\nlayouts: {full: 1, small: 2}\nwindows: [v]")
        );
        // The inputs are left as they were.
        assert_eq!(
            base,
            y("name: a\nvars: {x: 1, y: 2}\nlayouts: {full: 1}\nwindows: [w]")
        );
    }

    #[test]
    fn split_takes_a_key_out() {
        let (rest, taken) = split(&y("a: 1\nb: 2"), "a");
        assert_eq!((rest, taken), (y("b: 2"), Some(Value::from(1))));
        let (rest, taken) = split_value(&Value::from("text"), "a");
        assert_eq!((rest, taken), (Value::from("text"), None));
    }
}
