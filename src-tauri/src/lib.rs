use cutest_disk_tree::index_directory_serializable;
use std::path::Path;

#[tauri::command]
fn scan_directory(path: String) -> Result<cutest_disk_tree::ScanResult, String> {
    let path_buf = Path::new(&path);
    if !path_buf.is_dir() {
        return Err(format!("Not a directory: {}", path));
    }
    index_directory_serializable(path_buf).ok_or_else(|| "Indexing failed".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![scan_directory])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
