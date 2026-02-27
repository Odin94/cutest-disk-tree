use cutest_disk_tree::{db, index_directory, to_scan_result};
use std::path::PathBuf;
use rusqlite::Connection;

#[test]
fn indexing_and_db_round_trip_works() {
    let dir = tempfile::tempdir().unwrap();
    let root_dir = dir.path().join("test-data");
    std::fs::create_dir_all(&root_dir).unwrap();

    std::fs::write(root_dir.join("one.txt"), b"hello").unwrap();
    std::fs::write(root_dir.join("two.txt"), b"world").unwrap();

    let db_path: PathBuf = dir.path().join("test-data.db");

    let (files, folder_sizes) = index_directory(&root_dir);
    let scan_before = to_scan_result(&root_dir, &files, &folder_sizes).unwrap();

    let conn = db::open_db(&db_path).expect("failed to open db with migrations");
    db::write_scan(&conn, &root_dir.to_string_lossy(), &files, &folder_sizes)
        .expect("failed to write scan");

    let loaded = db::get_scan_result(&conn, &root_dir.to_string_lossy())
        .expect("failed to load scan")
        .expect("expected some scan data");

    assert_eq!(loaded.root, scan_before.root);
    assert_eq!(loaded.files.len(), scan_before.files.len());
    assert_eq!(loaded.folder_sizes.len(), scan_before.folder_sizes.len());

    // Validate name and type columns in the files table.
    let conn2 = Connection::open(&db_path).unwrap();
    let mut stmt = conn2
        .prepare("SELECT name, type FROM files ORDER BY path")
        .unwrap();
    let rows: Vec<(Option<String>, Option<String>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert_eq!(rows.len(), 2);
    assert!(rows.contains(&(Some("one.txt".to_string()), Some("txt".to_string()))));
    assert!(rows.contains(&(Some("two.txt".to_string()), Some("txt".to_string()))));
}

