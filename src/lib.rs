use std::collections::{HashMap, HashSet};
use std::path::Path;
use walkdir::{DirEntry, WalkDir};

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct FileKey {
    pub dev: u64,
    pub ino: u64,
}

#[cfg(unix)]
fn file_key_from_path(path: &Path) -> Option<FileKey> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| FileKey {
        dev: m.dev(),
        ino: m.ino(),
    })
}

#[cfg(windows)]
fn file_key_from_path(path: &Path) -> Option<FileKey> {
    win_file_id::get_file_id(path).ok().map(|id| {
        let (dev, ino) = match id {
            win_file_id::FileId::LowRes {
                volume_serial_number,
                file_index,
            } => (volume_serial_number as u64, file_index),
            win_file_id::FileId::HighRes {
                volume_serial_number,
                file_id,
            } => {
                let ino64 = (file_id as u64) ^ ((file_id >> 64) as u64);
                (volume_serial_number, ino64)
            }
        };
        FileKey { dev, ino }
    })
}

#[cfg(not(any(unix, windows)))]
fn file_key_from_path(_path: &Path) -> Option<FileKey> {
    None
}

fn is_symlink(entry: &DirEntry) -> bool {
    entry.path_is_symlink()
}

fn path_ancestors(path: &Path) -> Vec<std::path::PathBuf> {
    let mut ancestors = Vec::new();
    let mut current = path.to_path_buf();
    while current.pop() {
        if !current.as_os_str().is_empty() {
            ancestors.push(current.clone());
        }
    }
    ancestors
}

pub type FileEntry = (std::path::PathBuf, u64, FileKey);

pub fn index_directory(root: &Path) -> (Vec<FileEntry>, HashMap<std::path::PathBuf, u64>) {
    let mut files: Vec<FileEntry> = Vec::new();
    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_symlink(e));

    for entry in walker.filter_map(Result::ok) {
        let path = entry.path().to_path_buf();
        if entry.file_type().is_file() {
            if let Ok(meta) = entry.metadata() {
                let size = meta.len();
                if let Some(key) = file_key_from_path(&path) {
                    files.push((path, size, key));
                }
            }
        }
    }

    let mut seen: HashSet<FileKey> = HashSet::new();
    let mut folder_sizes: HashMap<std::path::PathBuf, u64> = HashMap::new();
    let root_buf = root.to_path_buf();

    for (path, size, key) in &files {
        if seen.contains(key) {
            continue;
        }
        seen.insert(*key);
        *folder_sizes.entry(root_buf.clone()).or_insert(0) += size;
        for ancestor in path_ancestors(path) {
            if ancestor != root_buf && ancestor.starts_with(root) {
                *folder_sizes.entry(ancestor).or_insert(0) += size;
            }
        }
    }

    (files, folder_sizes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn path_ancestors_works() {
        let p = PathBuf::from("/a/b/c/file.txt");
        let a = path_ancestors(&p);
        assert!(a.iter().any(|x| x == PathBuf::from("/a/b/c")));
        assert!(a.iter().any(|x| x == PathBuf::from("/a/b")));
        assert!(a.iter().any(|x| x == PathBuf::from("/a")));
    }
}
