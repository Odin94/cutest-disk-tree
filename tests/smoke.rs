use cutest_disk_tree::{index_directory, index_directory_with_progress, to_scan_result};

#[test]
fn index_directory_finds_files_and_aggregates_folder_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    std::fs::write(root.join("a.txt"), b"hello").unwrap();
    std::fs::write(root.join("b.txt"), b"world").unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("sub").join("c.txt"), b"xyz").unwrap();

    let (files, folder_sizes) = index_directory(root);

    assert_eq!(files.len(), 3, "should find 3 files");
    let paths: Vec<_> = files.iter().map(|(p, _, _)| p.clone()).collect();
    assert!(paths.iter().any(|p| p.ends_with("a.txt")));
    assert!(paths.iter().any(|p| p.ends_with("b.txt")));
    assert!(paths.iter().any(|p| p.ends_with("c.txt")));

    let root_size = folder_sizes.get(root).copied().unwrap_or(0);
    assert_eq!(root_size, 5 + 5 + 3, "root should sum to 13 bytes");

    let sub = root.join("sub");
    let sub_size = folder_sizes.get(&sub).copied().unwrap_or(0);
    assert_eq!(sub_size, 3, "sub should be 3 bytes");
}

#[test]
fn index_directory_with_progress_invokes_callback() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("f1.txt"), b"a").unwrap();
    std::fs::write(root.join("f2.txt"), b"b").unwrap();

    let mut progress_count = 0;
    let (files, _) = index_directory_with_progress(root, |p| {
        progress_count += 1;
        assert!(p.files_count <= 2);
    });

    assert_eq!(files.len(), 2);
    assert!(progress_count >= 1, "progress should be called at least once (start and/or updates)");
}

#[test]
fn to_scan_result_produces_serializable_result() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("x.txt"), b"data").unwrap();

    let (files, folder_sizes) = index_directory(root);
    let result = to_scan_result(root, &files, &folder_sizes).unwrap();

    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].size, 4);
    assert_eq!(result.folder_sizes.len(), 1);
    assert!(!result.root.is_empty());
}
