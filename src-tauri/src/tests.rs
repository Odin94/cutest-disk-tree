use super::*;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

fn make_disk_object(name: &str) -> DiskObject {
    DiskObject {
        path: format!("/root/{}", name),
        path_lower: format!("/root/{}", name.to_ascii_lowercase()),
        parent_path: Some("/root".to_string()),
        name: name.to_string(),
        name_lower: name.to_ascii_lowercase(),
        ext: None,
        kind: DiskObjectKind::File,
        size: Some(1),
        recursive_size: None,
        dev: None,
        ino: None,
        mtime: None,
    }
}

#[test]
fn paginate_scan_empty_query_respects_offset_and_limit() {
    let entries: Vec<DiskObject> = (0..10)
        .map(|i| make_disk_object(&format!("file{}", i)))
        .collect();
    let (page1, next1) =
        paginate_scan(&entries, 0, 3, false, &Vec::new(), |_i, _e| true);
    assert_eq!(page1.len(), 3);
    assert_eq!(page1[0].path, "/root/file0");
    assert_eq!(next1, Some(3));

    let (page2, next2) =
        paginate_scan(&entries, next1.unwrap(), 3, false, &Vec::new(), |_i, _e| true);
    assert_eq!(page2.len(), 3);
    assert_eq!(page2[0].path, "/root/file3");
    assert_eq!(next2, Some(6));

    let (page_last, next_last) =
        paginate_scan(&entries, next2.unwrap(), 10, false, &Vec::new(), |_i, _e| true);
    assert_eq!(page_last.len(), 4);
    assert_eq!(page_last[0].path, "/root/file6");
    assert_eq!(next_last, None);
}

#[test]
fn paginate_scan_with_filter_skips_non_matches() {
    let entries = vec![
        make_disk_object("keep1"),
        make_disk_object("skip"),
        make_disk_object("keep2"),
        make_disk_object("keep3"),
    ];
    let (page, next) = paginate_scan(
        &entries,
        0,
        2,
        false,
        &Vec::new(),
        |_i, e| e.name.starts_with("keep"),
    );
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].path, "/root/keep1");
    assert_eq!(page[1].path, "/root/keep2");
    assert_eq!(next, Some(3));

    let (page2, next2) =
        paginate_scan(&entries, next.unwrap(), 2, false, &Vec::new(), |_i, e| {
            e.name.starts_with("keep")
        });
    assert_eq!(page2.len(), 1);
    assert_eq!(page2[0].path, "/root/keep3");
    assert_eq!(next2, None);
}

#[test]
fn paginate_scan_limit_larger_than_total() {
    let entries: Vec<DiskObject> = (0..3)
        .map(|i| make_disk_object(&format!("f{}", i)))
        .collect();
    let (page, next) =
        paginate_scan(&entries, 0, 100, false, &Vec::new(), |_i, _e| true);
    assert_eq!(page.len(), 3);
    assert_eq!(next, None);
}

#[test]
fn paginate_scan_offset_past_end() {
    let entries: Vec<DiskObject> = (0..3)
        .map(|i| make_disk_object(&format!("f{}", i)))
        .collect();
    let (page, next) =
        paginate_scan(&entries, 10, 3, false, &Vec::new(), |_i, _e| true);
    assert!(page.is_empty());
    assert_eq!(next, None);
}

#[test]
fn scan_guard_prevents_concurrent_scans() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("a.txt"), b"hello").unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("sub").join("b.txt"), b"world").unwrap();

    let is_scanning = Arc::new(AtomicBool::new(false));
    let completed_scans = Arc::new(AtomicUsize::new(0));
    let rejected_scans = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(std::sync::Barrier::new(4));

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let root = root.to_path_buf();
            let is_scanning = Arc::clone(&is_scanning);
            let completed = Arc::clone(&completed_scans);
            let rejected = Arc::clone(&rejected_scans);
            let barrier = Arc::clone(&barrier);

            std::thread::spawn(move || {
                barrier.wait();

                if is_scanning
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                {
                    rejected.fetch_add(1, Ordering::SeqCst);
                    return;
                }

                let (files, _folder_paths) =
                    cutest_disk_tree::index_directory_ignore_with_progress(&root, |_| {});
                assert!(files.len() >= 2, "scan should find at least 2 files");

                is_scanning.store(false, Ordering::SeqCst);
                completed.fetch_add(1, Ordering::SeqCst);
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    let completed = completed_scans.load(Ordering::SeqCst);
    let rejected = rejected_scans.load(Ordering::SeqCst);

    assert!(completed >= 1, "at least one scan should complete");
    assert!(rejected >= 1, "at least one concurrent attempt should be rejected");
    assert_eq!(completed + rejected, 4, "all threads should either complete or be rejected");
}
