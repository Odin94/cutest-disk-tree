use cutest_disk_tree::{db, DiskObject, DiskObjectKind};
use std::collections::{HashMap, HashSet};
use std::path::Path;
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
use chrono::Utc;
use std::io::Write;
use sysinfo::{Pid, System};

#[derive(Clone, Serialize)]
struct FolderSizesReady {
    root: String,
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
mod tests {
    use super::*;

    fn make_disk_object(name: &str) -> DiskObject {
        DiskObject {
            root: "root".to_string(),
            path: format!("/root/{}", name),
            path_lower: format!("/root/{}", name.to_ascii_lowercase()),
            parent_path: Some("/root".to_string()),
            name: name.to_string(),
            name_lower: name.to_ascii_lowercase(),
            ext: None,
            kind: DiskObjectKind::File,
            size: Some(1),
            recursive_size: None,
            dev: None,
            ino: None,
            mtime: None,
        }
    }

    #[test]
    fn paginate_scan_empty_query_respects_offset_and_limit() {
        let entries: Vec<DiskObject> = (0..10)
            .map(|i| make_disk_object(&format!("file{}", i)))
            .collect();
        let (page1, next1) =
            paginate_scan(&entries, 0, 3, false, &Vec::new(), |_i, _e| true);
        assert_eq!(page1.len(), 3);
        assert_eq!(page1[0].path, "/root/file0");
        assert_eq!(next1, Some(3));

        let (page2, next2) =
            paginate_scan(&entries, next1.unwrap(), 3, false, &Vec::new(), |_i, _e| true);
        assert_eq!(page2.len(), 3);
        assert_eq!(page2[0].path, "/root/file3");
        assert_eq!(next2, Some(6));

        let (page_last, next_last) =
            paginate_scan(&entries, next2.unwrap(), 10, false, &Vec::new(), |_i, _e| true);
        assert_eq!(page_last.len(), 4);
        assert_eq!(page_last[0].path, "/root/file6");
        assert_eq!(next_last, None);
    }

    #[test]
    fn paginate_scan_with_filter_skips_non_matches() {
        let entries = vec![
            make_disk_object("keep1"),
            make_disk_object("skip"),
            make_disk_object("keep2"),
            make_disk_object("keep3"),
        ];
        let (page, next) = paginate_scan(
            &entries,
            0,
            2,
            false,
            &Vec::new(),
            |_i, e| e.name.starts_with("keep"),
        );
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].path, "/root/keep1");
        assert_eq!(page[1].path, "/root/keep2");
        assert_eq!(next, Some(3));

        let (page2, next2) =
            paginate_scan(&entries, next.unwrap(), 2, false, &Vec::new(), |_i, e| {
                e.name.starts_with("keep")
            });
        assert_eq!(page2.len(), 1);
        assert_eq!(page2[0].path, "/root/keep3");
        assert_eq!(next2, None);
    }
}

struct AppState {
    db_path: std::path::PathBuf,
    debug_log: Mutex<Option<std::path::PathBuf>>,
    disk_objects: Mutex<HashMap<String, Arc<Vec<DiskObject>>>>,
    name_reverse_index: Mutex<HashMap<String, Arc<SuffixIndex>>>,
    phase2_cancel: Mutex<Arc<AtomicBool>>,
}

#[allow(dead_code)]
fn make_trigrams(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 3 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(chars.len().saturating_sub(2));
    for i in 0..(chars.len() - 2) {
        out.push(chars[i..i + 3].iter().collect());
    }
    out
}

#[allow(dead_code)]
fn make_bigrams(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(chars.len().saturating_sub(1));
    for i in 0..(chars.len() - 1) {
        out.push(chars[i..i + 2].iter().collect());
    }
    out
}

#[allow(dead_code)]
struct NameReverseIndex {
    trigram_to_indices: HashMap<String, HashSet<usize>>,
}

