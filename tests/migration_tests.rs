use rusqlite::Connection;

#[test]
fn fresh_db_has_expected_tables_after_all_migrations() {
    let conn = Connection::open_in_memory().unwrap();
    let migrations = cutest_disk_tree::db::migrations::migrations();
    migrations.to_latest(&mut conn.into()).unwrap_or_else(|e| {
        panic!("migrations failed: {:?}", e);
    });

    // The into() consumed conn, so reopen. For in-memory we use a file-based temp instead.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = cutest_disk_tree::db::open_db(&db_path).unwrap();

    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert!(tables.contains(&"disk_objects".to_string()));
    assert!(tables.contains(&"cached_trees".to_string()));
    assert!(tables.contains(&"scan_metadata".to_string()));
    assert!(tables.contains(&"suffix_index_data".to_string()));
    assert!(!tables.contains(&"file_search_trigrams".to_string()), "trigrams table should be dropped");
}

#[test]
fn disk_objects_has_path_as_primary_key() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = cutest_disk_tree::db::open_db(&db_path).unwrap();

    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'disk_objects'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert!(!sql.contains("root TEXT"), "root column should not exist after migration 3");
    assert!(sql.contains("path TEXT NOT NULL PRIMARY KEY"));
}

#[test]
fn cached_trees_pk_is_depth_children() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = cutest_disk_tree::db::open_db(&db_path).unwrap();

    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'cached_trees'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert!(!sql.contains("root TEXT"));
    assert!(sql.contains("PRIMARY KEY (max_depth, max_children)"));
}

#[test]
fn scan_metadata_is_single_row() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = cutest_disk_tree::db::open_db(&db_path).unwrap();

    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'scan_metadata'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    assert!(!sql.contains("root TEXT"));
    assert!(sql.contains("CHECK(id = 1)"));
}
