//! Background file-system watcher that keeps a [`TrigramIndex`] in sync with
//! filesystem changes without requiring a full rescan.
//!
//! # Design
//!
//! ```text
//! notify callback ──► mpsc::Sender ──► background thread ──► lock index ──► add/remove
//! ```
//!
//! The notify callback is intentionally minimal (just a channel send) so it never blocks the
//! OS notification thread.  All index mutations happen on a single background thread, which also
//! decides when to call [`TrigramIndex::compact`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};

use crate::{DiskObject, DiskObjectKind};
use crate::core::indexing::ngram::TrigramIndex;

// ── Public API ───────────────────────────────────────────────────────────────

/// Watches one or more directory trees and keeps a [`TrigramIndex`] up to date.
///
/// Dropping this struct stops the watcher. The index is not modified after drop.
pub struct IndexWatcher {
    /// Kept alive solely for its Drop impl — dropping the watcher stops OS event delivery.
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
            // Best-effort send; ignore if the receiver has been dropped.
            let _ = tx.send(res);
        })?;

        for path in &paths {
            watcher.watch(path, RecursiveMode::Recursive)?;
        }

        // Background thread: dequeue events and apply index mutations.
        thread::spawn(move || {
            let mut removal_count: u32 = 0;

            for res in rx {
                let event = match res {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                handle_event(event, &index, &mut removal_count);
            }
        });

        Ok(IndexWatcher { _watcher: watcher })
    }
}

// ── Event handling ────────────────────────────────────────────────────────────

fn handle_event(event: Event, index: &Arc<Mutex<TrigramIndex>>, removal_count: &mut u32) {
    match event.kind {
        EventKind::Create(CreateKind::File) | EventKind::Create(CreateKind::Any) => {
            for path in &event.paths {
                if path.is_file() {
                    if let Some(obj) = disk_object_from_path(path) {
                        if let Ok(mut idx) = index.lock() {
                            idx.add(obj);
                        }
                    }
                }
            }
        }

        EventKind::Create(CreateKind::Folder) => {
            for path in &event.paths {
                if let Some(obj) = disk_object_from_path(path) {
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
                        *removal_count += 1;
                        maybe_compact(&mut idx, removal_count);
                    }
                }
            }
        }

        // rename: both old and new paths known in one event
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
            if event.paths.len() >= 2 {
                let from = &event.paths[0];
                let to = &event.paths[1];
                if let Ok(mut idx) = index.lock() {
                    let from_str = from.to_string_lossy();
                    if idx.remove(from_str.as_ref()) {
                        *removal_count += 1;
                    }
                    if let Some(obj) = disk_object_from_path(to) {
                        idx.add(obj);
                    }
                    maybe_compact(&mut idx, removal_count);
                }
            }
        }

        // rename: only the old path is known yet
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            for path in &event.paths {
                let path_str = path.to_string_lossy();
                if let Ok(mut idx) = index.lock() {
                    if idx.remove(path_str.as_ref()) {
                        *removal_count += 1;
                        maybe_compact(&mut idx, removal_count);
                    }
                }
            }
        }

        // rename: only the new path is known
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
            for path in &event.paths {
                if let Some(obj) = disk_object_from_path(path) {
                    if let Ok(mut idx) = index.lock() {
                        idx.add(obj);
                    }
                }
            }
        }

        _ => {}
    }
}

/// Compact the index if the tombstone ratio exceeds ~5% or exceeds 100 removals.
fn maybe_compact(idx: &mut TrigramIndex, removal_count: &mut u32) {
    let live = idx.live_count();
    if *removal_count >= 100 || (*removal_count as usize * 20 > live) {
        idx.compact();
        *removal_count = 0;
    }
}

// ── Path → DiskObject ─────────────────────────────────────────────────────────

/// Build a [`DiskObject`] from a filesystem path by reading its metadata.
///
/// Returns `None` if metadata cannot be read (e.g. the file was deleted before we got here).
fn disk_object_from_path(path: &Path) -> Option<DiskObject> {
    let meta = std::fs::metadata(path).ok()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    let path_str = path.to_string_lossy().into_owned();
    let parent = path.parent().map(|p| p.to_string_lossy().into_owned());

    let kind = if meta.is_dir() {
        DiskObjectKind::Folder
    } else {
        DiskObjectKind::File
    };

    let ext = if kind == DiskObjectKind::File {
        path.extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
    } else {
        None
    };

    let size = if kind == DiskObjectKind::File {
        Some(meta.len())
    } else {
        None
    };

    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    let name_lower = name.to_lowercase();
    let path_lower = path_str.to_lowercase();

    Some(DiskObject {
        path: path_str,
        path_lower,
        parent_path: parent,
        name,
        name_lower,
        ext,
        kind,
        size,
        recursive_size: None,
        // dev/ino are not tracked for watcher-created entries; they are populated
        // on a full rescan via file_key_from_path in the scanning utilities.
        dev: None,
        ino: None,
        mtime,
    })
}
