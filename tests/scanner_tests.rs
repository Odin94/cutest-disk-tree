use cutest_disk_tree::{
    index_directory, index_directory_with_progress, index_directory_ignore_with_progress,
};

#[test]
fn scan_empty_directory() {
    let dir = tempfile::tempdir().unwrap();
    let (files, folder_sizes) = index_directory(dir.path());
    assert_eq!(files.len(), 0);
    let root_size = folder_sizes.get(dir.path()).copied().unwrap_or(0);
    assert_eq!(root_size, 0);
}

#[test]
fn scan_single_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("only.txt"), b"content").unwrap();

    let (files, folder_sizes) = index_directory(dir.path());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].size, 7);
    assert_eq!(folder_sizes.get(dir.path()).copied().unwrap_or(0), 7);
}

#[test]
fn scan_deeply_nested_paths() {
    let dir = tempfile::tempdir().unwrap();
    let deep = dir.path().join("a").join("b").join("c").join("d");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("leaf.txt"), b"x").unwrap();

    let (files, folder_sizes) = index_directory(dir.path());
    assert_eq!(files.len(), 1);
    assert!(folder_sizes.get(dir.path()).is_some());
    assert!(folder_sizes.get(&dir.path().join("a")).is_some());
    assert!(folder_sizes.get(&deep).is_some());
}

#[test]
fn scan_unicode_filenames() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("日本語.txt"), b"data").unwrap();
    std::fs::write(dir.path().join("émojis_🎉.log"), b"yay").unwrap();

    let (files, _) = index_directory(dir.path());
    assert_eq!(files.len(), 2);
    let names: Vec<String> = files.iter().map(|f| f.path.to_string_lossy().to_string()).collect();
    assert!(names.iter().any(|n| n.contains("日本語")));
    assert!(names.iter().any(|n| n.contains("émojis")));
}

#[test]
fn index_directory_with_progress_reports_counts() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..5 {
        std::fs::write(dir.path().join(format!("f{}.txt", i)), b"x").unwrap();
    }

    let mut max_count = 0u64;
    let (files, _) = index_directory_with_progress(dir.path(), |p| {
        if p.files_count > max_count {
            max_count = p.files_count;
        }
    });

    assert_eq!(files.len(), 5);
}

#[test]
fn ignore_walker_finds_same_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"aa").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub").join("b.txt"), b"bb").unwrap();

    let (files, folders) = index_directory_ignore_with_progress(dir.path(), |_| {});
    assert_eq!(files.len(), 2);
    assert!(folders.len() >= 1);
}
