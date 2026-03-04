use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde::Serialize;
use rayon::prelude::*;

pub mod db;
pub mod core;
pub mod logging;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime: Option<i64>,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum DiskObjectKind {
    File,
    Folder,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct DiskObject {
    pub path: String,
    pub path_lower: String,
    pub parent_path: Option<String>,
    pub name: String,
    pub name_lower: String,
    pub ext: Option<String>,
    pub kind: DiskObjectKind,
    pub size: Option<u64>,
    pub recursive_size: Option<u64>,
    pub dev: Option<u64>,
    pub ino: Option<u64>,
    pub mtime: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ScanResult {
    pub roots: Vec<String>,
    pub files: Vec<FileEntrySer>,
    pub folder_sizes: HashMap<String, u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScanSummary {
    pub roots: Vec<String>,
    pub files_count: u64,
    pub folders_count: u64,
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

#[derive(Clone, Copy, Debug)]
pub enum IndexMode {
    Full,
    Minimal,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct IndexStats {
    pub files: usize,
    pub folders: usize,
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
    start_path: &str,
    max_children_per_node: usize,
    max_depth: usize,
) -> Option<DiskTreeNode> {
    let _timings = build_disk_tree_timed(scan, start_path, max_children_per_node, max_depth);
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
    start_path: &str,
    max_children_per_node: usize,
    max_depth: usize,
) -> (Option<DiskTreeNode>, BuildTreeTimings) {
    use std::time::Instant;
    let _root_size = match scan.folder_sizes.get(start_path).copied() {
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
        start_path,
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
    start_path: &str,
    max_children_per_node: usize,
    max_depth: usize,
) -> Option<DiskTreeNode> {
    let size = db::get_folder_size(conn, start_path).ok()??;
    let (folders, files) = db::get_children_for_path(conn, start_path).ok()?;
    if folders.is_empty() && files.is_empty() {
        return None;
    }
    build_node_from_db(conn, start_path, 0, max_depth, max_children_per_node, size)
}

fn build_node_from_db(
    conn: &rusqlite::Connection,
    path: &str,
    depth: usize,
    max_depth: usize,
    max_children: usize,
    size: u64,
) -> Option<DiskTreeNode> {
    let (folders, files) = db::get_children_for_path(conn, path).ok()?;
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

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub file_key: FileKey,
    pub mtime: Option<i64>,
}

pub use crate::core::scanning::utils::{file_key_from_path, PROGRESS_INTERVAL};

/// Returns the filesystem root paths for the current OS.
/// On Windows, returns every drive letter that currently exists (e.g. `C:\`, `D:\`).
/// On other platforms, returns `["/"]`.
pub fn get_filesystem_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut roots = Vec::new();
        for letter in b'A'..=b'Z' {
            let path = PathBuf::from(format!("{}:\\", letter as char));
            if path.exists() {
                roots.push(path);
            }
        }
        if roots.is_empty() {
            roots.push(PathBuf::from("C:\\"));
        }
        roots
    }
    #[cfg(not(windows))]
    {
        vec![PathBuf::from("/")]
    }
}

pub use crate::core::scanning::walkdir::{
    index_directory,
    index_directory_minimal,
    index_directory_with_progress,
};

pub fn compute_folder_sizes(
    root: &Path,
    files: &[FileEntry],
) -> HashMap<std::path::PathBuf, u64> {
    crate::core::folder_sizes::compute_folder_sizes(root, files)
}

pub use crate::core::scanning::jwalk::{
    index_directory_parallel_minimal,
    index_directory_parallel_with_progress,
};

#[cfg(windows)]
pub use crate::core::scanning::ntfs::index_directory_ntfs_with_progress;

pub fn index_directory_serializable(root: &Path) -> Option<ScanResult> {
    let (files, folder_sizes) = index_directory(root);
    to_scan_result(&[root], &files, &folder_sizes)
}

pub fn to_scan_result(
    roots: &[&Path],
    files: &[FileEntry],
    folder_sizes: &HashMap<std::path::PathBuf, u64>,
) -> Option<ScanResult> {
    let roots_str: Vec<String> = roots.iter().map(|r| r.to_string_lossy().to_string()).collect();
    let files_ser: Vec<FileEntrySer> = files
        .iter()
        .map(|entry| FileEntrySer {
            path: entry.path.to_string_lossy().to_string(),
            size: entry.size,
            file_key: entry.file_key,
            mtime: entry.mtime,
        })
        .collect();
    let folder_sizes_ser: HashMap<String, u64> = folder_sizes
        .iter()
        .map(|(p, s)| (p.to_string_lossy().to_string(), *s))
        .collect();
    Some(ScanResult {
        roots: roots_str,
        files: files_ser,
        folder_sizes: folder_sizes_ser,
    })
}

pub use crate::core::scanning::ignore_scanner::index_directory_ignore_with_progress;
pub use crate::core::scanning::lolcate::{
    index_directory_lolcate_full,
    index_directory_lolcate_like,
};

#[cfg(test)]
mod tests;
