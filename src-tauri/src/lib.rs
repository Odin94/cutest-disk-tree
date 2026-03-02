use cutest_disk_tree::{db, index_directory_parallel_with_progress};
use std::path::Path;
use std::sync::Mutex;
use std::process;
use std::time::Instant;
use tauri::Manager;
use tauri::Emitter;
use nucleo::{Matcher, Config};
use nucleo::pattern::{Pattern, CaseMatching, Normalization};
use serde::Serialize;
use chrono::Utc;
use std::io::Write;
use sysinfo::{Pid, System};

#[derive(Serialize)]
struct SearchEntry {
    path: String,
    size: u64,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_key: Option<cutest_disk_tree::FileKey>,
}

struct AppState {
    db_path: std::path::PathBuf,
    debug_log: Mutex<Option<std::path::PathBuf>>,
}

fn write_debug_log(state: &AppState, message: &str) {
    let path = state
        .db_path
        .parent()
        .map(|p| p.join("debug.log"))
        .unwrap_or_else(|| std::path::PathBuf::from("debug.log"));
    let mut guard = state.debug_log.lock().unwrap();
    if guard.is_none() {
        *guard = Some(path.clone());
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{} {}", Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"), message);
        let _ = f.flush();
    }
}

#[tauri::command]
fn debug_log(state: tauri::State<AppState>, message: String) -> Result<(), String> {
    write_debug_log(&state, &message);
    Ok(())
}

#[tauri::command]
fn get_debug_log_path(state: tauri::State<AppState>) -> Result<String, String> {
    let path = state
        .db_path
        .parent()
        .map(|p| p.join("debug.log"))
        .unwrap_or_else(|| std::path::PathBuf::from("debug.log"));
    Ok(path.display().to_string())
}

fn log_memory_and_message(state: &AppState, message: &str) {
    let mem_mb = (|| {
        let mut sys = System::new_all();
        sys.refresh_all();
        #[cfg(windows)]
        let pid = Pid::from(process::id() as usize);
        #[cfg(not(windows))]
        let pid = Pid::from_u32(process::id());
        sys.process(pid)
            .map(|p| p.memory() / (1024 * 1024))
            .unwrap_or(0)
    })();
    let line = format!("memory_mb={} {}", mem_mb, message);
    write_debug_log(state, &line);
}

#[tauri::command]
fn debug_log_stats(state: tauri::State<AppState>, message: String) -> Result<(), String> {
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        log_memory_and_message(&state, &message);
    }));
    if res.is_err() {
        write_debug_log(&state, &format!("memory_mb=panic {}", message));
    }
    Ok(())
}

#[tauri::command]
async fn scan_directory(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<cutest_disk_tree::ScanResult, String> {
    write_debug_log(&state, &format!("scan_directory started path={}", path));
    let path_buf = Path::new(&path).to_path_buf();
    if !path_buf.is_dir() {
        let e = format!("Not a directory: {}", path);
        write_debug_log(&state, &format!("error scan_directory: {}", e));
        return Err(e);
    }
    let db_path = state.db_path.clone();
    let path_for_db = path.clone();

    let result = match tauri::async_runtime::spawn_blocking(move || {
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
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            write_debug_log(&state, &format!("error scan_directory: {}", e));
            return Err(e);
        }
        Err(e) => {
            write_debug_log(&state, &format!("error scan_directory spawn: {}", e));
            return Err(e.to_string());
        }
    };

    write_debug_log(&state, &format!("scan_directory done path={}", path));
    Ok(result)
}

#[tauri::command]
async fn list_cached_roots(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    write_debug_log(&state, "list_cached_roots started");
    let db_path = state.db_path.clone();
    let roots = match tauri::async_runtime::spawn_blocking(move || {
        let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;
        db::list_roots(&conn).map_err(|e| e.to_string())
    })
    .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            write_debug_log(&state, &format!("error list_cached_roots: {}", e));
            return Err(e);
        }
        Err(e) => {
            write_debug_log(&state, &format!("error list_cached_roots spawn: {}", e));
            return Err(e.to_string());
        }
    };
    write_debug_log(&state, &format!("list_cached_roots done count={}", roots.len()));
    Ok(roots)
}

#[tauri::command]
async fn load_cached_scan(
    state: tauri::State<'_, AppState>,
    root: String,
) -> Result<Option<cutest_disk_tree::ScanResult>, String> {
    write_debug_log(&state, &format!("load_cached_scan started root={}", root));
    let db_path = state.db_path.clone();
    let root_clone = root.clone();
    let result = match tauri::async_runtime::spawn_blocking(move || {
        let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;
        let res = db::get_scan_result(&conn, &root_clone).map_err(|e| e.to_string())?;
        Ok::<_, String>(res)
    })
    .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            write_debug_log(&state, &format!("error load_cached_scan: {}", e));
            return Err(e);
        }
        Err(e) => {
            write_debug_log(&state, &format!("error load_cached_scan spawn: {}", e));
            return Err(e.to_string());
        }
    };
    write_debug_log(
        &state,
        &format!(
            "load_cached_scan done root={} has_result={}",
            root,
            result.is_some()
        ),
    );
    Ok(result)
}

