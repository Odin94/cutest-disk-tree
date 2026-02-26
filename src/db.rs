use rusqlite::Connection;
use std::path::Path;

use crate::FileKey;

const SCHEMA: &str = "
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
";

pub fn create_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)
}

pub fn write_scan(
    conn: &Connection,
    root: &str,
    files: &[(std::path::PathBuf, u64, FileKey)],
    folder_sizes: &std::collections::HashMap<std::path::PathBuf, u64>,
) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM files WHERE root = ?1", [root])?;
    conn.execute("DELETE FROM folders WHERE root = ?1", [root])?;

    let mut file_stmt = conn.prepare(
        "INSERT INTO files (root, path, size, dev, ino, hash, mtime) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for (path, size, key) in files {
        let path_str = path.to_string_lossy();
        file_stmt.execute((
            root,
            path_str.as_ref(),
            *size as i64,
            key.dev as i64,
            key.ino as i64,
            None::<&str>,
            None::<i64>,
        ))?;
    }

    let mut folder_stmt =
        conn.prepare("INSERT INTO folders (root, path, recursive_size) VALUES (?1, ?2, ?3)")?;
    for (path, size) in folder_sizes {
        let path_str = path.to_string_lossy();
        folder_stmt.execute((root, path_str.as_ref(), *size as i64))?;
    }

    Ok(())
}

pub fn list_roots(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT DISTINCT root FROM folders ORDER BY root")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    rows.collect()
}

pub fn get_scan_result(
    conn: &Connection,
    root: &str,
) -> rusqlite::Result<Option<crate::ScanResult>> {
    let file_count: i64 = conn.query_row(
        "SELECT COUNT(1) FROM files WHERE root = ?1",
        [root],
        |row| row.get(0),
    )?;
    if file_count == 0 {
        return Ok(None);
    }

    let mut file_stmt = conn.prepare(
        "SELECT path, size, dev, ino FROM files WHERE root = ?1 ORDER BY path",
    )?;
    let files: Vec<crate::FileEntrySer> = file_stmt
        .query_map([root], |row| {
            Ok(crate::FileEntrySer {
                path: row.get(0)?,
                size: row.get::<_, i64>(1)? as u64,
                file_key: FileKey {
                    dev: row.get::<_, i64>(2)? as u64,
                    ino: row.get::<_, i64>(3)? as u64,
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut folder_stmt = conn.prepare(
        "SELECT path, recursive_size FROM folders WHERE root = ?1 ORDER BY recursive_size DESC",
    )?;
    let folder_sizes: std::collections::HashMap<String, u64> = folder_stmt
        .query_map([root], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u64,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .collect();

    Ok(Some(crate::ScanResult {
        root: root.to_string(),
        files,
        folder_sizes,
    }))
}

pub fn open_db(db_path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(db_path)?;
    create_tables(&conn)?;
    Ok(conn)
}