#[allow(dead_code)]
fn build_name_reverse_index(objects: &[DiskObject]) -> NameReverseIndex {
    let mut trigram_to_indices: HashMap<String, HashSet<usize>> = HashMap::new();
    for (i, o) in objects.iter().enumerate() {
        if !matches!(o.kind, DiskObjectKind::File) {
            continue;
        }
        let s = o.name_lower.to_ascii_lowercase();
        for t in make_trigrams(&s) {
            trigram_to_indices.entry(t).or_default().insert(i);
        }
    }
    NameReverseIndex {
        trigram_to_indices,
    }
}

#[allow(dead_code)]
fn candidate_indices_from_reverse_index(
    index: &NameReverseIndex,
    q_lower: &str,
) -> Option<HashSet<usize>> {
    let chars_count = q_lower.chars().count();
    if chars_count < 3 {
        return None;
    }
    let tokens: Vec<String> = make_trigrams(q_lower);
    if tokens.is_empty() {
        return None;
    }
    let mut candidate_set: Option<HashSet<usize>> = None;
    let map = &index.trigram_to_indices;
    for t in &tokens {
        let set = map.get(t)?;
        candidate_set = Some(match candidate_set {
            Some(acc) => acc.intersection(set).copied().collect(),
            None => set.clone(),
        });
        if candidate_set.as_ref().map_or(true, |s| s.is_empty()) {
            return candidate_set;
        }
    }
    candidate_set
}

// Suffix-array based file name index.
//
// `buffer` is a single string of all indexed file names in lowercase, each
// separated by a '\0' byte so that queries cannot bleed across name boundaries.
// `offsets[i]` is the byte offset in `buffer` where the i-th indexed file's
// name starts, and `disk_object_indices[i]` is that file's position in the
// parent `Vec<DiskObject>`.  The two vecs are parallel and both sorted by
// ascending offset.
struct SuffixIndex {
    st: SuffixTable<'static, 'static>,
    offsets: Vec<usize>,
    disk_object_indices: Vec<usize>,
    /// The concatenated `name_lower\0` string passed to `SuffixTable::new`.
    /// Kept so the index can be serialised to the database without re-scanning
    /// the DiskObject vec.
    buffer: String,
}

// Returns (index, concat_ms, table_ms) so callers can log the internal breakdown.
// concat_ms  = time to build the concatenated name buffer + offset vecs
// table_ms   = time to construct the suffix array (O(n log n), the expensive part)
fn build_suffix_index(objects: &[DiskObject]) -> (SuffixIndex, u128, u128) {
    let concat_start = Instant::now();
    let mut buffer = String::with_capacity(objects.len() * 16);
    let mut offsets: Vec<usize> = Vec::with_capacity(objects.len());
    let mut disk_object_indices: Vec<usize> = Vec::with_capacity(objects.len());

    for (i, o) in objects.iter().enumerate() {
        if !matches!(o.kind, DiskObjectKind::File) {
            continue;
        }
        offsets.push(buffer.len());
        disk_object_indices.push(i);
        buffer.push_str(&o.name_lower);
        buffer.push('\0');
    }
    let concat_ms = concat_start.elapsed().as_millis();

    let table_start = Instant::now();
    let st = SuffixTable::new(buffer.clone());
    let table_ms = table_start.elapsed().as_millis();

    (SuffixIndex { st, offsets, disk_object_indices, buffer }, concat_ms, table_ms)
}

// Returns None when the index is empty (no candidates to prune) or the query
// is empty.  Returns Some(empty set) when no names match.  Otherwise returns
// Some with the set of disk_objects indices whose names contain `q_lower`.
fn search_suffix_index(index: &SuffixIndex, q_lower: &str) -> Option<HashSet<usize>> {
    if q_lower.is_empty() || index.offsets.is_empty() {
        return None;
    }

    let positions = index.st.positions(q_lower);

    if positions.is_empty() {
        return Some(HashSet::new());
    }

    let mut result: HashSet<usize> = HashSet::new();
    for &pos in positions {
        let pos = pos as usize;
        // Find the last offset that is <= pos: that is the file owning this position.
        let local_idx = index.offsets.partition_point(|&x| x <= pos) - 1;
        result.insert(index.disk_object_indices[local_idx]);
    }
    Some(result)
}

