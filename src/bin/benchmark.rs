//! Benchmark comparing the three search indexing strategies:
//!   suffix          – in-memory suffix array (files only, indexed by filename)
//!   sqlite          – on-disk SQLite with name_lower index (files + folders)
//!   compressed-text – on-disk LZ4-compressed path list (files + folders)
//!
//! Reads CUTE_DISK_TREE_SCAN_PATH from .env using the same resolution logic
//! as the Tauri app.  Runs 3 iterations and prints a structured report.
//!
//! Usage:
//!   cargo run --bin benchmark --release

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use cutest_disk_tree::core::indexing::compressed_text_index::{
    build_index as cti_build_index, find_files as cti_find_files,
    build_in_memory_index as cti_build_in_memory_index,
    find_files_in_memory as cti_find_files_in_memory,
    InMemoryIndex as CtiInMemoryIndex,
};
use cutest_disk_tree::core::indexing::sqlite::{find_files as sqlite_find_files, SearchFilter};
use cutest_disk_tree::core::indexing::ngram::{
    build_index as trigram_build_index, find_files as trigram_find_files, TrigramIndex,
};
use cutest_disk_tree::core::indexing::suffix::{
    build_index as suffix_build_index, find_files as suffix_find_files, SuffixIndex,
};
use cutest_disk_tree::{db, compute_folder_sizes, DiskObject, DiskObjectKind, FileEntry};

// ── Constants ──────────────────────────────────────────────────────────────

const ITERATIONS: usize = 3;
const LIMIT: usize = 500;

/// (query string, human label)
/// Chosen to cover: empty baseline, two short (<3 char) cases, one medium, two ~6-char cases.
const QUERIES: &[(&str, &str)] = &[
    ("",       "empty  "),
    ("rs",     "2-char "),
    ("txt",    "3-char "),
    ("main",   "4-char "),
    ("readme", "6-char "),
    ("config", "6-char "),
];

// ── .env loading ───────────────────────────────────────────────────────────

fn load_dotenv() {
    // Mirror the resolution order used by the Tauri app:
    //   1. workspace root  (parent of this crate's manifest dir)
    //   2. crate manifest dir
    //   3. current working directory
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates: Vec<PathBuf> = [
        manifest_dir.parent().map(|p| p.join(".env")),
        Some(manifest_dir.join(".env")),
        std::env::current_dir().ok().map(|d| d.join(".env")),
    ]
    .into_iter()
    .flatten()
    .collect();

    for path in candidates {
        if path.is_file() {
            let _ = dotenvy::from_path(&path);
            return;
        }
    }
}

// ── DiskObject construction (mirrors src-tauri/src/lib.rs) ────────────────

fn make_disk_object(path_string: String, kind: DiskObjectKind, size: Option<u64>) -> DiskObject {
    let path_lower = path_string.to_ascii_lowercase();
    let parent = cutest_disk_tree::parent_dir(&path_string);
    let name = Path::new(&path_string)
        .file_name()
        .and_then(|os| os.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path_string.clone());
    let name_lower = name.to_ascii_lowercase();
    let ext = if matches!(kind, DiskObjectKind::File) {
        Path::new(&path_string)
            .extension()
            .and_then(|os| os.to_str())
            .map(|s| s.to_ascii_lowercase())
    } else {
        None
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
        recursive_size: None,
        dev: None,
        ino: None,
        mtime: None,
    }
}

fn build_disk_objects(files: &[FileEntry], folder_paths: &HashSet<PathBuf>) -> Vec<DiskObject> {
    let mut objs = Vec::with_capacity(files.len() + folder_paths.len());
    for f in files {
        objs.push(make_disk_object(
            f.path.to_string_lossy().into_owned(),
            DiskObjectKind::File,
            Some(f.size),
        ));
    }
    for folder in folder_paths {
        objs.push(make_disk_object(
            folder.to_string_lossy().into_owned(),
            DiskObjectKind::Folder,
            None,
        ));
    }
    objs.sort_by(|a, b| a.path.cmp(&b.path));
    objs
}

// ── Result types ───────────────────────────────────────────────────────────

