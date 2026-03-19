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
mod tests {
    use super::*;
    use tempfile::TempDir;
    use crate::core::indexing::ngram::build_index;
    use crate::core::file_updating::disk_object_from_path;

    /// Poll `check` every 50 ms until it returns true or `timeout` elapses.
    fn poll_until(check: impl Fn() -> bool, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if check() { return true; }
            thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// Return the path string the reconciler/walkdir will use for a given PathBuf.
    fn path_str(p: &std::path::Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn adds_files_missing_from_index() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("newfile.txt");
        std::fs::write(&file, b"hello").unwrap();

        let index = Arc::new(Mutex::new(build_index(&[])));
        let scan = Arc::new(AtomicBool::new(false));
        let _rec = IndexReconciler::new(
            Arc::clone(&index),
            vec![dir.path().to_path_buf()],
            scan,
        );

        let added = poll_until(
            || index.lock().unwrap().path_to_idx.contains_key(&path_str(&file)),
            Duration::from_secs(2),
        );
        assert!(added, "reconciler should have added newfile.txt to the index");
        assert_eq!(index.lock().unwrap().live_count(), 1);
    }

    #[test]
    fn removes_indexed_paths_that_no_longer_exist() {
        let dir = TempDir::new().unwrap();
        // Build an index that references a path which does NOT exist on disk.
        let fake_path = dir.path().join("ghost.txt").to_string_lossy().into_owned();
        let ghost = crate::DiskObject {
            path: fake_path.clone(),
            path_lower: fake_path.to_lowercase(),
            parent_path: None,
            name: "ghost.txt".into(),
            name_lower: "ghost.txt".into(),
            ext: Some("txt".into()),
            kind: crate::DiskObjectKind::File,
            size: Some(0),
            recursive_size: None,
            dev: None,
            ino: None,
            mtime: None,
        };
        let index = Arc::new(Mutex::new(build_index(&[ghost])));
        assert_eq!(index.lock().unwrap().live_count(), 1);

        let scan = Arc::new(AtomicBool::new(false));
        let _rec = IndexReconciler::new(
            Arc::clone(&index),
            vec![dir.path().to_path_buf()],
            scan,
        );

        let removed = poll_until(
            || index.lock().unwrap().live_count() == 0,
            Duration::from_secs(2),
        );
        assert!(removed, "reconciler should have removed the nonexistent ghost.txt");
    }

    #[test]
    fn does_not_duplicate_already_indexed_files() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("existing.txt");
        std::fs::write(&file, b"data").unwrap();

        // Pre-index the file using the same path format the reconciler will see.
        let obj = disk_object_from_path(&file).expect("file exists");
        let index = Arc::new(Mutex::new(build_index(&[obj])));
        assert_eq!(index.lock().unwrap().live_count(), 1);

        let scan = Arc::new(AtomicBool::new(false));
        let _rec = IndexReconciler::new(
            Arc::clone(&index),
            vec![dir.path().to_path_buf()],
            Arc::clone(&scan),
        );

        // Give the reconciler enough time to complete a full pass.
        thread::sleep(Duration::from_millis(500));
        assert_eq!(
            index.lock().unwrap().live_count(),
            1,
            "reconciler must not add a duplicate for a file already in the index"
        );
    }

    #[test]
    fn pauses_while_scan_in_progress_then_resumes() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("blocked.txt");
        std::fs::write(&file, b"x").unwrap();

        let index = Arc::new(Mutex::new(build_index(&[])));
        let scan = Arc::new(AtomicBool::new(true)); // scan is running
        let _rec = IndexReconciler::new(
            Arc::clone(&index),
            vec![dir.path().to_path_buf()],
            Arc::clone(&scan),
        );

        // The reconciler polls scan_in_progress before sleeping 1 s.  After 300 ms it
        // is guaranteed to be mid-sleep, so the index must still be empty.
        thread::sleep(Duration::from_millis(300));
        assert!(
            !index.lock().unwrap().path_to_idx.contains_key(&path_str(&file)),
            "reconciler must not run while scan_in_progress is true"
        );

        // Release the scan lock — the reconciler wakes on its next 1 s tick.
        scan.store(false, Ordering::Relaxed);

        let added = poll_until(
            || index.lock().unwrap().path_to_idx.contains_key(&path_str(&file)),
            Duration::from_secs(3),
        );
        assert!(added, "reconciler should have run after scan_in_progress was cleared");
    }

    #[test]
    fn drop_stops_the_thread() {
        let dir = TempDir::new().unwrap();
        let index = Arc::new(Mutex::new(build_index(&[])));
        let scan = Arc::new(AtomicBool::new(false));

        let rec = IndexReconciler::new(
            Arc::clone(&index),
            vec![dir.path().to_path_buf()],
            scan,
        );

        // Drop the reconciler — this signals cancel = true.
        drop(rec);

        // Create a file AFTER drop.  The reconciler thread may still be alive for a
        // brief moment, but it will exit on its next cancellation check before adding.
        // We verify the index count doesn't grow beyond what it was at drop time.
        let count_at_drop = index.lock().unwrap().live_count();
        let file = dir.path().join("late.txt");
        std::fs::write(&file, b"late").unwrap();

        thread::sleep(Duration::from_millis(200));
        assert_eq!(
            index.lock().unwrap().live_count(),
            count_at_drop,
            "reconciler should not update the index after being dropped"
        );
    }
}