fn write_debug_log(state: &AppState, message: &str) {
    let mut guard = state.debug_log.lock().unwrap();
    let path = guard
        .get_or_insert_with(|| {
            state
                .db_path
                .parent()
                .map(|p| p.join("debug.log"))
                .unwrap_or_else(|| std::path::PathBuf::from("debug.log"))
        })
        .clone();
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
    let guard = state.debug_log.lock().unwrap();
    let path = guard
        .as_ref()
        .cloned()
        .unwrap_or_else(|| {
            state
                .db_path
                .parent()
                .map(|p| p.join("debug.log"))
                .unwrap_or_else(|| std::path::PathBuf::from("debug.log"))
        });
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

// Shared constructor that derives all path-based fields from a raw path string,
// eliminating duplicated decomposition logic across the scan phases.
fn make_disk_object_from_path(
    root: &str,
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
        root: root.to_string(),
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

// Build the initial Vec<DiskObject> for a root from raw walker output.
// Files and folders are both included and come directly from the walker —
// folders do not have recursive sizes yet (those are added by apply_folder_sizes).
fn build_disk_objects(
    root: &str,
    files: &[cutest_disk_tree::FileEntry],
    folder_paths: &std::collections::HashSet<std::path::PathBuf>,
) -> Vec<DiskObject> {
    let mut objs: Vec<DiskObject> = Vec::with_capacity(files.len() + folder_paths.len());
    for f in files {
        objs.push(make_disk_object_from_path(
            root,
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
            root,
            folder.to_string_lossy().into_owned(),
            DiskObjectKind::Folder,
            None, None, None, None, None,
        ));
    }
    objs.sort_by(|a, b| a.path.cmp(&b.path));
    objs
}

// Back-fill recursive folder sizes (computed in the background phase) into an
// existing DiskObject vec.  Folders present in folder_sizes but missing from
// objs (can happen when the size-walk discovers additional dirs) are inserted.
fn apply_folder_sizes(
    mut objs: Vec<DiskObject>,
    folder_sizes: &HashMap<std::path::PathBuf, u64>,
    root: &str,
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
                root, path_string, DiskObjectKind::Folder,
                None, Some(s), None, None, None,
            ));
        }
    }
    objs.sort_by(|a, b| a.path.cmp(&b.path));
    objs
}

// Phase 1b: convert raw scan results into DiskObjects (files + folders together),
// build the suffix index, and install both into AppState.  After this returns,
// find_files is fully operational for the root.
fn activate_initial_index(
    app: &tauri::AppHandle,
    state: &AppState,
    root: &str,
    files: &[cutest_disk_tree::FileEntry],
    folder_paths: &std::collections::HashSet<std::path::PathBuf>,
) {
    let t0 = Instant::now();
    let _ = app.emit("scan-phase-status", "building search index...".to_string());

    let build_start = Instant::now();
    let objs = build_disk_objects(root, files, folder_paths);
    let build_ms = build_start.elapsed().as_millis();

    let (index, suffix_concat_ms, suffix_table_ms) = build_suffix_index(&objs);

    write_debug_log(state, &format!(
        "activate_initial_index done root={} files={} folders={} objects={} build_disk_objs_ms={} suffix_concat_ms={} suffix_table_ms={} total_ms={}",
        root, files.len(), folder_paths.len(), objs.len(), build_ms, suffix_concat_ms, suffix_table_ms, t0.elapsed().as_millis(),
    ));

    {
        let mut disk_map = state.disk_objects.lock().unwrap();
        disk_map.insert(root.to_string(), Arc::new(objs));
        let mut idx_map = state.name_reverse_index.lock().unwrap();
        idx_map.insert(root.to_string(), Arc::new(index));
    }
    let _ = app.emit("scan-phase-status", "".to_string());
}

