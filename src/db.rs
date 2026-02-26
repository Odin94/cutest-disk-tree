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

const BATCH_SIZE: usize = 200;

pub fn write_scan(
    conn: &Connection,
    root: &str,
    files: &[(std::path::PathBuf, u64, FileKey)],
    folder_sizes: &std::collections::HashMap<std::path::PathBuf, u64>,
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute("DELETE FROM files WHERE root = ?1", [root])?;
    tx.execute("DELETE FROM folders WHERE root = ?1", [root])?;

    for chunk in files.chunks(BATCH_SIZE) {
        let row_placeholders = (0..chunk.len()).map(|_| "(?, ?, ?, ?, ?, ?, ?)").collect::<Vec<_>>().join(", ");
        let sql = format!(
            "INSERT INTO files (root, path, size, dev, ino, hash, mtime) VALUES {}",
            row_placeholders
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql + '_>> = Vec::with_capacity(chunk.len() * 7);
        for (path, size, key) in chunk {
            let path_str = path.to_string_lossy().to_string();
            params.push(Box::new(root));
            params.push(Box::new(path_str));
            params.push(Box::new(*size as i64));
            params.push(Box::new(key.dev as i64));
            params.push(Box::new(key.ino as i64));
            params.push(Box::new(None::<String>));
            params.push(Box::new(None::<i64>));
        }
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        tx.execute(&sql, rusqlite::params_from_iter(param_refs))?;
    }

    let folder_vec: Vec<_> = folder_sizes.iter().collect();
    for chunk in folder_vec.chunks(BATCH_SIZE) {
        let row_placeholders = (0..chunk.len()).map(|_| "(?, ?, ?)").collect::<Vec<_>>().join(", ");
        let sql = format!(
            "INSERT INTO folders (root, path, recursive_size) VALUES {}",
            row_placeholders
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql + '_>> = Vec::with_capacity(chunk.len() * 3);
        for (path, size) in chunk.iter() {
            let path_str = path.to_string_lossy().to_string();
            params.push(Box::new(root));
            params.push(Box::new(path_str));
            params.push(Box::new(**size as i64));
        }
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        tx.execute(&sql, rusqlite::params_from_iter(param_refs))?;
    }

    tx.commit()?;
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
