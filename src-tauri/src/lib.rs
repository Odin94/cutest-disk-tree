use cutest_disk_tree::{db, index_directory};
use std::path::Path;
use tauri::Manager;

struct AppState {
    db_path: std::path::PathBuf,
}

#[tauri::command]
fn scan_directory(
    state: tauri::State<AppState>,
    path: String,
) -> Result<cutest_disk_tree::ScanResult, String> {
    let path_buf = Path::new(&path);
    if !path_buf.is_dir() {
        return Err(format!("Not a directory: {}", path));
    }
    let (files, folder_sizes) = index_directory(path_buf);
    let result = cutest_disk_tree::to_scan_result(path_buf, &files, &folder_sizes)
        .ok_or("Indexing failed")?;
    let conn = db::open_db(&state.db_path).map_err(|e| e.to_string())?;
    db::write_scan(&conn, &path, &files, &folder_sizes).map_err(|e| e.to_string())?;
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
