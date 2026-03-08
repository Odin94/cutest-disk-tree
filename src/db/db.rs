use rusqlite::Connection;
use rusqlite::OptionalExtension;
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

const SECONDARY_INDEXES: &[&str] = &[
    "idx_disk_objects_parent_kind",
    "idx_disk_objects_kind_ext",
    "idx_disk_objects_kind_ext_name_lower",
    "idx_disk_objects_dev_ino",
    "idx_disk_objects_path_lower",
    "idx_disk_objects_name_lower",
];

const CREATE_SECONDARY_INDEXES: &[&str] = &[
    "CREATE INDEX idx_disk_objects_parent_kind ON disk_objects(parent_path, kind)",
    "CREATE INDEX idx_disk_objects_kind_ext ON disk_objects(kind, ext)",
    "CREATE INDEX idx_disk_objects_kind_ext_name_lower ON disk_objects(kind, ext, name_lower)",
    "CREATE INDEX idx_disk_objects_dev_ino ON disk_objects(dev, ino)",
    "CREATE INDEX idx_disk_objects_path_lower ON disk_objects(path_lower)",
    "CREATE INDEX idx_disk_objects_name_lower ON disk_objects(name_lower)",
];

pub fn write_scan(
    conn: &Connection,
    files: &[FileEntry],
    folder_sizes: &std::collections::HashMap<std::path::PathBuf, u64>,
    update_id: i64,
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;

    tx.execute("DELETE FROM disk_objects", [])?;
    tx.execute("DELETE FROM cached_trees", [])?;

    for name in SECONDARY_INDEXES {
        tx.execute(&format!("DROP INDEX IF EXISTS {}", name), [])?;
    }

    {
        let mut stmt = tx.prepare(
            "INSERT INTO disk_objects \
             (path, path_lower, parent_path, name, name_lower, ext, kind, size, recursive_size, dev, ino, mtime) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
        )?;

        for entry in files {
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

            stmt.execute(rusqlite::params![
                path_str,
                path_lower,
                parent_path,
                name,
                name_lower,
                ext,
                "file",
                entry.size as i64,
                None::<i64>,
                entry.file_key.dev as i64,
                entry.file_key.ino as i64,
                entry.mtime.unwrap_or(0),
            ])?;
        }
    }

    {
        let mut stmt = tx.prepare(
            "INSERT INTO disk_objects \
             (path, path_lower, parent_path, name, name_lower, ext, kind, size, recursive_size, dev, ino, mtime) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
        )?;

        for (path, size) in folder_sizes.iter() {
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

            stmt.execute(rusqlite::params![
                path_str,
                path_lower,
                parent_path,
                name,
                name_lower,
                None::<String>,
                "folder",
                None::<i64>,
                *size as i64,
                None::<i64>,
                None::<i64>,
                None::<i64>,
            ])?;
        }
    }

    for ddl in CREATE_SECONDARY_INDEXES {
        tx.execute(ddl, [])?;
    }

    tx.execute(
        "INSERT INTO scan_metadata \
            (id, disk_objects_update_id, disk_objects_last_updated, \
             suffix_index_update_id, suffix_index_last_updated, \
             cached_trees_update_id, cached_trees_last_updated) \
         VALUES (1, ?1, ?1, 0, 0, 0, 0) \
         ON CONFLICT(id) DO UPDATE SET \
            disk_objects_update_id = excluded.disk_objects_update_id, \
            disk_objects_last_updated = excluded.disk_objects_last_updated",
        rusqlite::params![update_id],
    )?;

    tx.commit()?;
    Ok(())
}

