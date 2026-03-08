use cutest_disk_tree::{db, DiskObject, DiskObjectKind};
use cutest_disk_tree::core::indexing::suffix::{SuffixIndex, build_suffix_index, search_suffix_index};
use cutest_disk_tree::core::indexing::sqlite::search_disk_objects_by_name;
use std::collections::{HashMap, HashSet};
use suffix::SuffixTable;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::process;
use std::time::Instant;
use tauri::Manager;
use tauri::Emitter;
use nucleo::{Matcher, Config};
use nucleo::pattern::{Pattern, CaseMatching, Normalization};
use serde::Serialize;
use std::io::Write;
use sysinfo::{Pid, System};

mod category_filter {
    use cutest_disk_tree::{DiskObject, DiskObjectKind};
    use std::collections::HashSet;

    const AUDIO: &[&str] = &["mp3", "wav", "flac", "m4a", "ogg", "aac", "opus"];
    const DOCUMENT: &[&str] = &[
        "pdf", "txt", "md", "rtf", "doc", "docx", "odt", "xls", "xlsx", "csv",
        "ppt", "pptx",
    ];
    const VIDEO: &[&str] = &["mp4", "mkv", "mov", "avi", "webm", "m4v"];
    const IMAGE: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "heic", "bmp", "tiff", "svg"];
    const EXECUTABLE: &[&str] = &[
        "exe", "dll", "so", "dylib", "bin", "sh", "bat", "cmd", "appimage",
    ];
    const COMPRESSED: &[&str] = &["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz"];
    const CONFIG: &[&str] = &[
        "cfg", "conf", "ini", "json", "yaml", "yml", "toml", "xml", "props",
        "properties", "rc", "config", "env",
    ];

    fn all_known_extensions() -> HashSet<&'static str> {
        AUDIO
            .iter()
            .chain(DOCUMENT)
            .chain(VIDEO)
            .chain(IMAGE)
            .chain(EXECUTABLE)
            .chain(COMPRESSED)
            .chain(CONFIG)
            .copied()
            .collect()
    }

    fn extension_set(category: &str) -> Option<HashSet<&'static str>> {
        let set: HashSet<&str> = match category {
            "audio" => AUDIO.iter().copied().collect(),
            "document" => DOCUMENT.iter().copied().collect(),
            "video" => VIDEO.iter().copied().collect(),
            "image" => IMAGE.iter().copied().collect(),
            "executable" => EXECUTABLE.iter().copied().collect(),
            "compressed" => COMPRESSED.iter().copied().collect(),
            "config" => CONFIG.iter().copied().collect(),
            _ => return None,
        };
        Some(set)
    }

    pub fn category_allowed(category: Option<&str>, obj: &DiskObject) -> bool {
        let category = match category {
            None => return true,
            Some("all") | Some("") => return true,
            Some(c) => c.trim(),
        };
        if category.is_empty() {
            return true;
        }
        match category {
            "folder" => matches!(obj.kind, DiskObjectKind::Folder),
            "other" => {
                if matches!(obj.kind, DiskObjectKind::Folder) {
                    return false;
                }
                let known = all_known_extensions();
                match &obj.ext {
                    None => true,
                    Some(ext) => {
                        let ext_lower = ext.trim().to_lowercase();
                        !known.contains(ext_lower.as_str())
                    }
                }
            }
            _ => {
                if matches!(obj.kind, DiskObjectKind::Folder) {
                    return true;
                }
                let set = match extension_set(category) {
                    Some(s) => s,
                    None => return true,
                };
                obj.ext.as_ref().map(|e| {
                    let ext_lower = e.trim().to_lowercase();
                    set.contains(ext_lower.as_str())
                }).unwrap_or(false)
            }
        }
    }
}

#[derive(Clone, Serialize)]
struct FolderSizesReady {
    folder_sizes: HashMap<String, u64>,
}

#[derive(Serialize)]
struct SearchEntry {
    path: String,
    size: u64,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_key: Option<cutest_disk_tree::FileKey>,
}

