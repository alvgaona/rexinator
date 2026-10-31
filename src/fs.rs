//! Read-only file access, passed into the pure code as a record of functions.
//! The shell uses the real file system; tests use files held in memory.

use std::io;
use std::path::{Path, PathBuf};

type Read = Box<dyn Fn(&Path) -> io::Result<String>>;
type Check = Box<dyn Fn(&Path) -> bool>;
type List = Box<dyn Fn(&Path) -> Vec<PathBuf>>;

pub struct Fs {
    pub read: Read,
    pub is_file: Check,
    pub is_dir: Check,
    /// The entries in a folder; empty if it can't be read.
    pub list: List,
}

impl Fs {
    pub fn real() -> Self {
        Fs {
            read: Box::new(|p: &Path| std::fs::read_to_string(p)),
            is_file: Box::new(|p: &Path| p.is_file()),
            is_dir: Box::new(|p: &Path| p.is_dir()),
            list: Box::new(|p: &Path| {
                std::fs::read_dir(p)
                    .map(|entries| entries.filter_map(|e| e.ok().map(|e| e.path())).collect())
                    .unwrap_or_default()
            }),
        }
    }

    /// Files held in memory, by absolute path. Folders are implied by the files in them.
    #[cfg(test)]
    pub fn memory(files: &[(&str, &str)]) -> Self {
        use std::collections::{HashMap, HashSet};
        use std::rc::Rc;

        let files: Rc<HashMap<PathBuf, String>> = Rc::new(
            files
                .iter()
                .map(|(p, text)| (PathBuf::from(p), text.to_string()))
                .collect(),
        );
        let dirs: Rc<HashSet<PathBuf>> = Rc::new(
            files
                .keys()
                .flat_map(|p| p.ancestors().skip(1).map(Path::to_path_buf))
                .collect(),
        );
        let (read, is_file, list) = (files.clone(), files.clone(), files);
        let (is_dir, list_dirs) = (dirs.clone(), dirs);
        Fs {
            read: Box::new(move |p: &Path| {
                read.get(p).cloned().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("{} not found", p.display()),
                    )
                })
            }),
            is_file: Box::new(move |p: &Path| is_file.contains_key(p)),
            is_dir: Box::new(move |p: &Path| is_dir.contains(p)),
            list: Box::new(move |dir: &Path| {
                list.keys()
                    .chain(list_dirs.iter())
                    .filter(|p| p.parent() == Some(dir))
                    .cloned()
                    .collect()
            }),
        }
    }
}
