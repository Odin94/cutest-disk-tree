use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use lz4_flex::frame::{FrameDecoder, FrameEncoder};
use rayon::prelude::*;

use crate::{DiskObject, DiskObjectKind, FileEntry};
use crate::core::indexing::sqlite::SearchFilter;
use crate::parent_dir;

pub fn build_index(
    index_path: &Path,
    files: &[FileEntry],
    folder_sizes: &std::collections::HashMap<std::path::PathBuf, u64>,
) -> CompressedTextIndexResult<()> {
    write_compressed_text_index(index_path, files, folder_sizes)
}

pub fn find_files(
    index_path: &Path,
    query: &str,
    filter: &SearchFilter,
    limit: usize,
    offset: usize,
) -> CompressedTextIndexResult<(Vec<DiskObject>, bool)> {
    let (results, has_more, _) = search_compressed_text_index(index_path, query, filter, limit, offset)?;
    Ok((results, has_more))
}

const KIND_FILE: u8 = b'f';
const KIND_FOLDER: u8 = b'd';

const CTI_MAX_ENTRIES_PER_SHARD: usize = 200_000;

type CompressedTextIndexResult<T> = Result<T, CompressedTextIndexError>;

#[derive(Debug)]
pub enum CompressedTextIndexError {
    Io(std::io::Error),
    Lz4(lz4_flex::frame::Error),
    Parse(String),
}

impl From<std::io::Error> for CompressedTextIndexError {
    fn from(e: std::io::Error) -> Self {
        CompressedTextIndexError::Io(e)
    }
}

impl From<lz4_flex::frame::Error> for CompressedTextIndexError {
    fn from(e: lz4_flex::frame::Error) -> Self {
        CompressedTextIndexError::Lz4(e)
    }
}

fn basename(path: &str) -> &str {
    let sep = if path.contains('\\') { '\\' } else { '/' };
    path.rsplit(sep).next().unwrap_or(path)
}

fn ascii_case_insensitive_contains(haystack: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    let needle_bytes = needle_lower.as_bytes();
    let nlen = needle_bytes.len();
    let hay_bytes = haystack.as_bytes();
    if nlen > hay_bytes.len() {
        return false;
    }
    'outer: for i in 0..=hay_bytes.len() - nlen {
        for j in 0..nlen {
            if hay_bytes[i + j].to_ascii_lowercase() != needle_bytes[j] {
                continue 'outer;
            }
        }
        return true;
    }
    false
}

fn parse_path_line(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    // Support both the new "path only" format and the older
    // tab-separated format by always taking just the first field.
    if let Some((path, _rest)) = line.split_once('\t') {
        if path.is_empty() {
            None
        } else {
            Some(path)
        }
    } else {
        Some(line)
    }
}

fn build_disk_object_from_path(path: &str) -> DiskObject {
    let path_string = path.to_string();
    let path_lower = path_string.to_ascii_lowercase();
    let parent = parent_dir(path);
    let name_string = basename(path).to_string();
    let name_lower = name_string.to_ascii_lowercase();
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());

    DiskObject {
        path: path_string,
        path_lower,
        parent_path: if parent.is_empty() { None } else { Some(parent) },
        name: name_string,
        name_lower,
        ext,
        kind: DiskObjectKind::File,
        size: None,
        recursive_size: None,
        dev: None,
        ino: None,
        mtime: None,
    }
}

