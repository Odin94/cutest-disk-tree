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
    update_id: i64,
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute("DELETE FROM disk_objects WHERE root = ?1", [root])?;
    tx.execute("DELETE FROM cached_trees WHERE root = ?1", [root])?;
    tx.execute("DELETE FROM file_search_trigrams WHERE root = ?1", [root])?;
    for chunk in files.chunks(BATCH_SIZE) {
        let row_placeholders_items = (0..chunk.len())
            .map(|_| "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .collect::<Vec<_>>()
            .join(", ");
        let sql_items = format!(
            "INSERT INTO disk_objects (root, path, path_lower, parent_path, name, name_lower, ext, kind, size, recursive_size, dev, ino, mtime) VALUES {}",
            row_placeholders_items
        );
        let mut item_params: Vec<Box<dyn rusqlite::ToSql + '_>> =
            Vec::with_capacity(chunk.len() * 13);
        for entry in chunk {
            let path_str = entry.path.to_string_lossy().to_string();
            let path_lower = path_str.to_ascii_lowercase();
            let parent_path = parent_dir(&path_str);
            let name: Option<String> = entry
                .path
                .file_name()
                .and_then(|os| os.to_str())
                .map(|s| s.to_string());
            let name_lower: Option<String> = name
                .as_ref()
                .map(|s| s.to_ascii_lowercase());
            let ext: Option<String> = entry
                .path
                .extension()
                .and_then(|os| os.to_str())
                .map(|s| s.to_ascii_lowercase());

            item_params.push(Box::new(root));
            item_params.push(Box::new(path_str));
            item_params.push(Box::new(path_lower));
            item_params.push(Box::new(parent_path));
            item_params.push(Box::new(name));
            item_params.push(Box::new(name_lower));
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
        let row_placeholders_items = (0..chunk.len())
            .map(|_| "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .collect::<Vec<_>>()
            .join(", ");
        let sql_items = format!(
            "INSERT INTO disk_objects (root, path, path_lower, parent_path, name, name_lower, ext, kind, size, recursive_size, dev, ino, mtime) VALUES {}",
            row_placeholders_items
        );
        let mut item_params: Vec<Box<dyn rusqlite::ToSql + '_>> =
            Vec::with_capacity(chunk.len() * 13);
        for (path, size) in chunk.iter() {
            let path_str = path.to_string_lossy().to_string();
            let path_lower = path_str.to_ascii_lowercase();
            let parent_path = parent_dir(&path_str);
            let name = path
                .file_name()
                .and_then(|os| os.to_str())
                .map(|s| s.to_string());
            let name_lower: Option<String> = name
                .as_ref()
                .map(|s| s.to_ascii_lowercase());
            item_params.push(Box::new(root));
            item_params.push(Box::new(path_str));
            item_params.push(Box::new(path_lower));
            item_params.push(Box::new(parent_path));
            item_params.push(Box::new(name));
            item_params.push(Box::new(name_lower));
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

    tx.execute(
        "INSERT INTO scan_metadata \
            (root, disk_objects_update_id, disk_objects_last_updated, \
             suffix_index_update_id, suffix_index_last_updated, \
             cached_trees_update_id, cached_trees_last_updated) \
         VALUES (?1, ?2, ?2, 0, 0, 0, 0) \
         ON CONFLICT(root) DO UPDATE SET \
            disk_objects_update_id = excluded.disk_objects_update_id, \
            disk_objects_last_updated = excluded.disk_objects_last_updated",
        rusqlite::params![root, update_id],
    )?;

    tx.commit()?;
    Ok(())
}

pub fn list_roots(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT DISTINCT root FROM disk_objects ORDER BY root")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    rows.collect()
}

pub fn get_file_index(
    conn: &Connection,
    root: &str,
) -> rusqlite::Result<Vec<(String, u64, u64, u64, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT path, size, dev, ino, ext FROM disk_objects WHERE root = ?1 AND kind = 'file'",
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
        "SELECT path, path_lower, parent_path, name, name_lower, ext, kind, size, recursive_size, dev, ino, mtime \
         FROM disk_objects WHERE root = ?1",
    )?;
    let rows = stmt.query_map([root], |row| {
        let kind_str: String = row.get(6)?;
        let kind = match kind_str.as_str() {
            "folder" => crate::DiskObjectKind::Folder,
            _ => crate::DiskObjectKind::File,
        };
        let size_opt: Option<i64> = row.get(7)?;
        let rec_opt: Option<i64> = row.get(8)?;
        let dev_opt: Option<i64> = row.get(9)?;
        let ino_opt: Option<i64> = row.get(10)?;
        let mtime_opt: Option<i64> = row.get(11)?;
        let path: String = row.get(0)?;
        let path_lower_from_db: Option<String> = row.get(1)?;
        let name_opt: Option<String> = row.get(3)?;
        let name = name_opt.unwrap_or_default();
        let name_lower_from_db: Option<String> = row.get(4)?;
        Ok(crate::DiskObject {
            root: root.to_string(),
            path: path.clone(),
            path_lower: path_lower_from_db.unwrap_or_else(|| path.to_ascii_lowercase()),
            parent_path: row.get::<_, Option<String>>(2)?,
            name: name.clone(),
            name_lower: name_lower_from_db.unwrap_or_else(|| name.to_ascii_lowercase()),
            ext: row.get::<_, Option<String>>(5)?,
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
        "SELECT f.path, f.size, f.dev, f.ino, f.ext \
         FROM file_search_trigrams t \
         JOIN disk_objects f ON f.root = t.root AND f.path = t.path \
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
            sql.push_str(" AND f.ext IN (");
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
            " GROUP BY f.path, f.size, f.dev, f.ino, f.ext \
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
        "SELECT path, recursive_size FROM disk_objects WHERE root = ?1 AND kind = 'folder'",
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

    let object_count: i64 = conn.query_row(
        "SELECT COUNT(1) FROM disk_objects WHERE root = ?1",
        [root],
        |row| row.get(0),
    )?;
    if object_count == 0 {
        return Ok((None, timings));
    }

    let t0 = Instant::now();
    let mut file_stmt = conn.prepare(
        "SELECT path, size, dev, ino, mtime FROM disk_objects WHERE root = ?1 AND kind = 'file' ORDER BY path",
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
        "SELECT path, recursive_size FROM disk_objects WHERE root = ?1 AND kind = 'folder' ORDER BY recursive_size DESC",
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
        "SELECT path, recursive_size FROM disk_objects WHERE root = ?1 AND parent_path = ?2 AND kind = 'folder' ORDER BY recursive_size DESC",
    )?;
    let folders: Vec<(String, u64)> = folder_stmt
        .query_map(rusqlite::params![root, parent_path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut file_stmt = conn.prepare(
        "SELECT path, size FROM disk_objects WHERE root = ?1 AND parent_path = ?2 AND kind = 'file'",
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
        "SELECT recursive_size FROM disk_objects WHERE root = ?1 AND path = ?2 AND kind = 'folder'",
        rusqlite::params![root, root],
        |row| row.get::<_, i64>(0).map(|n| n as u64),
    )
    .optional()
}

#[derive(Debug, Clone)]
pub struct ScanMetadata {
    pub disk_objects_update_id: i64,
    pub disk_objects_last_updated: i64,
    pub suffix_index_update_id: i64,
    pub suffix_index_last_updated: i64,
    pub cached_trees_update_id: i64,
    pub cached_trees_last_updated: i64,
}

pub fn read_scan_metadata(
    conn: &Connection,
    root: &str,
) -> rusqlite::Result<Option<ScanMetadata>> {
    let mut stmt = conn.prepare(
        "SELECT disk_objects_update_id, disk_objects_last_updated, \
                suffix_index_update_id, suffix_index_last_updated, \
                cached_trees_update_id, cached_trees_last_updated \
         FROM scan_metadata WHERE root = ?1",
    )?;
    stmt.query_row([root], |row| {
        Ok(ScanMetadata {
            disk_objects_update_id: row.get(0)?,
            disk_objects_last_updated: row.get(1)?,
            suffix_index_update_id: row.get(2)?,
            suffix_index_last_updated: row.get(3)?,
            cached_trees_update_id: row.get(4)?,
            cached_trees_last_updated: row.get(5)?,
        })
    })
    .optional()
}

/// Persists a pre-built suffix index and stamps its `update_id` in
/// `scan_metadata`. Both writes happen in a single transaction so they
/// stay consistent if the process is interrupted.
pub fn write_suffix_index_data(
    conn: &Connection,
    root: &str,
    update_id: i64,
    buffer: &str,
    offsets: &[usize],
    disk_object_indices: &[usize],
) -> rusqlite::Result<()> {
    let offsets_blob: Vec<u8> = offsets
        .iter()
        .flat_map(|&x| (x as u64).to_le_bytes())
        .collect();
    let doi_blob: Vec<u8> = disk_object_indices
        .iter()
        .flat_map(|&x| (x as u64).to_le_bytes())
        .collect();

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT OR REPLACE INTO suffix_index_data (root, buffer, offsets, disk_object_indices) \
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![root, buffer, offsets_blob, doi_blob],
    )?;
    tx.execute(
        "INSERT INTO scan_metadata \
            (root, disk_objects_update_id, disk_objects_last_updated, \
             suffix_index_update_id, suffix_index_last_updated, \
             cached_trees_update_id, cached_trees_last_updated) \
         VALUES (?1, 0, 0, ?2, ?2, 0, 0) \
         ON CONFLICT(root) DO UPDATE SET \
            suffix_index_update_id = excluded.suffix_index_update_id, \
            suffix_index_last_updated = excluded.suffix_index_last_updated",
        rusqlite::params![root, update_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// Loads a previously persisted suffix index buffer and index maps from the
/// database. Returns `None` if no data is stored for this root.
pub fn read_suffix_index_data(
    conn: &Connection,
    root: &str,
) -> rusqlite::Result<Option<(String, Vec<usize>, Vec<usize>)>> {
    let mut stmt = conn.prepare(
        "SELECT buffer, offsets, disk_object_indices \
         FROM suffix_index_data WHERE root = ?1",
    )?;
    stmt.query_row([root], |row| {
        let buffer: String = row.get(0)?;
        let offsets_blob: Vec<u8> = row.get(1)?;
        let doi_blob: Vec<u8> = row.get(2)?;
        let offsets = offsets_blob
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()) as usize)
            .collect();
        let doi = doi_blob
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()) as usize)
            .collect();
        Ok((buffer, offsets, doi))
    })
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

    // WAL mode keeps readers and the background writer from blocking each other.
    // synchronous=NORMAL is safe with WAL and avoids the per-commit full fsync
    // that makes bulk inserts slow on Windows.
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

    // Make concurrent access more robust in the face of background writers.
    // busy_timeout allows SQLite to wait for a short period instead of
    // immediately returning "database is locked".
    conn.busy_timeout(Duration::from_millis(5000))?;

    Ok(conn)
}
