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
