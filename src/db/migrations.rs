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

pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(MIGRATION_1_INITIAL_SCHEMA),
        M::up(MIGRATION_2_ADD_NAMES_AND_TYPES),
    ])
}