struct BuildResult {
    steps: Vec<(&'static str, u128)>,
    total_ms: u128,
}

struct QueryResult {
    query: &'static str,
    label: &'static str,
    find_ms: u128,
    count: usize,
}

struct IterResult {
    strategy: &'static str,
    build: BuildResult,
    queries: Vec<QueryResult>,
    /// Bytes written to disk (SQLite .db, CTI .lz4 shards). None if strategy is pure in-memory.
    disk_bytes: Option<u64>,
    /// Bytes held in RAM for the index (disk_objects + suffix array, or CTI decompressed buffer).
    /// None if strategy is entirely disk-backed.
    memory_bytes: Option<u64>,
}

// ── Resource helpers ───────────────────────────────────────────────────────

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.1} GiB", bytes as f64 / (1u64 << 30) as f64)
    } else if bytes >= 1 << 20 {
        format!("{:.1} MiB", bytes as f64 / (1u64 << 20) as f64)
    } else if bytes >= 1 << 10 {
        format!("{:.1} KiB", bytes as f64 / (1u64 << 10) as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Sum of file sizes for all CTI shards (main path + numbered shards).
fn cti_disk_bytes(cti_path: &Path, temp_dir: &Path, iteration: usize) -> u64 {
    let mut total = 0u64;
    if cti_path.is_file() {
        total += std::fs::metadata(cti_path).map(|m| m.len()).unwrap_or(0);
    }
    for i in 0.. {
        let shard = temp_dir.join(format!(
            "cutest_bench_cti_{}.lz4.{}",
            iteration, i
        ));
        if shard.is_file() {
            total += std::fs::metadata(&shard).map(|m| m.len()).unwrap_or(0);
        } else {
            break;
        }
    }
    total
}

/// Approximate heap bytes used by a slice of DiskObjects.
///
/// Accounts for the fixed struct size plus the heap-allocated String data.  Does not include
/// Vec capacity overshoot beyond len.
fn disk_objects_heap_bytes(objects: &[DiskObject]) -> u64 {
    let fixed = std::mem::size_of::<DiskObject>() * objects.len();
    let string_heap: usize = objects.iter().map(|o| {
        o.path.len()
            + o.path_lower.len()
            + o.name.len()
            + o.name_lower.len()
            + o.parent_path.as_ref().map_or(0, |s| s.len())
            + o.ext.as_ref().map_or(0, |s| s.len())
    }).sum();
    (fixed + string_heap) as u64
}

/// Approximate heap bytes used by a TrigramIndex (delegates to its own method).
fn trigram_index_heap_bytes(index: &TrigramIndex) -> u64 {
    index.size_bytes() as u64
}

/// Approximate heap bytes used by a SuffixIndex.
///
/// - `buffer` is stored in the struct AND cloned into `SuffixTable` → 2× buffer.len()
/// - The suffix array inside `SuffixTable` is a `Vec<u32>`, one entry per byte → 4× buffer.len()
/// - `offsets` and `disk_object_indices`: one `usize` (8 bytes) per file entry each
///
/// Total ≈ buffer.len() × 6  +  offsets.len() × 16
fn suffix_index_heap_bytes(index: &cutest_disk_tree::core::indexing::suffix::SuffixIndex) -> u64 {
    let buf = index.buffer.len() as u64;
    let entries = index.offsets.len() as u64;
    buf * 6 + entries * 16
}

// ── Strategy benchmarks ────────────────────────────────────────────────────

/// Suffix strategy.
///
/// Build:  construct Vec<DiskObject> from scan results, then build suffix array over filenames.
/// Search: suffix::find_files returns Option<HashSet<usize>> (indices into disk_objects).
///         We then iterate disk_objects to count matching entries – this is the same work
///         the Tauri app does to produce results and must be included for a fair comparison.
///
/// FAIRNESS NOTE: The suffix array is built over *filenames only* (folders are skipped during
/// index construction).  An empty query returns None (no candidate filter), meaning all
/// disk_objects are included in the result count.
fn bench_suffix(files: &[FileEntry], folder_paths: &HashSet<PathBuf>) -> IterResult {
    // Step 1: materialise DiskObject structs (always required to map search results back to items)
    let objs_start = Instant::now();
    let disk_objects = build_disk_objects(files, folder_paths);
    let objs_ms = objs_start.elapsed().as_millis();

    // Step 2: build suffix array index over name_lower (files only)
    let idx_start = Instant::now();
    let index: SuffixIndex = suffix_build_index(&disk_objects);
    let idx_ms = idx_start.elapsed().as_millis();

    let build = BuildResult {
        steps: vec![
            ("build_disk_objects", objs_ms),
            ("suffix::build_index", idx_ms),
        ],
        total_ms: objs_ms + idx_ms,
    };

    // Queries: find candidate index set, then iterate disk_objects to count hits
    let queries = QUERIES
        .iter()
        .map(|(q, label)| {
            let t = Instant::now();
            let candidate_set: Option<HashSet<usize>> = suffix_find_files(&index, q);

            // Collect results – same work the Tauri app does to paginate results
            let count = match &candidate_set {
                None => disk_objects.len().min(LIMIT), // empty query → no filter → all objects
                Some(cs) => {
                    let mut n = 0;
                    for i in 0..disk_objects.len() {
                        if cs.contains(&i) {
                            n += 1;
                            if n >= LIMIT {
                                break;
                            }
                        }
                    }
                    n
                }
            };
            let find_ms = t.elapsed().as_millis();
            QueryResult { query: q, label, find_ms, count }
        })
        .collect();

    let memory_bytes = disk_objects_heap_bytes(&disk_objects) + suffix_index_heap_bytes(&index);

    IterResult { strategy: "suffix", build, queries, disk_bytes: None, memory_bytes: Some(memory_bytes) }
}

/// SQLite strategy.
///
/// Build:  open a fresh temp DB (runs migrations), then write_scan which inserts all files
///         and folders and creates secondary indexes (name_lower, ext, parent_path, etc.).
/// Search: open a fresh DB connection per query (mirrors the Tauri app) and run a SQL LIKE
///         query on the indexed name_lower column.
///
/// FAIRNESS NOTE: Opening a fresh connection per query is how the app works; it adds a small
/// but real overhead compared to the in-memory strategies.  This reflects real-world cost.
fn bench_sqlite(
    files: &[FileEntry],
    folder_sizes: &HashMap<PathBuf, u64>,
    temp_dir: &Path,
    iteration: usize,
) -> IterResult {
    let db_path = temp_dir.join(format!("cutest_bench_sqlite_{}.db", iteration));
    // Remove any leftover from a previous failed run
    let _ = std::fs::remove_file(&db_path);

    // Build: open (applies migrations) + write scan + create indexes
    let open_start = Instant::now();
    let conn = db::open_db(&db_path).expect("open temp sqlite db");
    let open_ms = open_start.elapsed().as_millis();

    let write_start = Instant::now();
    let update_id = std::time::SystemTime::UNIX_EPOCH
        .elapsed()
        .unwrap()
        .as_millis() as i64;
    db::write_scan(&conn, files, folder_sizes, update_id).expect("write_scan");
    let write_ms = write_start.elapsed().as_millis();
    drop(conn);

    let build = BuildResult {
        steps: vec![
            ("db::open_db + migrations", open_ms),
            ("db::write_scan + indexes", write_ms),
        ],
        total_ms: open_ms + write_ms,
    };

    // Queries: open fresh connection per query (same as Tauri app does in find_files_in_db)
    let queries = QUERIES
        .iter()
        .map(|(q, label)| {
            let t = Instant::now();
            let fresh_conn = db::open_db(&db_path).expect("open db for query");
            let (results, _) =
                sqlite_find_files(&fresh_conn, q, &SearchFilter::None, LIMIT, 0)
                    .expect("sqlite_find_files");
            let find_ms = t.elapsed().as_millis();
            QueryResult { query: q, label, find_ms, count: results.len() }
        })
        .collect();

    let disk_bytes = std::fs::metadata(&db_path).map(|m| m.len()).ok();
    let _ = std::fs::remove_file(&db_path);
    IterResult { strategy: "sqlite", build, queries, disk_bytes, memory_bytes: None }
}

/// Compressed-text strategy.
///
/// Build:  write an LZ4-compressed newline-delimited list of all paths (files + folders)
///         to a temp file.  Multiple shard files may be created for large datasets.
/// Search: open the compressed file(s) and do a linear scan matching filenames against the
///         query.  Results are sorted and paginated.
///
/// FAIRNESS NOTE: The SearchFilter parameter is currently unused inside cti_find_files
/// (parameter is named `_filter`), so all queries effectively run without category filtering –
/// the same as SearchFilter::None for the other strategies.
fn bench_cti(
    files: &[FileEntry],
    folder_sizes: &HashMap<PathBuf, u64>,
    temp_dir: &Path,
    iteration: usize,
) -> IterResult {
    let cti_path = temp_dir.join(format!("cutest_bench_cti_{}.lz4", iteration));
    // Clean up any shards from a previous failed run
    for i in 0.. {
        let shard = temp_dir.join(format!("cutest_bench_cti_{}.lz4.{}", iteration, i));
        if shard.is_file() { let _ = std::fs::remove_file(shard); } else { break; }
    }

    // Build: write compressed path list
    let write_start = Instant::now();
    cti_build_index(&cti_path, files, folder_sizes).expect("cti_build_index");
    let write_ms = write_start.elapsed().as_millis();

    let build = BuildResult {
        steps: vec![("cti::build_index (lz4 write)", write_ms)],
        total_ms: write_ms,
    };

    // Queries: each call opens and linearly scans the compressed file(s)
    let queries = QUERIES
        .iter()
        .map(|(q, label)| {
            let t = Instant::now();
            let (results, _) =
                cti_find_files(&cti_path, q, &SearchFilter::None, LIMIT, 0)
                    .expect("cti_find_files");
            let find_ms = t.elapsed().as_millis();
            QueryResult { query: q, label, find_ms, count: results.len() }
        })
        .collect();

    // Measure disk usage before cleanup
    let disk_bytes = Some(cti_disk_bytes(&cti_path, temp_dir, iteration));

    // Clean up temp files (main + any shards)
    let _ = std::fs::remove_file(&cti_path);
    for i in 0.. {
        let shard = temp_dir.join(format!("cutest_bench_cti_{}.lz4.{}", iteration, i));
        if shard.is_file() { let _ = std::fs::remove_file(shard); } else { break; }
    }

    IterResult { strategy: "compressed-text", build, queries, disk_bytes, memory_bytes: None }
}

/// Trigram index strategy.
///
/// Build:  construct Vec<DiskObject> from scan results (same as suffix), then build a
///         HashMap<trigram, Vec<object_idx>> over lowercased filenames of all objects
///         (files + folders, unlike suffix which is files-only).
/// Search: for queries ≥ 3 chars, intersect posting lists of all query trigrams using the
///         shortest list as the probe, then verify surviving candidates with str::contains to
///         eliminate false positives.  For < 3 char queries, fall back to a linear scan.
///
/// FAIRNESS NOTE: Like suffix, the build cost includes Vec<DiskObject> construction since the
/// TrigramIndex stores the objects internally to serve results directly.  Unlike suffix, folders
/// are indexed too (consistent with sqlite and cti).
fn bench_trigram(files: &[FileEntry], folder_paths: &HashSet<PathBuf>) -> IterResult {
    // Step 1: materialise DiskObjects (needed to build and serve results)
    let objs_start = Instant::now();
    let disk_objects = build_disk_objects(files, folder_paths);
    let objs_ms = objs_start.elapsed().as_millis();

    // Step 2: build trigram posting lists
    let idx_start = Instant::now();
    let index: TrigramIndex = trigram_build_index(&disk_objects);
    let idx_ms = idx_start.elapsed().as_millis();

    let memory_bytes = trigram_index_heap_bytes(&index);

    let build = BuildResult {
        steps: vec![
            ("build_disk_objects", objs_ms),
            ("trigram::build_index", idx_ms),
        ],
        total_ms: objs_ms + idx_ms,
    };

    let queries = QUERIES
        .iter()
        .map(|(q, label)| {
            let t = Instant::now();
            let (results, _) =
                trigram_find_files(&index, q, &SearchFilter::None, LIMIT, 0);
            let find_ms = t.elapsed().as_millis();
            QueryResult { query: q, label, find_ms, count: results.len() }
        })
        .collect();

    IterResult {
        strategy: "trigram",
        build,
        queries,
        disk_bytes: None,
        memory_bytes: Some(memory_bytes),
    }
}

/// CTI-in-memory strategy.
///
/// Build:  same LZ4 write as `bench_cti`, then decompress all shards into a single heap buffer.
/// Search: linear scan of the in-memory buffer — no disk I/O per query.
///
/// disk_bytes  = compressed LZ4 file(s) written during build.
/// memory_bytes = the decompressed buffer held in RAM during search.
fn bench_cti_in_memory(
    files: &[FileEntry],
    folder_sizes: &HashMap<PathBuf, u64>,
    temp_dir: &Path,
    iteration: usize,
) -> IterResult {
    let cti_path = temp_dir.join(format!("cutest_bench_ctimem_{}.lz4", iteration));
    for i in 0.. {
        let shard = temp_dir.join(format!("cutest_bench_ctimem_{}.lz4.{}", iteration, i));
        if shard.is_file() { let _ = std::fs::remove_file(shard); } else { break; }
    }

    // Build step 1: write compressed index to disk
    let write_start = Instant::now();
    cti_build_index(&cti_path, files, folder_sizes).expect("cti_build_index (in-memory)");
    let write_ms = write_start.elapsed().as_millis();

    // Measure compressed size before loading
    let disk_bytes = Some(cti_disk_bytes(&cti_path, temp_dir, iteration));

    // Build step 2: decompress into RAM
    let load_start = Instant::now();
    let mem_index: CtiInMemoryIndex =
        cti_build_in_memory_index(&cti_path).expect("cti_build_in_memory_index");
    let load_ms = load_start.elapsed().as_millis();

    let memory_bytes = Some(mem_index.size_bytes() as u64);

    // Disk files no longer needed — clean up
    let _ = std::fs::remove_file(&cti_path);
    for i in 0.. {
        let shard = temp_dir.join(format!("cutest_bench_ctimem_{}.lz4.{}", iteration, i));
        if shard.is_file() { let _ = std::fs::remove_file(shard); } else { break; }
    }

    let build = BuildResult {
        steps: vec![
            ("cti::build_index (lz4 write)", write_ms),
            ("cti::build_in_memory_index (decompress)", load_ms),
        ],
        total_ms: write_ms + load_ms,
    };

    // Queries: pure in-memory linear scan
    let queries = QUERIES
        .iter()
        .map(|(q, label)| {
            let t = Instant::now();
            let (results, _) =
                cti_find_files_in_memory(&mem_index, q, &SearchFilter::None, LIMIT, 0)
                    .expect("cti_find_files_in_memory");
            let find_ms = t.elapsed().as_millis();
            QueryResult { query: q, label, find_ms, count: results.len() }
        })
        .collect();

    IterResult { strategy: "cti-in-memory", build, queries, disk_bytes, memory_bytes }
}

// ── Report ─────────────────────────────────────────────────────────────────

fn print_report(
    scan_path: &str,
    scan_ms: u128,
    folder_sizes_ms: u128,
    file_count: usize,
    folder_count: usize,
    all_results: &[Vec<IterResult>], // [iteration][strategy_idx]
) {
    let sep = "═".repeat(76);
    let thin = "─".repeat(76);

    println!("\n{sep}");
    println!("  INDEXING STRATEGY BENCHMARK");
    println!("{sep}");
    println!("  path        : {scan_path}");
    println!("  files       : {file_count}");
    println!("  folders     : {folder_count}");
    println!("  scan time   : {scan_ms}ms  (shared pre-step, not charged to any strategy)");
    println!("  sizes time  : {folder_sizes_ms}ms  (shared pre-step, not charged to any strategy)");
    println!("  iterations  : {ITERATIONS}");
    println!("  result limit: {LIMIT} per query");

    println!("\n  FAIRNESS NOTES");
    println!("  {}", "─".repeat(72));
    println!("  suffix  : indexes filenames of FILES ONLY (folders skipped in suffix array).");
    println!("            Build cost includes constructing Vec<DiskObject> (needed to serve");
    println!("            results).  Search returns a HashSet of indices; iterating those is");
    println!("            included in the find timing.  Pure in-memory – zero disk I/O after build.");
    println!("  trigram : indexes files+folders.  Build constructs Vec<DiskObject> + HashMap of");
    println!("            trigram→posting-list.  Search for ≥3-char queries intersects posting");
    println!("            lists (shortest first) then verifies with str::contains; <3-char queries");
    println!("            fall back to linear scan.  Pure in-memory — zero disk I/O after build.");
    println!("  sqlite  : indexes files+folders with a B-tree on name_lower.  Build writes all");
    println!("            rows and creates 6 secondary indexes.  Each query opens a fresh");
    println!("            connection (mirrors the app) – warm OS file cache after iteration 1.");
    println!("  cti     : indexes files+folders as a sorted, LZ4-compressed newline list.");
    println!("            Build is a single sequential write.  Search is an O(n) linear scan;");
    println!("            OS page cache warms up after the first query in each iteration.");
    println!("            SearchFilter is currently unused inside cti_find_files (_filter).");

    // ── Detailed per-iteration results ────────────────────────────────────
    println!("\n  DETAILED RESULTS");
    println!("  {}", "─".repeat(72));

    for (iter_idx, strategies) in all_results.iter().enumerate() {
        println!("\n  ── Iteration {} ──────────────────────────────────────────────────", iter_idx + 1);
        for res in strategies {
            let strat_w = 16;
            println!("  {:strat_w$}  BUILD", res.strategy);
            for (step, ms) in &res.build.steps {
                println!("  {:strat_w$}    {:<38} {:>7}ms", "", step, ms);
            }
            println!("  {:strat_w$}    {:<38} {:>7}ms  ← total", "", "BUILD TOTAL", res.build.total_ms);
            println!("  {:strat_w$}  SEARCH", res.strategy);
            for q in &res.queries {
                let qd = if q.query.is_empty() { "(empty)" } else { q.query };
                println!(
                    "  {:strat_w$}    {:12} [{}]  →  {:>5} results   {:>5}ms",
                    "", qd, q.label, q.count, q.find_ms
                );
            }
        }
    }

    // ── Overall averages ──────────────────────────────────────────────────
    println!("\n{thin}");
    println!("  OVERALL AVERAGES (across {ITERATIONS} iterations)");
    println!("{thin}");

    let strategy_names: Vec<&str> = all_results[0].iter().map(|r| r.strategy).collect();
    let num_queries = QUERIES.len();

    println!(
        "  {:<20} {:>12}  {:>12}  {:>12}  {:>12}",
        "Strategy", "Avg Build", "Min Build", "Avg Find", "Worst Find"
    );
    println!("  {}", "─".repeat(72));

    for (s_idx, &strategy) in strategy_names.iter().enumerate() {
        let builds: Vec<u128> = all_results.iter().map(|it| it[s_idx].build.total_ms).collect();
        let avg_build = builds.iter().sum::<u128>() / builds.len() as u128;
        let min_build = *builds.iter().min().unwrap_or(&0);

        let finds: Vec<u128> = all_results
            .iter()
            .flat_map(|it| it[s_idx].queries.iter().map(|q| q.find_ms))
            .collect();
        let avg_find = finds.iter().sum::<u128>() / finds.len() as u128;
        let worst_find = *finds.iter().max().unwrap_or(&0);

        println!(
            "  {:<20} {:>11}ms  {:>11}ms  {:>11}ms  {:>11}ms",
            strategy, avg_build, min_build, avg_find, worst_find
        );
    }

    // ── Per-query averages ────────────────────────────────────────────────
    println!("\n  PER-QUERY AVERAGE FIND TIME (ms)");
    println!("  {}", "─".repeat(72));

    // header
    print!("  {:<20}", "Strategy");
    for (q, _label) in QUERIES {
        let qd = if q.is_empty() { "(empty)" } else { q };
        print!("  {:>8}", qd);
    }
    println!();
    println!("  {}", "─".repeat(20 + 10 * num_queries));

    for (s_idx, &strategy) in strategy_names.iter().enumerate() {
        print!("  {:<20}", strategy);
        for q_idx in 0..num_queries {
            let times: Vec<u128> = all_results
                .iter()
                .map(|it| it[s_idx].queries[q_idx].find_ms)
                .collect();
            let avg = times.iter().sum::<u128>() / times.len() as u128;
            print!("  {:>8}", avg);
        }
        println!();
    }

    // ── Resource usage ────────────────────────────────────────────────────
    println!("\n{thin}");
    println!("  RESOURCE USAGE (averaged across {ITERATIONS} iterations)");
    println!("{thin}");
    println!(
        "  {:<20} {:>14}  {:>16}",
        "Strategy", "Disk (compressed)", "RAM (index)"
    );
    println!("  {}", "─".repeat(56));

    for (s_idx, &strategy) in strategy_names.iter().enumerate() {
        let avg_disk: Option<u64> = {
            let vals: Vec<u64> = all_results
                .iter()
                .filter_map(|it| it[s_idx].disk_bytes)
                .collect();
            if vals.is_empty() { None } else { Some(vals.iter().sum::<u64>() / vals.len() as u64) }
        };
        let avg_ram: Option<u64> = {
            let vals: Vec<u64> = all_results
                .iter()
                .filter_map(|it| it[s_idx].memory_bytes)
                .collect();
            if vals.is_empty() { None } else { Some(vals.iter().sum::<u64>() / vals.len() as u64) }
        };

        let disk_str = avg_disk.map(format_bytes).unwrap_or_else(|| "—".to_string());
        let ram_str  = avg_ram .map(format_bytes).unwrap_or_else(|| "—".to_string());
        println!("  {:<20} {:>14}  {:>16}", strategy, disk_str, ram_str);
    }

    // ── Summary ───────────────────────────────────────────────────────────
    println!("\n{thin}");
    println!("  SUMMARY");
    println!("{thin}");

    for (s_idx, &strategy) in strategy_names.iter().enumerate() {
        let builds: Vec<u128> = all_results.iter().map(|it| it[s_idx].build.total_ms).collect();
        let avg_build = builds.iter().sum::<u128>() / builds.len() as u128;
        let worst_build = *builds.iter().max().unwrap_or(&0);

        let finds: Vec<u128> = all_results
            .iter()
            .flat_map(|it| it[s_idx].queries.iter().map(|q| q.find_ms))
            .collect();
        let avg_find = finds.iter().sum::<u128>() / finds.len() as u128;
        let worst_find = *finds.iter().max().unwrap_or(&0);

        println!(
            "  {:<20}  build  avg={:>6}ms  worst={:>6}ms    find  avg={:>5}ms  worst={:>5}ms",
            strategy, avg_build, worst_build, avg_find, worst_find
        );
    }

    println!("\n{sep}\n");
}

// ── main ───────────────────────────────────────────────────────────────────

fn main() {
    load_dotenv();

    let scan_path_str = std::env::var("CUTE_DISK_TREE_SCAN_PATH")
        .expect("CUTE_DISK_TREE_SCAN_PATH must be set (in .env or environment)");

    let scan_root = PathBuf::from(&scan_path_str);
    if !scan_root.is_dir() {
        eprintln!("Error: '{}' is not a directory", scan_root.display());
        std::process::exit(1);
    }

    // ── Shared scan (not charged to any strategy) ─────────────────────────
    println!("Scanning '{}'...", scan_root.display());
    let scan_start = Instant::now();
    let (files_arc, folder_paths, _roots) =
        cutest_disk_tree::core::scanning::ignore_scanner::scan_roots_with_ignore(
            &[scan_root.clone()],
            |_| {},
        );
    let scan_ms = scan_start.elapsed().as_millis();

    let sizes_start = Instant::now();
    let folder_sizes = compute_folder_sizes(&scan_root, &files_arc);
    let folder_sizes_ms = sizes_start.elapsed().as_millis();

    println!(
        "Done: {} files, {} folders — scan {}ms, sizes {}ms",
        files_arc.len(),
        folder_paths.len(),
        scan_ms,
        folder_sizes_ms,
    );

    let files: &[FileEntry] = &files_arc;
    let temp_dir = std::env::temp_dir();

    // ── Iterations ────────────────────────────────────────────────────────
    let mut all_results: Vec<Vec<IterResult>> = Vec::with_capacity(ITERATIONS);

    for iter in 0..ITERATIONS {
        println!("\nIteration {} / {}...", iter + 1, ITERATIONS);

        let suffix_result    = bench_suffix(files, &folder_paths);
        let trigram_result   = bench_trigram(files, &folder_paths);
        let sqlite_result    = bench_sqlite(files, &folder_sizes, &temp_dir, iter);
        let cti_result       = bench_cti(files, &folder_sizes, &temp_dir, iter);
        let cti_mem_result   = bench_cti_in_memory(files, &folder_sizes, &temp_dir, iter);

        println!(
            "  build — suffix {}ms  trigram {}ms  sqlite {}ms  cti {}ms  cti-mem {}ms",
            suffix_result.build.total_ms,
            trigram_result.build.total_ms,
            sqlite_result.build.total_ms,
            cti_result.build.total_ms,
            cti_mem_result.build.total_ms,
        );

        all_results.push(vec![suffix_result, trigram_result, sqlite_result, cti_result, cti_mem_result]);
    }

    print_report(
        &scan_path_str,
        scan_ms,
        folder_sizes_ms,
        files.len(),
        folder_paths.len(),
        &all_results,
    );
}
