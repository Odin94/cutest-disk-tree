use rusqlite_migration::{Migrations, M};

pub const MIGRATION_1_INITIAL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS files (
    root TEXT NOT NULL,
    path TEXT NOT NULL,
    size INTEGER NOT NULL,
    dev INTEGER NOT NULL,
    ino INTEGER NOT NULL,
    hash TEXT,
    mtime INTEGER
);
CREATE INDEX IF NOT EXISTS idx_files_root_path ON files(root, path);
CREATE INDEX IF NOT EXISTS idx_files_root_size ON files(root, size DESC);
CREATE INDEX IF NOT EXISTS idx_files_dev_ino ON files(dev, ino);
CREATE INDEX IF NOT EXISTS idx_files_hash ON files(hash) WHERE hash IS NOT NULL;

CREATE TABLE IF NOT EXISTS folders (
    root TEXT NOT NULL,
    path TEXT NOT NULL,
    recursive_size INTEGER NOT NULL,
    PRIMARY KEY (root, path)
);
CREATE INDEX IF NOT EXISTS idx_folders_root_size ON folders(root, recursive_size DESC);
"#;

pub const MIGRATION_2_ADD_NAMES_AND_TYPES: &str = r#"
ALTER TABLE files ADD COLUMN name TEXT;
ALTER TABLE files ADD COLUMN type TEXT;
ALTER TABLE folders ADD COLUMN name TEXT;
"#;

pub const MIGRATION_3_CACHED_TREES: &str = r#"
CREATE TABLE IF NOT EXISTS cached_trees (
    root TEXT NOT NULL,
    max_depth INTEGER NOT NULL,
    max_children INTEGER NOT NULL,
    tree_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (root, max_depth, max_children)
);
"#;

pub const MIGRATION_4_PARENT_PATH: &str = r#"
ALTER TABLE files ADD COLUMN parent_path TEXT;
ALTER TABLE folders ADD COLUMN parent_path TEXT;
CREATE INDEX IF NOT EXISTS idx_files_root_parent ON files(root, parent_path);
CREATE INDEX IF NOT EXISTS idx_folders_root_parent ON folders(root, parent_path);
"#;

pub const MIGRATION_5_CLEAR_CACHED_TREES_OTHER_FIX: &str = r#"
DELETE FROM cached_trees;
"#;

pub const MIGRATION_6_REVERSE_INDEX: &str = r#"
CREATE TABLE IF NOT EXISTS file_search_trigrams (
    root TEXT NOT NULL,
    path TEXT NOT NULL,
    trigram TEXT NOT NULL,
    PRIMARY KEY (root, path, trigram)
);
CREATE INDEX IF NOT EXISTS idx_trigrams_root_token ON file_search_trigrams(root, trigram);
"#;

pub const MIGRATION_7_UNIFIED_ITEMS: &str = r#"
CREATE TABLE IF NOT EXISTS items (
    root TEXT NOT NULL,
    path TEXT NOT NULL,
    parent_path TEXT,
    name TEXT,
    ext TEXT,
    kind TEXT NOT NULL,
    size INTEGER,
    recursive_size INTEGER,
    dev INTEGER,
    ino INTEGER,
    mtime INTEGER,
    PRIMARY KEY (root, path)
);
CREATE INDEX IF NOT EXISTS idx_items_root_parent_kind ON items(root, parent_path, kind);
CREATE INDEX IF NOT EXISTS idx_items_root_kind_ext ON items(root, kind, ext);
CREATE INDEX IF NOT EXISTS idx_items_dev_ino ON items(dev, ino);
"#;

pub const MIGRATION_8_ITEMS_LOWER: &str = r#"
ALTER TABLE items ADD COLUMN path_lower TEXT;
ALTER TABLE items ADD COLUMN name_lower TEXT;
CREATE INDEX IF NOT EXISTS idx_items_root_path_lower ON items(root, path_lower);
CREATE INDEX IF NOT EXISTS idx_items_root_name_lower ON items(root, name_lower);
"#;

pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(MIGRATION_1_INITIAL_SCHEMA),
        M::up(MIGRATION_2_ADD_NAMES_AND_TYPES),
        M::up(MIGRATION_3_CACHED_TREES),
        M::up(MIGRATION_4_PARENT_PATH),
        M::up(MIGRATION_5_CLEAR_CACHED_TREES_OTHER_FIX),
        M::up(MIGRATION_6_REVERSE_INDEX),
        M::up(MIGRATION_7_UNIFIED_ITEMS),
        M::up(MIGRATION_8_ITEMS_LOWER),
    ])
}