// Phase 2 (background): compute recursive folder sizes, update the DiskObject
// vec with those sizes, then persist everything to SQLite.
fn run_phase2(
    app_bg: tauri::AppHandle,
    db_path_bg: std::path::PathBuf,
    root_str: String,
    root_buf: std::path::PathBuf,
    files_bg: Vec<cutest_disk_tree::FileEntry>,
    cancel: Arc<AtomicBool>,
) {
    let state_ptr: tauri::State<AppState> = app_bg.state();
    let total_start = Instant::now();

    // Step 1: compute recursive folder sizes.
    let _ = app_bg.emit("scan-phase-status", "aggregating folder sizes...".to_string());
    let sizes_start = Instant::now();
    let folder_sizes = cutest_disk_tree::compute_folder_sizes(&root_buf, &files_bg);
    let sizes_ms = sizes_start.elapsed().as_millis();
    write_debug_log(&state_ptr, &format!(
        "phase2 folder_sizes_done root={} folders={} ms={}",
        root_str, folder_sizes.len(), sizes_ms,
    ));

    if cancel.load(Ordering::Relaxed) {
        write_debug_log(&state_ptr, &format!("phase2 cancelled after step1 root={}", root_str));
        return;
    }

    // Step 2: apply folder sizes to the DiskObject vec.
    // The suffix index built in phase 1b only indexes file names, which haven't
    // changed, so it stays valid and does not need to be rebuilt here.
    let _ = app_bg.emit("scan-phase-status", "updating search index...".to_string());
    let existing_arc = {
        let guard = state_ptr.disk_objects.lock().unwrap();
        guard.get(&root_str).cloned()
    };
    if let Some(arc) = existing_arc {
        let apply_start = Instant::now();
        let new_objs = apply_folder_sizes((*arc).clone(), &folder_sizes, &root_str);
        let apply_ms = apply_start.elapsed().as_millis();

        write_debug_log(&state_ptr, &format!(
            "phase2 index_updated root={} objects={} apply_folder_sizes_ms={} total_ms={}",
            root_str, new_objs.len(), apply_ms, total_start.elapsed().as_millis(),
        ));

        {
            let mut disk_map = state_ptr.disk_objects.lock().unwrap();
            disk_map.insert(root_str.clone(), Arc::new(new_objs));
        }
    }

    // Folder sizes are now in memory — notify the frontend so it can populate
    // the "Largest folders" tab without waiting for the DB write.
    let folder_sizes_ser: HashMap<String, u64> = folder_sizes
        .iter()
        .map(|(p, s)| (p.to_string_lossy().to_string(), *s))
        .collect();
    let _ = app_bg.emit("scan-folder-sizes-ready", FolderSizesReady {
        root: root_str.clone(),
        folder_sizes: folder_sizes_ser,
    });
    // User-visible work is done — clear the spinner before the slow DB write.
    let _ = app_bg.emit("scan-phase-status", "".to_string());

    if cancel.load(Ordering::Relaxed) {
        write_debug_log(&state_ptr, &format!("phase2 cancelled after step2 root={}", root_str));
        return;
    }

    // Step 3: persist to SQLite (background, no longer blocks the UI).
    let update_id = chrono::Utc::now().timestamp_millis();
    let db_open_start = Instant::now();
    let conn = match db::open_db(&db_path_bg) {
        Ok(c) => c,
        Err(e) => {
            write_debug_log(&state_ptr, &format!(
                "phase2 db_open_failed root={} error={} total_ms={}",
                root_str, e, total_start.elapsed().as_millis(),
            ));
            return;
        }
    };
    let db_open_ms = db_open_start.elapsed().as_millis();

    let db_write_start = Instant::now();
    let write_result = db::write_scan(&conn, &root_str, &files_bg, &folder_sizes, update_id);
    let db_write_ms = db_write_start.elapsed().as_millis();

    // Step 4: persist the suffix index so startup can skip rebuilding it.
    let index_write_ms = if write_result.is_ok() {
        let index_arc = {
            let guard = state_ptr.name_reverse_index.lock().unwrap();
            guard.get(&root_str).cloned()
        };
        if let Some(arc) = index_arc {
            let idx_start = Instant::now();
            let _ = db::write_suffix_index_data(
                &conn, &root_str, update_id,
                &arc.buffer, &arc.offsets, &arc.disk_object_indices,
            );
            idx_start.elapsed().as_millis()
        } else {
            0
        }
    } else {
        0
    };

    let total_ms = total_start.elapsed().as_millis();

    match write_result {
        Ok(()) => {
            write_debug_log(&state_ptr, &format!(
                "phase2 done root={} files={} folders={} sizes_ms={} db_open_ms={} db_write_ms={} index_write_ms={} total_ms={}",
                root_str, files_bg.len(), folder_sizes.len(), sizes_ms, db_open_ms, db_write_ms, index_write_ms, total_ms,
            ));
        }
        Err(e) => {
            write_debug_log(&state_ptr, &format!(
                "phase2 db_write_failed root={} error={} sizes_ms={} db_open_ms={} db_write_ms={} total_ms={}",
                root_str, e, sizes_ms, db_open_ms, db_write_ms, total_ms,
            ));
        }
    }
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

    // Phase 1: parallel filesystem walk (blocking).
    let root_for_scan = path_buf.clone();
    let app_for_scan = app.clone();
    let scan_start = Instant::now();
    let (result, files, folder_paths) = match tauri::async_runtime::spawn_blocking(move || {
        let (files, folder_paths) =
            cutest_disk_tree::index_directory_ignore_with_progress(&root_for_scan, |p| {
                let _ = app_for_scan.emit("scan-progress", &p);
            });
        let empty_folder_sizes: HashMap<std::path::PathBuf, u64> = HashMap::new();
        let result =
            cutest_disk_tree::to_scan_result(&root_for_scan, &files, &empty_folder_sizes)
                .ok_or_else(|| "Indexing failed".to_string())?;
        Ok::<_, String>((result, files, folder_paths))
    })
    .await
    {
        Ok(Ok(triple)) => triple,
        Ok(Err(e)) => {
            write_debug_log(&state, &format!("error scan_directory phase1: {}", e));
            return Err(e);
        }
        Err(e) => {
            write_debug_log(&state, &format!("error scan_directory phase1 spawn: {}", e));
            return Err(e.to_string());
        }
    };
    write_debug_log(&state, &format!(
        "scan_directory phase1_done path={} files={} folders={} ms={}",
        path, files.len(), folder_paths.len(), scan_start.elapsed().as_millis(),
    ));

    // Phase 1b: build DiskObjects + suffix index so find_files works immediately.
    activate_initial_index(&app, &state, &path, &files, &folder_paths);

    // Phase 2: background — compute folder sizes, update index, persist to SQLite.
    // Cancel any still-running phase 2 before spawning a new one.
    let cancel_token = {
        let mut guard = state.phase2_cancel.lock().unwrap();
        guard.store(true, Ordering::Relaxed);
        let fresh = Arc::new(AtomicBool::new(false));
        *guard = fresh.clone();
        fresh
    };
    let app_bg = app.clone();
    let files_bg = files.clone();
    let root_str_bg = path.clone();
    let root_buf_bg = path_buf.clone();
    tauri::async_runtime::spawn_blocking(move || {
        run_phase2(app_bg, db_path, root_str_bg, root_buf_bg, files_bg, cancel_token);
    });

    write_debug_log(&state, &format!("scan_directory done path={}", path));
    Ok(result)
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn search_entry_from_disk_object_maps_file_and_folder() {
        let file = DiskObject {
            root: "C:/root".to_string(),
            path: "C:/root/file.txt".to_string(),
            path_lower: "c:/root/file.txt".to_string(),
            parent_path: Some("C:/root".to_string()),
            name: "file.txt".to_string(),
            name_lower: "file.txt".to_string(),
            ext: Some("txt".to_string()),
            kind: DiskObjectKind::File,
            size: Some(10),
            recursive_size: None,
            dev: Some(1),
            ino: Some(2),
            mtime: None,
        };
        let folder = DiskObject {
            root: "C:/root".to_string(),
            path: "C:/root/folder".to_string(),
            path_lower: "c:/root/folder".to_string(),
            parent_path: Some("C:/root".to_string()),
            name: "folder".to_string(),
            name_lower: "folder".to_string(),
            ext: None,
            kind: DiskObjectKind::Folder,
            size: None,
            recursive_size: Some(20),
            dev: None,
            ino: None,
            mtime: None,
        };

        let file_entry = search_entry_from_disk_object(&file);
        assert_eq!(file_entry.path, "C:/root/file.txt");
        assert_eq!(file_entry.size, 10);
        assert_eq!(file_entry.kind, "file");
        assert!(file_entry.file_key.is_some());
        let fk = file_entry.file_key.unwrap();
        assert_eq!(fk.dev, 1);
        assert_eq!(fk.ino, 2);

        let folder_entry = search_entry_from_disk_object(&folder);
        assert_eq!(folder_entry.path, "C:/root/folder");
        assert_eq!(folder_entry.size, 20);
        assert_eq!(folder_entry.kind, "folder");
        assert!(folder_entry.file_key.is_none());
    }

    fn make_file(root: &str, name: &str, ino: u64) -> DiskObject {
        DiskObject {
            root: root.to_string(),
            path: format!("{}/{}", root, name),
            path_lower: format!("{}/{}", root, name.to_ascii_lowercase()),
            parent_path: Some(root.to_string()),
            name: name.to_string(),
            name_lower: name.to_ascii_lowercase(),
            ext: name.rsplit('.').next().map(|e| e.to_ascii_lowercase()),
            kind: DiskObjectKind::File,
            size: Some(0),
            recursive_size: None,
            dev: Some(1),
            ino: Some(ino),
            mtime: None,
        }
    }

    #[test]
    fn suffix_index_finds_both_names_containing_query() {
        let objs = vec![
            make_file("C:/root", "AbstractButton.qml", 1),
            make_file("C:/root", "Button.txt", 2),
        ];

        let (index, ..) = build_suffix_index(&objs);
        let candidates = search_suffix_index(&index, "button")
            .expect("should return some candidates");

        assert!(candidates.contains(&0));
        assert!(candidates.contains(&1));
    }

    #[test]
    fn suffix_index_returns_empty_set_for_no_match() {
        let objs = vec![
            make_file("C:/root", "readme.md", 1),
            make_file("C:/root", "main.rs", 2),
        ];

        let (index, ..) = build_suffix_index(&objs);
        let candidates = search_suffix_index(&index, "zzznomatch")
            .expect("should return Some (empty set)");

        assert!(candidates.is_empty());
    }

    #[test]
    fn suffix_index_no_bleed_across_name_boundary() {
        // "file" ends in 'e', "exe" starts with 'e' — "lee" must NOT match across the boundary.
        let objs = vec![
            make_file("C:/root", "file.txt", 1),
            make_file("C:/root", "exe.bin", 2),
        ];

        let (index, ..) = build_suffix_index(&objs);
        // "lee" would only exist if "file\0exe" were treated as one string ("filee" wouldn't match "lee").
        // The null separator prevents cross-name matches.
        let candidates = search_suffix_index(&index, "lee");
        // Either None (empty index path) or Some(empty) — definitely not matching either file.
        if let Some(c) = candidates {
            assert!(c.is_empty());
        }
    }

    #[test]
    fn suffix_index_skips_folders() {
        let mut objs = vec![
            make_file("C:/root", "notes.txt", 1),
        ];
        objs.push(DiskObject {
            root: "C:/root".to_string(),
            path: "C:/root/notes_folder".to_string(),
            path_lower: "c:/root/notes_folder".to_string(),
            parent_path: Some("C:/root".to_string()),
            name: "notes_folder".to_string(),
            name_lower: "notes_folder".to_string(),
            ext: None,
            kind: DiskObjectKind::Folder,
            size: None,
            recursive_size: Some(0),
            dev: None,
            ino: None,
            mtime: None,
        });

        let (index, ..) = build_suffix_index(&objs);
        let candidates = search_suffix_index(&index, "notes")
            .expect("should match the file");

        // Only the file (index 0) should appear; the folder (index 1) is not indexed.
        assert!(candidates.contains(&0));
        assert!(!candidates.contains(&1));
    }

}

