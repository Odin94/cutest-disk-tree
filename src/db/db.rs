use rusqlite::Connection;
use rusqlite::OptionalExtension;
use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use crate::{FileEntry, FileKey};
use crate::DiskTreeNode;
use crate::parent_dir;
use super::migrations::migrations;

#[derive(Clone, Debug, Default)]
pub struct GetScanTimings {
    pub files_query_ms: u64,
    pub folders_query_ms: u64,
}

const BATCH_SIZE: usize = 1000;

fn make_trigrams(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 3 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(chars.len().saturating_sub(2));
    for i in 0..(chars.len() - 2) {
        let trigram: String = chars[i..i + 3].iter().collect();
        out.push(trigram);
    }
    out
}

pub fn write_scan(
    conn: &Connection,
    root: &str,
    files: &[FileEntry],
    folder_sizes: &std::collections::HashMap<std::path::PathBuf, u64>,
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute("DELETE FROM files WHERE root = ?1", [root])?;
    tx.execute("DELETE FROM folders WHERE root = ?1", [root])?;
    tx.execute("DELETE FROM cached_trees WHERE root = ?1", [root])?;
    tx.execute("DELETE FROM file_search_trigrams WHERE root = ?1", [root])?;
    tx.execute("DELETE FROM items WHERE root = ?1", [root])?;

    for chunk in files.chunks(BATCH_SIZE) {
        let row_placeholders = (0..chunk.len())
            .map(|_| "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT INTO files (root, path, size, dev, ino, hash, mtime, name, type, parent_path) VALUES {}",
            row_placeholders
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql + '_>> =
            Vec::with_capacity(chunk.len() * 10);
        for entry in chunk {
            let path_str = entry.path.to_string_lossy().to_string();
            let parent_path = parent_dir(&path_str);
            let name: Option<String> = entry
                .path
                .file_name()
                .and_then(|os| os.to_str())
                .map(|s| s.to_string());
            let file_type: Option<String> = entry
                .path
                .extension()
                .and_then(|os| os.to_str())
                .map(|s| s.to_ascii_lowercase());

            params.push(Box::new(root));
            params.push(Box::new(path_str));
            params.push(Box::new(entry.size as i64));
            params.push(Box::new(entry.file_key.dev as i64));
            params.push(Box::new(entry.file_key.ino as i64));
            params.push(Box::new(None::<String>));
            params.push(Box::new(entry.mtime.unwrap_or(0)));
            params.push(Box::new(name));
            params.push(Box::new(file_type));
            params.push(Box::new(parent_path));
        }
        let param_refs: Vec<&dyn rusqlite::ToSql> =
            params.iter().map(|b| b.as_ref()).collect();
        tx.execute(&sql, rusqlite::params_from_iter(param_refs))?;

        let row_placeholders_items = (0..chunk.len())
            .map(|_| "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .collect::<Vec<_>>()
            .join(", ");
        let sql_items = format!(
            "INSERT INTO items (root, path, parent_path, name, ext, kind, size, recursive_size, dev, ino, mtime) VALUES {}",
            row_placeholders_items
        );
        let mut item_params: Vec<Box<dyn rusqlite::ToSql + '_>> =
            Vec::with_capacity(chunk.len() * 11);
        for entry in chunk {
            let path_str = entry.path.to_string_lossy().to_string();
            let parent_path = parent_dir(&path_str);
            let name: Option<String> = entry
                .path
                .file_name()
                .and_then(|os| os.to_str())
                .map(|s| s.to_string());
            let ext: Option<String> = entry
                .path
                .extension()
                .and_then(|os| os.to_str())
                .map(|s| s.to_ascii_lowercase());

            item_params.push(Box::new(root));
            item_params.push(Box::new(path_str));
            item_params.push(Box::new(parent_path));
            item_params.push(Box::new(name));
            item_params.push(Box::new(ext));
            item_params.push(Box::new("file"));
            item_params.push(Box::new(entry.size as i64));
            item_params.push(Box::new(None::<i64>));
            item_params.push(Box::new(entry.file_key.dev as i64));
            item_params.push(Box::new(entry.file_key.ino as i64));
            item_params.push(Box::new(entry.mtime.unwrap_or(0)));
        }
        let item_param_refs: Vec<&dyn rusqlite::ToSql> =
            item_params.iter().map(|b| b.as_ref()).collect();
        tx.execute(&sql_items, rusqlite::params_from_iter(item_param_refs))?;
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

        // Also populate unified items table for folders
        let row_placeholders_items = (0..chunk.len())
            .map(|_| "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .collect::<Vec<_>>()
            .join(", ");
        let sql_items = format!(
            "INSERT INTO items (root, path, parent_path, name, ext, kind, size, recursive_size, dev, ino, mtime) VALUES {}",
            row_placeholders_items
        );
        let mut item_params: Vec<Box<dyn rusqlite::ToSql + '_>> =
            Vec::with_capacity(chunk.len() * 11);
        for (path, size) in chunk.iter() {
            let path_str = path.to_string_lossy().to_string();
            let parent_path = parent_dir(&path_str);
            let name = path
                .file_name()
                .and_then(|os| os.to_str())
                .map(|s| s.to_string());
            item_params.push(Box::new(root));
            item_params.push(Box::new(path_str));
            item_params.push(Box::new(parent_path));
            item_params.push(Box::new(name));
            item_params.push(Box::new(None::<String>));
            item_params.push(Box::new("folder"));
            item_params.push(Box::new(None::<i64>));
            item_params.push(Box::new(**size as i64));
            item_params.push(Box::new(None::<i64>));
            item_params.push(Box::new(None::<i64>));
            item_params.push(Box::new(None::<i64>));
        }
        let item_param_refs: Vec<&dyn rusqlite::ToSql> =
            item_params.iter().map(|b| b.as_ref()).collect();
        tx.execute(&sql_items, rusqlite::params_from_iter(item_param_refs))?;
    }

    tx.commit()?;
    Ok(())
}

pub fn list_roots(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT DISTINCT root FROM folders ORDER BY root")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    rows.collect()
}

pub fn get_file_index(
    conn: &Connection,
    root: &str,
) -> rusqlite::Result<Vec<(String, u64, u64, u64, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT path, size, dev, ino, type FROM files WHERE root = ?1",
    )?;
    let rows = stmt.query_map([root], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as u64,
            row.get::<_, i64>(2)? as u64,
            row.get::<_, i64>(3)? as u64,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    rows.collect()
}

pub fn get_disk_objects_for_root(
    conn: &Connection,
    root: &str,
) -> rusqlite::Result<Vec<crate::DiskObject>> {
    let mut stmt = conn.prepare(
        "SELECT path, parent_path, name, ext, kind, size, recursive_size, dev, ino, mtime \
         FROM items WHERE root = ?1",
    )?;
    let rows = stmt.query_map([root], |row| {
        let kind_str: String = row.get(4)?;
        let kind = match kind_str.as_str() {
            "folder" => crate::DiskObjectKind::Folder,
            _ => crate::DiskObjectKind::File,
        };
        let size_opt: Option<i64> = row.get(5)?;
        let rec_opt: Option<i64> = row.get(6)?;
        let dev_opt: Option<i64> = row.get(7)?;
        let ino_opt: Option<i64> = row.get(8)?;
        let mtime_opt: Option<i64> = row.get(9)?;
        Ok(crate::DiskObject {
            root: root.to_string(),
            path: row.get::<_, String>(0)?,
            parent_path: row.get::<_, Option<String>>(1)?,
            name: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            ext: row.get::<_, Option<String>>(3)?,
            kind,
            size: size_opt.map(|n| n as u64),
            recursive_size: rec_opt.map(|n| n as u64),
            dev: dev_opt.map(|n| n as u64),
            ino: ino_opt.map(|n| n as u64),
            mtime: mtime_opt,
        })
    })?;
    rows.collect()
}

pub fn search_files_by_substring(
    conn: &Connection,
    root: &str,
    query: &str,
    extension_set: Option<&HashSet<String>>,
    limit: usize,
) -> rusqlite::Result<Vec<(String, u64, u64, u64, Option<String>)>> {
    let normalized_query = query.to_lowercase();
    let trigrams = make_trigrams(&normalized_query);
    if trigrams.is_empty() {
        return Ok(Vec::new());
    }

    let mut sql = String::from(
        "SELECT f.path, f.size, f.dev, f.ino, f.type \
         FROM file_search_trigrams t \
         JOIN files f ON f.root = t.root AND f.path = t.path \
         WHERE t.root = ?1 AND t.trigram IN (",
    );

    let mut placeholders: Vec<String> = Vec::with_capacity(trigrams.len());
    for i in 0..trigrams.len() {
        placeholders.push(format!("?{}", i + 2));
    }
    sql.push_str(&placeholders.join(", "));
    sql.push(')');

    let mut ext_values: Vec<String> = Vec::new();
    if let Some(set) = extension_set {
        if !set.is_empty() {
            sql.push_str(" AND f.type IN (");
            let mut ext_placeholders: Vec<String> = Vec::with_capacity(set.len());
            for i in 0..set.len() {
                ext_placeholders.push(format!("?{}", trigrams.len() + 2 + i));
            }
            sql.push_str(&ext_placeholders.join(", "));
            sql.push(')');
            ext_values.extend(set.iter().cloned());
        }
    }

    let count_param_index = trigrams.len() + ext_values.len() + 2;
    let limit_param_index = count_param_index + 1;

    sql.push_str(
        &format!(
            " GROUP BY f.path, f.size, f.dev, f.ino, f.type \
               HAVING COUNT(*) >= ?{} \
               ORDER BY f.path \
               LIMIT ?{}",
            count_param_index, limit_param_index
        ),
    );

    let mut params: Vec<Box<dyn rusqlite::ToSql + '_>> =
        Vec::with_capacity(2 + trigrams.len() + ext_values.len());
    params.push(Box::new(root));
    for t in &trigrams {
        params.push(Box::new(t));
    }
    for ext in &ext_values {
        params.push(Box::new(ext));
    }
    params.push(Box::new(trigrams.len() as i64));
    params.push(Box::new(limit as i64));

    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(param_refs), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as u64,
                row.get::<_, i64>(3)? as u64,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let filtered: Vec<(String, u64, u64, u64, Option<String>)> = rows
        .into_iter()
        .filter(|(path, _, _, _, _)| path.to_lowercase().contains(&normalized_query))
        .collect();

    Ok(filtered)
}

pub fn get_folders_for_root(
    conn: &Connection,
    root: &str,
) -> rusqlite::Result<Vec<(String, u64)>> {
    let mut stmt = conn.prepare(
        "SELECT path, recursive_size FROM folders WHERE root = ?1",
    )?;
    let rows = stmt
        .query_map([root], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
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
        "SELECT path, size, dev, ino, mtime FROM files WHERE root = ?1 ORDER BY path",
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
                mtime: row.get::<_, Option<i64>>(4)?,
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
    use std::time::Duration;

    let mut conn = Connection::open(db_path)?;

    // Apply pending schema migrations. If this fails due to an underlying
    // rusqlite error (e.g. "database is locked"), propagate that exact error
    // instead of masking it as ExecuteReturnedResults.
    if let Err(e) = migrations().to_latest(&mut conn) {
        return match e {
            rusqlite_migration::Error::RusqliteError { err, .. } => Err(err),
            other => Err(rusqlite::Error::ToSqlConversionFailure(Box::new(other))),
        };
    }

    // Make concurrent access more robust in the face of background writers.
    // busy_timeout allows SQLite to wait for a short period instead of
    // immediately returning "database is locked".
    conn.busy_timeout(Duration::from_millis(5000))?;

    Ok(conn)
}
