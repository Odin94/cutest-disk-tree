use cutest_disk_tree::{FileEntry, FileKey};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use walkdir::WalkDir;
use jwalk::WalkDir as JwalkDir;
use ignore::WalkBuilder;

fn main() {
    // Default to "C:/Program Files" as requested, but allow overriding via CLI arg.
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:/Program Files"));

    if !root.is_dir() {
        eprintln!("Not a directory: {}", root.display());
        std::process::exit(1);
    }

    println!("Benchmarking filesystem indexing algorithms");
    println!("Root: {}", root.display());
    println!();

    let mut log = open_log_file("benchmark_results.txt");

    // Two modes: without metadata (path + filename only) and with metadata (also file size).
    let modes = [false, true];

    // Aggregates keyed by (algo, with_metadata)
    let mut aggregates: HashMap<(String, bool), (usize, u128, usize)> = HashMap::new();

    for with_metadata in modes {
        println!(
            "=== Benchmarking with_metadata={} ({} runs) ===",
            with_metadata, 3
        );

        for iteration in 1..=3 {
            println!("--- iteration {} ---", iteration);

            let algorithms: Vec<&str> = vec![
                "serial_walkdir",
                "parallel_jwalk",
                "lolcate_ignore_parallel",
            ];

            for name in algorithms {
                println!("=== {} ===", name);

                let scan_start = Instant::now();
                let (files, folder_sizes) = match name {
                    "serial_walkdir" => {
                        let (f, _folders) = scan_walkdir(root.as_path(), with_metadata);
                        let fs = compute_folder_sizes_minimal(&root, &f);
                        (f, fs)
                    }
                    "parallel_jwalk" => {
                        let (f, _folders) = scan_jwalk(root.as_path(), with_metadata);
                        let fs = compute_folder_sizes_minimal(&root, &f);
                        (f, fs)
                    }
                    "lolcate_ignore_parallel" => {
                        let (f, _folders) = scan_ignore(root.as_path(), with_metadata);
                        let fs = compute_folder_sizes_minimal(&root, &f);
                        (f, fs)
                    }
                    _ => unreachable!(),
                };
                let scan_elapsed = scan_start.elapsed();

                let stats = compute_stats(
                    name,
                    &root,
                    &files,
                    &folder_sizes,
                    scan_elapsed,
                    with_metadata,
                    iteration,
                );

                println!(
                    "algo={} root={} with_metadata={} iter={} files={} folders={} paths_total={} scan_ms={} scan_paths_per_s={:.0}",
                    stats.algo,
                    stats.root,
                    stats.with_metadata,
                    stats.iteration,
                    stats.files,
                    stats.folders,
                    stats.paths_total,
                    stats.scan_ms,
                    stats.scan_paths_per_second,
                );
                println!();

                let _ = writeln!(
                    &mut log,
                    "{},{},{},{},{},{},{},{},{}",
                    stats.algo,
                    stats.root,
                    stats.with_metadata,
                    stats.iteration,
                    stats.files,
                    stats.folders,
                    stats.paths_total,
                    stats.scan_ms,
                    stats.scan_paths_per_second,
                );

                let key = (stats.algo.clone(), stats.with_metadata);
                let entry = aggregates.entry(key).or_insert((0, 0, 0));
                entry.0 += 1;
                entry.1 += stats.scan_ms;
                entry.2 += stats.paths_total;
            }
        }
    }

    println!("=== Averages across runs ===");
    for ((algo, with_metadata), (runs, total_ms, total_paths)) in aggregates {
        if runs == 0 || total_ms == 0 {
            continue;
        }
        let avg_ms = total_ms as f64 / runs as f64;
        let total_s = (total_ms as f64) / 1000.0;
        let paths_per_s = total_paths as f64 / total_s.max(0.000_001);
        println!(
            "algo={} with_metadata={} runs={} avg_scan_ms={:.1} avg_paths_per_s={:.0}",
            algo, with_metadata, runs, avg_ms, paths_per_s
        );
    }
}

struct Stats {
    algo: String,
    root: String,
    with_metadata: bool,
    iteration: usize,
    files: usize,
    folders: usize,
    paths_total: usize,
    scan_ms: u128,
    scan_paths_per_second: f64,
}

fn compute_stats(
    algo: &str,
    root: &Path,
    files: &[FileEntry],
    folder_sizes: &std::collections::HashMap<PathBuf, u64>,
    scan_elapsed: Duration,
    with_metadata: bool,
    iteration: usize,
) -> Stats {
    let files_count = files.len();
    let folders = folder_sizes.len();
    let paths_total = files_count + folders;

    let scan_ms = scan_elapsed.as_millis();
    let scan_s = scan_elapsed.as_secs_f64().max(0.000_001);
    let scan_paths_per_second = paths_total as f64 / scan_s;

    Stats {
        algo: algo.to_string(),
        root: root.display().to_string(),
        with_metadata,
        iteration,
        files: files_count,
        folders,
        paths_total,
        scan_ms,
        scan_paths_per_second,
    }
}