pub fn has_disk_objects(conn: &Connection) -> rusqlite::Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(1) FROM disk_objects LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub fn get_file_index(
    conn: &Connection,
) -> rusqlite::Result<Vec<(String, u64, u64, u64, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT path, size, dev, ino, ext FROM disk_objects WHERE kind = 'file'",
    )?;
    let rows = stmt.query_map([], |row| {
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

pub fn get_disk_objects(
    conn: &Connection,
) -> rusqlite::Result<Vec<crate::DiskObject>> {
    let mut stmt = conn.prepare(
        "SELECT path, path_lower, parent_path, name, name_lower, ext, kind, size, recursive_size, dev, ino, mtime \
         FROM disk_objects",
    )?;
    let rows = stmt.query_map([], |row| {
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

pub fn get_folders(
    conn: &Connection,
) -> rusqlite::Result<Vec<(String, u64)>> {
    let mut stmt = conn.prepare(
        "SELECT path, recursive_size FROM disk_objects WHERE kind = 'folder'",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get_scan_result(
    conn: &Connection,
) -> rusqlite::Result<Option<crate::ScanResult>> {
    let (result, _) = get_scan_result_timed(conn)?;
    Ok(result)
}

pub fn get_scan_summary(
    conn: &Connection,
) -> rusqlite::Result<Option<crate::ScanSummary>> {
    let object_count: i64 = conn.query_row(
        "SELECT COUNT(1) FROM disk_objects",
        [],
        |row| row.get(0),
    )?;
    if object_count == 0 {
        return Ok(None);
    }

    let files_count: i64 = conn.query_row(
        "SELECT COUNT(1) FROM disk_objects WHERE kind = 'file'",
        [],
        |row| row.get(0),
    )?;

    let mut folder_stmt = conn.prepare(
        "SELECT path, recursive_size FROM disk_objects WHERE kind = 'folder' ORDER BY recursive_size DESC",
    )?;
    let folder_sizes: std::collections::HashMap<String, u64> = folder_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u64,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .collect();

    let folders_count = folder_sizes.len() as u64;

    let roots: Vec<String> = folder_sizes
        .iter()
        .filter(|(path, _)| {
            let p = std::path::Path::new(path);
            p.parent().is_none() || p.parent() == Some(std::path::Path::new(""))
                || path.len() <= 3
        })
        .map(|(path, _)| path.clone())
        .collect();

    Ok(Some(crate::ScanSummary {
        roots,
        files_count: files_count as u64,
        folders_count,
        folder_sizes,
    }))
}

pub fn get_scan_result_timed(
    conn: &Connection,
) -> rusqlite::Result<(Option<crate::ScanResult>, GetScanTimings)> {
    let mut timings = GetScanTimings::default();

    let object_count: i64 = conn.query_row(
        "SELECT COUNT(1) FROM disk_objects",
        [],
        |row| row.get(0),
    )?;
    if object_count == 0 {
        return Ok((None, timings));
    }

    let t0 = Instant::now();
    let mut file_stmt = conn.prepare(
        "SELECT path, size, dev, ino, mtime FROM disk_objects WHERE kind = 'file' ORDER BY path",
    )?;
    let files: Vec<crate::FileEntrySer> = file_stmt
        .query_map([], |row| {
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
        "SELECT path, recursive_size FROM disk_objects WHERE kind = 'folder' ORDER BY recursive_size DESC",
    )?;
    let folder_sizes: std::collections::HashMap<String, u64> = folder_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u64,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .collect();
    timings.folders_query_ms = t1.elapsed().as_millis() as u64;

    let roots: Vec<String> = folder_sizes
        .iter()
        .filter(|(path, _)| {
            let p = std::path::Path::new(path);
            p.parent().is_none() || p.parent() == Some(std::path::Path::new(""))
                || path.len() <= 3
        })
        .map(|(path, _)| path.clone())
        .collect();

    Ok((
        Some(crate::ScanResult {
            roots,
            files,
            folder_sizes,
        }),
        timings,
    ))
}

pub fn get_cached_tree(
    conn: &Connection,
    max_depth: u32,
    max_children: u32,
) -> rusqlite::Result<Option<DiskTreeNode>> {
    let max_d = max_depth as i64;
    let max_c = max_children as i64;
    let json: Option<String> = conn
        .query_row(
            "SELECT tree_json FROM cached_trees WHERE max_depth = ?1 AND max_children = ?2",
            rusqlite::params![max_d, max_c],
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
        "INSERT OR REPLACE INTO cached_trees (max_depth, max_children, tree_json, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![max_depth as i64, max_children as i64, tree_json, created_at],
    )?;
    Ok(())
}

pub fn list_cached_tree_depths(
    conn: &Connection,
    max_children: u32,
) -> rusqlite::Result<Vec<u32>> {
    let max_c = max_children as i64;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT max_depth FROM cached_trees WHERE max_children = ?1 ORDER BY max_depth",
    )?;
    let depths: Vec<u32> = stmt
        .query_map(rusqlite::params![max_c], |row| row.get::<_, i64>(0).map(|d| d as u32))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(depths)
}

pub fn get_children_for_path(
    conn: &Connection,
    parent_path: &str,
) -> rusqlite::Result<(
    Vec<(String, u64)>,
    Vec<(String, u64)>,
)> {
    let mut folder_stmt = conn.prepare(
        "SELECT path, recursive_size FROM disk_objects WHERE parent_path = ?1 AND kind = 'folder' ORDER BY recursive_size DESC",
    )?;
    let folders: Vec<(String, u64)> = folder_stmt
        .query_map(rusqlite::params![parent_path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut file_stmt = conn.prepare(
        "SELECT path, size FROM disk_objects WHERE parent_path = ?1 AND kind = 'file'",
    )?;
    let files: Vec<(String, u64)> = file_stmt
        .query_map(rusqlite::params![parent_path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok((folders, files))
}

pub fn get_folder_size(conn: &Connection, path: &str) -> rusqlite::Result<Option<u64>> {
    conn.query_row(
        "SELECT recursive_size FROM disk_objects WHERE path = ?1 AND kind = 'folder'",
        rusqlite::params![path],
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
) -> rusqlite::Result<Option<ScanMetadata>> {
    let mut stmt = conn.prepare(
        "SELECT disk_objects_update_id, disk_objects_last_updated, \
                suffix_index_update_id, suffix_index_last_updated, \
                cached_trees_update_id, cached_trees_last_updated \
         FROM scan_metadata WHERE id = 1",
    )?;
    stmt.query_row([], |row| {
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

pub fn write_suffix_index_data(
    conn: &Connection,
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
        "INSERT OR REPLACE INTO suffix_index_data (id, buffer, offsets, disk_object_indices) \
         VALUES (1, ?1, ?2, ?3)",
        rusqlite::params![buffer, offsets_blob, doi_blob],
    )?;
    tx.execute(
        "INSERT INTO scan_metadata \
            (id, disk_objects_update_id, disk_objects_last_updated, \
             suffix_index_update_id, suffix_index_last_updated, \
             cached_trees_update_id, cached_trees_last_updated) \
         VALUES (1, 0, 0, ?1, ?1, 0, 0) \
         ON CONFLICT(id) DO UPDATE SET \
            suffix_index_update_id = excluded.suffix_index_update_id, \
            suffix_index_last_updated = excluded.suffix_index_last_updated",
        rusqlite::params![update_id],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn read_suffix_index_data(
    conn: &Connection,
) -> rusqlite::Result<Option<(String, Vec<usize>, Vec<usize>)>> {
    let mut stmt = conn.prepare(
        "SELECT buffer, offsets, disk_object_indices \
         FROM suffix_index_data WHERE id = 1",
    )?;
    stmt.query_row([], |row| {
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

    if let Err(e) = migrations().to_latest(&mut conn) {
        return match e {
            rusqlite_migration::Error::RusqliteError { err, .. } => Err(err),
            other => Err(rusqlite::Error::ToSqlConversionFailure(Box::new(other))),
        };
    }

    conn.execute_batch(
        "PRAGMA journal_mode=WAL; \
         PRAGMA synchronous=NORMAL; \
         PRAGMA cache_size=-32000; \
         PRAGMA temp_store=MEMORY;"
    )?;
    conn.busy_timeout(Duration::from_millis(5000))?;

    Ok(conn)
}