#[tauri::command]
async fn list_cached_tree_depths(
    state: tauri::State<'_, AppState>,
    root: String,
    max_children: u32,
) -> Result<Vec<u32>, String> {
    let db_path = state.db_path.clone();
    let root_clone = root.clone();
    let depths = match tauri::async_runtime::spawn_blocking(move || {
        let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;
        db::list_cached_tree_depths(&conn, &root_clone, max_children).map_err(|e| e.to_string())
    })
    .await
    {
        Ok(Ok(d)) => d,
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(e.to_string()),
    };
    Ok(depths)
}

#[derive(Clone, Debug)]
struct BuildDiskTreeProfile {
    open_db_ms: u64,
    files_query_ms: u64,
    folders_query_ms: u64,
    get_scan_result_total_ms: u64,
    build_disk_tree_ms: u64,
    total_ms: u64,
    tree_collect_folders_ms: u64,
    tree_collect_files_ms: u64,
    tree_sort_combine_ms: u64,
    tree_recurse_ms: u64,
}

#[tauri::command]
async fn build_disk_tree_cached(
    state: tauri::State<'_, AppState>,
    root: String,
    max_children_per_node: u32,
    max_depth: u32,
) -> Result<Option<cutest_disk_tree::DiskTreeNode>, String> {
    write_debug_log(&state, &format!("build_disk_tree_cached started root={} max_depth={}", root, max_depth));
    let db_path = state.db_path.clone();
    let root_clone = root.clone();
    let result = match tauri::async_runtime::spawn_blocking(move || {
        let total_start = Instant::now();

        let t0 = Instant::now();
        let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;
        let open_db_ms = t0.elapsed().as_millis() as u64;

        if let Some(cached) = db::get_cached_tree(&conn, &root_clone, max_depth, max_children_per_node).map_err(|e| e.to_string())? {
            let total_ms = total_start.elapsed().as_millis() as u64;
            let profile = BuildDiskTreeProfile {
                open_db_ms,
                files_query_ms: 0,
                folders_query_ms: 0,
                get_scan_result_total_ms: 0,
                build_disk_tree_ms: 0,
                total_ms,
                tree_collect_folders_ms: 0,
                tree_collect_files_ms: 0,
                tree_sort_combine_ms: 0,
                tree_recurse_ms: 0,
            };
            return Ok((Some(cached), profile));
        }

        if let Some(tree_from_db) = cutest_disk_tree::build_disk_tree_from_db(
            &conn,
            &root_clone,
            max_children_per_node as usize,
            max_depth as usize,
        ) {
            let total_ms = total_start.elapsed().as_millis() as u64;
            let _ = db::write_cached_tree(&conn, &root_clone, max_depth, max_children_per_node, &tree_from_db);
            let profile = BuildDiskTreeProfile {
                open_db_ms,
                files_query_ms: 0,
                folders_query_ms: 0,
                get_scan_result_total_ms: 0,
                build_disk_tree_ms: total_ms.saturating_sub(open_db_ms),
                total_ms,
                tree_collect_folders_ms: 0,
                tree_collect_files_ms: 0,
                tree_sort_combine_ms: 0,
                tree_recurse_ms: 0,
            };
            return Ok((Some(tree_from_db), profile));
        }

        let t1 = Instant::now();
        let (scan, timings) = db::get_scan_result_timed(&conn, &root_clone).map_err(|e| e.to_string())?;
        let get_scan_result_total_ms = t1.elapsed().as_millis() as u64;
        let scan = match scan {
            Some(s) => s,
            None => return Ok::<_, String>((None, BuildDiskTreeProfile {
                open_db_ms,
                files_query_ms: timings.files_query_ms,
                folders_query_ms: timings.folders_query_ms,
                get_scan_result_total_ms,
                build_disk_tree_ms: 0,
                total_ms: total_start.elapsed().as_millis() as u64,
                tree_collect_folders_ms: 0,
                tree_collect_files_ms: 0,
                tree_sort_combine_ms: 0,
                tree_recurse_ms: 0,
            })),
        };

        let t2 = Instant::now();
        let (tree, tree_timings) = cutest_disk_tree::build_disk_tree_timed(
            &scan,
            max_children_per_node as usize,
            max_depth as usize,
        );
        let build_disk_tree_ms = t2.elapsed().as_millis() as u64;

        if let Some(ref node) = tree {
            let _ = db::write_cached_tree(&conn, &root_clone, max_depth, max_children_per_node, node);
        }

        let profile = BuildDiskTreeProfile {
            open_db_ms,
            files_query_ms: timings.files_query_ms,
            folders_query_ms: timings.folders_query_ms,
            get_scan_result_total_ms,
            build_disk_tree_ms,
            total_ms: total_start.elapsed().as_millis() as u64,
            tree_collect_folders_ms: tree_timings.collect_folders_ms,
            tree_collect_files_ms: tree_timings.collect_files_ms,
            tree_sort_combine_ms: tree_timings.sort_combine_ms,
            tree_recurse_ms: tree_timings.recurse_ms,
        };
        Ok((tree, profile))
    })
    .await
    {
        Ok(Ok((tree, profile))) => {
            write_debug_log(
                &state,
                &format!(
                    "build_disk_tree_cached profile open_db_ms={} files_query_ms={} folders_query_ms={} get_scan_total_ms={} build_disk_tree_ms={} (tree: collect_folders_ms={} collect_files_ms={} sort_combine_ms={} recurse_ms={}) total_ms={}",
                    profile.open_db_ms,
                    profile.files_query_ms,
                    profile.folders_query_ms,
                    profile.get_scan_result_total_ms,
                    profile.build_disk_tree_ms,
                    profile.tree_collect_folders_ms,
                    profile.tree_collect_files_ms,
                    profile.tree_sort_combine_ms,
                    profile.tree_recurse_ms,
                    profile.total_ms,
                ),
            );
            write_debug_log(
                &state,
                &format!(
                    "build_disk_tree_cached done root={} has_tree={}",
                    root,
                    tree.is_some()
                ),
            );
            tree
        }
        Ok(Err(e)) => {
            write_debug_log(&state, &format!("error build_disk_tree_cached: {}", e));
            return Err(e);
        }
        Err(e) => {
            write_debug_log(&state, &format!("error build_disk_tree_cached spawn: {}", e));
            return Err(e.to_string());
        }
    };
    Ok(result)
}

