use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use walkdir::{DirEntry, WalkDir};
use jwalk::WalkDir as JwalkDir;
use serde::Serialize;
use rayon::prelude::*;
use ignore::WalkBuilder;

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
    pub root: String,
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


#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub file_key: FileKey,
    pub mtime: Option<i64>,
}

const PROGRESS_INTERVAL: u64 = 200;

pub fn index_directory(root: &Path) -> (Vec<FileEntry>, HashMap<std::path::PathBuf, u64>) {
    index_directory_with_progress(root, |_| {})
}

pub fn index_directory_minimal(root: &Path) -> IndexStats {
    let (_files, _folders, stats) =
        index_directory_internal(root, &mut |_| {}, IndexMode::Minimal);
    stats
}

pub fn index_directory_with_progress<F>(root: &Path, mut progress: F) -> (Vec<FileEntry>, HashMap<std::path::PathBuf, u64>)
where
    F: FnMut(ScanProgress),
{
    let (files, folder_sizes, _stats) =
        index_directory_internal(root, &mut progress, IndexMode::Full);
    (files, folder_sizes)
}

fn index_directory_internal<F>(
    root: &Path,
    progress: &mut F,
    mode: IndexMode,
) -> (Vec<FileEntry>, HashMap<std::path::PathBuf, u64>, IndexStats)
where
    F: FnMut(ScanProgress),
{
    progress(ScanProgress {
        files_count: 0,
        current_path: None,
        status: None,
    });

    let mut files: Vec<FileEntry> = Vec::new();
    let mut stats = IndexStats::default();

    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_symlink(e));

    for entry in walker.filter_map(Result::ok) {
        let path = entry.path().to_path_buf();
        let file_type = entry.file_type();
        if file_type.is_dir() {
            stats.folders += 1;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        match mode {
            IndexMode::Full => {
                if let Ok(meta) = entry.metadata() {
                    let size = meta.len();
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64);
                    if let Some(key) = file_key_from_path(&path) {
                        files.push(FileEntry {
                            path: path.clone(),
                            size,
                            file_key: key,
                            mtime,
                        });
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
            IndexMode::Minimal => {
                stats.files += 1;
                if stats.files as u64 % PROGRESS_INTERVAL == 0 {
                    progress(ScanProgress {
                        files_count: stats.files as u64,
                        current_path: Some(path.to_string_lossy().to_string()),
                        status: None,
                    });
                }
            }
        }
    }

    match mode {
        IndexMode::Full => {
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
            let file_count = files.len();

            (
                files,
                folder_sizes,
                IndexStats {
                    files: file_count,
                    folders: stats.folders,
                },
            )
        }
        IndexMode::Minimal => {
            progress(ScanProgress {
                files_count: stats.files as u64,
                current_path: None,
                status: None,
            });

            (Vec::new(), HashMap::new(), stats)
        }
    }
}

fn aggregate_folder_sizes(
    root: &Path,
    files: &[FileEntry],
) -> HashMap<std::path::PathBuf, u64> {
    let root_len = root.as_os_str().len();

    // Sequential dedup: Rayon workers can't share a HashSet without a lock.
    let mut seen: HashSet<FileKey> = HashSet::with_capacity(files.len());
    let unique: Vec<&FileEntry> = files.iter()
        .filter(|e| seen.insert(e.file_key))
        .collect();

    // Parallel ancestor walk: each file is independent once dedup is done.
    // Root size is accumulated as a plain u64 to avoid cloning the root PathBuf per file.
    let (root_size, mut folder_sizes) = unique
        .par_iter()
        .fold(
            || (0u64, HashMap::<PathBuf, u64>::new()),
            |(mut rs, mut map), entry| {
                rs += entry.size;
                let mut a = entry.path.parent();
                while let Some(anc) = a {
                    if anc.as_os_str().len() <= root_len {
                        break;
                    }
                    *map.entry(anc.to_path_buf()).or_insert(0) += entry.size;
                    a = anc.parent();
                }
                (rs, map)
            },
        )
        .reduce(
            || (0u64, HashMap::new()),
            |(r1, mut m1), (r2, m2)| {
                for (k, v) in m2 {
                    *m1.entry(k).or_insert(0) += v;
                }
                (r1 + r2, m1)
            },
        );

    folder_sizes.insert(root.to_path_buf(), root_size);
    folder_sizes
}

/// High-level helper to compute recursive folder sizes from a flat file list.
///
/// This is a thin wrapper around the internal aggregation used by the app,
/// exposed so that other crates (like the Tauri shell) can reuse the same logic.
pub fn compute_folder_sizes(
    root: &Path,
    files: &[FileEntry],
) -> HashMap<std::path::PathBuf, u64> {
    aggregate_folder_sizes(root, files)
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

    #[cfg(windows)]
    {
        if let Some((files, folder_sizes)) =
            index_directory_ntfs_with_progress(root, &mut progress)
        {
            return (files, folder_sizes);
        }
        progress(ScanProgress {
            files_count: 0,
            current_path: None,
            status: Some("Falling back to directory walk…".into()),
        });
    }

    let (files, folder_sizes, _stats) =
        index_directory_parallel_jwalk_internal(root, progress, IndexMode::Full);

    (files, folder_sizes)
}

pub fn index_directory_parallel_minimal(root: &Path) -> IndexStats {
    let (_files, _folders, stats) = index_directory_parallel_jwalk_internal(
        root,
        |_| {},
        IndexMode::Minimal,
    );
    stats
}

fn index_directory_parallel_jwalk_internal<F>(
    root: &Path,
    mut progress: F,
    mode: IndexMode,
) -> (Vec<FileEntry>, HashMap<std::path::PathBuf, u64>, IndexStats)
where
    F: FnMut(ScanProgress),
{
    let mut stats = IndexStats::default();

    let mut files: Vec<FileEntry> = Vec::new();
    let walk = match JwalkDir::new(root).follow_links(false).try_into_iter() {
        Ok(w) => w,
        Err(_) => {
            progress(ScanProgress {
                files_count: 0,
                current_path: None,
                status: Some("Scan failed (try_into_iter)".into()),
            });
            return (files, HashMap::new(), stats);
        }
    };

    for entry in walk.filter_map(Result::ok) {
        if entry.path_is_symlink() {
            continue;
        }
        let file_type = entry.file_type();
        let path = entry.path();

        if file_type.is_dir() {
            stats.folders += 1;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        match mode {
            IndexMode::Full => {
                let meta = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let size = meta.len();
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64);
                let key = match file_key_from_path(&path) {
                    Some(k) => k,
                    None => continue,
                };
                files.push(FileEntry {
                    path: path.clone(),
                    size,
                    file_key: key,
                    mtime,
                });
                if files.len() as u64 % PROGRESS_INTERVAL == 0 {
                    progress(ScanProgress {
                        files_count: files.len() as u64,
                        current_path: Some(path.to_string_lossy().to_string()),
                        status: None,
                    });
                }
            }
            IndexMode::Minimal => {
                stats.files += 1;
                if stats.files as u64 % PROGRESS_INTERVAL == 0 {
                    progress(ScanProgress {
                        files_count: stats.files as u64,
                        current_path: Some(path.to_string_lossy().to_string()),
                        status: None,
                    });
                }
            }
        }
    }

    match mode {
        IndexMode::Full => {
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
            let file_count = files.len();

            (
                files,
                folder_sizes,
                IndexStats {
                    files: file_count,
                    folders: stats.folders,
                },
            )
        }
        IndexMode::Minimal => {
            progress(ScanProgress {
                files_count: stats.files as u64,
                current_path: None,
                status: None,
            });

            (Vec::new(), HashMap::new(), stats)
        }
    }
}

#[cfg(windows)]
fn index_directory_ntfs_with_progress<F>(
    root: &Path,
    mut progress: F,
) -> Option<(Vec<FileEntry>, HashMap<PathBuf, u64>)>
where
    F: FnMut(ScanProgress),
{
    use std::ffi::OsString;
    use usn_journal_rs::{mft::Mft, volume::Volume};
    use usn_journal_rs::path::PathResolvableEntry;

    let root_str = root.to_string_lossy();
    let drive_letter = root_str.chars().next().unwrap_or_default();
    if !drive_letter.is_ascii_alphabetic() {
        return None;
    }

    let volume = match Volume::from_drive_letter(drive_letter) {
        Ok(v) => v,
        Err(_) => return None,
    };

    let mft = Mft::new(&volume);
    let mut files: Vec<FileEntry> = Vec::new();

    for entry_res in mft.iter() {
        let entry = match entry_res {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.is_dir() {
            continue;
        }

        // Best-effort path reconstruction: use file_name for now; a full
        // PathResolver-based parent chain walk can be added later.
        let name: &OsString = entry.file_name();
        let name_str = name.to_string_lossy().to_string();

        let candidate_path = PathBuf::from(format!("{}:\\{}", drive_letter, name_str));
        let meta = std::fs::metadata(&candidate_path).ok();
        let (size, mtime) = if let Some(m) = meta {
            let sz = m.len();
            let mt = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            (sz, mt)
        } else {
            (0, None)
        };

        if let Some(key) = file_key_from_path(&candidate_path) {
            files.push(FileEntry {
                path: candidate_path.clone(),
                size,
                file_key: key,
                mtime,
            });
            if files.len() as u64 % PROGRESS_INTERVAL == 0 {
                progress(ScanProgress {
                    files_count: files.len() as u64,
                    current_path: Some(candidate_path.to_string_lossy().to_string()),
                    status: None,
                });
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

    Some((files, folder_sizes))
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
        root: root_str,
        files: files_ser,
        folder_sizes: folder_sizes_ser,
    })
}

pub fn index_directory_lolcate_like(root: &Path, _mode: IndexMode) -> IndexStats {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .parents(false)
        .follow_links(false)
        .ignore(true)
        .git_global(false)
        .git_ignore(false)
        .git_exclude(false)
        .threads(4);

    let files = Arc::new(AtomicUsize::new(0));
    let folders = Arc::new(AtomicUsize::new(0));

    let walk = builder.build_parallel();
    walk.run(|| {
        let files = Arc::clone(&files);
        let folders = Arc::clone(&folders);
        Box::new(move |entry| {
            use ignore::WalkState;
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return WalkState::Continue,
            };
            if let Some(ft) = entry.file_type() {
                if ft.is_symlink() {
                    return WalkState::Continue;
                }
                if ft.is_dir() {
                    folders.fetch_add(1, Ordering::Relaxed);
                } else if ft.is_file() {
                    files.fetch_add(1, Ordering::Relaxed);
                }
            }
            WalkState::Continue
        })
    });

    IndexStats {
        files: files.load(Ordering::Relaxed),
        folders: folders.load(Ordering::Relaxed),
    }
}

/// Ignore-based parallel indexer used for the main application scan path.
///
/// Collects files with path, size and last-modified timestamp, and reports
/// progress periodically. Also collects folder paths seen during the walk.
/// Folder sizes are computed separately.
pub fn index_directory_ignore_with_progress<F>(
    root: &Path,
    progress: F,
) -> (Vec<FileEntry>, std::collections::HashSet<std::path::PathBuf>)
where
    F: FnMut(ScanProgress) + Send,
{
    let progress = Arc::new(Mutex::new(progress));

    {
        let mut cb = progress.lock().unwrap();
        cb(ScanProgress {
            files_count: 0,
            current_path: None,
            status: Some("Scanning files…".into()),
        });
    }

    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .parents(false)
        .follow_links(false)
        .ignore(true)
        .git_global(false)
        .git_ignore(false)
        .git_exclude(false)
        .threads(4);

    let files_acc: Arc<Mutex<Vec<FileEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let folders_acc: Arc<Mutex<std::collections::HashSet<std::path::PathBuf>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));
    let counter = Arc::new(AtomicUsize::new(0));

    let walk = builder.build_parallel();
    walk.run(|| {
        let files_acc = Arc::clone(&files_acc);
        let folders_acc = Arc::clone(&folders_acc);
        let counter = Arc::clone(&counter);
        let progress = Arc::clone(&progress);
        Box::new(move |entry| {
            use ignore::WalkState;
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return WalkState::Continue,
            };
            let ft = match entry.file_type() {
                Some(ft) => ft,
                None => return WalkState::Continue,
            };
            if ft.is_symlink() {
                return WalkState::Continue;
            }
            if ft.is_dir() {
                if let Ok(mut guard) = folders_acc.lock() {
                    guard.insert(entry.path().to_path_buf());
                }
                return WalkState::Continue;
            }
            if !ft.is_file() {
                return WalkState::Continue;
            }

            let path = entry.path().to_path_buf();
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => return WalkState::Continue,
            };
            let size = meta.len();
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            let key = match file_key_from_path(&path) {
                Some(k) => k,
                None => return WalkState::Continue,
            };

            {
                let mut guard = files_acc.lock().unwrap();
                guard.push(FileEntry {
                    path: path.clone(),
                    size,
                    file_key: key,
                    mtime,
                });
            }

            let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
            if n as u64 % PROGRESS_INTERVAL == 0 {
                if let Ok(mut cb) = progress.lock() {
                    cb(ScanProgress {
                        files_count: n as u64,
                        current_path: Some(path.to_string_lossy().to_string()),
                        status: None,
                    });
                }
            }

            WalkState::Continue
        })
    });

    let files = match Arc::try_unwrap(files_acc) {
        Ok(mutex) => mutex.into_inner().unwrap(),
        Err(arc) => arc.lock().unwrap().clone(),
    };
    let folders = match Arc::try_unwrap(folders_acc) {
        Ok(mutex) => mutex.into_inner().unwrap(),
        Err(arc) => arc.lock().unwrap().clone(),
    };

    if let Ok(mut cb) = progress.lock() {
        cb(ScanProgress {
            files_count: files.len() as u64,
            current_path: None,
            status: None,
        });
    }

    (files, folders)
}

