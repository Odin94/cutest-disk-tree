use cutest_disk_tree::{db, DiskObject, DiskObjectKind};
use cutest_disk_tree::core::indexing::compressed_text_index::{
    build_index as cti_build_index, find_files as cti_find_files,
    compressed_text_index_exists, write_scan_metadata, read_scan_metadata,
    read_scan_result_from_compressed_text_index,
};
use cutest_disk_tree::core::indexing::ngram::{
    build_index as trigram_build_index, find_files as trigram_find_files, TrigramIndex,
};
use cutest_disk_tree::core::file_updating::{IndexWatcher, IndexReconciler};
use cutest_disk_tree::core::indexing::suffix::{
    SuffixIndex, build_index as suffix_build_index, find_files as suffix_find_files,
};
use cutest_disk_tree::core::indexing::sqlite::{find_files as sqlite_find_files, SearchFilter};
use cutest_disk_tree::core::search_category;
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
                    return false;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchIndexMode {
    Sqlite,
    InMemorySuffix,
    InMemoryNgrams,
    CompressedText,
}

fn parse_index_mode() -> SearchIndexMode {
    match std::env::var("CUTE_DISK_TREE_INDEX_MODE").as_deref() {
        Ok("sqlite") => SearchIndexMode::Sqlite,
        Ok("in-memory-suffix") => SearchIndexMode::InMemorySuffix,
        Ok("in-memory-ngrams") => SearchIndexMode::InMemoryNgrams,
        Ok("compressed-text") => SearchIndexMode::CompressedText,
        _ => SearchIndexMode::InMemoryNgrams,
    }
}

fn uses_in_memory_index(mode: SearchIndexMode) -> bool {
    matches!(mode, SearchIndexMode::InMemorySuffix | SearchIndexMode::InMemoryNgrams)
}

