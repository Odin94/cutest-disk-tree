//! Background reconciler that slowly walks the filesystem and patches the index
//! for any files that appeared or disappeared while the program wasn't running
//! (or while the OS watcher missed an event).
//!
//! See the module README for how this fits into the overall update strategy.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use walkdir::WalkDir;

use crate::core::indexing::ngram::TrigramIndex;
use super::{disk_object_from_path, should_skip};

const LOG_TARGET: &str = "disk_tree::reconciler";

/// How long to sleep every [`SLEEP_EVERY`] entries during a reconciliation walk.
/// 2 ms per 10 entries → ~20 seconds per 100k files — deliberately slow.
#[cfg(not(test))]
const ENTRY_SLEEP: Duration = Duration::from_millis(2);
#[cfg(test)]
const ENTRY_SLEEP: Duration = Duration::from_micros(100);

/// How long to rest between complete reconciliation passes.
#[cfg(not(test))]
const PASS_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Long enough that tests never accidentally trigger a second pass.
#[cfg(test)]
const PASS_INTERVAL: Duration = Duration::from_secs(3600);

/// Sleep once every this many entries rather than on every entry.
const SLEEP_EVERY: u32 = 10;

// ── Public API ────────────────────────────────────────────────────────────────

/// Runs a background thread that slowly walks `roots` and reconciles the
/// [`TrigramIndex`] with the actual filesystem state.
///
/// - Pauses automatically whenever `scan_in_progress` is `true` so it never
///   races with a full rescan.
/// - After a full scan completes the reconciler restarts its walk from scratch
///   so it always works from a fresh baseline.
/// - Uses a small stack, serial (non-parallel) walking, and a per-entry sleep
///   to keep CPU and I/O impact negligible.
///
/// Dropping this struct signals the background thread to stop.
pub struct IndexReconciler {
    cancel: Arc<AtomicBool>,
    // Stored so the thread is joined (and thus stops) when the struct is dropped.
    // We intentionally leak the join handle on drop rather than blocking — callers
    // should drop this during shutdown where a brief background-thread teardown is fine.
    _thread: thread::JoinHandle<()>,
}

impl IndexReconciler {
    /// Spawn the reconciler thread.
    ///
    /// - `index` — the live index to patch.
    /// - `roots` — directory trees to walk (same roots used for scanning).
    /// - `scan_in_progress` — set to `true` by the caller while a full disk scan
    ///   is running; the reconciler pauses until it becomes `false`.
    pub fn new(
        index: Arc<Mutex<TrigramIndex>>,
        roots: Vec<PathBuf>,
        scan_in_progress: Arc<AtomicBool>,
    ) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = Arc::clone(&cancel);

        let thread = thread::Builder::new()
            .name("index-reconciler".into())
            .stack_size(256 * 1024) // 256 KB — walkdir is iterative, no deep recursion
            .spawn(move || {
                run(index, roots, scan_in_progress, cancel_clone);
            })
            .expect("failed to spawn reconciler thread");

        IndexReconciler { cancel, _thread: thread }
    }
}

impl Drop for IndexReconciler {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

// ── Reconciliation loop ───────────────────────────────────────────────────────

fn run(
    index: Arc<Mutex<TrigramIndex>>,
    roots: Vec<PathBuf>,
    scan_in_progress: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) {
    loop {
        // ── Wait until no scan is running ────────────────────────────────────
        loop {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            if !scan_in_progress.load(Ordering::Relaxed) {
                break;
            }
            thread::sleep(Duration::from_secs(1));
        }

        // ── Walk phase: find new entries ──────────────────────────────────────
        // Track which index entries we actually see so we can detect deletions.
        let mut seen_indices: HashSet<u32> = HashSet::new();
        let mut aborted = false;
        let mut tick: u32 = 0;

        'walk: for root in &roots {
            let walker = WalkDir::new(root).follow_links(false).into_iter();
            for result in walker.filter_entry(|e| {
                // Prune ignored directory subtrees (depth == 0 is the root itself — never prune).
                e.depth() == 0 || !e.file_type().is_dir() || !should_skip(e.path())
            }) {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                if scan_in_progress.load(Ordering::Relaxed) {
                    // A new scan started — restart the entire pass after it finishes.
                    aborted = true;
                    break 'walk;
                }

                tick += 1;
                if tick % SLEEP_EVERY == 0 {
                    thread::sleep(ENTRY_SLEEP);
                }

                let entry = match result {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                // Skip the root entry itself (it's always in the index after a scan).
                if entry.depth() == 0 {
                    continue;
                }

                // Skip files whose path falls inside an ignored directory.
                if should_skip(entry.path()) {
                    continue;
                }

                let path_str = entry.path().to_string_lossy().into_owned();

                let existing_idx = {
                    let idx = index.lock().unwrap();
                    idx.path_to_idx.get(&path_str).copied()
                };

                match existing_idx {
                    Some(i) => {
                        seen_indices.insert(i);
                    }
                    None => {
                        // File/folder exists on disk but is missing from the index.
                        if let Some(obj) = disk_object_from_path(entry.path()) {
                            log::debug!(target: LOG_TARGET, "add {}", path_str);
                            if let Ok(mut idx) = index.lock() {
                                idx.add(obj);
                            }
                        }
                    }
                }
            }
        }

        if aborted {
            seen_indices.clear();
            continue; // re-enter the outer loop to wait for scan to finish
        }

        // ── Deletion phase: find indexed entries that no longer exist ─────────
        // Snapshot the candidates while holding the lock for a moment, then check
        // each path on disk without holding the lock.
        let paths_to_check: Vec<String> = {
            let idx = index.lock().unwrap();
            idx.path_to_idx
                .iter()
                .filter(|(_, &i)| !idx.deleted.contains(&i) && !seen_indices.contains(&i))
                .map(|(p, _)| p.clone())
                .collect()
        };

        for path in paths_to_check {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            if scan_in_progress.load(Ordering::Relaxed) {
                break; // scan started — the next pass will start fresh
            }

            tick += 1;
            if tick % SLEEP_EVERY == 0 {
                thread::sleep(ENTRY_SLEEP);
            }

            if !Path::new(&path).exists() {
                log::debug!(target: LOG_TARGET, "remove {}", path);
                if let Ok(mut idx) = index.lock() {
                    idx.remove(&path);
                }
            }
        }

        // ── Rest before the next pass ─────────────────────────────────────────
        let mut elapsed = Duration::ZERO;
        while elapsed < PASS_INTERVAL {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_secs(1));
            elapsed += Duration::from_secs(1);
        }
    }
}

#[cfg(test)]
mod tests;