pub fn index_directory_lolcate_full(
    root: &Path,
) -> (Vec<FileEntry>, HashMap<std::path::PathBuf, u64>) {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .parents(false)
        .follow_links(false)
        .ignore(true)
        .git_global(false)
        .git_ignore(false)
        .git_exclude(false)
        .threads(4);

    let files_acc: Arc<Mutex<Vec<FileEntry>>> = Arc::new(Mutex::new(Vec::new()));

    let walk = builder.build_parallel();
    walk.run(|| {
        let files_acc = Arc::clone(&files_acc);
        Box::new(move |entry| {
            use ignore::WalkState;
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return WalkState::Continue,
            };
            if let Some(ft) = entry.file_type() {
                if ft.is_symlink() {
                    return WalkState::Continue;
                }
                if !ft.is_file() {
                    return WalkState::Continue;
                }
            } else {
                return WalkState::Continue;
            }

            let path = entry.path();
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => return WalkState::Continue,
            };
            let size = meta.len();
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            let key = match file_key_from_path(&path) {
                Some(k) => k,
                None => return WalkState::Continue,
            };

            let mut guard = files_acc.lock().unwrap();
            guard.push(FileEntry {
                path: path.to_path_buf(),
                size,
                file_key: key,
                mtime,
            });

            WalkState::Continue
        })
    });

    let files = match Arc::try_unwrap(files_acc) {
        Ok(mutex) => mutex.into_inner().unwrap(),
        Err(arc) => arc.lock().unwrap().clone(),
    };
    let folder_sizes = aggregate_folder_sizes(root, &files);

    (files, folder_sizes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn build_disk_tree_creates_expected_structure() {
        // Construct a small synthetic ScanResult:
        //
        // /root
        //   /root/sub
        //     /root/sub/file1 (size 10)
        //   /root/file2 (size 5)
        let root = "/root".to_string();

        let files_ser = vec![
            FileEntrySer {
                path: "/root/sub/file1".to_string(),
                size: 10,
                file_key: FileKey { dev: 1, ino: 1 },
                mtime: None,
            },
            FileEntrySer {
                path: "/root/file2".to_string(),
                size: 5,
                file_key: FileKey { dev: 1, ino: 2 },
                mtime: None,
            },
        ];

        let mut folder_sizes: HashMap<String, u64> = HashMap::new();
        folder_sizes.insert("/root".to_string(), 15);
        folder_sizes.insert("/root/sub".to_string(), 10);

        let scan = ScanResult {
            root: root.clone(),
            files: files_ser,
            folder_sizes,
        };

        let (tree_opt, _timings) = build_disk_tree_timed(&scan, 10, 10);
        let tree = tree_opt.expect("tree should be built");

        fn collect_paths(node: &DiskTreeNode, out: &mut Vec<(String, u64, bool)>) {
            out.push((node.path.clone(), node.size, node.children.is_some()));
            if let Some(children) = &node.children {
                for child in children {
                    collect_paths(child, out);
                }
            }
        }

        let mut all: Vec<(String, u64, bool)> = Vec::new();
        collect_paths(&tree, &mut all);

        // Root and subfolder sizes must match the synthetic folder_sizes map.
        assert!(all.iter().any(|(p, s, _)| p == "/root" && *s == 15));
        assert!(all.iter().any(|(p, s, _)| p == "/root/sub" && *s == 10));
    }
}
