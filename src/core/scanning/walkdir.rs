use std::collections::HashMap;
use std::path::Path;

use walkdir::WalkDir;

use crate::{FileEntry, IndexMode, IndexStats, ScanProgress};
use crate::core::folder_sizes::aggregate_folder_sizes;
use crate::core::scanning::utils::{file_key_from_path, PROGRESS_INTERVAL};

pub fn index_directory(root: &Path) -> (Vec<FileEntry>, HashMap<std::path::PathBuf, u64>) {
    index_directory_with_progress(root, |_| {})
}

pub fn index_directory_minimal(root: &Path) -> IndexStats {
    let (_files, _folders, stats) =
        index_directory_internal(root, &mut |_| {}, IndexMode::Minimal);
    stats
}

pub fn index_directory_with_progress<F>(
    root: &Path,
    mut progress: F,
) -> (Vec<FileEntry>, HashMap<std::path::PathBuf, u64>)
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
        .filter_entry(|e| !e.path_is_symlink());

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

