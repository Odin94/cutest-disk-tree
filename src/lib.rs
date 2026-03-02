use std::collections::{HashMap, HashSet};
use std::path::Path;
use walkdir::{DirEntry, WalkDir};
use jwalk::WalkDir as JwalkDir;
use serde::Serialize;
use rayon::prelude::*;

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

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct DiskTreeNode {
    pub path: String,
    pub name: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<DiskTreeNode>>,
}

fn path_separator(path: &str) -> char {
    if path.contains('\\') {
        '\\'
    } else {
        '/'
    }
}

fn normalize_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    let sep = path_separator(trimmed);
    let parts: Vec<&str> = trimmed.split(|c| c == '/' || c == '\\').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        trimmed.to_string()
    } else {
        parts.join(&sep.to_string())
    }
}

pub fn parent_dir(path: &str) -> String {
    let sep = path_separator(path);
    let parts: Vec<&str> = path.split(sep).collect();
    if parts.len() <= 1 {
        String::new()
    } else {
        parts[..parts.len() - 1].join(&sep.to_string())
    }
}

fn basename(path: &str) -> String {
    let sep = path_separator(path);
    let parts: Vec<&str> = path.split(sep).filter(|s| !s.is_empty()).collect();
    parts.last().map(|s| (*s).to_string()).unwrap_or_default()
}

#[derive(Clone, Debug, Default)]
struct ParentIndex {
    folders_by_parent: HashMap<String, Vec<(String, u64)>>,
    files_by_parent: HashMap<String, Vec<(String, String, u64)>>,
}

fn build_parent_index(scan: &ScanResult) -> ParentIndex {
    let mut folders_by_parent: HashMap<String, Vec<(String, u64)>> = HashMap::new();
    for (path, size) in &scan.folder_sizes {
        let parent = parent_dir(path);
        folders_by_parent
            .entry(parent)
            .or_default()
            .push((path.clone(), *size));
    }
    let mut files_by_parent: HashMap<String, Vec<(String, String, u64)>> = HashMap::new();
    for f in &scan.files {
        let path_norm = normalize_path(&f.path);
        let parent = parent_dir(&path_norm);
        let name = basename(&path_norm);
        files_by_parent
            .entry(parent)
            .or_default()
            .push((f.path.clone(), name, f.size));
    }
    ParentIndex {
        folders_by_parent,
        files_by_parent,
    }
}

pub fn build_disk_tree(
    scan: &ScanResult,
    max_children_per_node: usize,
    max_depth: usize,
) -> Option<DiskTreeNode> {
    let _timings = build_disk_tree_timed(scan, max_children_per_node, max_depth);
    _timings.0
}

#[derive(Clone, Debug, Default)]
pub struct BuildTreeTimings {
    pub collect_folders_ms: u64,
    pub collect_files_ms: u64,
    pub sort_combine_ms: u64,
    pub recurse_ms: u64,
}

pub fn build_disk_tree_timed(
    scan: &ScanResult,
    max_children_per_node: usize,
    max_depth: usize,
) -> (Option<DiskTreeNode>, BuildTreeTimings) {
    use std::time::Instant;
    let _root_size = match scan.folder_sizes.get(&scan.root).copied() {
        Some(s) => s,
        None => return (None, BuildTreeTimings::default()),
    };

    let index = build_parent_index(scan);

    fn build_node(
        path: &str,
        depth: usize,
        scan: &ScanResult,
        index: &ParentIndex,
        max_children: usize,
        max_d: usize,
    ) -> (DiskTreeNode, BuildTreeTimings) {
        let size = scan.folder_sizes.get(path).copied().unwrap_or(0);
        let mut timings = BuildTreeTimings::default();

        let t0 = Instant::now();
        let folder_children: Vec<(String, u64)> = index
            .folders_by_parent
            .get(path)
            .map(|v| v.clone())
            .unwrap_or_default();
        timings.collect_folders_ms = t0.elapsed().as_millis() as u64;

        let t1 = Instant::now();
        let file_children: Vec<(String, String, u64)> = index
            .files_by_parent
            .get(path)
            .map(|v| v.clone())
            .unwrap_or_default();
        timings.collect_files_ms = t1.elapsed().as_millis() as u64;

        let t2 = Instant::now();
        let mut combined: Vec<(String, String, u64, bool)> = folder_children
            .into_iter()
            .map(|path_size| (path_size.0.clone(), basename(&path_size.0), path_size.1, true))
            .chain(
                file_children
                    .into_iter()
                    .map(|(path, name, size)| (path, name, size, false)),
            )
            .collect();
        combined.sort_by(|a, b| b.2.cmp(&a.2));
        let take_count = (max_children - 1).min(combined.len());
        let limited: Vec<_> = combined.drain(..take_count).collect();
        let rest: Vec<_> = combined;
        timings.sort_combine_ms = t2.elapsed().as_millis() as u64;

        if depth >= max_d || (limited.is_empty() && rest.is_empty()) {
            return (DiskTreeNode {
                path: path.to_string(),
                name: basename(path),
                size,
                children: None,
            }, timings);
        }

        let t3 = Instant::now();
        let child_results: Vec<(DiskTreeNode, BuildTreeTimings)> = limited
            .par_iter()
            .map(|(child_path, name, child_size, is_folder)| {
                if *is_folder {
                    build_node(child_path, depth + 1, scan, index, max_children, max_d)
                } else {
                    (DiskTreeNode {
                        path: child_path.clone(),
                        name: name.clone(),
                        size: *child_size,
                        children: None,
                    }, BuildTreeTimings::default())
                }
            })
            .collect();
        timings.recurse_ms = t3.elapsed().as_millis() as u64;

        for (_, t) in &child_results {
            timings.collect_folders_ms += t.collect_folders_ms;
            timings.collect_files_ms += t.collect_files_ms;
            timings.sort_combine_ms += t.sort_combine_ms;
            timings.recurse_ms += t.recurse_ms;
        }

        let mut children: Vec<DiskTreeNode> = child_results.into_iter().map(|(n, _)| n).collect();
        if !rest.is_empty() {
            let other_children: Vec<DiskTreeNode> = rest
                .into_iter()
                .map(|(child_path, name, size, _)| DiskTreeNode {
                    path: child_path,
                    name,
                    size,
                    children: None,
                })
                .collect();
            let other_size: u64 = other_children.iter().map(|n| n.size).sum();
            children.push(DiskTreeNode {
                path: format!("{}__other", path),
                name: "Other".to_string(),
                size: other_size,
                children: Some(other_children),
            });
        }

        (DiskTreeNode {
            path: path.to_string(),
            name: basename(path),
            size,
            children: Some(children),
        }, timings)
    }

    let (node, timings) = build_node(
        &scan.root,
        0,
        scan,
        &index,
        max_children_per_node,
        max_depth,
    );
    (Some(node), timings)
}