fn open_log_file(path: &str) -> std::fs::File {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("failed to open benchmark log file");

    file
}

fn scan_walkdir(root: &Path, with_metadata: bool) -> (Vec<FileEntry>, usize) {
    let mut files: Vec<FileEntry> = Vec::new();
    let mut folders: usize = 0;

    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter();

    for entry in walker.filter_map(Result::ok) {
        let ft = entry.file_type();
        let path = entry.path().to_path_buf();
        if ft.is_dir() {
            folders += 1;
            continue;
        }
        if !ft.is_file() {
            continue;
        }
        let size = if with_metadata {
            match entry.metadata() {
                Ok(m) => m.len(),
                Err(_) => continue,
            }
        } else {
            0
        };
        let idx = files.len() as u64;
        files.push(FileEntry {
            path,
            size,
            file_key: FileKey { dev: 0, ino: idx },
            mtime: None,
        });
    }

    (files, folders)
}

fn scan_jwalk(root: &Path, with_metadata: bool) -> (Vec<FileEntry>, usize) {
    let mut files: Vec<FileEntry> = Vec::new();
    let mut folders: usize = 0;

    let walk = match JwalkDir::new(root).follow_links(false).try_into_iter() {
        Ok(w) => w,
        Err(_) => return (files, folders),
    };

    for entry in walk.filter_map(Result::ok) {
        if entry.path_is_symlink() {
            continue;
        }
        let ft = entry.file_type();
        let path = entry.path();
        if ft.is_dir() {
            folders += 1;
            continue;
        }
        if !ft.is_file() {
            continue;
        }
        let size = if with_metadata {
            match entry.metadata() {
                Ok(m) => m.len(),
                Err(_) => continue,
            }
        } else {
            0
        };
        let idx = files.len() as u64;
        files.push(FileEntry {
            path: path.to_path_buf(),
            size,
            file_key: FileKey { dev: 0, ino: idx },
            mtime: None,
        });
    }

    (files, folders)
}

fn scan_ignore(root: &Path, with_metadata: bool) -> (Vec<FileEntry>, usize) {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .parents(false)
        .follow_links(false)
        .ignore(true)
        .git_global(false)
        .git_ignore(false)
        .git_exclude(false)
        .threads(4);

    let files_acc: Arc<Mutex<Vec<FileEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let folders_acc: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));

    let walk = builder.build_parallel();
    walk.run(|| {
        let files_acc = Arc::clone(&files_acc);
        let folders_acc = Arc::clone(&folders_acc);
        Box::new(move |entry| {
            use ignore::WalkState;
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return WalkState::Continue,
            };
            let ft = match entry.file_type() {
                Some(ft) => ft,
                None => return WalkState::Continue,
            };
            let path = entry.path().to_path_buf();
            if ft.is_symlink() {
                return WalkState::Continue;
            }
            if ft.is_dir() {
                let mut guard = folders_acc.lock().unwrap();
                *guard += 1;
                return WalkState::Continue;
            }
            if !ft.is_file() {
                return WalkState::Continue;
            }
            let size = if with_metadata {
                match entry.metadata() {
                    Ok(m) => m.len(),
                    Err(_) => return WalkState::Continue,
                }
            } else {
                0
            };
            let mut guard = files_acc.lock().unwrap();
            let idx = guard.len() as u64;
            guard.push(FileEntry {
                path,
                size,
                file_key: FileKey { dev: 0, ino: idx },
                mtime: None,
            });

            WalkState::Continue
        })
    });

    let files = match Arc::try_unwrap(files_acc) {
        Ok(mutex) => mutex.into_inner().unwrap(),
        Err(arc) => arc.lock().unwrap().clone(),
    };
    let folders = match Arc::try_unwrap(folders_acc) {
        Ok(mutex) => mutex.into_inner().unwrap(),
        Err(arc) => *arc.lock().unwrap(),
    };

    (files, folders)
}

fn compute_folder_sizes_minimal(
    root: &Path,
    files: &[FileEntry],
) -> HashMap<PathBuf, u64> {
    let mut folder_sizes: HashMap<PathBuf, u64> = HashMap::new();
    let root_buf = root.to_path_buf();

    for entry in files {
        let size = entry.size;
        *folder_sizes.entry(root_buf.clone()).or_insert(0) += size;

        let mut current = entry.path.clone();
        while current.pop() {
            if current.as_os_str().is_empty() {
                break;
            }
            if current == root_buf {
                continue;
            }
            if current.starts_with(root) {
                *folder_sizes.entry(current.clone()).or_insert(0) += size;
            }
        }
    }

    folder_sizes
}
