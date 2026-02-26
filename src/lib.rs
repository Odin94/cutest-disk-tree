use std::collections::{HashMap, HashSet};
use std::path::Path;
use walkdir::{DirEntry, WalkDir};
use jwalk::WalkDir as JwalkDir;
use serde::Serialize;

pub mod db;

#[derive(Clone, Debug, Serialize)]
pub struct ScanProgress {
    pub files_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Serialize)]
pub struct FileKey {
    pub dev: u64,
    pub ino: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileEntrySer {
    pub path: String,
    pub size: u64,
    pub file_key: FileKey,
}

#[derive(Debug, Serialize)]
pub struct ScanResult {
    pub root: String,
    pub files: Vec<FileEntrySer>,
    pub folder_sizes: HashMap<String, u64>,
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

const PROGRESS_INTERVAL: u64 = 200;

pub fn index_directory(root: &Path) -> (Vec<FileEntry>, HashMap<std::path::PathBuf, u64>) {
    index_directory_with_progress(root, |_| {})
}

pub fn index_directory_with_progress<F>(root: &Path, mut progress: F) -> (Vec<FileEntry>, HashMap<std::path::PathBuf, u64>)
where
    F: FnMut(ScanProgress),
{
    progress(ScanProgress {
        files_count: 0,
        current_path: None,
        status: None,
    });

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
                    files.push((path.clone(), size, key));
                    if files.len() as u64 % PROGRESS_INTERVAL == 0 {
                        progress(ScanProgress {
                            files_count: files.len() as u64,
                            current_path: Some(path.to_string_lossy().to_string()),
                            status: None,
                        });
                    }
                }
            }
        }
    }

    progress(ScanProgress {
        files_count: files.len() as u64,
        current_path: None,
        status: None,
    });

    progress(ScanProgress {
        files_count: files.len() as u64,
        current_path: None,
        status: Some("Computing folder sizes…".into()),
    });

    let folder_sizes = aggregate_folder_sizes(root, &files);

    (files, folder_sizes)
}

fn aggregate_folder_sizes(
    root: &Path,
    files: &[FileEntry],
) -> HashMap<std::path::PathBuf, u64> {
    let mut seen: HashSet<FileKey> = HashSet::new();
    let mut folder_sizes: HashMap<std::path::PathBuf, u64> = HashMap::new();
    let root_buf = root.to_path_buf();
    for (path, size, key) in files {
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
    folder_sizes
}

pub fn index_directory_parallel_with_progress<F>(
    root: &Path,
    mut progress: F,
) -> (Vec<FileEntry>, HashMap<std::path::PathBuf, u64>)
where
    F: FnMut(ScanProgress),
{
    progress(ScanProgress {
        files_count: 0,
        current_path: None,
        status: Some("Scanning files…".into()),
    });

    let mut files: Vec<FileEntry> = Vec::new();
    let walk = match JwalkDir::new(root).follow_links(false).try_into_iter() {
        Ok(w) => w,
        Err(_) => {
            progress(ScanProgress {
                files_count: 0,
                current_path: None,
                status: Some("Scan failed (try_into_iter)".into()),
            });
            return (files, HashMap::new());
        }
    };

    for entry in walk.filter_map(Result::ok) {
        if entry.path_is_symlink() || !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = meta.len();
        let key = match file_key_from_path(&path) {
            Some(k) => k,
            None => continue,
        };
        files.push((path.clone(), size, key));
        if files.len() as u64 % PROGRESS_INTERVAL == 0 {
            progress(ScanProgress {
                files_count: files.len() as u64,
                current_path: Some(path.to_string_lossy().to_string()),
                status: None,
            });
        }
    }

    progress(ScanProgress {
        files_count: files.len() as u64,
        current_path: None,
        status: None,
    });

    progress(ScanProgress {
        files_count: files.len() as u64,
        current_path: None,
        status: Some("Computing folder sizes…".into()),
    });

    let folder_sizes = aggregate_folder_sizes(root, &files);

    (files, folder_sizes)
}

pub fn index_directory_serializable(root: &Path) -> Option<ScanResult> {
    let (files, folder_sizes) = index_directory(root);
    to_scan_result(root, &files, &folder_sizes)
}

pub fn to_scan_result(
    root: &Path,
    files: &[FileEntry],
    folder_sizes: &HashMap<std::path::PathBuf, u64>,
) -> Option<ScanResult> {
    let root_str = root.to_string_lossy().to_string();
    let files_ser: Vec<FileEntrySer> = files
        .iter()
        .map(|(p, s, k)| FileEntrySer {
            path: p.to_string_lossy().to_string(),
            size: *s,
            file_key: *k,
        })
        .collect();
    let folder_sizes_ser: HashMap<String, u64> = folder_sizes
        .iter()
        .map(|(p, s)| (p.to_string_lossy().to_string(), *s))
        .collect();
    Some(ScanResult {
        root: root_str,
        files: files_ser,
        folder_sizes: folder_sizes_ser,
    })
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
