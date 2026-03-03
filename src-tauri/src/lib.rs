use cutest_disk_tree::{db, DiskObject, DiskObjectKind};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
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
    name_reverse_index: Mutex<HashMap<String, Arc<NameReverseIndex>>>,
}

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

fn make_unigrams(s: &str) -> Vec<String> {
    s.chars().map(|c| c.to_string()).collect()
}

struct NameReverseIndex {
    trigram_to_indices: HashMap<String, HashSet<usize>>,
    bigram_to_indices: HashMap<String, HashSet<usize>>,
    unigram_to_indices: HashMap<String, HashSet<usize>>,
}

fn build_name_reverse_index(objects: &[DiskObject]) -> NameReverseIndex {
    let mut trigram_to_indices: HashMap<String, HashSet<usize>> = HashMap::new();
    let mut bigram_to_indices: HashMap<String, HashSet<usize>> = HashMap::new();
    let mut unigram_to_indices: HashMap<String, HashSet<usize>> = HashMap::new();
    for (i, o) in objects.iter().enumerate() {
        let s = o.name.to_ascii_lowercase();
        for t in make_trigrams(&s) {
            trigram_to_indices.entry(t).or_default().insert(i);
        }
        for b in make_bigrams(&s) {
            bigram_to_indices.entry(b).or_default().insert(i);
        }
        for u in make_unigrams(&s) {
            unigram_to_indices.entry(u).or_default().insert(i);
        }
    }
    NameReverseIndex {
        trigram_to_indices,
        bigram_to_indices,
        unigram_to_indices,
    }
}

