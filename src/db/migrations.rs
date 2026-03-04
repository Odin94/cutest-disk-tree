use rusqlite_migration::{Migrations, M};

/// Initial (and current) schema for the database.
///
/// This is designed for the unified `DiskObject` world, where we only
/// persist `disk_objects` instead of separate `files` / `folders` tables.
pub const MIGRATION_1_INITIAL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS disk_objects (
    root TEXT NOT NULL,
    path TEXT NOT NULL,
    path_lower TEXT,
    parent_path TEXT,
    name TEXT,
    name_lower TEXT,
    ext TEXT,
    kind TEXT NOT NULL,
    size INTEGER,
    recursive_size INTEGER,
    dev INTEGER,
    ino INTEGER,
    mtime INTEGER,
    PRIMARY KEY (root, path)
);
CREATE INDEX IF NOT EXISTS idx_disk_objects_root_parent_kind ON disk_objects(root, parent_path, kind);
CREATE INDEX IF NOT EXISTS idx_disk_objects_root_kind_ext ON disk_objects(root, kind, ext);
CREATE INDEX IF NOT EXISTS idx_disk_objects_dev_ino ON disk_objects(dev, ino);
CREATE INDEX IF NOT EXISTS idx_disk_objects_root_path_lower ON disk_objects(root, path_lower);
CREATE INDEX IF NOT EXISTS idx_disk_objects_root_name_lower ON disk_objects(root, name_lower);

CREATE TABLE IF NOT EXISTS cached_trees (
    root TEXT NOT NULL,
    max_depth INTEGER NOT NULL,
    max_children INTEGER NOT NULL,
    tree_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (root, max_depth, max_children)
);

CREATE TABLE IF NOT EXISTS file_search_trigrams (
    root TEXT NOT NULL,
    path TEXT NOT NULL,
    trigram TEXT NOT NULL,
    PRIMARY KEY (root, path, trigram)
);
CREATE INDEX IF NOT EXISTS idx_trigrams_root_token ON file_search_trigrams(root, trigram);
"#;

pub const MIGRATION_2_SCAN_METADATA: &str = r#"
CREATE TABLE IF NOT EXISTS scan_metadata (
    root TEXT NOT NULL PRIMARY KEY,
    disk_objects_update_id INTEGER NOT NULL DEFAULT 0,
    disk_objects_last_updated INTEGER NOT NULL DEFAULT 0,
    suffix_index_update_id INTEGER NOT NULL DEFAULT 0,
    suffix_index_last_updated INTEGER NOT NULL DEFAULT 0,
    cached_trees_update_id INTEGER NOT NULL DEFAULT 0,
    cached_trees_last_updated INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS suffix_index_data (
    root TEXT NOT NULL PRIMARY KEY,
    buffer TEXT NOT NULL,
    offsets BLOB NOT NULL,
    disk_object_indices BLOB NOT NULL
);
"#;

pub const MIGRATION_3_REMOVE_ROOT: &str = r#"
-- disk_objects: drop root column, PK becomes (path)
CREATE TABLE disk_objects_new (
    path TEXT NOT NULL PRIMARY KEY,
    path_lower TEXT,
    parent_path TEXT,
    name TEXT,
    name_lower TEXT,
    ext TEXT,
    kind TEXT NOT NULL,
    size INTEGER,
    recursive_size INTEGER,
    dev INTEGER,
    ino INTEGER,
    mtime INTEGER
);
INSERT OR IGNORE INTO disk_objects_new
    SELECT path, path_lower, parent_path, name, name_lower, ext, kind, size, recursive_size, dev, ino, mtime
    FROM disk_objects;
DROP TABLE disk_objects;
ALTER TABLE disk_objects_new RENAME TO disk_objects;

CREATE INDEX idx_disk_objects_parent_kind ON disk_objects(parent_path, kind);
CREATE INDEX idx_disk_objects_kind_ext ON disk_objects(kind, ext);
CREATE INDEX idx_disk_objects_dev_ino ON disk_objects(dev, ino);
CREATE INDEX idx_disk_objects_path_lower ON disk_objects(path_lower);
CREATE INDEX idx_disk_objects_name_lower ON disk_objects(name_lower);

-- cached_trees: drop root column
CREATE TABLE cached_trees_new (
    max_depth INTEGER NOT NULL,
    max_children INTEGER NOT NULL,
    tree_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (max_depth, max_children)
);
INSERT OR IGNORE INTO cached_trees_new
    SELECT max_depth, max_children, tree_json, created_at
    FROM cached_trees;
DROP TABLE cached_trees;
ALTER TABLE cached_trees_new RENAME TO cached_trees;

-- file_search_trigrams: dead code, drop entirely
DROP TABLE IF EXISTS file_search_trigrams;

-- scan_metadata: single-row table
CREATE TABLE scan_metadata_new (
    id INTEGER NOT NULL PRIMARY KEY DEFAULT 1 CHECK(id = 1),
    disk_objects_update_id INTEGER NOT NULL DEFAULT 0,
    disk_objects_last_updated INTEGER NOT NULL DEFAULT 0,
    suffix_index_update_id INTEGER NOT NULL DEFAULT 0,
    suffix_index_last_updated INTEGER NOT NULL DEFAULT 0,
    cached_trees_update_id INTEGER NOT NULL DEFAULT 0,
    cached_trees_last_updated INTEGER NOT NULL DEFAULT 0
);
INSERT OR IGNORE INTO scan_metadata_new
    (id, disk_objects_update_id, disk_objects_last_updated,
     suffix_index_update_id, suffix_index_last_updated,
     cached_trees_update_id, cached_trees_last_updated)
    SELECT 1, disk_objects_update_id, disk_objects_last_updated,
           suffix_index_update_id, suffix_index_last_updated,
           cached_trees_update_id, cached_trees_last_updated
    FROM scan_metadata LIMIT 1;
DROP TABLE scan_metadata;
ALTER TABLE scan_metadata_new RENAME TO scan_metadata;

-- suffix_index_data: single-row table
CREATE TABLE suffix_index_data_new (
    id INTEGER NOT NULL PRIMARY KEY DEFAULT 1 CHECK(id = 1),
    buffer TEXT NOT NULL,
    offsets BLOB NOT NULL,
    disk_object_indices BLOB NOT NULL
);
INSERT OR IGNORE INTO suffix_index_data_new (id, buffer, offsets, disk_object_indices)
    SELECT 1, buffer, offsets, disk_object_indices
    FROM suffix_index_data LIMIT 1;
DROP TABLE suffix_index_data;
ALTER TABLE suffix_index_data_new RENAME TO suffix_index_data;
"#;

pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(MIGRATION_1_INITIAL_SCHEMA),
        M::up(MIGRATION_2_SCAN_METADATA),
        M::up(MIGRATION_3_REMOVE_ROOT),
    ])
}