struct AppState {
    db_path: std::path::PathBuf,
    debug_log: Mutex<Option<std::path::PathBuf>>,
    disk_objects: Mutex<Option<Arc<Vec<DiskObject>>>>,
    name_reverse_index: Mutex<Option<Arc<SuffixIndex>>>,
    trigram_index: Arc<Mutex<TrigramIndex>>,
    phase2_cancel: Mutex<Arc<AtomicBool>>,
    is_scanning: Arc<AtomicBool>,
    scan_path_override: Option<String>,
    index_mode: SearchIndexMode,
    _watcher: Mutex<Option<IndexWatcher>>,
    _reconciler: Mutex<Option<IndexReconciler>>,
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

fn start_file_watchers(state: &AppState, roots: Vec<std::path::PathBuf>) {
    let index = Arc::clone(&state.trigram_index);
    let scan_flag = Arc::clone(&state.is_scanning);
    match IndexWatcher::new(Arc::clone(&index), roots.clone()) {
        Ok(w) => { *state._watcher.lock().unwrap() = Some(w); }
        Err(e) => { write_debug_log(state, &format!("start_file_watchers: watcher error: {:?}", e)); }
    }
    *state._reconciler.lock().unwrap() = Some(IndexReconciler::new(index, roots, scan_flag));
    write_debug_log(state, "start_file_watchers: watcher and reconciler started");
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
    mode: SearchIndexMode,
) {
    let t0 = Instant::now();

    write_debug_log(state, &format!(
        "activate_initial_index starting mode={:?} files={} folders={}",
        mode, files.len(), folder_paths.len(),
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

    match mode {
        SearchIndexMode::InMemoryNgrams => {
            let _ = app.emit("scan-phase-status", "building trigram index...".to_string());
            write_debug_log(state, "activate_initial_index building trigram index");
            let idx_start = Instant::now();
            let index = trigram_build_index(&objs);
            let idx_ms = idx_start.elapsed().as_millis();
            write_debug_log(state, &format!(
                "activate_initial_index trigram_build done idx_ms={} total_ms={}",
                idx_ms, t0.elapsed().as_millis(),
            ));

            if cancel.load(Ordering::Relaxed) {
                write_debug_log(state, "activate_initial_index cancelled after trigram build");
                return;
            }

            {
                let mut disk_guard = state.disk_objects.lock().unwrap();
                *disk_guard = Some(Arc::new(objs));
                *state.trigram_index.lock().unwrap() = index;
            }
        }
        _ => {
            // InMemorySuffix
            let _ = app.emit("scan-phase-status", "building suffix index...".to_string());
            write_debug_log(state, "activate_initial_index building suffix index");
            let idx_start = Instant::now();
            let index = suffix_build_index(&objs);
            let idx_ms = idx_start.elapsed().as_millis();
            write_debug_log(state, &format!(
                "activate_initial_index suffix_build done idx_ms={} total_ms={}",
                idx_ms, t0.elapsed().as_millis(),
            ));

            if cancel.load(Ordering::Relaxed) {
                write_debug_log(state, &format!(
                    "activate_initial_index cancelled after suffix build ms={}", t0.elapsed().as_millis(),
                ));
                return;
            }

            {
                let mut disk_guard = state.disk_objects.lock().unwrap();
                *disk_guard = Some(Arc::new(objs));
                let mut idx_guard = state.name_reverse_index.lock().unwrap();
                *idx_guard = Some(Arc::new(index));
            }
        }
    }

    write_debug_log(state, &format!(
        "activate_initial_index done mode={:?} files={} folders={} build_disk_objs_ms={} total_ms={}",
        mode, files.len(), folder_paths.len(), build_ms, t0.elapsed().as_millis(),
    ));

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
    mode: SearchIndexMode,
) {
    let state_ptr: tauri::State<AppState> = app_bg.state();
    let total_start = Instant::now();

    write_debug_log(&state_ptr, &format!(
        "phase2 starting mode={:?} files={} folders={} roots={:?}",
        mode, files_bg.len(), folder_paths.len(), scan_roots,
    ));

    if uses_in_memory_index(mode) {
        activate_initial_index(&app_bg, &state_ptr, &files_bg, &folder_paths, &cancel, mode);
        if cancel.load(Ordering::Relaxed) {
            write_debug_log(&state_ptr, "phase2 cancelled after index build");
            let _ = app_bg.emit("scan-phase-status", "".to_string());
            return;
        }
    } else {
        let _ = app_bg.emit("scan-index-ready", true);
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

    if uses_in_memory_index(mode) {
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
                "phase2 apply_folder_sizes done objects={} ms={} total_ms={}",
                new_objs.len(), apply_ms, total_start.elapsed().as_millis(),
            ));
            let new_objs_arc = Arc::new(new_objs);
            {
                let mut disk_guard = state_ptr.disk_objects.lock().unwrap();
                *disk_guard = Some(Arc::clone(&new_objs_arc));
            }

            // For ngrams, rebuild the trigram index so search results carry folder sizes.
            if mode == SearchIndexMode::InMemoryNgrams {
                let rebuild_start = Instant::now();
                write_debug_log(&state_ptr, &format!(
                    "phase2 rebuilding trigram index after folder_sizes objects={}",
                    new_objs_arc.len(),
                ));
                let new_index = trigram_build_index(&new_objs_arc);
                let rebuild_ms = rebuild_start.elapsed().as_millis();
                write_debug_log(&state_ptr, &format!(
                    "phase2 trigram_rebuild done ms={} total_ms={}",
                    rebuild_ms, total_start.elapsed().as_millis(),
                ));
                *state_ptr.trigram_index.lock().unwrap() = new_index;
            }
        }
    }

    if mode == SearchIndexMode::InMemoryNgrams && !cancel.load(Ordering::Relaxed) {
        start_file_watchers(&state_ptr, scan_roots.clone());
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

    let roots_str: Vec<String> = scan_roots.iter().map(|r| r.to_string_lossy().to_string()).collect();

    if mode == SearchIndexMode::CompressedText {
        let app_data_dir = db_path_bg.parent().unwrap_or(&db_path_bg);
        let cti_path = app_data_dir.join("index.compressed-text-index.lz4");
        let metadata_path = app_data_dir.join("scan-metadata.json");
        let cti_start = Instant::now();
        write_debug_log(&state_ptr, &format!(
            "phase2 writing compressed text index files={} folders={}",
            files_bg.len(),
            folder_sizes.len(),
        ));
        let _ = app_bg.emit("scan-phase-status", "writing search index...".to_string());
        match cti_build_index(&cti_path, &files_bg, &folder_sizes) {
            Ok(()) => {
                let ms = cti_start.elapsed().as_millis();
                write_debug_log(
                    &state_ptr,
                    &format!(
                        "phase2 cti_write done path={} ms={} files={} folders={}",
                        cti_path.display(),
                        ms,
                        files_bg.len(),
                        folder_sizes.len(),
                    ),
                );
            }
            Err(e) => {
                let ms = cti_start.elapsed().as_millis();
                write_debug_log(
                    &state_ptr,
                    &format!(
                        "phase2 cti_write failed error={:?} ms={} files={} folders={}",
                        e,
                        ms,
                        files_bg.len(),
                        folder_sizes.len(),
                    ),
                );
            }
        }
        let meta_start = Instant::now();
        if let Err(e) = write_scan_metadata(&metadata_path, &roots_str, files_bg.len() as u64, &folder_sizes) {
            let ms = meta_start.elapsed().as_millis();
            write_debug_log(
                &state_ptr,
                &format!(
                    "phase2 scan_metadata write failed error={:?} ms={}",
                    e,
                    ms,
                ),
            );
        } else {
            let ms = meta_start.elapsed().as_millis();
            write_debug_log(
                &state_ptr,
                &format!(
                    "phase2 scan_metadata write done path={} ms={}",
                    metadata_path.display(),
                    ms,
                ),
            );
        }
    } else if mode == SearchIndexMode::Sqlite {
        write_debug_log(&state_ptr, "phase2 opening database");
        let _ = app_bg.emit("scan-phase-status", "saving to database...".to_string());
        let update_id = chrono::Utc::now().timestamp_millis();
        match db::open_db(&db_path_bg) {
            Ok(conn) => {
                if let Err(e) = db::write_scan(&conn, &files_bg, &folder_sizes, update_id) {
                    write_debug_log(&state_ptr, &format!("phase2 db_write failed error={:?}", e));
                } else {
                    write_debug_log(&state_ptr, "phase2 db_write done");
                }
            }
            Err(e) => write_debug_log(&state_ptr, &format!("phase2 db_open failed error={:?}", e)),
        }
    } else {
        write_debug_log(&state_ptr, "phase2 opening database");
        let _ = app_bg.emit("scan-phase-status", "saving to database...".to_string());
        let update_id = chrono::Utc::now().timestamp_millis();
        let db_start = Instant::now();
        match db::open_db(&db_path_bg) {
            Ok(conn) => {
                if let Err(e) = db::write_scan(&conn, &files_bg, &folder_sizes, update_id) {
                    write_debug_log(&state_ptr, &format!(
                        "phase2 db_write failed error={:?} ms={}", e, db_start.elapsed().as_millis()
                    ));
                } else if mode == SearchIndexMode::InMemorySuffix {
                    // Persist suffix index so startup can reload it without rebuilding.
                    let index_arc = { state_ptr.name_reverse_index.lock().unwrap().clone() };
                    if let Some(arc) = index_arc {
                        let _ = db::write_suffix_index_data(
                            &conn, update_id,
                            &arc.buffer, &arc.offsets, &arc.disk_object_indices,
                        );
                    }
                    write_debug_log(&state_ptr, &format!(
                        "phase2 db_write and suffix_index done ms={}", db_start.elapsed().as_millis()
                    ));
                } else {
                    // InMemoryNgrams: disk_objects persisted via write_scan; no suffix index needed.
                    write_debug_log(&state_ptr, &format!(
                        "phase2 db_write done (ngrams, no suffix_index) ms={}", db_start.elapsed().as_millis()
                    ));
                }
            }
            Err(e) => write_debug_log(&state_ptr, &format!("phase2 db_open failed error={:?}", e)),
        }
    }

    let _ = app_bg.emit("scan-phase-status", "".to_string());
}

/// Mirror of the types written by `mft-helper` — used only for JSON deserialisation.
#[derive(serde::Deserialize)]
struct MftFileEntry {
    path: String,
    size: u64,
    dev: u64,
    ino: u64,
    mtime: Option<i64>,
}

#[derive(serde::Deserialize)]
struct MftScanOutput {
    files: Vec<MftFileEntry>,
    folders: Vec<String>,
}

#[tauri::command]
fn get_scan_status(state: tauri::State<'_, AppState>) -> bool {
    state.is_scanning.load(Ordering::SeqCst)
}

/// Run a fast MFT scan by launching the `mft-helper` binary with UAC elevation
/// via PowerShell `Start-Process -Verb RunAs -Wait`. The helper writes JSON to
/// a temp file; we read it back and proceed exactly like `scan_directory`.
#[tauri::command]
async fn scan_directory_with_helper(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<ScanDirectoryResponse, String> {
    write_debug_log(&state, "scan_directory_with_helper called");

    if state.is_scanning.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        write_debug_log(&state, "scan_directory_with_helper rejected: scan already in progress");
        return Err("A scan is already in progress".to_string());
    }

    let scan_roots: Vec<std::path::PathBuf> = match &state.scan_path_override {
        Some(p) => vec![std::path::PathBuf::from(p)],
        None => cutest_disk_tree::get_filesystem_roots(),
    };

    write_debug_log(&state, &format!("scan_directory_with_helper roots={:?}", scan_roots));

    let db_path = state.db_path.clone();
    let scan_log_path = resolve_debug_log_path(&state);
    let app_for_scan = app.clone();
    let scan_start = Instant::now();

    // Find the helper binary: it lives next to the current executable.
    let helper_exe = std::env::current_exe()
        .map_err(|e| format!("cannot find current exe: {}", e))?
        .parent()
        .ok_or("cannot get exe directory")?
        .join("mft-helper.exe");

    if !helper_exe.exists() {
        state.is_scanning.store(false, Ordering::SeqCst);
        return Err(format!(
            "mft-helper.exe not found at {}",
            helper_exe.display()
        ));
    }

    // Collect MFT output from every scan root.
    let mut all_mft_files: Vec<MftFileEntry> = Vec::new();
    let mut all_folder_strings: Vec<String> = Vec::new();
    let mut roots_str: Vec<String> = Vec::new();

    for root in &scan_roots {
        roots_str.push(root.to_string_lossy().into_owned());

        // Create a temp file for the helper to write its JSON output into.
        let tmp_path = std::env::temp_dir().join(format!(
            "mft-scan-{}.json",
            root.to_string_lossy().replace([':', '\\', '/'], "_")
        ));

        let helper_path_str = helper_exe.to_string_lossy().into_owned();
        let root_str = root.to_string_lossy().into_owned();
        let tmp_str = tmp_path.to_string_lossy().into_owned();

        // Run: powershell -WindowStyle Hidden -Command
        //   "Start-Process -FilePath '<helper>' -ArgumentList '<root>','<tmp>' -Verb RunAs -Wait"
        let ps_command = format!(
            "Start-Process -FilePath '{}' -ArgumentList '{}','{}' -Verb RunAs -Wait -WindowStyle Hidden",
            helper_path_str.replace('\'', "''"),
            root_str.replace('\'', "''"),
            tmp_str.replace('\'', "''"),
        );

        write_debug_log(&state, &format!(
            "scan_directory_with_helper launching helper for root={}", root_str
        ));

        let _ = app_for_scan.emit("scan-progress", &cutest_disk_tree::ScanProgress {
            files_count: 0,
            current_path: None,
            status: Some(format!("Waiting for MFT scan of {} (UAC prompt)…", root_str)),
        });

        let status = tauri::async_runtime::spawn_blocking({
            let ps_command = ps_command.clone();
            move || {
                std::process::Command::new("powershell")
                    .args([
                        "-NoProfile",
                        "-NonInteractive",
                        "-WindowStyle", "Hidden",
                        "-Command",
                        &ps_command,
                    ])
                    .status()
            }
        })
        .await
        .map_err(|e| format!("spawn_blocking failed: {}", e))?
        .map_err(|e| format!("powershell failed to launch: {}", e))?;

        if !status.success() {
            write_debug_log(&state, &format!(
                "scan_directory_with_helper: helper exited with code {:?} for root={}",
                status.code(), root_str
            ));
            state.is_scanning.store(false, Ordering::SeqCst);
            return Err(format!(
                "mft-helper failed for {} (exit code {:?}). User may have cancelled the UAC prompt.",
                root_str, status.code()
            ));
        }

        // Read the JSON output the helper wrote.
        let json_bytes = std::fs::read(&tmp_path)
            .map_err(|e| format!("failed to read helper output for {}: {}", root_str, e))?;
        let _ = std::fs::remove_file(&tmp_path);

        let output: MftScanOutput = serde_json::from_slice(&json_bytes)
            .map_err(|e| format!("failed to parse helper JSON for {}: {}", root_str, e))?;

        cutest_disk_tree::logging::debug_log::write_debug_log(&scan_log_path, &format!(
            "scan_method: MFT scan via helper for {} files={} folders={}",
            root_str, output.files.len(), output.folders.len()
        ));

        all_folder_strings.extend(output.folders);
        all_mft_files.extend(output.files);
    }

    let _ = app_for_scan.emit("scan-progress", &cutest_disk_tree::ScanProgress {
        files_count: all_mft_files.len() as u64,
        current_path: None,
        status: Some("Building index…".into()),
    });

    // Convert MftFileEntry → FileEntry for phase2.
    let files_arc: Arc<Vec<cutest_disk_tree::FileEntry>> = Arc::new(
        all_mft_files
            .into_iter()
            .map(|f| cutest_disk_tree::FileEntry {
                path: std::path::PathBuf::from(&f.path),
                size: f.size,
                file_key: cutest_disk_tree::FileKey { dev: f.dev, ino: f.ino },
                mtime: f.mtime,
            })
            .collect(),
    );

    let all_folder_paths: std::collections::HashSet<std::path::PathBuf> = all_folder_strings
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect();

    write_debug_log(&state, &format!(
        "scan_directory_with_helper phase1_done files={} folders={} ms={}",
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
        *state.trigram_index.lock().unwrap() = trigram_build_index(&[]);
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
    let mode = state.index_mode;
    tauri::async_runtime::spawn_blocking(move || {
        run_phase2(app_bg, db_path, roots_bg, files_bg, folder_paths_bg, cancel_token, mode);
    });

    state.is_scanning.store(false, Ordering::SeqCst);
    let _ = app.emit("scan-complete", &response);
    write_debug_log(&state, "scan_directory_with_helper done");
    Ok(response)
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
    let scan_log_path = resolve_debug_log_path(&state);
    let (files_arc, all_folder_paths, roots_str) = match tauri::async_runtime::spawn_blocking(move || {
        let (files_arc, all_folders, roots_str) =
            cutest_disk_tree::core::scanning::ignore_scanner::scan_roots_with_ignore(
                &roots_for_scan,
                move |p| {
                    // Write scan-method status messages to the debug log so it's
                    // always clear whether we used the MFT or the directory walk.
                    if let Some(ref status) = p.status {
                        if status.contains("MFT") || status.contains("falling back") || status.contains("directory walk") {
                            cutest_disk_tree::logging::debug_log::write_debug_log(
                                &scan_log_path,
                                &format!("scan_method: {}", status),
                            );
                        }
                    }
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
        *state.trigram_index.lock().unwrap() = trigram_build_index(&[]);
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
    let mode = state.index_mode;
    tauri::async_runtime::spawn_blocking(move || {
        run_phase2(app_bg, db_path, roots_bg, files_bg, folder_paths_bg, cancel_token, mode);
    });

    state.is_scanning.store(false, Ordering::SeqCst);
    let _ = app.emit("scan-complete", &response);
    write_debug_log(&state, "scan_directory done");
    Ok(response)
}

#[tauri::command]
async fn load_cached_scan(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<cutest_disk_tree::ScanSummary>, String> {
    let t0 = Instant::now();
    write_debug_log(&state, &format!("load_cached_scan started mode={:?}", state.index_mode));
    let db_path = state.db_path.clone();
    let mode = state.index_mode;

    let summary = match mode {
        SearchIndexMode::CompressedText => {
            let metadata_path = db_path.parent()
                .map(|p| p.join("scan-metadata.json"))
                .unwrap_or_else(|| std::path::PathBuf::from("scan-metadata.json"));
            match tauri::async_runtime::spawn_blocking(move || {
                read_scan_metadata(&metadata_path).map_err(|e| e.to_string())
            }).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(e.to_string()),
            }
        }
        SearchIndexMode::Sqlite | SearchIndexMode::InMemorySuffix | SearchIndexMode::InMemoryNgrams => {
            match tauri::async_runtime::spawn_blocking({
                let db_path = db_path.clone();
                move || {
                    let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;
                    db::get_scan_summary(&conn).map_err(|e| e.to_string())
                }
            }).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(e.to_string()),
            }
        }
    };

    let ms = t0.elapsed().as_millis();
    write_debug_log(&state, &format!("load_cached_scan done has_result={} ms={}", summary.is_some(), ms));

    if summary.is_some() && uses_in_memory_index(mode) {
        let app_bg = app.clone();
        let cancel = { state.phase2_cancel.lock().unwrap().clone() };
        write_debug_log(&state, "load_cached_scan: spawning background index build from DB");
        tauri::async_runtime::spawn_blocking(move || {
            let state_ptr: tauri::State<AppState> = app_bg.state();
            let _ = app_bg.emit("scan-phase-status", "loading files from database...".to_string());
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
                Ok(None) | Err(_) => {
                    let _ = app_bg.emit("scan-phase-status", "".to_string());
                    return;
                }
            };
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
            activate_initial_index(&app_bg, &state_ptr, &files, &folder_paths, &cancel, state_ptr.index_mode);
            let _ = app_bg.emit("scan-phase-status", "".to_string());
        });
    }

    Ok(summary)
}

#[tauri::command]
async fn list_cached_tree_depths(
    state: tauri::State<'_, AppState>,
    max_children: u32,
) -> Result<Vec<u32>, String> {
    if matches!(state.index_mode, SearchIndexMode::CompressedText) {
        return Ok(Vec::new());
    }
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
    write_debug_log(&state, &format!("build_disk_tree_cached started mode={:?} start_path={} max_depth={}", state.index_mode, start_path, max_depth));
    let db_path = state.db_path.clone();
    let path_clone = start_path.clone();
    let mode = state.index_mode;

    if mode == SearchIndexMode::CompressedText {
        let cti_path = state.db_path.parent()
            .map(|p| p.join("index.compressed-text-index.lz4"))
            .unwrap_or_else(|| std::path::PathBuf::from("index.compressed-text-index.lz4"));
        let path_for_cti = path_clone.clone();
        return tauri::async_runtime::spawn_blocking(move || {
            if !compressed_text_index_exists(&cti_path) {
                return Ok(None);
            }
            let scan = read_scan_result_from_compressed_text_index(&cti_path)
                .map_err(|e| format!("{:?}", e))?;
            let tree = cutest_disk_tree::build_disk_tree(
                &scan,
                &path_for_cti,
                max_children_per_node as usize,
                max_depth as usize,
            );
            Ok(tree)
        }).await.map_err(|e| e.to_string())?;
    }

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

fn resolve_compressed_text_index_path(state: &AppState) -> std::path::PathBuf {
    state.db_path.parent()
        .map(|p| p.join("index.compressed-text-index.lz4"))
        .unwrap_or_else(|| std::path::PathBuf::from("index.compressed-text-index.lz4"))
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
    match state.index_mode {
        SearchIndexMode::CompressedText => find_files_in_compressed_text_index(&state, query, extensions, category, limit, use_fuzzy, offset),
        SearchIndexMode::Sqlite => find_files_in_db(&state, query, extensions, category, limit, use_fuzzy, offset),
        SearchIndexMode::InMemoryNgrams => {
            let has_index = !state.trigram_index.lock().unwrap().objects.is_empty();
            if has_index {
                find_files_in_ngram_index(&state, query, extensions, category, limit, use_fuzzy, offset)
            } else {
                find_files_in_db(&state, query, extensions, category, limit, use_fuzzy, offset)
            }
        }
        SearchIndexMode::InMemorySuffix => {
            let has_memory_index = state.disk_objects.lock().unwrap().is_some();
            if has_memory_index {
                find_files_in_memory(&state, query, extensions, category, limit, use_fuzzy, offset)
            } else {
                find_files_in_db(&state, query, extensions, category, limit, use_fuzzy, offset)
            }
        }
    }
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
                DiskObjectKind::Folder => false,
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
            .and_then(|idx| suffix_find_files(idx.as_ref(), &q))
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

fn nucleo_rank_objects(query: &str, candidates: Vec<&DiskObject>, limit: usize) -> Vec<SearchEntry> {
    let labels: Vec<&str> = candidates.iter().map(|o| o.name.as_str()).collect();
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut scored = pattern.match_list(labels.iter().copied(), &mut matcher);
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    let label_to_idx: HashMap<*const str, usize> = candidates
        .iter()
        .enumerate()
        .map(|(i, o)| (o.name.as_str() as *const str, i))
        .collect();
    let mut seen: HashSet<usize> = HashSet::new();
    scored
        .into_iter()
        .take(limit)
        .filter_map(|(label, _)| {
            let ptr = label as *const str;
            label_to_idx.get(&ptr).and_then(|&i| {
                if seen.insert(i) {
                    Some(search_entry_from_disk_object(candidates[i]))
                } else {
                    None
                }
            })
        })
        .collect()
}

fn find_files_in_ngram_index(
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
    let offset = offset.unwrap_or(0) as usize;
    let q_trimmed = query.trim();
    let q_len = q_trimmed.chars().count();
    let apply_fuzzy = use_fuzzy.unwrap_or(true) && !q_trimmed.is_empty() && q_len >= 3;
    let total_start = Instant::now();

    let guard = state.trigram_index.lock().unwrap();
    let index_arc = &*guard;
    if index_arc.objects.is_empty() {
        return Ok(FindFilesResponse { items: vec![], next_offset: None });
    }

    let has_filter = extensions.as_ref().map_or(false, |s| !s.trim().is_empty())
        || category.as_deref().map_or(false, |c| !c.trim().is_empty() && c.trim() != "all");

    let extension_set: Option<std::collections::HashSet<String>> = extensions.as_ref().and_then(|s| {
        let cleaned: Vec<String> = s.split(',')
            .map(|x| x.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|x| !x.is_empty())
            .collect();
        if cleaned.is_empty() { None } else { Some(cleaned.into_iter().collect()) }
    });

    let passes_filter = |o: &DiskObject| -> bool {
        if !has_filter { return true; }
        if let Some(ref set) = extension_set {
            match o.kind {
                DiskObjectKind::File => o.ext.as_ref().map(|e| set.contains(e)).unwrap_or(false),
                DiskObjectKind::Folder => false,
            }
        } else {
            category_filter::category_allowed(category.as_deref(), o)
        }
    };

    // For empty queries with a filter, scan all objects directly with correct pagination.
    // The trigram index empty-query path only returns a bounded window by object position,
    // which misses matching files that happen to sit beyond that window.
    let filter_start = Instant::now();
    let (items, next_offset) = if query.trim().is_empty() && has_filter {
        let objects = &index_arc.objects;
        let mut collected: Vec<SearchEntry> = Vec::new();
        let mut next_off: Option<usize> = None;
        let mut i = offset;
        while i < objects.len() {
            if passes_filter(&objects[i]) {
                collected.push(search_entry_from_disk_object(&objects[i]));
                if collected.len() == limit {
                    if i + 1 < objects.len() {
                        next_off = Some(i + 1);
                    }
                    break;
                }
            }
            i += 1;
        }
        (collected, next_off)
    } else if has_filter {
        // Non-empty query with a category/extension filter.
        //
        // Strategy: intersect two trigram-index result sets instead of overfetching
        // query results and post-filtering (which yields too few hits when the category
        // is sparsely represented among the top-N query matches).
        //
        // For categories with known extensions we search the trigram index for each
        // ".ext" suffix to get every file that belongs to the category, then intersect
        // with the query matches.  For "folder" / "other" (no known extensions) we fall
        // back to the overfetch approach since there's nothing extension-specific to
        // search for.
        let searchable_exts: Option<Vec<String>> = if let Some(ref set) = extension_set {
            // Manual extension filter supplied by the caller
            Some(set.iter().cloned().collect())
        } else if let Some(cat) = category.as_deref().map(|c| c.trim()) {
            search_category::extension_set(cat)
                .map(|exts| exts.iter().map(|&e| e.to_string()).collect())
        } else {
            None
        };

        let search_start = Instant::now();
        if let Some(exts) = searchable_exts {
            // Step 1: collect all object indices whose filename contains ".{ext}"
            let mut cat_set: HashSet<u32> = HashSet::new();
            for ext in &exts {
                let ext_query = format!(".{}", ext);
                let (ext_matches, _) = trigram_find_files(
                    &index_arc,
                    &ext_query,
                    &SearchFilter::None,
                    usize::MAX,
                    0,
                );
                cat_set.extend(ext_matches);
            }

            if apply_fuzzy {
                // Fuzzy: skip trigram query filter — nucleo scores all ext-matching candidates.
                let search_ms = search_start.elapsed().as_millis();
                write_debug_log(state, &format!(
                    "find_files_in_ngram_index ext_search mode=fuzzy query={:?} cat_candidates={} ms={}",
                    query, cat_set.len(), search_ms,
                ));
                let mut cat_sorted: Vec<u32> = cat_set.into_iter().collect();
                cat_sorted.sort_unstable();
                let candidates: Vec<&DiskObject> = cat_sorted.iter()
                    .map(|&i| &index_arc.objects[i as usize])
                    .collect();
                let items = nucleo_rank_objects(&query, candidates, limit);
                (items, None)
            } else {
                // Exact: Step 2 — intersect with trigram results for the query.
                let (query_matches, _) = trigram_find_files(
                    &index_arc,
                    &query,
                    &SearchFilter::None,
                    usize::MAX,
                    0,
                );
                let search_ms = search_start.elapsed().as_millis();
                write_debug_log(state, &format!(
                    "find_files_in_ngram_index ext_search mode=exact query={:?} cat_candidates={} query_candidates={} ms={}",
                    query, cat_set.len(), query_matches.len(), search_ms,
                ));
                let query_set: HashSet<u32> = query_matches.into_iter().collect();
                let mut combined: Vec<u32> = cat_set.into_iter()
                    .filter(|idx| query_set.contains(idx))
                    .collect();
                combined.sort_unstable();
                let s = offset.min(combined.len());
                let e = (s + limit).min(combined.len());
                let next_off = if e < combined.len() { Some(offset + limit) } else { None };
                let items: Vec<SearchEntry> = combined[s..e].iter()
                    .map(|&i| search_entry_from_disk_object(&index_arc.objects[i as usize]))
                    .collect();
                (items, next_off)
            }
        } else if apply_fuzzy {
            // Fuzzy, folder/other — scan all objects, post-filter by category, then nucleo.
            let search_ms = search_start.elapsed().as_millis();
            let candidates: Vec<&DiskObject> = index_arc.objects.iter()
                .filter(|o| passes_filter(o))
                .collect();
            write_debug_log(state, &format!(
                "find_files_in_ngram_index category_scan mode=fuzzy query={:?} candidates={} ms={}",
                query, candidates.len(), search_ms,
            ));
            let items = nucleo_rank_objects(&query, candidates, limit);
            (items, None)
        } else {
            // Exact, folder/other — overfetch from trigram index and post-filter.
            let fetch_limit = (limit * 4).max(2000);
            let (indices, _) = trigram_find_files(
                &index_arc,
                &query,
                &SearchFilter::None,
                fetch_limit,
                0,
            );
            let search_ms = search_start.elapsed().as_millis();
            write_debug_log(state, &format!(
                "find_files_in_ngram_index trigram_search mode=exact query={:?} candidates={} ms={}",
                query, indices.len(), search_ms,
            ));
            let filtered: Vec<SearchEntry> = indices.iter()
                .map(|&i| &index_arc.objects[i as usize])
                .filter(|o| passes_filter(o))
                .take(limit)
                .map(|o| search_entry_from_disk_object(o))
                .collect();
            (filtered, None)
        }
    } else if apply_fuzzy {
        // No filter, fuzzy — scan all objects and let nucleo rank them.
        let search_start = Instant::now();
        let candidates: Vec<&DiskObject> = index_arc.objects.iter().collect();
        let search_ms = search_start.elapsed().as_millis();
        write_debug_log(state, &format!(
            "find_files_in_ngram_index full_scan mode=fuzzy query={:?} candidates={} ms={}",
            query, candidates.len(), search_ms,
        ));
        let items = nucleo_rank_objects(&query, candidates, limit);
        (items, None)
    } else {
        // No filter, exact — trigram search with pagination.
        let search_start = Instant::now();
        let (indices, has_more_raw) = trigram_find_files(
            &index_arc,
            &query,
            &SearchFilter::None,
            limit,
            offset,
        );
        let search_ms = search_start.elapsed().as_millis();
        write_debug_log(state, &format!(
            "find_files_in_ngram_index trigram_search mode=exact query={:?} candidates={} ms={}",
            query, indices.len(), search_ms,
        ));
        let items: Vec<SearchEntry> = indices.iter()
            .map(|&i| search_entry_from_disk_object(&index_arc.objects[i as usize]))
            .collect();
        let next_offset = if has_more_raw { Some(offset + limit) } else { None };
        (items, next_offset)
    };
    let filter_ms = filter_start.elapsed().as_millis();

    let total_ms = total_start.elapsed().as_millis();
    write_debug_log(state, &format!(
        "find_files mode={} filter_ms={} total_ms={} count={} query={:?}",
        if apply_fuzzy { "fuzzy" } else { "exact" },
        filter_ms, total_ms, items.len(), query,
    ));

    Ok(FindFilesResponse { items, next_offset })
}

fn find_files_in_compressed_text_index(
    state: &tauri::State<AppState>,
    query: String,
    extensions: Option<String>,
    category: Option<String>,
    limit: Option<u32>,
    _use_fuzzy: Option<bool>,
    offset: Option<u32>,
) -> Result<FindFilesResponse, String> {
    const DEFAULT_LIMIT: u32 = 500;
    let limit = limit.unwrap_or(DEFAULT_LIMIT) as usize;
    let offset = offset.unwrap_or(0) as usize;
    let total_start = Instant::now();

    write_debug_log(
        state,
        &format!(
            ">>> find_files_in_compressed_text_index start query_len={} limit={} offset={}",
            query.len(), limit, offset
        ),
    );

    let cti_path = resolve_compressed_text_index_path(state);
    let filter = resolve_search_filter(extensions.as_deref(), category.as_deref());

    let (disk_entries, has_more) = cti_find_files(
        &cti_path,
        &query,
        &filter,
        limit,
        offset,
    ).map_err(|e| format!("compressed text index search failed: {:?}", e))?;

    let items: Vec<SearchEntry> = disk_entries
        .iter()
        .map(|o| search_entry_from_disk_object(o))
        .collect();

    let next_offset = if has_more { Some(offset + limit) } else { None };
    let total_ms = total_start.elapsed().as_millis();

    write_debug_log(
        state,
        &format!(
            "find_files_in_compressed_text_index done total_ms={} count={} next_offset={:?}",
            total_ms, items.len(), next_offset
        ),
    );

    Ok(FindFilesResponse {
        items,
        next_offset,
    })
}

fn resolve_search_filter(
    extensions: Option<&str>,
    category: Option<&str>,
) -> SearchFilter {
    let manual_exts: Option<Vec<String>> = extensions.and_then(|s| {
        let cleaned: Vec<String> = s
            .split(',')
            .map(|x| x.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|x| !x.is_empty())
            .collect();
        if cleaned.is_empty() {
            None
        } else {
            Some(cleaned)
        }
    });

    if let Some(exts) = manual_exts {
        return SearchFilter::Extensions(exts);
    }

    let category = category
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty() && s != "all");
    let cat = category.as_deref();

    match cat {
        None => SearchFilter::None,
        Some("folder") => SearchFilter::FoldersOnly,
        Some("other") => SearchFilter::Other,
        Some(c) => {
            if let Some(exts) = search_category::extension_set(c) {
                SearchFilter::Extensions(exts.iter().map(|s| (*s).to_string()).collect())
            } else {
                SearchFilter::None
            }
        }
    }
}

fn find_files_in_db(
    state: &tauri::State<AppState>,
    query: String,
    extensions: Option<String>,
    category: Option<String>,
    limit: Option<u32>,
    _use_fuzzy: Option<bool>,
    offset: Option<u32>,
) -> Result<FindFilesResponse, String> {
    const DEFAULT_LIMIT: u32 = 500;
    let limit = limit.unwrap_or(DEFAULT_LIMIT) as usize;
    let offset = offset.unwrap_or(0) as usize;

    let total_start = Instant::now();

    write_debug_log(
        state,
        &format!(
            ">>> find_files_in_db start query_len={} limit={} offset={} category={:?}",
            query.len(),
            limit,
            offset,
            category.as_deref()
        ),
    );

    let db_path = state.db_path.clone();
    let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;

    let resolve_start = Instant::now();
    let filter = resolve_search_filter(extensions.as_deref(), category.as_deref());
    let resolve_ms = resolve_start.elapsed().as_millis();

    let db_start = Instant::now();
    let (disk_entries, has_more) = sqlite_find_files(
        &conn,
        &query,
        &filter,
        limit,
        offset,
    )
    .map_err(|e| e.to_string())?;
    let db_total_ms = db_start.elapsed().as_millis();

    let serialize_start = Instant::now();
    let items: Vec<SearchEntry> = disk_entries
        .iter()
        .map(|o| search_entry_from_disk_object(o))
        .collect();
    let serialize_ms = serialize_start.elapsed().as_millis();

    let next_offset = if has_more {
        Some(offset + limit)
    } else {
        None
    };

    let total_ms = total_start.elapsed().as_millis();
    write_debug_log(
        state,
        &format!(
            "find_files_in_db profile resolve_ms={} db_total_ms={} serialize_ms={} total_ms={} count={} next_offset={:?}",
            resolve_ms,
            db_total_ms,
            serialize_ms,
            total_ms,
            items.len(),
            next_offset
        ),
    );

    Ok(FindFilesResponse {
        items,
        next_offset,
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
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(move |app| {
            let path = app.path().app_data_dir().map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            let db_path = path.join("index.db");
            let index_mode = parse_index_mode();

            if !matches!(index_mode, SearchIndexMode::CompressedText) {
                db::open_db(&db_path).map_err(|e| e.to_string())?;
            }

            let env_log_path = std::env::var("CUTE_DISK_TREE_DEBUG_LOG_PATH").ok();
            let debug_log_path = env_log_path
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| path.join("debug.log"));

            cutest_disk_tree::logging::debug_log::init_debug_logger(debug_log_path.clone());

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
                    writeln!(f, "CUTE_DISK_TREE_INDEX_MODE={:?}", index_mode)?;
                    if let Ok(p) = std::env::var("CUTE_DISK_TREE_DEBUG_LOG_PATH") {
                        writeln!(f, "CUTE_DISK_TREE_DEBUG_LOG_PATH={}", p)?;
                    }
                    if let Ok(p) = std::env::var("CUTE_DISK_TREE_SCAN_PATH") {
                        writeln!(f, "CUTE_DISK_TREE_SCAN_PATH={}", p)?;
                    }
                    writeln!(f)?;
                    f.flush()
                });

            let force_fresh = scan_path_override.is_some();

            // Initialize state immediately with no data so the window can open
            // and the frontend can render right away. Index loading happens in a
            // background task below.
            app.manage(AppState {
                db_path: db_path.clone(),
                debug_log: Mutex::new(Some(debug_log_path)),
                disk_objects: Mutex::new(None),
                name_reverse_index: Mutex::new(None),
                trigram_index: Arc::new(Mutex::new(trigram_build_index(&[]))),
                phase2_cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
                is_scanning: Arc::new(AtomicBool::new(false)),
                scan_path_override: scan_path_override.clone(),
                index_mode,
                _watcher: Mutex::new(None),
                _reconciler: Mutex::new(None),
            });

            let handle = app.handle().clone();
            if force_fresh {
                let state_ref: tauri::State<AppState> = app.state();
                write_debug_log(&state_ref, "setup: force_fresh=true, spawning auto-scan");
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
                if uses_in_memory_index(index_mode) {
                    tauri::async_runtime::spawn(async move {
                        let state: tauri::State<AppState> = handle.state();
                        let db_path = state.db_path.clone();
                        let result = tauri::async_runtime::spawn_blocking(move || {
                            let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;
                            if !db::has_disk_objects(&conn).unwrap_or(false) {
                                return Ok::<_, String>(None);
                            }
                            let mut objs = db::get_disk_objects(&conn).unwrap_or_default();
                            objs.sort_by(|a, b| a.path.cmp(&b.path));

                            if index_mode == SearchIndexMode::InMemoryNgrams {
                                let t0 = Instant::now();
                                let index = trigram_build_index(&objs);
                                let _ = writeln!(
                                    std::io::stderr(),
                                    "startup trigram_index objects={} ms={}",
                                    objs.len(), t0.elapsed().as_millis(),
                                );
                                Ok(Some((Arc::new(objs), None::<Arc<SuffixIndex>>, Some(index))))
                            } else {
                                // InMemorySuffix: load from DB if available, rebuild otherwise
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
                                            let idx = suffix_build_index(&objs);
                                            (idx, "rebuild-fallback")
                                        }
                                    }
                                } else {
                                    let idx = suffix_build_index(&objs);
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
                                    "startup suffix_index objects={} source={} ms={}",
                                    objs.len(), index_source, t_suffix.elapsed().as_millis(),
                                );
                                Ok(Some((Arc::new(objs), Some(Arc::new(name_index)), None::<TrigramIndex>)))
                            }
                        }).await;

                        if let Ok(Ok(Some((objs, name_idx, trigram_idx)))) = result {
                            *state.disk_objects.lock().unwrap() = Some(objs);
                            *state.name_reverse_index.lock().unwrap() = name_idx;
                            if let Some(ti) = trigram_idx {
                                *state.trigram_index.lock().unwrap() = ti;
                                let roots = match &state.scan_path_override {
                                    Some(p) => vec![std::path::PathBuf::from(p)],
                                    None => cutest_disk_tree::get_filesystem_roots(),
                                };
                                start_file_watchers(&state, roots);
                            }
                            write_debug_log(&state, "setup: background index load complete");
                        }
                    });
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_directory,
            scan_directory_with_helper,
            get_scan_status,
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
