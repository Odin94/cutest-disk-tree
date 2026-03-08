use rusqlite::Connection;
use rusqlite::types::Value;
use std::time::Instant;

use crate::{DiskObject, DiskObjectKind};
use crate::core::search_category;

#[derive(Clone, Debug)]
pub enum SearchFilter {
    None,
    FoldersOnly,
    Other,
    Extensions(Vec<String>),
}

fn row_to_disk_object(row: &rusqlite::Row<'_>) -> rusqlite::Result<DiskObject> {
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
        path_lower: path_lower_from_db.unwrap_or_else(|| path.to_lowercase()),
        parent_path: row.get::<_, Option<String>>(2)?,
        name: name.clone(),
        name_lower: name_lower_from_db.unwrap_or_else(|| name.to_lowercase()),
        ext: row.get::<_, Option<String>>(5)?,
        kind,
        size: size_opt.map(|n| n as u64),
        recursive_size: rec_opt.map(|n| n as u64),
        dev: dev_opt.map(|n| n as u64),
        ino: ino_opt.map(|n| n as u64),
        mtime: mtime_opt,
    })
}

fn filter_where_and_params(filter: &SearchFilter) -> (String, Vec<Value>) {
    let (condition, params) = match filter {
        SearchFilter::None => ("".into(), vec![]),
        SearchFilter::FoldersOnly => ("kind = 'folder'".into(), vec![]),
        SearchFilter::Other => {
            let known = search_category::all_known_extensions();
            if known.is_empty() {
                ("kind = 'file' AND ext IS NULL".into(), vec![])
            } else {
                let placeholders: Vec<String> = (0..known.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect();
                let sql = format!(
                    "kind = 'file' AND (ext IS NULL OR ext NOT IN ({}))",
                    placeholders.join(", ")
                );
                let vals = known
                    .into_iter()
                    .map(|s| Value::Text(s.to_string()))
                    .collect();
                (sql, vals)
            }
        }
        SearchFilter::Extensions(exts) => {
            if exts.is_empty() {
                ("".into(), vec![])
            } else {
                let placeholders: Vec<String> = (0..exts.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect();
                let sql = format!(
                    "kind = 'file' AND ext IN ({})",
                    placeholders.join(", ")
                );
                let vals = exts
                    .iter()
                    .map(|s| Value::Text(s.clone()))
                    .collect();
                (sql, vals)
            }
        }
    };
    (condition, params)
}

#[derive(Clone, Debug, Default)]
pub struct SearchTimings {
    pub prepare_ms: u128,
    pub query_map_ms: u128,
    pub collect_ms: u128,
}

pub fn search_disk_objects_by_name(
    conn: &Connection,
    query: &str,
    filter: &SearchFilter,
    limit: usize,
    offset: usize,
) -> rusqlite::Result<(Vec<DiskObject>, bool, SearchTimings)> {
    let (filter_condition, filter_params) = filter_where_and_params(filter);

    let pattern = format!("%{}%", query.to_lowercase());
    let limit_plus_one = limit.saturating_add(1).min(i64::MAX as usize) as i64;
    let offset_i64 = offset as i64;

    let (where_clause, param_order): (String, Vec<Value>) = if filter_condition.is_empty() {
        (
            "name_lower LIKE ?1".into(),
            vec![Value::Text(pattern)],
        )
    } else {
        let like_param = filter_params.len() + 1;
        (
            format!("{} AND name_lower LIKE ?{}", filter_condition, like_param),
            {
                let mut p = filter_params;
                p.push(Value::Text(pattern));
                p
            },
        )
    };

    let limit_idx = param_order.len() + 1;
    let offset_idx = limit_idx + 1;

    let sql = format!(
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
         WHERE {} \
         ORDER BY name_lower ASC \
         LIMIT ?{} OFFSET ?{}",
        where_clause,
        limit_idx,
        offset_idx,
    );

    let prepare_start = Instant::now();
    let mut stmt = conn.prepare(&sql)?;
    let prepare_ms = prepare_start.elapsed().as_millis();

    let mut params: Vec<Value> = param_order;
    params.push(Value::Integer(limit_plus_one));
    params.push(Value::Integer(offset_i64));

    let query_start = Instant::now();
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        row_to_disk_object(&row)
    })?;

    let collect_start = Instant::now();
    let results: Vec<DiskObject> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let collect_ms = collect_start.elapsed().as_millis();
    let query_ms = query_start.elapsed().as_millis();

    let has_more = results.len() > limit;
    let truncated = if has_more {
        results.into_iter().take(limit).collect()
    } else {
        results
    };

    let timings = SearchTimings {
        prepare_ms,
        query_map_ms: query_ms.saturating_sub(collect_ms),
        collect_ms,
    };

    Ok((truncated, has_more, timings))
}