pub fn build_disk_tree_from_db(
    conn: &rusqlite::Connection,
    root: &str,
    max_children_per_node: usize,
    max_depth: usize,
) -> Option<DiskTreeNode> {
    let size = db::get_root_size(conn, root).ok()??;
    let (folders, files) = db::get_children_for_path(conn, root, root).ok()?;
    if folders.is_empty() && files.is_empty() {
        return None;
    }
    build_node_from_db(conn, root, root, 0, max_depth, max_children_per_node, size)
}

fn build_node_from_db(
    conn: &rusqlite::Connection,
    root: &str,
    path: &str,
    depth: usize,
    max_depth: usize,
    max_children: usize,
    size: u64,
) -> Option<DiskTreeNode> {
    let (folders, files) = db::get_children_for_path(conn, root, path).ok()?;
    let mut combined: Vec<(String, String, u64, bool)> = folders
        .into_iter()
        .map(|(p, s)| (p.clone(), basename(&p), s, true))
        .chain(
            files
                .into_iter()
                .map(|(p, s)| (p.clone(), basename(&p), s, false)),
        )
        .collect();
    combined.sort_by(|a, b| b.2.cmp(&a.2));
    let take_count = (max_children - 1).min(combined.len());
    let limited: Vec<_> = combined.drain(..take_count).collect();
    let rest: Vec<_> = combined;

    if depth >= max_depth || (limited.is_empty() && rest.is_empty()) {
        return Some(DiskTreeNode {
            path: path.to_string(),
            name: basename(path),
            size,
            children: None,
        });
    }

    let mut children: Vec<DiskTreeNode> = limited
        .into_iter()
        .filter_map(|(child_path, name, child_size, is_folder)| {
            if is_folder {
                build_node_from_db(
                    conn,
                    root,
                    &child_path,
                    depth + 1,
                    max_depth,
                    max_children,
                    child_size,
                )
            } else {
                Some(DiskTreeNode {
                    path: child_path,
                    name,
                    size: child_size,
                    children: None,
                })
            }
        })
        .collect();
    if !rest.is_empty() {
        let other_children: Vec<DiskTreeNode> = rest
            .into_iter()
            .map(|(child_path, name, size, _)| DiskTreeNode {
                path: child_path,
                name,
                size,
                children: None,
            })
            .collect();
        let other_size: u64 = other_children.iter().map(|n| n.size).sum();
        children.push(DiskTreeNode {
            path: format!("{}__other", path),
            name: "Other".to_string(),
            size: other_size,
            children: Some(other_children),
        });
    }

    Some(DiskTreeNode {
        path: path.to_string(),
        name: basename(path),
        size,
        children: if children.is_empty() {
            None
        } else {
            Some(children)
        },
    })
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
        assert!(a.iter().any(|x| *x == PathBuf::from("/a/b/c")));
        assert!(a.iter().any(|x| *x == PathBuf::from("/a/b")));
        assert!(a.iter().any(|x| *x == PathBuf::from("/a")));
    }
}