#[tauri::command]
async fn list_cached_roots(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    let t0 = Instant::now();
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
    let ms = t0.elapsed().as_millis();
    write_debug_log(&state, &format!("list_cached_roots done count={} ms={}", roots.len(), ms));
    Ok(roots)
}

#[tauri::command]
async fn load_cached_scan(
    state: tauri::State<'_, AppState>,
    root: String,
) -> Result<Option<cutest_disk_tree::ScanResult>, String> {
    let t0 = Instant::now();
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
    let ms = t0.elapsed().as_millis();
    write_debug_log(
        &state,
        &format!(
            "load_cached_scan done root={} has_result={} ms={}",
            root,
            result.is_some(),
            ms
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
        &state,
        &format!(
            ">>> find_files start root={} query_len={} limit={}",
            root,
            query.len(),
            limit
        ),
    );

    let _log_err = |e: &dyn std::fmt::Display| {
        let s = e.to_string();
        write_debug_log(&state, &format!("error find_files: {}", s));
        s
    };

    let (disk_entries_arc, _from_cache) = {
        let guard = state.disk_objects.lock().unwrap();
        if let Some(entries) = guard.get(&root) {
            let t = total_start.elapsed().as_millis();
            write_debug_log(
                &state,
                &format!(
                    "find_files index from_cache root={} count={} ms={}", 
                    root, 
                    entries.len(), 
                    t
                ),
            );
            (entries.clone(), true)
        } else {
            write_debug_log(
                &state,
                &format!(
                    "find_files no_active_index root={} (returning empty results)", 
                    root
                ),
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

    // Precompute which indices pass the extension/kind filter when an extension filter is present.
    let (use_mask, allowed_indices, allowed_count) = match &extension_set {
        None => (false, Vec::<bool>::new(), disk_entries.len()),
        Some(set) => {
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
        }
    };

    let ext_filter_ms = ext_filter_start.elapsed().as_millis();
    write_debug_log(
        &state,
        &format!(
            "find_files after_ext_filter allowed_count={} total_entries={} has_ext_filter={} ext_filter_ms={}",
            allowed_count,
            disk_entries.len(),
            extension_set.is_some(),
            ext_filter_ms
        ),
    );

    if start_index >= disk_entries.len() {
        let total_ms = total_start.elapsed().as_millis();
        write_debug_log(
            &state,
            &format!(
                "find_files empty_page root={} start_index={} total_entries={} total_ms={}",
                root,
                start_index,
                disk_entries.len(),
                total_ms
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
            &state,
            &format!(
                "find_files done empty_query count={} start_index={} next_offset={:?} collect_take_ms={} total_ms={}",
                items.len(),
                start_index,
                next_offset,
                collect_ms,
                total_ms
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
            .get(&root)
            .and_then(|idx| search_suffix_index(idx.as_ref(), &q))
    };
    let suffix_search_ms = suffix_search_start.elapsed().as_millis();
    write_debug_log(&state, &format!(
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
            &state,
            &format!(
                "find_files {} q_len={} count={} start_index={} next_offset={:?} results_build_ms={} total_ms={}",
                label,
                q_len,
                items.len(),
                start_index,
                next_offset,
                build_ms,
                total_ms,
            ),
        );
        return Ok(FindFilesResponse {
            items,
            next_offset,
        });
    } else {
        // Full fuzzy (nucleo) path: keep global scoring but enforce limit on built results.
        let nucleo_start = Instant::now();
        let mut filtered: Vec<&DiskObject> = Vec::new();
        for (i, e) in disk_entries.iter().enumerate() {
            if use_mask && !allowed_indices[i] {
                continue;
            }
            if let Some(cs) = &candidate_set_opt {
                if !cs.contains(&i) {
                    continue;
                }
            }
            filtered.push(e);
        }
        let labels: Vec<&str> = filtered.iter().map(|e| e.name.as_str()).collect();
        let mut matcher = Matcher::new(Config::DEFAULT);
        let pattern =
            Pattern::parse(&query, CaseMatching::Smart, Normalization::Smart);
        let mut scored = pattern.match_list(labels.iter().copied(), &mut matcher);
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        let scored_len = scored.len();
        let nucleo_ms = nucleo_start.elapsed().as_millis();
        let select_start = Instant::now();

        let mut items: Vec<SearchEntry> = Vec::new();
        for (label, _score) in scored.into_iter().take(limit) {
            if let Some(e) = filtered.iter().find(|e| e.name.as_str() == label) {
                items.push(search_entry_from_disk_object(e));
            }
        }

        let select_ms = select_start.elapsed().as_millis();
        let total_ms = total_start.elapsed().as_millis();
        write_debug_log(
            &state,
            &format!(
                "find_files nucleo_done q_len={} scored={} taken={} nucleo_ms={} select_build_ms={} total_ms={}",
                q_len,
                scored_len,
                items.len(),
                nucleo_ms,
                select_ms,
                total_ms,
            ),
        );
        return Ok(FindFilesResponse {
            items,
            next_offset: None,
        });
    }
}

fn load_dotenv_from_repo() {
    let mut dir = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(_) => return,
    };
    loop {
        dir.pop();
        let env_path = dir.join(".env");
        if env_path.is_file() {
            let _ = dotenvy::from_path(env_path);
            break;
        }
        if !dir.pop() {
            break;
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    load_dotenv_from_repo();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
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

            // Preload active memory from SQLite if prior indexes exist.
            let mut disk_objects_map: HashMap<String, Arc<Vec<DiskObject>>> = HashMap::new();
            let mut name_reverse_index_map: HashMap<String, Arc<SuffixIndex>> = HashMap::new();

            let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;
            let roots = db::list_roots(&conn).map_err(|e| e.to_string())?;
            for root in roots {
                let mut objs = db::get_disk_objects_for_root(&conn, &root).unwrap_or_default();
                objs.sort_by(|a, b| a.path.cmp(&b.path));

                let t_suffix = Instant::now();
                let meta = db::read_scan_metadata(&conn, &root).ok().flatten();
                let index_is_current = meta.as_ref().map_or(false, |m| {
                    m.disk_objects_update_id != 0
                        && m.suffix_index_update_id == m.disk_objects_update_id
                });

                let (name_index, index_source) = if index_is_current {
                    // Try to load the pre-built index from the database.
                    match db::read_suffix_index_data(&conn, &root) {
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
                    // Stale or missing: build from disk objects and persist so
                    // the next startup can skip this step.
                    let (idx, ..) = build_suffix_index(&objs);
                    if let Some(m) = &meta {
                        if m.disk_objects_update_id != 0 {
                            let _ = db::write_suffix_index_data(
                                &conn, &root, m.disk_objects_update_id,
                                &idx.buffer, &idx.offsets, &idx.disk_object_indices,
                            );
                        }
                    }
                    (idx, "rebuild")
                };

                let _ = writeln!(
                    std::io::stderr(),
                    "startup suffix_index root={} objects={} source={} total_ms={}",
                    root, objs.len(), index_source, t_suffix.elapsed().as_millis(),
                );
                disk_objects_map.insert(root.clone(), Arc::new(objs));
                name_reverse_index_map.insert(root.clone(), Arc::new(name_index));
            }

            app.manage(AppState {
                db_path,
                debug_log: Mutex::new(Some(debug_log_path)),
                disk_objects: Mutex::new(disk_objects_map),
                name_reverse_index: Mutex::new(name_reverse_index_map),
                phase2_cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
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
