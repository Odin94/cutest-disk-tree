use cutest_disk_tree::{db, index_directory, to_scan_result};
use std::path::PathBuf;

#[test]
fn indexing_and_db_round_trip_works() {
    let dir = tempfile::tempdir().unwrap();
    let root_dir = dir.path().join("test-data");
    std::fs::create_dir_all(&root_dir).unwrap();

    std::fs::write(root_dir.join("one.txt"), b"hello").unwrap();
    std::fs::write(root_dir.join("two.txt"), b"world").unwrap();

    let db_path: PathBuf = dir.path().join("test-data.db");

    let (files, folder_sizes) = index_directory(&root_dir);
    let scan_before = to_scan_result(&[root_dir.as_path()], &files, &folder_sizes).unwrap();

    let conn = db::open_db(&db_path).expect("failed to open db with migrations");
    db::write_scan(&conn, &files, &folder_sizes, 1)
        .expect("failed to write scan");

    let loaded = db::get_scan_result(&conn)
        .expect("failed to load scan")
        .expect("expected some scan data");

    assert_eq!(loaded.files.len(), scan_before.files.len());
    assert_eq!(loaded.folder_sizes.len(), scan_before.folder_sizes.len());
}

#[test]
fn write_scan_then_get_disk_objects_count() {
    let dir = tempfile::tempdir().unwrap();
    let root_dir = dir.path().join("data");
    std::fs::create_dir_all(&root_dir).unwrap();
    std::fs::write(root_dir.join("a.txt"), b"aa").unwrap();
    std::fs::write(root_dir.join("b.txt"), b"bb").unwrap();

    let db_path = dir.path().join("test.db");
    let (files, folder_sizes) = index_directory(&root_dir);
    let conn = db::open_db(&db_path).unwrap();
    db::write_scan(&conn, &files, &folder_sizes, 1).unwrap();

    let objs = db::get_disk_objects(&conn).unwrap();
    let file_count = objs.iter().filter(|o| matches!(o.kind, cutest_disk_tree::DiskObjectKind::File)).count();
    assert_eq!(file_count, 2);
    let folder_count = objs.iter().filter(|o| matches!(o.kind, cutest_disk_tree::DiskObjectKind::Folder)).count();
    assert!(folder_count >= 1, "should have at least the root folder");
}

#[test]
fn get_cached_tree_miss_and_hit() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let miss = db::get_cached_tree(&conn, 3, 8).unwrap();
    assert!(miss.is_none(), "should miss when no tree is cached");

    let node = cutest_disk_tree::DiskTreeNode {
        path: "/root".to_string(),
        name: "root".to_string(),
        size: 100,
        children: None,
    };
    db::write_cached_tree(&conn, 3, 8, &node).unwrap();

    let hit = db::get_cached_tree(&conn, 3, 8).unwrap();
    assert!(hit.is_some(), "should hit after write");
    let cached = hit.unwrap();
    assert_eq!(cached.path, "/root");
    assert_eq!(cached.size, 100);
}

#[test]
fn suffix_index_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    let buffer = "hello\0world\0";
    let offsets: Vec<usize> = vec![0, 6];
    let disk_object_indices: Vec<usize> = vec![0, 1];
    db::write_suffix_index_data(&conn, 42, buffer, &offsets, &disk_object_indices).unwrap();

    let loaded = db::read_suffix_index_data(&conn)
        .unwrap()
        .expect("expected suffix index data");
    assert_eq!(loaded.0, buffer);
    assert_eq!(loaded.1, offsets);
    assert_eq!(loaded.2, disk_object_indices);
}

#[test]
fn get_folder_size_works() {
    let dir = tempfile::tempdir().unwrap();
    let root_dir = dir.path().join("data");
    std::fs::create_dir_all(&root_dir).unwrap();
    std::fs::write(root_dir.join("f.txt"), b"12345").unwrap();

    let db_path = dir.path().join("test.db");
    let (files, folder_sizes) = index_directory(&root_dir);
    let conn = db::open_db(&db_path).unwrap();
    db::write_scan(&conn, &files, &folder_sizes, 1).unwrap();

    let root_str = root_dir.to_string_lossy();
    let size = db::get_folder_size(&conn, &root_str).unwrap();
    assert!(size.is_some());
    assert_eq!(size.unwrap(), 5);
}

#[test]
fn read_scan_metadata_after_write() {
    let dir = tempfile::tempdir().unwrap();
    let root_dir = dir.path().join("data");
    std::fs::create_dir_all(&root_dir).unwrap();
    std::fs::write(root_dir.join("f.txt"), b"x").unwrap();

    let db_path = dir.path().join("test.db");
    let (files, folder_sizes) = index_directory(&root_dir);
    let conn = db::open_db(&db_path).unwrap();
    db::write_scan(&conn, &files, &folder_sizes, 99).unwrap();

    let meta = db::read_scan_metadata(&conn).unwrap();
    assert!(meta.is_some());
    let m = meta.unwrap();
    assert_eq!(m.disk_objects_update_id, 99);
}

#[test]
fn has_disk_objects_empty_and_populated() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = db::open_db(&db_path).unwrap();

    assert!(!db::has_disk_objects(&conn).unwrap());

    let root_dir = dir.path().join("data");
    std::fs::create_dir_all(&root_dir).unwrap();
    std::fs::write(root_dir.join("f.txt"), b"x").unwrap();
    let (files, folder_sizes) = index_directory(&root_dir);
    db::write_scan(&conn, &files, &folder_sizes, 1).unwrap();

    assert!(db::has_disk_objects(&conn).unwrap());
}
