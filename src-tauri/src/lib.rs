use cutest_disk_tree::{db, index_directory_parallel_with_progress};
use std::path::Path;
use tauri::Manager;
use tauri::Emitter;
use nucleo::{Matcher, Config};
use nucleo::pattern::{Pattern, CaseMatching, Normalization};

struct AppState {
    db_path: std::path::PathBuf,
}

#[tauri::command]
async fn scan_directory(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<cutest_disk_tree::ScanResult, String> {
    let path_buf = Path::new(&path).to_path_buf();
    if !path_buf.is_dir() {
        return Err(format!("Not a directory: {}", path));
    }
    let db_path = state.db_path.clone();
    let path_for_db = path.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        let (files, folder_sizes) = index_directory_parallel_with_progress(&path_buf, |p| {
            let _ = app.emit("scan-progress", &p);
        });
        let result = cutest_disk_tree::to_scan_result(&path_buf, &files, &folder_sizes)
            .ok_or_else(|| "Indexing failed".to_string())?;
        let _ = app.emit(
            "scan-progress",
            &cutest_disk_tree::ScanProgress {
                files_count: files.len() as u64,
                current_path: None,
                status: Some("Saving to database…".into()),
            },
        );
        let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;
        db::write_scan(&conn, &path_for_db, &files, &folder_sizes).map_err(|e| e.to_string())?;
        Ok::<_, String>(result)
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(result)
}

#[tauri::command]
fn list_cached_roots(state: tauri::State<AppState>) -> Result<Vec<String>, String> {
    let conn = db::open_db(&state.db_path).map_err(|e| e.to_string())?;
    db::list_roots(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn load_cached_scan(
    state: tauri::State<AppState>,
    root: String,
) -> Result<Option<cutest_disk_tree::ScanResult>, String> {
    let conn = db::open_db(&state.db_path).map_err(|e| e.to_string())?;
    db::get_scan_result(&conn, &root).map_err(|e| e.to_string())
}

#[tauri::command]
fn find_files(
    state: tauri::State<AppState>,
    root: String,
    query: String,
    extensions: Option<String>,
) -> Result<Vec<cutest_disk_tree::FileEntrySer>, String> {
    let conn = db::open_db(&state.db_path).map_err(|e| e.to_string())?;

    let mut sql = String::from(
        "SELECT path, size, dev, ino FROM files WHERE root = ?1",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(root.clone())];

    if let Some(exts) = extensions {
        let cleaned: Vec<String> = exts
            .split(',')
            .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if !cleaned.is_empty() {
            sql.push_str(" AND type IN (");
            for (i, _e) in cleaned.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                sql.push_str(&format!("?{}", i + 2));
            }
            sql.push(')');
            for e in cleaned {
                params.push(Box::new(e));
            }
        }
    }

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Ok(Vec::new());
    }

    if query.trim().is_empty() {
        let mut results: Vec<cutest_disk_tree::FileEntrySer> = rows
            .into_iter()
            .map(|(path, size, dev, ino)| cutest_disk_tree::FileEntrySer {
                path,
                size,
                file_key: cutest_disk_tree::FileKey { dev, ino },
            })
            .collect();

        results.sort_by(|a, b| a.path.cmp(&b.path));

        return Ok(results);
    }

    let q = query.trim().to_lowercase();
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|(path, _, _, _)| path.to_lowercase().contains(&q))
        .collect();

    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let labels: Vec<String> = rows.iter().map(|(path, _, _, _)| path.clone()).collect();

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(
        &query,
        CaseMatching::Smart,
        Normalization::Smart,
    );

    let mut scored = pattern.match_list(
        labels.iter().map(|s| s.as_str()),
        &mut matcher,
    );

    scored.sort_by(|a, b| b.1.cmp(&a.1));

    let mut results: Vec<cutest_disk_tree::FileEntrySer> = Vec::new();
    for (label, _score) in scored.into_iter().take(200) {
        if let Some((path, size, dev, ino)) = rows
            .iter()
            .find(|(p, _, _, _)| p == label)
        {
            results.push(cutest_disk_tree::FileEntrySer {
                path: path.clone(),
                size: *size,
                file_key: cutest_disk_tree::FileKey {
                    dev: *dev,
                    ino: *ino,
                },
            });
        }
    }

    Ok(results)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let path = app.path().app_data_dir().map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            let db_path = path.join("index.db");
            db::open_db(&db_path).map_err(|e| e.to_string())?;
            app.manage(AppState { db_path });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_directory,
            list_cached_roots,
            load_cached_scan,
            find_files,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
