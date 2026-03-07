use rusqlite::Connection;

use crate::{DiskObject, DiskObjectKind};

pub fn search_disk_objects_by_name(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> rusqlite::Result<Vec<DiskObject>> {
    let mut stmt = conn.prepare(
        "SELECT \
            path, \
            path_lower, \
            parent_path, \
            name, \
            name_lower, \
            ext, \
            kind, \
            size, \
            recursive_size, \
            dev, \
            ino, \
            mtime \
         FROM disk_objects \
         WHERE kind = 'file' AND name_lower LIKE ?1 \
         ORDER BY name_lower ASC \
         LIMIT ?2",
    )?;

    let q_lower = query.to_ascii_lowercase();
    let pattern = format!("%{}%", q_lower);
    let limit_i64 = limit as i64;

    let results = stmt
        .query_map(rusqlite::params![pattern, limit_i64], |row| {
            let kind_str: String = row.get(6)?;
            let kind = match kind_str.as_str() {
                "folder" => DiskObjectKind::Folder,
                _ => DiskObjectKind::File,
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

            Ok(DiskObject {
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
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(results)
}

