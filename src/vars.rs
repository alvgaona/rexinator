//! `${name}` substitution and variable defaults. Pure.

use anyhow::{Context, Result, anyhow, bail};
use serde_yaml::Value;
use std::collections::HashMap;

use crate::yaml::scalar;

/// Where a variable's value comes from.
pub type Lookup<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// Replace `${name}` and `${name:-default}`. `$$` is a literal `$`. A bare
/// `$NAME` is left alone so the shell can expand it.
pub fn substitute(s: &str, lookup: &Lookup) -> Result<String> {
    expand(s, s, lookup)
}

fn expand(rest: &str, whole: &str, lookup: &Lookup) -> Result<String> {
    let Some(i) = rest.find('$') else {
        return Ok(rest.to_string());
    };
    let (value, tail) = reference(&rest[i + 1..], whole, lookup)?;
    Ok(format!(
        "{}{value}{}",
        &rest[..i],
        expand(tail, whole, lookup)?
    ))
}

/// What follows a `$`: the text it stands for, and the rest of the string.
fn reference<'s>(after: &'s str, whole: &str, lookup: &Lookup) -> Result<(String, &'s str)> {
    if let Some(tail) = after.strip_prefix('$') {
        return Ok(("$".into(), tail));
    }
    let Some(inner) = after.strip_prefix('{') else {
        return Ok(("$".into(), after));
    };
    let end = inner
        .find('}')
        .ok_or_else(|| anyhow!("unclosed `${{` in {whole:?}"))?;
    let (name, default) = match inner[..end].split_once(":-") {
        Some((name, default)) => (name, Some(default)),
        None => (&inner[..end], None),
    };
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("bad variable name `{name}` in {whole:?}");
    }
    let value = lookup(name)
        .or_else(|| default.map(String::from))
        .ok_or_else(|| {
            anyhow!("unknown variable `{name}`: add it to `vars`, or pass {name}=VALUE")
        })?;
    Ok((value, &inner[end + 1..]))
}

/// Variable values: defaults from `vars:`, replaced by command-line
/// overrides. A default can use earlier variables and `env`.
pub fn resolve(
    defs: Option<&Value>,
    overrides: &HashMap<String, String>,
    env: &Lookup,
) -> Result<HashMap<String, String>> {
    let Some(defs) = defs else {
        return Ok(HashMap::new());
    };
    let Value::Mapping(defs) = defs else {
        bail!("`vars` must be a mapping of names to defaults")
    };
    defs.iter().try_fold(
        HashMap::new(),
        |known, (k, v)| -> Result<HashMap<String, String>> {
            let name = scalar(k, "variable name")?;
            let value = match overrides.get(&name) {
                Some(value) => value.clone(),
                None => {
                    let raw = if v.is_null() {
                        String::new()
                    } else {
                        scalar(v, &name)?
                    };
                    let lookup = |n: &str| {
                        overrides
                            .get(n)
                            .or_else(|| known.get(n))
                            .cloned()
                            .or_else(|| env(n))
                    };
                    substitute(&raw, &lookup).with_context(|| format!("in variable `{name}`"))?
                }
            };
            Ok(with(known, name, value))
        },
    )
}

fn with(mut m: HashMap<String, String>, k: String, v: String) -> HashMap<String, String> {
    m.insert(k, v);
    m
}

/// `v` with variables substituted in every string, mapping keys included.
pub fn substitute_value(v: Value, lookup: &Lookup) -> Result<Value> {
    Ok(match v {
        Value::String(s) => Value::String(substitute(&s, lookup)?),
        Value::Sequence(items) => Value::Sequence(
            items
                .into_iter()
                .map(|i| substitute_value(i, lookup))
                .collect::<Result<_>>()?,
        ),
        Value::Mapping(m) => Value::Mapping(
            m.into_iter()
                .map(|(k, v)| Ok((substitute_value(k, lookup)?, substitute_value(v, lookup)?)))
                .collect::<Result<_>>()?,
        ),
        other => other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn sub(s: &str, pairs: &[(&str, &str)]) -> Result<String> {
        let m = map(pairs);
        substitute(s, &|n| m.get(n).cloned())
    }

    #[test]
    fn substitutes_variables() {
        assert_eq!(sub("a-${x}-b", &[("x", "1")]).unwrap(), "a-1-b");
        assert_eq!(sub("${y:-dflt}", &[]).unwrap(), "dflt");
        assert_eq!(sub("$$HOME and $PATH", &[]).unwrap(), "$HOME and $PATH");
        assert_eq!(sub("${a}${b}", &[("a", "1"), ("b", "2")]).unwrap(), "12");
        assert!(
            sub("${nope}", &[])
                .unwrap_err()
                .to_string()
                .contains("unknown variable `nope`")
        );
        assert!(sub("${open", &[]).is_err());
        assert!(sub("${bad name}", &[]).is_err());
    }

    #[test]
    fn defaults_use_earlier_vars_overrides_and_env() {
        let defs: Value =
            serde_yaml::from_str("{port: 3000, url: 'http://x:${port}', home: '${HOME}'}").unwrap();
        let env = |n: &str| (n == "HOME").then(|| "/h".to_string());
        let vars = resolve(Some(&defs), &map(&[("port", "9")]), &env).unwrap();
        assert_eq!(
            vars,
            map(&[("port", "9"), ("url", "http://x:9"), ("home", "/h")])
        );
    }
}