pub fn write_compressed_text_index(
    index_path: &Path,
    files: &[FileEntry],
    folder_sizes: &std::collections::HashMap<std::path::PathBuf, u64>,
) -> CompressedTextIndexResult<()> {
    let mut paths: Vec<String> = Vec::with_capacity(files.len() + folder_sizes.len());
    for f in files {
        let path_str = f.path.to_string_lossy().to_string();
        paths.push(path_str);
    }
    for (path, _size) in folder_sizes {
        let path_str = path.to_string_lossy().to_string();
        paths.push(path_str);
    }
    paths.sort();

    if paths.is_empty() {
        let file = File::create(index_path)?;
        let writer = BufWriter::new(file);
        let mut encoder = FrameEncoder::new(writer);
        encoder.finish()?;
        return Ok(());
    }

    let mut shard_index: usize = 0;
    for chunk in paths.chunks(CTI_MAX_ENTRIES_PER_SHARD) {
        let shard_path = if paths.len() <= CTI_MAX_ENTRIES_PER_SHARD {
            index_path.to_path_buf()
        } else {
            std::path::PathBuf::from(format!("{}.{}", index_path.to_string_lossy(), shard_index))
        };
        shard_index += 1;

        let file = File::create(shard_path)?;
        let writer = BufWriter::new(file);
        let mut encoder = FrameEncoder::new(writer);

        for path in chunk {
            encoder.write_all(path.as_bytes())?;
            encoder.write_all(b"\n")?;
        }
        encoder.finish()?;
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct CompressedTextIndexSearchTimings {
    pub open_ms: u128,
    pub scan_ms: u128,
    pub filter_ms: u128,
}

pub fn search_compressed_text_index(
    index_path: &Path,
    query: &str,
    _filter: &SearchFilter,
    limit: usize,
    offset: usize,
) -> CompressedTextIndexResult<(Vec<DiskObject>, bool, CompressedTextIndexSearchTimings)> {
    use std::time::Instant;

    let query_lower = query.to_ascii_lowercase();
    let query_is_empty = query_lower.is_empty();
    let global_needed = limit.saturating_add(offset).saturating_add(1);

    let open_start = Instant::now();
    let shard_paths = resolve_shard_paths(index_path);
    let open_ms = open_start.elapsed().as_millis();

    let scan_start = Instant::now();
    let per_shard_limit = global_needed;

    let shard_results: Vec<CompressedTextIndexResult<Vec<DiskObject>>> = shard_paths
        .par_iter()
        .map(|shard_path| {
            search_shard(shard_path, &query_lower, query_is_empty, per_shard_limit)
        })
        .collect();

    let mut combined: Vec<DiskObject> = Vec::new();
    for res in shard_results {
        let mut v = res?;
        combined.append(&mut v);
    }

    combined.sort_by(|a, b| a.path.cmp(&b.path));

    let has_more = combined.len() > global_needed;

    let start = offset.min(combined.len());
    let end = (start + limit).min(combined.len());
    let truncated = combined[start..end].to_vec();

    let scan_ms = scan_start.elapsed().as_millis();

    let timings = CompressedTextIndexSearchTimings {
        open_ms,
        scan_ms,
        filter_ms: 0,
    };

    Ok((truncated, has_more, timings))
}

fn resolve_shard_paths(index_path: &Path) -> Vec<std::path::PathBuf> {
    if index_path.is_file() {
        return vec![index_path.to_path_buf()];
    }
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    let mut shard_index: usize = 0;
    loop {
        let shard_path = std::path::PathBuf::from(format!(
            "{}.{}",
            index_path.to_string_lossy(),
            shard_index
        ));
        if !shard_path.is_file() {
            break;
        }
        paths.push(shard_path);
        shard_index += 1;
    }
    paths
}

fn search_shard(
    shard_path: &Path,
    query_lower: &str,
    query_is_empty: bool,
    limit: usize,
) -> CompressedTextIndexResult<Vec<DiskObject>> {
    let file = File::open(shard_path)?;
    let decoder = FrameDecoder::new(file);
    let mut reader = BufReader::new(decoder);

    let mut results: Vec<DiskObject> = Vec::with_capacity(limit);
    let mut line_buf = String::new();
    loop {
        line_buf.clear();
        let bytes_read = reader.read_line(&mut line_buf)?;
        if bytes_read == 0 {
            break;
        }
        let path = match parse_path_line(&line_buf) {
            Some(p) => p,
            None => continue,
        };
        let name = basename(path);
        if !query_is_empty && !ascii_case_insensitive_contains(name, query_lower) {
            continue;
        }
        results.push(build_disk_object_from_path(path));
        if results.len() >= limit {
            break;
        }
    }
    Ok(results)
}

pub fn compressed_text_index_exists(index_path: &Path) -> bool {
    if index_path.is_file() {
        return true;
    }
    let first_shard = std::path::PathBuf::from(format!("{}.0", index_path.to_string_lossy()));
    first_shard.is_file()
}

pub fn write_scan_metadata(
    metadata_path: &Path,
    roots: &[String],
    files_count: u64,
    folder_sizes: &std::collections::HashMap<std::path::PathBuf, u64>,
) -> std::io::Result<()> {
    let folder_sizes_ser: std::collections::HashMap<String, u64> = folder_sizes
        .iter()
        .map(|(k, v)| (k.to_string_lossy().to_string(), *v))
        .collect();
    let summary = crate::ScanSummary {
        roots: roots.to_vec(),
        files_count,
        folders_count: folder_sizes.len() as u64,
        folder_sizes: folder_sizes_ser,
    };
    let json = serde_json::to_string_pretty(&summary).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(metadata_path, json)
}

pub fn read_scan_metadata(metadata_path: &Path) -> std::io::Result<Option<crate::ScanSummary>> {
    let contents = match std::fs::read_to_string(metadata_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let summary: crate::ScanSummary = serde_json::from_str(&contents).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Some(summary))
}

pub fn read_scan_result_from_compressed_text_index(
    index_path: &Path,
) -> CompressedTextIndexResult<crate::ScanResult> {
    let shard_paths = resolve_shard_paths(index_path);

    let mut files: Vec<crate::FileEntrySer> = Vec::new();
    let mut folder_sizes: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    for shard_path in shard_paths {
        let file = File::open(&shard_path)?;
        let decoder = FrameDecoder::new(file);
        let mut reader = BufReader::new(decoder);

        let mut line_buf = String::new();
        loop {
            line_buf.clear();
            let bytes_read = reader.read_line(&mut line_buf)?;
            if bytes_read == 0 {
                break;
            }
            let path = match parse_path_line(&line_buf) {
                Some(p) => p,
                None => continue,
            };
            files.push(crate::FileEntrySer {
                path: path.to_string(),
                size: 0,
                file_key: crate::FileKey { dev: 0, ino: 0 },
                mtime: None,
            });
        }
    }

    let roots: Vec<String> = files
        .iter()
        .filter_map(|f| {
            let p = std::path::Path::new(&f.path);
            if p.parent().is_none() || p.parent() == Some(std::path::Path::new("")) || f.path.len() <= 3 {
                Some(f.path.clone())
            } else {
                None
            }
        })
        .collect();

    Ok(crate::ScanResult {
        roots,
        files,
        folder_sizes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileKey;
    use std::collections::HashMap;

    #[test]
    fn compressed_text_index_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.compressed-text-index.lz4");
        let files = vec![
            FileEntry {
                path: std::path::PathBuf::from("C:/root/file1.txt"),
                size: 100,
                file_key: FileKey { dev: 1, ino: 10 },
                mtime: Some(12345),
            },
            FileEntry {
                path: std::path::PathBuf::from("C:/root/sub/readme.md"),
                size: 200,
                file_key: FileKey { dev: 1, ino: 11 },
                mtime: None,
            },
        ];
        let mut folder_sizes = HashMap::new();
        folder_sizes.insert(std::path::PathBuf::from("C:/root"), 300u64);
        folder_sizes.insert(std::path::PathBuf::from("C:/root/sub"), 200u64);

        write_compressed_text_index(&path, &files, &folder_sizes).unwrap();
        assert!(compressed_text_index_exists(&path));

        let (results, has_more, _) = search_compressed_text_index(
            &path,
            "readme",
            &SearchFilter::None,
            10,
            0,
        ).unwrap();
        assert!(!has_more);
        assert_eq!(results.len(), 1);
        assert!(results[0].path.contains("readme.md"));
    }
}
