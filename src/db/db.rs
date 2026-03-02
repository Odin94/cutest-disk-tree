use rusqlite::Connection;
use rusqlite::OptionalExtension;
use std::path::Path;
use std::time::Instant;

use crate::FileKey;
use crate::DiskTreeNode;
use crate::parent_dir;
use super::migrations::migrations;

#[derive(Clone, Debug, Default)]
pub struct GetScanTimings {
    pub files_query_ms: u64,
    pub folders_query_ms: u64,
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
    tx.execute("DELETE FROM cached_trees WHERE root = ?1", [root])?;

    for chunk in files.chunks(BATCH_SIZE) {
        let row_placeholders = (0..chunk.len())
            .map(|_| "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT INTO files (root, path, size, dev, ino, hash, mtime, name, type, parent_path) VALUES {}",
            row_placeholders
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql + '_>> = Vec::with_capacity(chunk.len() * 10);
        for (path, size, key) in chunk {
            let path_str = path.to_string_lossy().to_string();
            let parent_path = parent_dir(&path_str);
            let name: Option<String> = path
                .file_name()
                .and_then(|os| os.to_str())
                .map(|s| s.to_string());
            let file_type: Option<String> = path
                .extension()
                .and_then(|os| os.to_str())
                .map(|s| s.to_ascii_lowercase());

            params.push(Box::new(root));
            params.push(Box::new(path_str));
            params.push(Box::new(*size as i64));
            params.push(Box::new(key.dev as i64));
            params.push(Box::new(key.ino as i64));
            params.push(Box::new(None::<String>));
            params.push(Box::new(None::<i64>));
            params.push(Box::new(name));
            params.push(Box::new(file_type));
            params.push(Box::new(parent_path));
        }
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        tx.execute(&sql, rusqlite::params_from_iter(param_refs))?;
    }

    let folder_vec: Vec<_> = folder_sizes.iter().collect();
    for chunk in folder_vec.chunks(BATCH_SIZE) {
        let row_placeholders = (0..chunk.len()).map(|_| "(?, ?, ?, ?)").collect::<Vec<_>>().join(", ");
        let sql = format!(
            "INSERT INTO folders (root, path, recursive_size, parent_path) VALUES {}",
            row_placeholders
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql + '_>> = Vec::with_capacity(chunk.len() * 4);
        for (path, size) in chunk.iter() {
            let path_str = path.to_string_lossy().to_string();
            let parent_path = parent_dir(&path_str);
            params.push(Box::new(root));
            params.push(Box::new(path_str));
            params.push(Box::new(**size as i64));
            params.push(Box::new(parent_path));
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
    let (result, _) = get_scan_result_timed(conn, root)?;
    Ok(result)
}

pub fn get_scan_result_timed(
    conn: &Connection,
    root: &str,
) -> rusqlite::Result<(Option<crate::ScanResult>, GetScanTimings)> {
    let mut timings = GetScanTimings::default();

    let file_count: i64 = conn.query_row(
        "SELECT COUNT(1) FROM files WHERE root = ?1",
        [root],
        |row| row.get(0),
    )?;
    if file_count == 0 {
        return Ok((None, timings));
    }

    let t0 = Instant::now();
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
    timings.files_query_ms = t0.elapsed().as_millis() as u64;

    let t1 = Instant::now();
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
    timings.folders_query_ms = t1.elapsed().as_millis() as u64;

    Ok((
        Some(crate::ScanResult {
            root: root.to_string(),
            files,
            folder_sizes,
        }),
        timings,
    ))
}

pub fn get_cached_tree(
    conn: &Connection,
    root: &str,
    max_depth: u32,
    max_children: u32,
) -> rusqlite::Result<Option<DiskTreeNode>> {
    let max_d = max_depth as i64;
    let max_c = max_children as i64;
    let json: Option<String> = conn
        .query_row(
            "SELECT tree_json FROM cached_trees WHERE root = ?1 AND max_depth = ?2 AND max_children = ?3",
            rusqlite::params![root, max_d, max_c],
            |row| row.get(0),
        )
        .optional()?;
    let tree = match json {
        Some(s) => serde_json::from_str(&s).ok(),
        None => None,
    };
    Ok(tree)
}

pub fn write_cached_tree(
    conn: &Connection,
    root: &str,
    max_depth: u32,
    max_children: u32,
    tree: &DiskTreeNode,
) -> rusqlite::Result<()> {
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let tree_json = serde_json::to_string(tree).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "INSERT OR REPLACE INTO cached_trees (root, max_depth, max_children, tree_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![root, max_depth as i64, max_children as i64, tree_json, created_at],
    )?;
    Ok(())
}

pub fn list_cached_tree_depths(
    conn: &Connection,
    root: &str,
    max_children: u32,
) -> rusqlite::Result<Vec<u32>> {
    let max_c = max_children as i64;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT max_depth FROM cached_trees WHERE root = ?1 AND max_children = ?2 ORDER BY max_depth",
    )?;
    let depths: Vec<u32> = stmt
        .query_map(rusqlite::params![root, max_c], |row| row.get::<_, i64>(0).map(|d| d as u32))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(depths)
}

pub fn get_children_for_path(
    conn: &Connection,
    root: &str,
    parent_path: &str,
) -> rusqlite::Result<(
    Vec<(String, u64)>,
    Vec<(String, u64)>,
)> {
    let mut folder_stmt = conn.prepare(
        "SELECT path, recursive_size FROM folders WHERE root = ?1 AND parent_path = ?2 ORDER BY recursive_size DESC",
    )?;
    let folders: Vec<(String, u64)> = folder_stmt
        .query_map(rusqlite::params![root, parent_path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut file_stmt = conn.prepare(
        "SELECT path, size FROM files WHERE root = ?1 AND parent_path = ?2",
    )?;
    let files: Vec<(String, u64)> = file_stmt
        .query_map(rusqlite::params![root, parent_path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok((folders, files))
}

pub fn get_root_size(conn: &Connection, root: &str) -> rusqlite::Result<Option<u64>> {
    conn.query_row(
        "SELECT recursive_size FROM folders WHERE root = ?1 AND path = ?2",
        rusqlite::params![root, root],
        |row| row.get::<_, i64>(0).map(|n| n as u64),
    )
    .optional()
}

pub fn open_db(db_path: &Path) -> rusqlite::Result<Connection> {
    let mut conn = Connection::open(db_path)?;
    migrations()
        .to_latest(&mut conn)
        .map_err(|_e| rusqlite::Error::ExecuteReturnedResults)?;
    Ok(conn)
}