fn candidate_indices_from_reverse_index(
    index: &NameReverseIndex,
    q_lower: &str,
) -> Option<HashSet<usize>> {
    let chars_count = q_lower.chars().count();
    let tokens: Vec<String> = if chars_count >= 3 {
        make_trigrams(q_lower)
    } else if chars_count >= 2 {
        make_bigrams(q_lower)
    } else if chars_count >= 1 {
        make_unigrams(q_lower)
    } else {
        return None;
    };
    if tokens.is_empty() {
        return None;
    }
    let mut candidate_set: Option<HashSet<usize>> = None;
    let map = if chars_count >= 3 {
        &index.trigram_to_indices
    } else if chars_count >= 2 {
        &index.bigram_to_indices
    } else {
        &index.unigram_to_indices
    };
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

    // Phase 1: fast in-memory scan using the ignore-based walker.
    let root_for_scan = path_buf.clone();
    let app_for_scan = app.clone();
    let (result, files) = match tauri::async_runtime::spawn_blocking(move || {
        let files =
            cutest_disk_tree::index_directory_ignore_with_progress(&root_for_scan, |p| {
                let _ = app_for_scan.emit("scan-progress", &p);
            });
        // For the immediate response, we don't include recursive folder sizes yet.
        let empty_folder_sizes: std::collections::HashMap<std::path::PathBuf, u64> =
            std::collections::HashMap::new();
        let result =
            cutest_disk_tree::to_scan_result(&root_for_scan, &files, &empty_folder_sizes)
                .ok_or_else(|| "Indexing failed".to_string())?;
        Ok::<_, String>((result, files))
    })
    .await
    {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => {
            write_debug_log(&state, &format!("error scan_directory: {}", e));
            return Err(e);
        }
        Err(e) => {
            write_debug_log(&state, &format!("error scan_directory spawn: {}", e));
            return Err(e.to_string());
        }
    };

    // Phase 1b: build active in-memory indexes for this root from the scan results.
    {
        // Build DiskObjects for files only at this stage.
        let mut disk_objs: Vec<DiskObject> = files
            .iter()
            .map(|f| {
                let path_string = f.path.to_string_lossy().to_string();
                let path_lower = path_string.to_ascii_lowercase();
                let parent = cutest_disk_tree::parent_dir(&path_string);
                let name = std::path::Path::new(&path_string)
                    .file_name()
                    .and_then(|os| os.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| path_string.clone());
                let name_lower = name.to_ascii_lowercase();
                let ext = std::path::Path::new(&path_string)
                    .extension()
                    .and_then(|os| os.to_str())
                    .map(|s| s.to_ascii_lowercase());
                DiskObject {
                    root: path.clone(),
                    path: path_string.clone(),
                    path_lower,
                    parent_path: if parent.is_empty() { None } else { Some(parent) },
                    name: name.clone(),
                    name_lower,
                    ext,
                    kind: DiskObjectKind::File,
                    size: Some(f.size),
                    recursive_size: None,
                    dev: Some(f.file_key.dev),
                    ino: Some(f.file_key.ino),
                    mtime: f.mtime,
                }
            })
            .collect();
        // Keep disk_objects lexicographically sorted by path.
        disk_objs.sort_by(|a, b| a.path.cmp(&b.path));
        let name_index = build_name_reverse_index(&disk_objs);

        let mut disk_map = state.disk_objects.lock().unwrap();
        disk_map.insert(path.clone(), Arc::new(disk_objs));
        let mut rev_map = state.name_reverse_index.lock().unwrap();
        rev_map.insert(path.clone(), Arc::new(name_index));
    }

    // Phase 2: in the background, compute recursive folder sizes, write everything to SQLite,
    // and then swap the updated folder index into active memory.
    {
        let db_path_bg = db_path.clone();
        let path_for_db_bg = path_for_db.clone();
        let root_bg = path_buf.clone();
        let files_bg = files.clone();
        let app_bg = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            use std::time::Instant;

            let total_start = Instant::now();
            let folder_sizes_start = Instant::now();
            let folder_sizes = cutest_disk_tree::compute_folder_sizes(&root_bg, &files_bg);
            let folder_sizes_ms = folder_sizes_start.elapsed().as_millis();

            // Update in-memory disk objects and name reverse index for this root to include folders.
            let in_memory_start = Instant::now();
            let state_ptr: tauri::State<AppState> = app_bg.state();
            write_debug_log(
                &state_ptr,
                &format!(
                    "scan_directory phase2 backfill start root={} files={} folder_entries={} compute_folder_sizes_ms={}",
                    path_for_db_bg,
                    files_bg.len(),
                    folder_sizes.len(),
                    folder_sizes_ms,
                ),
            );

            // Build a new DiskObject Vec and NameReverseIndex off to the side,
            // then swap Arc pointers into AppState to minimize lock contention.
            let existing_arc_opt = {
                let disk_map = state_ptr.disk_objects.lock().unwrap();
                disk_map.get(&path_for_db_bg).cloned()
            };

            if let Some(existing_arc) = existing_arc_opt {
                let mut new_objs: Vec<DiskObject> = (*existing_arc).clone();

                for (p, s) in &folder_sizes {
                    let path_string = p.to_string_lossy().to_string();
                    if new_objs.iter().any(|o| o.path == path_string) {
                        continue;
                    }
                    let path_lower = path_string.to_ascii_lowercase();
                    let parent = cutest_disk_tree::parent_dir(&path_string);
                    let name = std::path::Path::new(&path_string)
                        .file_name()
                        .and_then(|os| os.to_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| path_string.clone());
                    let name_lower = name.to_ascii_lowercase();
                    new_objs.push(DiskObject {
                        root: path_for_db_bg.clone(),
                        path: path_string.clone(),
                        path_lower,
                        parent_path: if parent.is_empty() { None } else { Some(parent) },
                        name,
                        name_lower,
                        ext: None,
                        kind: DiskObjectKind::Folder,
                        size: None,
                        recursive_size: Some(*s),
                        dev: None,
                        ino: None,
                        mtime: None,
                    });
                }
                new_objs.sort_by(|a, b| a.path.cmp(&b.path));

                let name_index = build_name_reverse_index(&new_objs);

                let new_objs_arc = Arc::new(new_objs);
                let name_index_arc = Arc::new(name_index);

                {
                    let mut disk_map = state_ptr.disk_objects.lock().unwrap();
                    disk_map.insert(path_for_db_bg.clone(), new_objs_arc);
                    let mut rev_map = state_ptr.name_reverse_index.lock().unwrap();
                    rev_map.insert(path_for_db_bg.clone(), name_index_arc);
                }
            }
            let in_memory_ms = in_memory_start.elapsed().as_millis();

            // Persist to SQLite in the background.
            let db_open_start = Instant::now();
            let conn = match db::open_db(&db_path_bg) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("background db open failed: {}", e);
                    write_debug_log(
                        &state_ptr,
                        &format!(
                            "scan_directory phase2 db_open_failed root={} error={} total_ms={}",
                            path_for_db_bg,
                            e,
                            total_start.elapsed().as_millis(),
                        ),
                    );
                    return;
                }
            };
            let db_open_ms = db_open_start.elapsed().as_millis();

            let db_write_start = Instant::now();
            let write_result = db::write_scan(&conn, &path_for_db_bg, &files_bg, &folder_sizes);
            let db_write_ms = db_write_start.elapsed().as_millis();
            let total_ms = total_start.elapsed().as_millis();

            match write_result {
                Ok(()) => {
                    write_debug_log(
                        &state_ptr,
                        &format!(
                            "scan_directory phase2 done root={} files={} folder_entries={} compute_folder_sizes_ms={} in_memory_ms={} db_open_ms={} db_write_ms={} total_ms={}",
                            path_for_db_bg,
                            files_bg.len(),
                            folder_sizes.len(),
                            folder_sizes_ms,
                            in_memory_ms,
                            db_open_ms,
                            db_write_ms,
                            total_ms,
                        ),
                    );
                }
                Err(e) => {
                    eprintln!("background write_scan failed: {}", e);
                    write_debug_log(
                        &state_ptr,
                        &format!(
                            "scan_directory phase2 db_write_failed root={} error={} compute_folder_sizes_ms={} in_memory_ms={} db_open_ms={} db_write_ms={} total_ms={}",
                            path_for_db_bg,
                            e,
                            folder_sizes_ms,
                            in_memory_ms,
                            db_open_ms,
                            db_write_ms,
                            total_ms,
                        ),
                    );
                }
            }
        });
    }

    write_debug_log(&state, &format!("scan_directory done path={}", path));
    Ok(result)
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn build_name_reverse_index_uses_diskobject_name() {
        let objs = vec![
            DiskObject {
                root: "C:/root".to_string(),
                path: "C:/root/AbstractButton.qml".to_string(),
                path_lower: "c:/root/abstractbutton.qml".to_string(),
                parent_path: Some("C:/root".to_string()),
                name: "AbstractButton.qml".to_string(),
                name_lower: "abstractbutton.qml".to_string(),
                ext: Some("qml".to_string()),
                kind: DiskObjectKind::File,
                size: Some(0),
                recursive_size: None,
                dev: Some(1),
                ino: Some(1),
                mtime: None,
            },
            DiskObject {
                root: "C:/root".to_string(),
                path: "C:/root/Button.txt".to_string(),
                path_lower: "c:/root/button.txt".to_string(),
                parent_path: Some("C:/root".to_string()),
                name: "Button.txt".to_string(),
                name_lower: "button.txt".to_string(),
                ext: Some("txt".to_string()),
                kind: DiskObjectKind::File,
                size: Some(0),
                recursive_size: None,
                dev: Some(1),
                ino: Some(2),
                mtime: None,
            },
        ];

        let index = build_name_reverse_index(&objs);
        let candidates = candidate_indices_from_reverse_index(&index, "button")
            .expect("should return some candidates");

        // Both names contain "button" (case-insensitive) so both should be candidates.
        assert!(candidates.contains(&0));
        assert!(candidates.contains(&1));
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

    let candidate_set_opt = {
        let guard = state.name_reverse_index.lock().unwrap();
        guard
            .get(&root)
            .and_then(|idx| candidate_indices_from_reverse_index(idx.as_ref(), &q))
    };

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
            let mut name_reverse_index_map: HashMap<String, Arc<NameReverseIndex>> = HashMap::new();

            let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;
            let roots = db::list_roots(&conn).map_err(|e| e.to_string())?;
            for root in roots {
                let mut objs = db::get_disk_objects_for_root(&conn, &root).unwrap_or_default();
                // Ensure deterministic lexicographic order in memory.
                objs.sort_by(|a, b| a.path.cmp(&b.path));
                let name_index = build_name_reverse_index(&objs);
                disk_objects_map.insert(root.clone(), Arc::new(objs));
                name_reverse_index_map.insert(root.clone(), Arc::new(name_index));
            }

            app.manage(AppState {
                db_path,
                debug_log: Mutex::new(Some(debug_log_path)),
                disk_objects: Mutex::new(disk_objects_map),
                name_reverse_index: Mutex::new(name_reverse_index_map),
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
