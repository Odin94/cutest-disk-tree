//! Real-time index updates via OS filesystem notifications.
//!
//! See [`IndexWatcher`] and the module README for the overall strategy.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};

use crate::core::indexing::ngram::TrigramIndex;
use super::{disk_object_from_path, should_skip};

const LOG_TARGET: &str = "disk_tree::watcher";

// ── Public API ────────────────────────────────────────────────────────────────

/// Watches one or more directory trees and keeps a [`TrigramIndex`] up to date.
///
/// Uses OS kernel notifications (`ReadDirectoryChangesW` / `FSEvents` / `inotify`) — no
/// polling, negligible CPU when idle, millisecond latency.  Works without admin/sudo; only
/// requires read permission on the watched directories.
///
/// Dropping this struct stops the watcher and ends the background thread.
pub struct IndexWatcher {
    /// Kept alive solely for its Drop impl — dropping it stops OS event delivery and
    /// closes the mpsc channel, which causes the background thread to exit cleanly.
    _watcher: RecommendedWatcher,
}

impl IndexWatcher {
    /// Start watching `paths` (recursively) and update `index` on filesystem events.
    ///
    /// Returns immediately; event processing runs on a background thread.
    pub fn new(
        index: Arc<Mutex<TrigramIndex>>,
        paths: Vec<PathBuf>,
    ) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel::<notify::Result<Event>>();

        let mut watcher = notify::recommended_watcher(move |res| {
            // Best-effort send — ignore if the receiver has been dropped.
            let _ = tx.send(res);
        })?;

        for path in &paths {
            watcher.watch(path, RecursiveMode::Recursive)?;
        }

        thread::Builder::new()
            .name("index-watcher".into())
            .spawn(move || {
                let mut removal_count: u32 = 0;
                for res in rx {
                    let event = match res {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    handle_event(event, &index, &mut removal_count);
                }
            })
            .expect("failed to spawn watcher thread");

        Ok(IndexWatcher { _watcher: watcher })
    }
}

// ── Event handling ────────────────────────────────────────────────────────────

fn handle_event(event: Event, index: &Arc<Mutex<TrigramIndex>>, removal_count: &mut u32) {
    match event.kind {
        EventKind::Create(CreateKind::File) | EventKind::Create(CreateKind::Any) => {
            for path in &event.paths {
                if should_skip(path) { continue; }
                if path.is_file() {
                    if let Some(obj) = disk_object_from_path(path) {
                        log::debug!(target: LOG_TARGET, "add {}", obj.path);
                        if let Ok(mut idx) = index.lock() {
                            idx.add(obj);
                        }
                    }
                }
            }
        }

        EventKind::Create(CreateKind::Folder) => {
            for path in &event.paths {
                if should_skip(path) { continue; }
                if let Some(obj) = disk_object_from_path(path) {
                    log::debug!(target: LOG_TARGET, "add {}", obj.path);
                    if let Ok(mut idx) = index.lock() {
                        idx.add(obj);
                    }
                }
            }
        }

        EventKind::Remove(RemoveKind::File)
        | EventKind::Remove(RemoveKind::Folder)
        | EventKind::Remove(RemoveKind::Any) => {
            for path in &event.paths {
                let path_str = path.to_string_lossy();
                if let Ok(mut idx) = index.lock() {
                    if idx.remove(path_str.as_ref()) {
                        log::debug!(target: LOG_TARGET, "remove {}", path_str);
                        *removal_count += 1;
                        maybe_compact(&mut idx, removal_count);
                    }
                }
            }
        }

        // Rename: both old and new paths known in one event
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
            if event.paths.len() >= 2 {
                let from = &event.paths[0];
                let to = &event.paths[1];
                if let Ok(mut idx) = index.lock() {
                    let from_str = from.to_string_lossy();
                    if idx.remove(from_str.as_ref()) {
                        log::debug!(target: LOG_TARGET, "rename remove {}", from_str);
                        *removal_count += 1;
                    }
                    if !should_skip(to) {
                        if let Some(obj) = disk_object_from_path(to) {
                            log::debug!(target: LOG_TARGET, "rename add {}", obj.path);
                            idx.add(obj);
                        }
                    }
                    maybe_compact(&mut idx, removal_count);
                }
            }
        }

        // Rename: only the old path is known yet
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            for path in &event.paths {
                let path_str = path.to_string_lossy();
                if let Ok(mut idx) = index.lock() {
                    if idx.remove(path_str.as_ref()) {
                        log::debug!(target: LOG_TARGET, "rename remove {}", path_str);
                        *removal_count += 1;
                        maybe_compact(&mut idx, removal_count);
                    }
                }
            }
        }

        // Rename: only the new path is known
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
            for path in &event.paths {
                if should_skip(path) { continue; }
                if let Some(obj) = disk_object_from_path(path) {
                    log::debug!(target: LOG_TARGET, "rename add {}", obj.path);
                    if let Ok(mut idx) = index.lock() {
                        idx.add(obj);
                    }
                }
            }
        }

        _ => {}
    }
}

