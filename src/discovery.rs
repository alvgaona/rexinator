//! Finding project files: in the project you're in, or in the config folder.
//! Pure functions of the environment and a read-only file system.

use anyhow::{Result, anyhow, bail};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::env::{self, Env};
use crate::fs::Fs;
use crate::project::{LOCAL_DIR, LOCAL_NAMES};
use crate::template::Kind;

/// The .rexinator.yml in `dir`, if it has one.
pub fn local_file_in(fs: &Fs, dir: &Path) -> Option<PathBuf> {
    LOCAL_NAMES
        .iter()
        .map(|n| dir.join(n))
        .find(|p| (fs.is_file)(p))
}

/// `dir/NAME.yml` or `dir/NAME.yaml`, if it exists.
pub fn file_named(fs: &Fs, dir: &Path, name: &str) -> Option<PathBuf> {
    ["yml", "yaml"]
        .iter()
        .map(|ext| dir.join(format!("{name}.{ext}")))
        .find(|p| (fs.is_file)(p))
}

/// A project file in the config folder, if there is one by that name.
pub fn config_file(fs: &Fs, env: &Env, name: &str) -> Option<PathBuf> {
    file_named(fs, &env.config_dir, name)
}

/// Project files in a folder, by name, sorted. `.local` override files and
/// files starting with `_` (bases for `extends`) aren't projects.
pub fn projects_in(fs: &Fs, dir: &Path) -> Vec<(String, PathBuf)> {
    (fs.list)(dir)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "yml" || e == "yaml"))
        .filter_map(|p| {
            let stem = p.file_stem()?.to_str()?.to_string();
            (!stem.ends_with(".local") && !stem.starts_with('_')).then_some((stem, p))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The nearest folder, `cwd` or a parent, with a .rexinator.yml or a .rexinator/ folder.
pub fn find_project_dir(fs: &Fs, cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|dir| local_file_in(fs, dir).is_some() || (fs.is_dir)(&dir.join(LOCAL_DIR)))
        .map(Path::to_path_buf)
}

/// A folder's project: its .rexinator.yml, else .rexinator/default.yml, else
/// the only file in .rexinator/.
pub fn folder_project(fs: &Fs, dir: &Path) -> Result<PathBuf> {
    if let Some(p) = local_file_in(fs, dir) {
        return Ok(p);
    }
    let files = projects_in(fs, &dir.join(LOCAL_DIR));
    if let Some((_, p)) = files.iter().find(|(n, _)| n == "default") {
        return Ok(p.clone());
    }
    match files.as_slice() {
        [(_, only)] => Ok(only.clone()),
        [] => bail!("there's no .rexinator.yml in {}", dir.display()),
        many => bail!(
            "this project has several configs; pick one: {}",
            many.iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// A project file by name: this project's .rexinator/ folder first, then the config folder.
pub fn named_file(fs: &Fs, env: &Env, name: &str) -> Option<PathBuf> {
    find_project_dir(fs, &env.cwd)
        .and_then(|d| file_named(fs, &d.join(LOCAL_DIR), name))
        .or_else(|| config_file(fs, env, name))
}

/// Whether a PROJECT argument is a path rather than a name.
fn is_path(name: &str) -> bool {
    name == "." || name.contains('/') || name.ends_with(".yml") || name.ends_with(".yaml")
}

/// The project file a PROJECT argument means. With none, the project you're in.
pub fn find_project(fs: &Fs, env: &Env, name: Option<&str>) -> Result<PathBuf> {
    match name {
        None => {
            let dir = find_project_dir(fs, &env.cwd).ok_or_else(|| {
                anyhow!("no project given, and there's no .rexinator.yml or .rexinator/ folder here or in a parent folder")
            })?;
            folder_project(fs, &dir)
        }
        Some(name) if is_path(name) => {
            let path = env::absolute(&env.cwd, Path::new(name));
            if (fs.is_dir)(&path) {
                folder_project(fs, &path)
            } else {
                Ok(path)
            }
        }
        Some(name) => named_file(fs, env, name).ok_or_else(|| {
            anyhow!(
                "no project `{name}` in this project's .rexinator/ folder or in {} (create it with `rexinator new {name}`)",
                env.config_dir.display()
            )
        }),
    }
}

/// This project's files, then the config folder's, as `list` names them.
pub fn all_projects(fs: &Fs, env: &Env) -> Vec<(String, PathBuf)> {
    let here = find_project_dir(fs, &env.cwd);
    let main = here
        .as_deref()
        .and_then(|d| local_file_in(fs, d))
        .map(|p| ("(here)".to_string(), p));
    let folder = here
        .as_deref()
        .map(|d| projects_in(fs, &d.join(LOCAL_DIR)))
        .unwrap_or_default()
        .into_iter()
        .map(|(n, p)| (format!("{n} (here)"), p));
    main.into_iter()
        .chain(folder)
        .chain(projects_in(fs, &env.config_dir))
        .collect()
}

/// The names `start NAME` accepts, sorted, for shell completion.
pub fn project_names(fs: &Fs, env: &Env) -> Vec<String> {
    find_project_dir(fs, &env.cwd)
        .map(|d| projects_in(fs, &d.join(LOCAL_DIR)))
        .unwrap_or_default()
        .into_iter()
        .chain(projects_in(fs, &env.config_dir))
        .map(|(n, _)| n)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The file `edit` opens, and the template for it if it doesn't exist yet.
pub fn edit_target<'a>(
    fs: &Fs,
    env: &Env,
    name: Option<&'a str>,
    here: bool,
) -> (PathBuf, Kind<'a>) {
    let project_dir = || find_project_dir(fs, &env.cwd);
    match (name, here) {
        // In this project's .rexinator/ folder, or this folder's if it isn't in a project yet.
        (Some(n), true) => {
            let dir = project_dir()
                .unwrap_or_else(|| env.cwd.clone())
                .join(LOCAL_DIR);
            let path = file_named(fs, &dir, n).unwrap_or_else(|| dir.join(format!("{n}.yml")));
            (path, Kind::Folder(n))
        }
        (Some(n), false) => {
            let path =
                named_file(fs, env, n).unwrap_or_else(|| env.config_dir.join(format!("{n}.yml")));
            (path, Kind::Config(n))
        }
        (None, true) => {
            let path = local_file_in(fs, &env.cwd).unwrap_or_else(|| env.cwd.join(LOCAL_NAMES[0]));
            (path, Kind::Local)
        }
        (None, false) => {
            let path = project_dir()
                .and_then(|d| local_file_in(fs, &d))
                .unwrap_or_else(|| env.cwd.join(LOCAL_NAMES[0]));
            (path, Kind::Local)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(cwd: &str) -> Env {
        let vars = HashMap::from([("REXINATOR_CONFIG".to_string(), "/cfg".to_string())]);
        Env::new(PathBuf::from(cwd), vars)
    }

    fn files() -> Fs {
        Fs::memory(&[
            ("/git/app/.rexinator.yml", ""),
            ("/git/app/.rexinator/api.yml", ""),
            ("/git/app/.rexinator/_base.yml", ""),
            ("/git/app/.rexinator/api.local.yml", ""),
            ("/git/app/src/main.rs", ""),
            ("/git/multi/.rexinator/a.yml", ""),
            ("/git/multi/.rexinator/b.yml", ""),
            ("/git/one/.rexinator/only.yaml", ""),
            ("/cfg/shop.yml", ""),
            ("/cfg/api.yml", ""),
            ("/cfg/shop.local.yml", ""),
        ])
    }

    fn path(p: &str) -> PathBuf {
        PathBuf::from(p)
    }

    #[test]
    fn finds_the_project_you_are_in() {
        let fs = files();
        assert_eq!(
            find_project_dir(&fs, Path::new("/git/app/src")),
            Some(path("/git/app"))
        );
        assert_eq!(find_project_dir(&fs, Path::new("/elsewhere")), None);
        assert_eq!(
            find_project(&fs, &env("/git/app/src"), None).unwrap(),
            path("/git/app/.rexinator.yml")
        );
        assert_eq!(
            find_project(&fs, &env("/git/one"), None).unwrap(),
            path("/git/one/.rexinator/only.yaml")
        );
        let err = find_project(&fs, &env("/git/multi"), None).unwrap_err();
        assert!(err.to_string().contains("pick one: a, b"), "{err}");
        assert!(find_project(&fs, &env("/elsewhere"), None).is_err());
    }

    #[test]
    fn names_look_in_the_project_first_then_the_config_folder() {
        let fs = files();
        let in_app = env("/git/app/src");
        assert_eq!(
            find_project(&fs, &in_app, Some("api")).unwrap(),
            path("/git/app/.rexinator/api.yml")
        );
        assert_eq!(
            find_project(&fs, &env("/elsewhere"), Some("api")).unwrap(),
            path("/cfg/api.yml")
        );
        assert_eq!(
            find_project(&fs, &in_app, Some("shop")).unwrap(),
            path("/cfg/shop.yml")
        );
        assert!(find_project(&fs, &in_app, Some("nope")).is_err());
        // Paths are resolved against the working directory.
        assert!(
            find_project(&fs, &env("/git"), Some("one"))
                .unwrap_err()
                .to_string()
                .contains("no project `one`")
        );
        assert_eq!(
            find_project(&fs, &env("/git"), Some("./one")).unwrap(),
            path("/git/one/.rexinator/only.yaml")
        );
    }

    #[test]
    fn lists_projects() {
        let fs = files();
        let names: Vec<String> = all_projects(&fs, &env("/git/app"))
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, ["(here)", "api (here)", "api", "shop"]);
        assert_eq!(project_names(&fs, &env("/git/app")), ["api", "shop"]);
    }

    #[test]
    fn edit_targets() {
        let fs = files();
        let e = env("/git/app/src");
        let (p, k) = edit_target(&fs, &e, Some("worker"), true);
        assert_eq!(
            (p, k),
            (
                path("/git/app/.rexinator/worker.yml"),
                Kind::Folder("worker")
            )
        );
        let (p, _) = edit_target(&fs, &e, Some("api"), false);
        assert_eq!(p, path("/git/app/.rexinator/api.yml"));
        let (p, _) = edit_target(&fs, &e, Some("new"), false);
        assert_eq!(p, path("/cfg/new.yml"));
        let (p, k) = edit_target(&fs, &e, None, false);
        assert_eq!((p, k), (path("/git/app/.rexinator.yml"), Kind::Local));
        let (p, _) = edit_target(&fs, &e, None, true);
        assert_eq!(p, path("/git/app/src/.rexinator.yml"));
    }
}
