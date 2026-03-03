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

pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(MIGRATION_1_INITIAL_SCHEMA)])
}