#[tauri::command]
fn find_files(
    state: tauri::State<AppState>,
    root: String,
    query: String,
    extensions: Option<String>,
) -> Result<Vec<SearchEntry>, String> {
    write_debug_log(
        &state,
        &format!("find_files started root={} query_len={}", root, query.len()),
    );
    let log_err = |e: &dyn std::fmt::Display| {
        let s = e.to_string();
        write_debug_log(&state, &format!("error find_files: {}", s));
        s
    };
    let conn = db::open_db(&state.db_path).map_err(|e| log_err(&e))?;

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
        .map_err(|e| log_err(&e))?;

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
        .map_err(|e| log_err(&e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| log_err(&e))?;

    if query.trim().is_empty() {
        if rows.is_empty() {
            write_debug_log(&state, "find_files done count=0 (empty query, no rows)");
            return Ok(Vec::new());
        }

        let mut results: Vec<SearchEntry> = rows
            .into_iter()
            .map(|(path, size, dev, ino)| SearchEntry {
                path,
                size,
                kind: "file".to_string(),
                file_key: Some(cutest_disk_tree::FileKey { dev, ino }),
            })
            .collect();

        results.sort_by(|a, b| a.path.cmp(&b.path));

        write_debug_log(&state, &format!("find_files done count={}", results.len()));
        return Ok(results);
    }

    let q = query.trim().to_lowercase();
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|(path, _, _, _)| path.to_lowercase().contains(&q))
        .collect();

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

    let mut results: Vec<SearchEntry> = Vec::new();
    for (label, _score) in scored.into_iter().take(200) {
        if let Some((path, size, dev, ino)) = rows
            .iter()
            .find(|(p, _, _, _)| p == label)
        {
            results.push(SearchEntry {
                path: path.clone(),
                size: *size,
                kind: "file".to_string(),
                file_key: Some(cutest_disk_tree::FileKey {
                    dev: *dev,
                    ino: *ino,
                }),
            });
        }
    }

    if !query.trim().is_empty() {
        let mut folder_stmt = conn
            .prepare("SELECT path, recursive_size FROM folders WHERE root = ?1")
            .map_err(|e| log_err(&e))?;
        let folder_rows = folder_stmt
            .query_map([&root], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? as u64,
                ))
            })
            .map_err(|e| log_err(&e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| log_err(&e))?;

        for (path, recursive_size) in folder_rows.into_iter() {
            if path.to_lowercase().contains(&q) {
                results.push(SearchEntry {
                    path,
                    size: recursive_size,
                    kind: "folder".to_string(),
                    file_key: None,
                });
            }
        }
    }

    write_debug_log(&state, &format!("find_files done count={}", results.len()));
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
            let debug_log_path = path.join("debug.log");
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&debug_log_path)
                .and_then(|mut f| {
                    use std::io::Write;
                    f.write_all(b"=== starting cutest disk tree ===\n\n")?;
                    f.flush()
                });
            app.manage(AppState {
                db_path,
                debug_log: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_directory,
            list_cached_roots,
            load_cached_scan,
            list_cached_tree_depths,
            build_disk_tree_cached,
            find_files,
            debug_log,
            get_debug_log_path,
            debug_log_stats,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
