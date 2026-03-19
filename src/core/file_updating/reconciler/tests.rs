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
