use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use ignore::WalkBuilder;

use crate::{FileEntry, ScanProgress};
use crate::core::scanning::utils::{PROGRESS_INTERVAL, file_key_from_path};

pub fn index_directory_ignore_with_progress<F>(
    root: &Path,
    progress: F,
) -> (Vec<FileEntry>, HashSet<PathBuf>)
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
    let folders_acc: Arc<Mutex<HashSet<PathBuf>>> =
        Arc::new(Mutex::new(HashSet::new()));
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

            let n = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
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

pub fn scan_roots_with_ignore<F>(
    roots: &[PathBuf],
    mut progress: F,
) -> (Arc<Vec<FileEntry>>, HashSet<PathBuf>, Vec<String>)
where
    F: FnMut(ScanProgress) + Send,
{
    let mut all_files: Vec<FileEntry> = Vec::new();
    let mut all_folders: HashSet<PathBuf> = HashSet::new();
    let mut cumulative_offset: u64 = 0;

    for root in roots {
        let offset = cumulative_offset;
        let (files, folder_paths) =
            index_directory_ignore_with_progress(root, |p| {
                let mut adjusted = p.clone();
                adjusted.files_count += offset;
                progress(adjusted);
            });
        cumulative_offset += files.len() as u64;
        all_files.extend(files);
        all_folders.extend(folder_paths);
    }

    let roots_str: Vec<String> = roots
        .iter()
        .map(|r| r.to_string_lossy().to_string())
        .collect();
    let files_arc = Arc::new(all_files);

    (files_arc, all_folders, roots_str)
}

