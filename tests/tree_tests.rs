use cutest_disk_tree::{
    db, index_directory, to_scan_result, build_disk_tree, build_disk_tree_from_db,
    FileEntrySer, FileKey, ScanResult,
};
use std::collections::HashMap;

fn make_scan(root: &str, files: Vec<FileEntrySer>, folder_sizes: HashMap<String, u64>) -> ScanResult {
    ScanResult {
        roots: vec![root.to_string()],
        files,
        folder_sizes,
    }
}

#[test]
fn build_disk_tree_with_varied_max_depth() {
    let mut folder_sizes = HashMap::new();
    folder_sizes.insert("C:\\data".to_string(), 100);
    folder_sizes.insert("C:\\data\\sub".to_string(), 60);
    folder_sizes.insert("C:\\data\\sub\\deep".to_string(), 30);

    let files = vec![
        FileEntrySer { path: "C:\\data\\sub\\deep\\f.txt".to_string(), size: 30, file_key: FileKey { dev: 1, ino: 1 }, mtime: None },
        FileEntrySer { path: "C:\\data\\sub\\g.txt".to_string(), size: 30, file_key: FileKey { dev: 1, ino: 2 }, mtime: None },
        FileEntrySer { path: "C:\\data\\h.txt".to_string(), size: 40, file_key: FileKey { dev: 1, ino: 3 }, mtime: None },
    ];

    let scan = make_scan("C:\\data", files, folder_sizes);

    let tree_d0 = build_disk_tree(&scan, "C:\\data", 10, 0).expect("depth-0 tree");
    assert_eq!(tree_d0.size, 100);
    assert!(tree_d0.children.is_none(), "depth 0 should have no children expanded");

    let tree_d2 = build_disk_tree(&scan, "C:\\data", 10, 2).expect("depth-2 tree");
    assert!(tree_d2.children.is_some());
    let children = tree_d2.children.unwrap();
    assert!(children.len() >= 2, "should have folder and file children");
}

#[test]
fn build_disk_tree_sizes_roll_up() {
    let mut folder_sizes = HashMap::new();
    folder_sizes.insert("/root".to_string(), 15);
    folder_sizes.insert("/root/sub".to_string(), 10);

    let files = vec![
        FileEntrySer { path: "/root/sub/file1".to_string(), size: 10, file_key: FileKey { dev: 1, ino: 1 }, mtime: None },
        FileEntrySer { path: "/root/file2".to_string(), size: 5, file_key: FileKey { dev: 1, ino: 2 }, mtime: None },
    ];

    let scan = make_scan("/root", files, folder_sizes);
    let tree = build_disk_tree(&scan, "/root", 10, 10).expect("tree");

    assert_eq!(tree.size, 15);
    let children = tree.children.as_ref().expect("should have children");
    let sub = children.iter().find(|c| c.path == "/root/sub").expect("sub folder");
    assert_eq!(sub.size, 10);
}

#[test]
fn build_disk_tree_from_db_matches_in_memory() {
    let dir = tempfile::tempdir().unwrap();
    let root_dir = dir.path().join("data");
    std::fs::create_dir_all(root_dir.join("sub")).unwrap();
    std::fs::write(root_dir.join("a.txt"), b"hello").unwrap();
    std::fs::write(root_dir.join("sub").join("b.txt"), b"world!").unwrap();

    let (files, folder_sizes) = index_directory(&root_dir);
    let scan = to_scan_result(&[root_dir.as_path()], &files, &folder_sizes).unwrap();

    let db_path = dir.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();
    db::write_scan(&conn, &files, &folder_sizes, 1).unwrap();

    let root_str = root_dir.to_string_lossy().to_string();
    let from_memory = build_disk_tree(&scan, &root_str, 8, 3);
    let from_db = build_disk_tree_from_db(&conn, &root_str, 8, 3);

    assert!(from_memory.is_some());
    assert!(from_db.is_some());

    let mem_tree = from_memory.unwrap();
    let db_tree = from_db.unwrap();
    assert_eq!(mem_tree.size, db_tree.size);
    assert_eq!(mem_tree.name, db_tree.name);
}
