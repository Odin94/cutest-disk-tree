use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use ignore::WalkBuilder;

use crate::{FileEntry, IndexMode, IndexStats};
use crate::core::folder_sizes::aggregate_folder_sizes;
use crate::core::scanning::utils::file_key_from_path;

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
                    folders.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                } else if ft.is_file() {
                    files.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            WalkState::Continue
        })
    });

    IndexStats {
        files: files.load(std::sync::atomic::Ordering::Relaxed),
        folders: folders.load(std::sync::atomic::Ordering::Relaxed),
    }
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

    let files_acc: Arc<std::sync::Mutex<Vec<FileEntry>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

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