#[derive(Serialize)]
struct ScanDirectoryResponse {
    roots: Vec<String>,
    files_count: u64,
    folders_count: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FindFilesResponse {
    items: Vec<SearchEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_offset: Option<usize>,
}

fn search_entry_from_disk_object(o: &DiskObject) -> SearchEntry {
    SearchEntry {
        path: o.path.clone(),
        size: o.size.or(o.recursive_size).unwrap_or(0),
        kind: match o.kind {
            DiskObjectKind::File => "file".to_string(),
            DiskObjectKind::Folder => "folder".to_string(),
        },
        file_key: match o.kind {
            DiskObjectKind::File => Some(cutest_disk_tree::FileKey {
                dev: o.dev.unwrap_or(0),
                ino: o.ino.unwrap_or(0),
            }),
            DiskObjectKind::Folder => None,
        },
    }
}

fn paginate_scan<F>(
    disk_entries: &[DiskObject],
    start_index: usize,
    limit: usize,
    use_mask: bool,
    allowed_indices: &[bool],
    mut predicate: F,
) -> (Vec<SearchEntry>, Option<usize>)
where
    F: FnMut(usize, &DiskObject) -> bool,
{
    if start_index >= disk_entries.len() || limit == 0 {
        return (Vec::new(), None);
    }

    let mut items: Vec<SearchEntry> = Vec::new();
    let mut produced = 0usize;
    let mut last_index = None;

    for (i, e) in disk_entries.iter().enumerate().skip(start_index) {
        if use_mask && !allowed_indices[i] {
            last_index = Some(i);
            continue;
        }
        if !predicate(i, e) {
            last_index = Some(i);
            continue;
        }
        items.push(search_entry_from_disk_object(e));
        produced += 1;
        last_index = Some(i);
        if produced == limit {
            break;
        }
    }

    let next_offset = match last_index {
        Some(i) if produced == limit && i + 1 < disk_entries.len() => Some(i + 1),
        _ => None,
    };

    (items, next_offset)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod search_tests;

struct AppState {
    db_path: std::path::PathBuf,
    debug_log: Mutex<Option<std::path::PathBuf>>,
    disk_objects: Mutex<Option<Arc<Vec<DiskObject>>>>,
    name_reverse_index: Mutex<Option<Arc<SuffixIndex>>>,
    phase2_cancel: Mutex<Arc<AtomicBool>>,
    is_scanning: AtomicBool,
    scan_path_override: Option<String>,
}

fn resolve_debug_log_path(state: &AppState) -> std::path::PathBuf {
    let mut guard = state.debug_log.lock().unwrap();
    guard
        .get_or_insert_with(|| {
            state
                .db_path
                .parent()
                .map(|p| p.join("debug.log"))
                .unwrap_or_else(|| std::path::PathBuf::from("debug.log"))
        })
        .clone()
}

fn write_debug_log(state: &AppState, message: &str) {
    let path = resolve_debug_log_path(state);
    cutest_disk_tree::logging::debug_log::write_debug_log(&path, message);
}

#[tauri::command]
fn debug_log(state: tauri::State<AppState>, message: String) -> Result<(), String> {
    write_debug_log(&state, &message);
    Ok(())
}

#[tauri::command]
fn get_debug_log_path(state: tauri::State<AppState>) -> Result<String, String> {
    let path = resolve_debug_log_path(&state);
    Ok(path.display().to_string())
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

fn make_disk_object_from_path(
    path_string: String,
    kind: DiskObjectKind,
    size: Option<u64>,
    recursive_size: Option<u64>,
    dev: Option<u64>,
    ino: Option<u64>,
    mtime: Option<i64>,
) -> DiskObject {
    let path_lower = path_string.to_ascii_lowercase();
    let parent = cutest_disk_tree::parent_dir(&path_string);
    let name = std::path::Path::new(&path_string)
        .file_name()
        .and_then(|os| os.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path_string.clone());
    let name_lower = name.to_ascii_lowercase();
    let ext = match kind {
        DiskObjectKind::File => std::path::Path::new(&path_string)
            .extension()
            .and_then(|os| os.to_str())
            .map(|s| s.to_ascii_lowercase()),
        DiskObjectKind::Folder => None,
    };
    DiskObject {
        path: path_string,
        path_lower,
        parent_path: if parent.is_empty() { None } else { Some(parent) },
        name,
        name_lower,
        ext,
        kind,
        size,
        recursive_size,
        dev,
        ino,
        mtime,
    }
}

fn build_disk_objects(
    files: &[cutest_disk_tree::FileEntry],
    folder_paths: &std::collections::HashSet<std::path::PathBuf>,
) -> Vec<DiskObject> {
    let mut objs: Vec<DiskObject> = Vec::with_capacity(files.len() + folder_paths.len());
    for f in files {
        objs.push(make_disk_object_from_path(
            f.path.to_string_lossy().into_owned(),
            DiskObjectKind::File,
            Some(f.size),
            None,
            Some(f.file_key.dev),
            Some(f.file_key.ino),
            f.mtime,
        ));
    }
    for folder in folder_paths {
        objs.push(make_disk_object_from_path(
            folder.to_string_lossy().into_owned(),
            DiskObjectKind::Folder,
            None, None, None, None, None,
        ));
    }
    objs.sort_by(|a, b| a.path.cmp(&b.path));
    objs
}

fn apply_folder_sizes(
    mut objs: Vec<DiskObject>,
    folder_sizes: &HashMap<std::path::PathBuf, u64>,
) -> Vec<DiskObject> {
    let mut path_to_index: HashMap<String, usize> = HashMap::new();
    for (i, o) in objs.iter().enumerate() {
        if matches!(o.kind, DiskObjectKind::Folder) {
            path_to_index.insert(o.path.clone(), i);
        }
    }
    for (p, &s) in folder_sizes {
        let path_string = p.to_string_lossy().into_owned();
        if let Some(&idx) = path_to_index.get(&path_string) {
            objs[idx].recursive_size = Some(s);
        } else {
            objs.push(make_disk_object_from_path(
                path_string, DiskObjectKind::Folder,
                None, Some(s), None, None, None,
            ));
        }
    }
    objs.sort_by(|a, b| a.path.cmp(&b.path));
    objs
}

fn activate_initial_index(
    app: &tauri::AppHandle,
    state: &AppState,
    files: &[cutest_disk_tree::FileEntry],
    folder_paths: &std::collections::HashSet<std::path::PathBuf>,
    cancel: &AtomicBool,
) {
    let t0 = Instant::now();

    write_debug_log(state, &format!(
        "activate_initial_index starting files={} folders={}",
        files.len(), folder_paths.len(),
    ));
    let _ = app.emit("scan-phase-status", "building disk objects...".to_string());

    let build_start = Instant::now();
    let objs = build_disk_objects(files, folder_paths);
    let build_ms = build_start.elapsed().as_millis();
    write_debug_log(state, &format!(
        "activate_initial_index build_disk_objects done objects={} ms={}",
        objs.len(), build_ms,
    ));

    if cancel.load(Ordering::Relaxed) {
        write_debug_log(state, &format!(
            "activate_initial_index cancelled after build_disk_objects ms={}", build_ms,
        ));
        return;
    }

    let _ = app.emit("scan-phase-status", "building suffix index...".to_string());
    write_debug_log(state, "activate_initial_index building suffix index");
    let (index, suffix_concat_ms, suffix_table_ms) = build_suffix_index(&objs);
    write_debug_log(state, &format!(
        "activate_initial_index build_suffix_index done suffix_concat_ms={} suffix_table_ms={} total_ms={}",
        suffix_concat_ms, suffix_table_ms, t0.elapsed().as_millis(),
    ));

    if cancel.load(Ordering::Relaxed) {
        write_debug_log(state, &format!(
            "activate_initial_index cancelled after build_suffix_index ms={}", t0.elapsed().as_millis(),
        ));
        return;
    }

    write_debug_log(state, &format!(
        "activate_initial_index done files={} folders={} objects={} build_disk_objs_ms={} suffix_concat_ms={} suffix_table_ms={} total_ms={}",
        files.len(), folder_paths.len(), objs.len(), build_ms, suffix_concat_ms, suffix_table_ms, t0.elapsed().as_millis(),
    ));

    {
        let mut disk_guard = state.disk_objects.lock().unwrap();
        *disk_guard = Some(Arc::new(objs));
        let mut idx_guard = state.name_reverse_index.lock().unwrap();
        *idx_guard = Some(Arc::new(index));
    }

    let _ = app.emit("scan-phase-status", "".to_string());
    let _ = app.emit("scan-index-ready", true);
}

fn run_phase2(
    app_bg: tauri::AppHandle,
    db_path_bg: std::path::PathBuf,
    scan_roots: Vec<std::path::PathBuf>,
    files_bg: Arc<Vec<cutest_disk_tree::FileEntry>>,
    folder_paths: std::collections::HashSet<std::path::PathBuf>,
    cancel: Arc<AtomicBool>,
) {
    let state_ptr: tauri::State<AppState> = app_bg.state();
    let total_start = Instant::now();

    write_debug_log(&state_ptr, &format!(
        "phase2 starting files={} folders={} roots={:?}",
        files_bg.len(), folder_paths.len(), scan_roots,
    ));

    activate_initial_index(&app_bg, &state_ptr, &files_bg, &folder_paths, &cancel);

    if cancel.load(Ordering::Relaxed) {
        write_debug_log(&state_ptr, "phase2 cancelled after index build");
        let _ = app_bg.emit("scan-phase-status", "".to_string());
        return;
    }

    write_debug_log(&state_ptr, "phase2 computing folder sizes");
    let _ = app_bg.emit("scan-phase-status", "aggregating folder sizes...".to_string());
    let sizes_start = Instant::now();
    let mut folder_sizes: HashMap<std::path::PathBuf, u64> = HashMap::new();
    for root in &scan_roots {
        let part = cutest_disk_tree::compute_folder_sizes(root, &files_bg);
        for (k, v) in part {
            *folder_sizes.entry(k).or_insert(0) += v;
        }
    }
    let sizes_ms = sizes_start.elapsed().as_millis();
    write_debug_log(&state_ptr, &format!(
        "phase2 folder_sizes_done folders={} ms={}",
        folder_sizes.len(), sizes_ms,
    ));

    if cancel.load(Ordering::Relaxed) {
        write_debug_log(&state_ptr, "phase2 cancelled after folder_sizes");
        let _ = app_bg.emit("scan-phase-status", "".to_string());
        return;
    }

    write_debug_log(&state_ptr, "phase2 applying folder sizes to index");
    let _ = app_bg.emit("scan-phase-status", "updating search index with sizes...".to_string());
    let existing_arc = {
        let guard = state_ptr.disk_objects.lock().unwrap();
        guard.clone()
    };
    if let Some(arc) = existing_arc {
        let apply_start = Instant::now();
        let new_objs = apply_folder_sizes((*arc).clone(), &folder_sizes);
        let apply_ms = apply_start.elapsed().as_millis();

        write_debug_log(&state_ptr, &format!(
            "phase2 index_updated objects={} apply_folder_sizes_ms={} total_ms={}",
            new_objs.len(), apply_ms, total_start.elapsed().as_millis(),
        ));

        {
            let mut disk_guard = state_ptr.disk_objects.lock().unwrap();
            *disk_guard = Some(Arc::new(new_objs));
        }
    }

    write_debug_log(&state_ptr, "phase2 emitting folder sizes to frontend");
    let folder_sizes_ser: HashMap<String, u64> = folder_sizes
        .iter()
        .map(|(p, s)| (p.to_string_lossy().to_string(), *s))
        .collect();
    let _ = app_bg.emit("scan-folder-sizes-ready", FolderSizesReady {
        folder_sizes: folder_sizes_ser,
    });

    if cancel.load(Ordering::Relaxed) {
        write_debug_log(&state_ptr, "phase2 cancelled after folder_sizes emit");
        let _ = app_bg.emit("scan-phase-status", "".to_string());
        return;
    }

    write_debug_log(&state_ptr, "phase2 opening database");
    let _ = app_bg.emit("scan-phase-status", "saving to database...".to_string());
    let update_id = chrono::Utc::now().timestamp_millis();
    let db_open_start = Instant::now();
    let conn = match db::open_db(&db_path_bg) {
        Ok(c) => c,
        Err(e) => {
            write_debug_log(&state_ptr, &format!(
                "phase2 db_open_failed error={} total_ms={}",
                e, total_start.elapsed().as_millis(),
            ));
            let _ = app_bg.emit("scan-phase-status", "".to_string());
            return;
        }
    };
    let db_open_ms = db_open_start.elapsed().as_millis();
    write_debug_log(&state_ptr, &format!("phase2 db_open done ms={}", db_open_ms));

    write_debug_log(&state_ptr, "phase2 writing scan data to db");
    let _ = app_bg.emit("scan-phase-status", "writing scan data to database...".to_string());
    let db_write_start = Instant::now();
    let write_result = db::write_scan(&conn, &files_bg, &folder_sizes, update_id);
    let db_write_ms = db_write_start.elapsed().as_millis();
    write_debug_log(&state_ptr, &format!(
        "phase2 db_write done ok={} ms={}",
        write_result.is_ok(), db_write_ms,
    ));

    write_debug_log(&state_ptr, "phase2 writing suffix index to db");
    let _ = app_bg.emit("scan-phase-status", "writing search index to database...".to_string());
    let index_write_ms = if write_result.is_ok() {
        let index_arc = {
            let guard = state_ptr.name_reverse_index.lock().unwrap();
            guard.clone()
        };
        if let Some(arc) = index_arc {
            let idx_start = Instant::now();
            let _ = db::write_suffix_index_data(
                &conn, update_id,
                &arc.buffer, &arc.offsets, &arc.disk_object_indices,
            );
            let ms = idx_start.elapsed().as_millis();
            write_debug_log(&state_ptr, &format!("phase2 suffix_index_write done ms={}", ms));
            ms
        } else {
            write_debug_log(&state_ptr, "phase2 no suffix index to write");
            0
        }
    } else {
        0
    };

    let total_ms = total_start.elapsed().as_millis();

    match write_result {
        Ok(()) => {
            write_debug_log(&state_ptr, &format!(
                "phase2 done files={} folders={} sizes_ms={} db_open_ms={} db_write_ms={} index_write_ms={} total_ms={}",
                files_bg.len(), folder_sizes.len(), sizes_ms, db_open_ms, db_write_ms, index_write_ms, total_ms,
            ));
        }
        Err(e) => {
            write_debug_log(&state_ptr, &format!(
                "phase2 db_write_failed error={} sizes_ms={} db_open_ms={} db_write_ms={} total_ms={}",
                e, sizes_ms, db_open_ms, db_write_ms, total_ms,
            ));
        }
    }
    let _ = app_bg.emit("scan-phase-status", "".to_string());
}

#[tauri::command]
async fn scan_directory(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<ScanDirectoryResponse, String> {
    write_debug_log(&state, &format!(
        "scan_directory called is_scanning={} scan_path_override={:?}",
        state.is_scanning.load(Ordering::SeqCst),
        state.scan_path_override,
    ));

    if state.is_scanning.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        write_debug_log(&state, "scan_directory rejected: scan already in progress");
        return Err("A scan is already in progress".to_string());
    }

    let scan_roots: Vec<std::path::PathBuf> = match &state.scan_path_override {
        Some(p) => vec![std::path::PathBuf::from(p)],
        None => cutest_disk_tree::get_filesystem_roots(),
    };

    write_debug_log(&state, &format!("scan_directory started roots={:?}", scan_roots));
    for root in &scan_roots {
        if !root.is_dir() {
            let e = format!("Not a directory: {}", root.display());
            write_debug_log(&state, &format!("error scan_directory: {}", e));
            state.is_scanning.store(false, Ordering::SeqCst);
            return Err(e);
        }
    }
    let db_path = state.db_path.clone();

    let app_for_scan = app.clone();
    let scan_start = Instant::now();
    let roots_for_scan = scan_roots.clone();
    let (files_arc, all_folder_paths, roots_str) = match tauri::async_runtime::spawn_blocking(move || {
        let (files_arc, all_folders, roots_str) =
            cutest_disk_tree::core::scanning::ignore_scanner::scan_roots_with_ignore(
                &roots_for_scan,
                move |p| {
                    let _ = app_for_scan.emit("scan-progress", &p);
                },
            );
        Ok::<_, String>((files_arc, all_folders, roots_str))
    })
    .await
    {
        Ok(Ok(triple)) => triple,
        Ok(Err(e)) => {
            write_debug_log(&state, &format!("error scan_directory phase1: {}", e));
            state.is_scanning.store(false, Ordering::SeqCst);
            return Err(e);
        }
        Err(e) => {
            write_debug_log(&state, &format!("error scan_directory phase1 spawn: {}", e));
            state.is_scanning.store(false, Ordering::SeqCst);
            return Err(e.to_string());
        }
    };
    write_debug_log(&state, &format!(
        "scan_directory phase1_done files={} folders={} ms={}",
        files_arc.len(), all_folder_paths.len(), scan_start.elapsed().as_millis(),
    ));

    let cancel_token = {
        let mut guard = state.phase2_cancel.lock().unwrap();
        guard.store(true, Ordering::Relaxed);
        let fresh = Arc::new(AtomicBool::new(false));
        *guard = fresh.clone();
        fresh
    };
    {
        let mut disk_guard = state.disk_objects.lock().unwrap();
        *disk_guard = None;
        let mut idx_guard = state.name_reverse_index.lock().unwrap();
        *idx_guard = None;
    }
    let response = ScanDirectoryResponse {
        roots: roots_str,
        files_count: files_arc.len() as u64,
        folders_count: all_folder_paths.len() as u64,
    };

    let app_bg = app.clone();
    let files_bg = Arc::clone(&files_arc);
    let roots_bg = scan_roots;
    let folder_paths_bg = all_folder_paths;
    tauri::async_runtime::spawn_blocking(move || {
        run_phase2(app_bg, db_path, roots_bg, files_bg, folder_paths_bg, cancel_token);
    });

    state.is_scanning.store(false, Ordering::SeqCst);
    write_debug_log(&state, "scan_directory done");
    Ok(response)
}

#[tauri::command]
async fn load_cached_scan(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<cutest_disk_tree::ScanSummary>, String> {
    let t0 = Instant::now();
    write_debug_log(&state, "load_cached_scan started");
    let db_path = state.db_path.clone();
    let summary = match tauri::async_runtime::spawn_blocking({
        let db_path = db_path.clone();
        move || {
            let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;
            let res = db::get_scan_summary(&conn).map_err(|e| e.to_string())?;
            Ok::<_, String>(res)
        }
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
    let ms = t0.elapsed().as_millis();
    write_debug_log(
        &state,
        &format!(
            "load_cached_scan done has_result={} ms={}",
            summary.is_some(),
            ms
        ),
    );

    if summary.is_some() {
        let app_bg = app.clone();
        let state_ref: &AppState = &state;
        let cancel = {
            let guard = state_ref.phase2_cancel.lock().unwrap();
            guard.clone()
        };
        write_debug_log(&state, "load_cached_scan: spawning background index build from DB");
        tauri::async_runtime::spawn_blocking(move || {
            let state_ptr: tauri::State<AppState> = app_bg.state();
            let t0 = Instant::now();
            let _ = app_bg.emit("scan-phase-status", "loading files from database...".to_string());
            write_debug_log(&state_ptr, "load_cached_scan: loading files from DB");

            let conn = match db::open_db(&db_path) {
                Ok(c) => c,
                Err(e) => {
                    write_debug_log(&state_ptr, &format!("load_cached_scan: db_open failed: {}", e));
                    let _ = app_bg.emit("scan-phase-status", "".to_string());
                    return;
                }
            };
            let scan_result = match db::get_scan_result(&conn) {
                Ok(Some(r)) => r,
                Ok(None) => {
                    write_debug_log(&state_ptr, "load_cached_scan: no scan data in DB");
                    let _ = app_bg.emit("scan-phase-status", "".to_string());
                    return;
                }
                Err(e) => {
                    write_debug_log(&state_ptr, &format!("load_cached_scan: get_scan_result failed: {}", e));
                    let _ = app_bg.emit("scan-phase-status", "".to_string());
                    return;
                }
            };
            let load_ms = t0.elapsed().as_millis();
            write_debug_log(&state_ptr, &format!(
                "load_cached_scan: loaded files={} from DB ms={}",
                scan_result.files.len(), load_ms,
            ));

            let files: Vec<cutest_disk_tree::FileEntry> = scan_result.files.iter().map(|f| {
                cutest_disk_tree::FileEntry {
                    path: std::path::PathBuf::from(&f.path),
                    size: f.size,
                    file_key: cutest_disk_tree::FileKey { dev: f.file_key.dev, ino: f.file_key.ino },
                    mtime: f.mtime,
                }
            }).collect();
            let folder_paths: std::collections::HashSet<std::path::PathBuf> = scan_result.folder_sizes.keys()
                .map(|p| std::path::PathBuf::from(p))
                .collect();

            activate_initial_index(&app_bg, &state_ptr, &files, &folder_paths, &cancel);
            write_debug_log(&state_ptr, &format!(
                "load_cached_scan: index build done total_ms={}",
                t0.elapsed().as_millis(),
            ));
        });
    }

    Ok(summary)
}

#[tauri::command]
async fn list_cached_tree_depths(
    state: tauri::State<'_, AppState>,
    max_children: u32,
) -> Result<Vec<u32>, String> {
    let db_path = state.db_path.clone();
    let depths = match tauri::async_runtime::spawn_blocking(move || {
        let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;
        db::list_cached_tree_depths(&conn, max_children).map_err(|e| e.to_string())
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
    start_path: String,
    max_children_per_node: u32,
    max_depth: u32,
) -> Result<Option<cutest_disk_tree::DiskTreeNode>, String> {
    write_debug_log(&state, &format!("build_disk_tree_cached started start_path={} max_depth={}", start_path, max_depth));
    let db_path = state.db_path.clone();
    let path_clone = start_path.clone();
    let result = match tauri::async_runtime::spawn_blocking(move || {
        let total_start = Instant::now();

        let t0 = Instant::now();
        let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;
        let open_db_ms = t0.elapsed().as_millis() as u64;

        if let Some(cached) = db::get_cached_tree(&conn, max_depth, max_children_per_node).map_err(|e| e.to_string())? {
            let total_ms = total_start.elapsed().as_millis() as u64;
            let profile = BuildDiskTreeProfile {
                open_db_ms,
                files_query_ms: 0, folders_query_ms: 0,
                get_scan_result_total_ms: 0, build_disk_tree_ms: 0,
                total_ms,
                tree_collect_folders_ms: 0, tree_collect_files_ms: 0,
                tree_sort_combine_ms: 0, tree_recurse_ms: 0,
            };
            return Ok((Some(cached), profile));
        }

        if let Some(tree_from_db) = cutest_disk_tree::build_disk_tree_from_db(
            &conn,
            &path_clone,
            max_children_per_node as usize,
            max_depth as usize,
        ) {
            let total_ms = total_start.elapsed().as_millis() as u64;
            let _ = db::write_cached_tree(&conn, max_depth, max_children_per_node, &tree_from_db);
            let profile = BuildDiskTreeProfile {
                open_db_ms,
                files_query_ms: 0, folders_query_ms: 0,
                get_scan_result_total_ms: 0,
                build_disk_tree_ms: total_ms.saturating_sub(open_db_ms),
                total_ms,
                tree_collect_folders_ms: 0, tree_collect_files_ms: 0,
                tree_sort_combine_ms: 0, tree_recurse_ms: 0,
            };
            return Ok((Some(tree_from_db), profile));
        }

        let t1 = Instant::now();
        let (scan, timings) = db::get_scan_result_timed(&conn).map_err(|e| e.to_string())?;
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
                tree_collect_folders_ms: 0, tree_collect_files_ms: 0,
                tree_sort_combine_ms: 0, tree_recurse_ms: 0,
            })),
        };

        let t2 = Instant::now();
        let (tree, tree_timings) = cutest_disk_tree::build_disk_tree_timed(
            &scan,
            &path_clone,
            max_children_per_node as usize,
            max_depth as usize,
        );
        let build_disk_tree_ms = t2.elapsed().as_millis() as u64;

        if let Some(ref node) = tree {
            let _ = db::write_cached_tree(&conn, max_depth, max_children_per_node, node);
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
                    profile.open_db_ms, profile.files_query_ms, profile.folders_query_ms,
                    profile.get_scan_result_total_ms, profile.build_disk_tree_ms,
                    profile.tree_collect_folders_ms, profile.tree_collect_files_ms,
                    profile.tree_sort_combine_ms, profile.tree_recurse_ms, profile.total_ms,
                ),
            );
            write_debug_log(
                &state,
                &format!(
                    "build_disk_tree_cached done start_path={} has_tree={}",
                    start_path,
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
    query: String,
    extensions: Option<String>,
    category: Option<String>,
    limit: Option<u32>,
    use_fuzzy: Option<bool>,
    offset: Option<u32>,
) -> Result<FindFilesResponse, String> {
    find_files_in_memory(&state, query, extensions, category, limit, use_fuzzy, offset)
}

fn find_files_in_memory(
    state: &tauri::State<AppState>,
    query: String,
    extensions: Option<String>,
    category: Option<String>,
    limit: Option<u32>,
    use_fuzzy: Option<bool>,
    offset: Option<u32>,
) -> Result<FindFilesResponse, String> {
    const DEFAULT_LIMIT: u32 = 500;
    let limit = limit.unwrap_or(DEFAULT_LIMIT) as usize;
    let use_fuzzy = use_fuzzy.unwrap_or(true);
    let start_index: usize = offset.unwrap_or(0) as usize;
    let total_start = Instant::now();

    write_debug_log(
        state,
        &format!(
            ">>> find_files start query_len={} limit={}",
            query.len(),
            limit
        ),
    );

    let disk_entries_arc = {
        let guard = state.disk_objects.lock().unwrap();
        if let Some(entries) = guard.as_ref() {
            let t = total_start.elapsed().as_millis();
            write_debug_log(
                state,
                &format!(
                    "find_files index from_cache count={} ms={}",
                    entries.len(),
                    t
                ),
            );
            entries.clone()
        } else {
            write_debug_log(
                state,
                "find_files no_active_index (returning empty results)",
            );
            return Ok(FindFilesResponse {
                items: Vec::new(),
                next_offset: None,
            });
        }
    };
    let disk_entries: &[DiskObject] = disk_entries_arc.as_ref();

    let ext_filter_start = Instant::now();
    let extension_set: Option<std::collections::HashSet<String>> = extensions.as_ref().and_then(|s| {
        let cleaned: Vec<String> = s
            .split(',')
            .map(|x| x.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|x| !x.is_empty())
            .collect();
        if cleaned.is_empty() {
            None
        } else {
            Some(cleaned.into_iter().collect())
        }
    });

    let (use_mask, allowed_indices, allowed_count) = if let Some(ref set) = extension_set {
        let mut mask: Vec<bool> = Vec::with_capacity(disk_entries.len());
        let mut count = 0usize;
        for o in disk_entries {
            let allowed = match o.kind {
                DiskObjectKind::File => o
                    .ext
                    .as_ref()
                    .map(|t| set.contains(t))
                    .unwrap_or(false),
                DiskObjectKind::Folder => true,
            };
            if allowed {
                count += 1;
            }
            mask.push(allowed);
        }
        (true, mask, count)
    } else {
        let category_ref = category.as_deref();
        if matches!(category_ref, Some(c) if !c.trim().is_empty() && c.trim() != "all") {
            let mut mask: Vec<bool> = Vec::with_capacity(disk_entries.len());
            let mut count = 0usize;
            for o in disk_entries {
                let allowed = category_filter::category_allowed(category_ref.map(|s| s.trim()), o);
                if allowed {
                    count += 1;
                }
                mask.push(allowed);
            }
            (true, mask, count)
        } else {
            (false, Vec::new(), disk_entries.len())
        }
    };

    let ext_filter_ms = ext_filter_start.elapsed().as_millis();
    write_debug_log(
        state,
        &format!(
            "find_files after_filter allowed_count={} total_entries={} use_extensions={} category={:?} ext_filter_ms={}",
            allowed_count, disk_entries.len(), extension_set.is_some(), category.as_deref(), ext_filter_ms
        ),
    );

    if start_index >= disk_entries.len() {
        let total_ms = total_start.elapsed().as_millis();
        write_debug_log(
            state,
            &format!(
                "find_files empty_page start_index={} total_entries={} total_ms={}",
                start_index, disk_entries.len(), total_ms
            ),
        );
        return Ok(FindFilesResponse {
            items: Vec::new(),
            next_offset: None,
        });
    }

    if query.trim().is_empty() {
        let collect_start = Instant::now();
        let (items, next_offset) =
            paginate_scan(disk_entries, start_index, limit, use_mask, &allowed_indices, |_i, _e| {
                true
            });
        let collect_ms = collect_start.elapsed().as_millis();
        let total_ms = total_start.elapsed().as_millis();
        write_debug_log(
            state,
            &format!(
                "find_files done empty_query count={} start_index={} next_offset={:?} collect_take_ms={} total_ms={}",
                items.len(), start_index, next_offset, collect_ms, total_ms
            ),
        );
        return Ok(FindFilesResponse {
            items,
            next_offset,
        });
    }

    let q_trimmed = query.trim();
    let q = q_trimmed.to_lowercase();
    let q_len = q_trimmed.chars().count();

    let suffix_search_start = Instant::now();
    let candidate_set_opt = {
        let guard = state.name_reverse_index.lock().unwrap();
        guard
            .as_ref()
            .and_then(|idx| search_suffix_index(idx.as_ref(), &q))
    };
    let suffix_search_ms = suffix_search_start.elapsed().as_millis();
    write_debug_log(state, &format!(
        "find_files suffix_search q_len={} candidates={} ms={}",
        q_len,
        candidate_set_opt.as_ref().map(|s| s.len()).unwrap_or(0),
        suffix_search_ms,
    ));

    if !use_fuzzy || q_len < 3 {
        let build_start = Instant::now();
        let (items, next_offset) = paginate_scan(
            disk_entries,
            start_index,
            limit,
            use_mask,
            &allowed_indices,
            |i, e| {
                if let Some(cs) = &candidate_set_opt {
                    if !cs.contains(&i) {
                        return false;
                    }
                }
                e.name.to_ascii_lowercase().contains(&q)
            },
        );
        let build_ms = build_start.elapsed().as_millis();
        let total_ms = total_start.elapsed().as_millis();
        let label = if !use_fuzzy {
            "substring_no_fuzzy_done"
        } else {
            "short_query_fuzzy"
        };
        write_debug_log(
            state,
            &format!(
                "find_files {} q_len={} count={} start_index={} next_offset={:?} results_build_ms={} total_ms={}",
                label, q_len, items.len(), start_index, next_offset, build_ms, total_ms,
            ),
        );
        return Ok(FindFilesResponse {
            items,
            next_offset,
        });
    } else {
        let nucleo_start = Instant::now();
        let mut filtered: Vec<(usize, &DiskObject)> = Vec::new();
        for (i, e) in disk_entries.iter().enumerate() {
            if use_mask && !allowed_indices[i] {
                continue;
            }
            if let Some(cs) = &candidate_set_opt {
                if !cs.contains(&i) {
                    continue;
                }
            }
            filtered.push((i, e));
        }
        let labels: Vec<&str> = filtered.iter().map(|(_, e)| e.name.as_str()).collect();
        let mut matcher = Matcher::new(Config::DEFAULT);
        let pattern =
            Pattern::parse(&query, CaseMatching::Smart, Normalization::Smart);
        let mut scored = pattern.match_list(labels.iter().copied(), &mut matcher);
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        let scored_len = scored.len();
        let nucleo_ms = nucleo_start.elapsed().as_millis();
        let select_start = Instant::now();

        let label_to_filtered: HashMap<*const str, usize> = filtered
            .iter()
            .enumerate()
            .map(|(fi, (_, e))| (e.name.as_str() as *const str, fi))
            .collect();

        let mut items: Vec<SearchEntry> = Vec::new();
        let mut seen_filtered_indices: HashSet<usize> = HashSet::new();
        for (label, _score) in scored.into_iter().take(limit) {
            let label_ptr = label as *const str;
            if let Some(&fi) = label_to_filtered.get(&label_ptr) {
                if seen_filtered_indices.insert(fi) {
                    items.push(search_entry_from_disk_object(filtered[fi].1));
                }
            }
        }

        let select_ms = select_start.elapsed().as_millis();
        let total_ms = total_start.elapsed().as_millis();
        write_debug_log(
            state,
            &format!(
                "find_files nucleo_done q_len={} scored={} taken={} nucleo_ms={} select_build_ms={} total_ms={}",
                q_len, scored_len, items.len(), nucleo_ms, select_ms, total_ms,
            ),
        );
        return Ok(FindFilesResponse {
            items,
            next_offset: None,
        });
    }
}

fn find_files_in_db(
    state: &tauri::State<AppState>,
    query: String,
    extensions: Option<String>,
    limit: Option<u32>,
    _use_fuzzy: Option<bool>,
    _offset: Option<u32>,
) -> Result<FindFilesResponse, String> {
    const DEFAULT_LIMIT: u32 = 500;
    let limit = limit.unwrap_or(DEFAULT_LIMIT) as usize;

    let total_start = Instant::now();

    write_debug_log(
        state,
        &format!(
            ">>> find_files_in_db start query_len={} limit={}",
            query.len(),
            limit
        ),
    );

    let db_path = state.db_path.clone();
    let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;

    let disk_entries = search_disk_objects_by_name(&conn, &query, limit)
        .map_err(|e| e.to_string())?;

    let ext_filter_start = Instant::now();
    let extension_set: Option<std::collections::HashSet<String>> = extensions.as_ref().and_then(|s| {
        let cleaned: Vec<String> = s
            .split(',')
            .map(|x| x.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|x| !x.is_empty())
            .collect();
        if cleaned.is_empty() {
            None
        } else {
            Some(cleaned.into_iter().collect())
        }
    });

    let filtered_entries: Vec<DiskObject> = match &extension_set {
        None => disk_entries,
        Some(set) => disk_entries
            .into_iter()
            .filter(|o| {
                match o.kind {
                    DiskObjectKind::File => o
                        .ext
                        .as_ref()
                        .map(|t| set.contains(t))
                        .unwrap_or(false),
                    DiskObjectKind::Folder => true,
                }
            })
            .collect(),
    };

    let ext_filter_ms = ext_filter_start.elapsed().as_millis();
    write_debug_log(
        state,
        &format!(
            "find_files_in_db after_ext_filter count={} ext_filter_ms={}",
            filtered_entries.len(),
            ext_filter_ms
        ),
    );

    let items: Vec<SearchEntry> = filtered_entries
        .iter()
        .take(limit)
        .map(|o| search_entry_from_disk_object(o))
        .collect();

    let total_ms = total_start.elapsed().as_millis();
    write_debug_log(
        state,
        &format!(
            "find_files_in_db done count={} total_ms={}",
            items.len(),
            total_ms
        ),
    );

    Ok(FindFilesResponse {
        items,
        next_offset: None,
    })
}

fn load_dotenv_from_repo() {
    use std::path::PathBuf;

    // 1. Try the workspace root (parent of the src-tauri crate)
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(parent) = manifest_dir.parent() {
        candidates.push(parent.join(".env"));
    }

    // 2. Try the src-tauri crate directory itself
    candidates.push(manifest_dir.join(".env"));

    // 3. Try the current working directory (useful in dev)
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(".env"));
    }

    for env_path in candidates {
        if env_path.is_file() {
            let _ = dotenvy::from_path(&env_path);
            break;
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    load_dotenv_from_repo();

    // TODOdin: Remember that we set this in .env!
    let scan_path_override = std::env::var("CUTE_DISK_TREE_SCAN_PATH").ok();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(move |app| {
            let path = app.path().app_data_dir().map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            let db_path = path.join("index.db");
            db::open_db(&db_path).map_err(|e| e.to_string())?;

            let env_log_path = std::env::var("CUTE_DISK_TREE_DEBUG_LOG_PATH").ok();
            let debug_log_path = env_log_path
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| path.join("debug.log"));

            let _ = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&debug_log_path)
                .and_then(|mut f| {
                    use std::io::Write;
                    f.write_all(b"=== starting cutest disk tree ===\n\n")?;
                    f.flush()
                });

            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&debug_log_path)
                .and_then(|mut f| {
                    use std::io::Write;
                    writeln!(f, "Loaded config from environment:")?;
                    if let Ok(path) = std::env::var("CUTE_DISK_TREE_DEBUG_LOG_PATH") {
                        writeln!(f, "CUTE_DISK_TREE_DEBUG_LOG_PATH={}", path)?;
                    }
                    if let Ok(path) = std::env::var("CUTE_DISK_TREE_SCAN_PATH") {
                        writeln!(f, "CUTE_DISK_TREE_SCAN_PATH={}", path)?;
                    }
                    writeln!(f)?;
                    f.flush()
                });

            let force_fresh = scan_path_override.is_some();

            let mut initial_disk_objects: Option<Arc<Vec<DiskObject>>> = None;
            let mut initial_name_index: Option<Arc<SuffixIndex>> = None;

            if !force_fresh {
                let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;
                if db::has_disk_objects(&conn).unwrap_or(false) {
                    let mut objs = db::get_disk_objects(&conn).unwrap_or_default();
                    objs.sort_by(|a, b| a.path.cmp(&b.path));

                    let t_suffix = Instant::now();
                    let meta = db::read_scan_metadata(&conn).ok().flatten();
                    let index_is_current = meta.as_ref().map_or(false, |m| {
                        m.disk_objects_update_id != 0
                            && m.suffix_index_update_id == m.disk_objects_update_id
                    });

                    let (name_index, index_source) = if index_is_current {
                        match db::read_suffix_index_data(&conn) {
                            Ok(Some((buffer, offsets, disk_object_indices))) => {
                                let st = SuffixTable::new(buffer.clone());
                                let idx = SuffixIndex { st, offsets, disk_object_indices, buffer };
                                (idx, "db")
                            }
                            _ => {
                                let (idx, ..) = build_suffix_index(&objs);
                                (idx, "rebuild-fallback")
                            }
                        }
                    } else {
                        let (idx, ..) = build_suffix_index(&objs);
                        if let Some(m) = &meta {
                            if m.disk_objects_update_id != 0 {
                                let _ = db::write_suffix_index_data(
                                    &conn, m.disk_objects_update_id,
                                    &idx.buffer, &idx.offsets, &idx.disk_object_indices,
                                );
                            }
                        }
                        (idx, "rebuild")
                    };

                    let _ = writeln!(
                        std::io::stderr(),
                        "startup suffix_index objects={} source={} total_ms={}",
                        objs.len(), index_source, t_suffix.elapsed().as_millis(),
                    );
                    initial_disk_objects = Some(Arc::new(objs));
                    initial_name_index = Some(Arc::new(name_index));
                }
            }

            app.manage(AppState {
                db_path,
                debug_log: Mutex::new(Some(debug_log_path)),
                disk_objects: Mutex::new(initial_disk_objects),
                name_reverse_index: Mutex::new(initial_name_index),
                phase2_cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
                is_scanning: AtomicBool::new(false),
                scan_path_override: scan_path_override.clone(),
            });

            if force_fresh {
                let state_ref: tauri::State<AppState> = app.state();
                write_debug_log(&state_ref, "setup: force_fresh=true, spawning auto-scan");
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state: tauri::State<AppState> = handle.state();
                    write_debug_log(&state, "setup: auto-scan task starting");
                    let _ = scan_directory(handle.clone(), state).await;
                });
            } else {
                let state_ref: tauri::State<AppState> = app.state();
                write_debug_log(&state_ref, &format!(
                    "setup: force_fresh=false scan_path_override={:?}",
                    scan_path_override
                ));
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_directory,
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