/// Compact the index if tombstones exceed ~5% of live entries or 100 absolute removals.
fn maybe_compact(idx: &mut TrigramIndex, removal_count: &mut u32) {
    let live = idx.live_count();
    if *removal_count >= 100 || (*removal_count as usize * 20 > live) {
        idx.compact();
        *removal_count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tempfile::TempDir;
    use crate::core::indexing::ngram::build_index;
    use crate::core::file_updating::disk_object_from_path;

    /// Poll `check` every 50 ms until it returns true or `timeout` elapses.
    fn poll_until(check: impl Fn() -> bool, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if check() { return true; }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn watcher_adds_newly_created_file() {
        let dir = TempDir::new().unwrap();
        let index = Arc::new(Mutex::new(build_index(&[])));
        let _w = IndexWatcher::new(Arc::clone(&index), vec![dir.path().to_path_buf()])
            .expect("watcher should start");

        let file = dir.path().join("created.txt");
        std::fs::write(&file, b"hello").unwrap();

        let found = poll_until(
            || index.lock().unwrap().objects.iter().any(|o| o.name == "created.txt"),
            Duration::from_secs(3),
        );
        assert!(found, "watcher should have added created.txt to the index");
    }

    #[test]
    fn watcher_removes_deleted_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("todelete.txt");
        std::fs::write(&file, b"bye").unwrap();

        // Pre-index the file so the watcher has a path to remove.
        let obj = disk_object_from_path(&file).expect("file exists");
        let path_key = obj.path.clone();
        let index = Arc::new(Mutex::new(build_index(&[obj])));
        assert_eq!(index.lock().unwrap().live_count(), 1);

        let _w = IndexWatcher::new(Arc::clone(&index), vec![dir.path().to_path_buf()])
            .expect("watcher should start");

        std::fs::remove_file(&file).unwrap();

        let removed = poll_until(
            || !index.lock().unwrap().path_to_idx.contains_key(&path_key),
            Duration::from_secs(3),
        );
        assert!(removed, "watcher should have removed todelete.txt from the index");
        assert_eq!(index.lock().unwrap().live_count(), 0);
    }

    #[test]
    fn watcher_handles_rename() {
        let dir = TempDir::new().unwrap();
        let old = dir.path().join("old.txt");
        let new = dir.path().join("new.txt");
        std::fs::write(&old, b"data").unwrap();

        let obj = disk_object_from_path(&old).expect("file exists");
        let old_key = obj.path.clone();
        let index = Arc::new(Mutex::new(build_index(&[obj])));

        let _w = IndexWatcher::new(Arc::clone(&index), vec![dir.path().to_path_buf()])
            .expect("watcher should start");

        std::fs::rename(&old, &new).unwrap();

        // Wait until new.txt appears and old.txt is gone.
        let settled = poll_until(
            || {
                let idx = index.lock().unwrap();
                let has_new = idx.objects.iter().any(|o| o.name == "new.txt");
                let old_gone = !idx.path_to_idx.contains_key(&old_key);
                has_new && old_gone
            },
            Duration::from_secs(3),
        );
        assert!(settled, "after rename, new.txt should be in index and old.txt should be gone");
    }

    #[test]
    fn watcher_creates_directory_without_panic() {
        // Smoke test: creating a subdirectory should not panic or error.
        let dir = TempDir::new().unwrap();
        let index = Arc::new(Mutex::new(build_index(&[])));
        let _w = IndexWatcher::new(Arc::clone(&index), vec![dir.path().to_path_buf()])
            .expect("watcher should start");

        let sub = dir.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();

        // Just verify no panic — folder may or may not be indexed depending on
        // whether the OS emits a Folder-specific create event.
        std::thread::sleep(Duration::from_millis(300));
    }
}
