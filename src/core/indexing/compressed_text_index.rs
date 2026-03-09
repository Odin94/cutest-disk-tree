use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use lz4_flex::frame::{FrameDecoder, FrameEncoder};

use crate::{DiskObject, DiskObjectKind, FileEntry};
use crate::core::indexing::sqlite::SearchFilter;
use crate::parent_dir;

const KIND_FILE: u8 = b'f';
const KIND_FOLDER: u8 = b'd';

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

fn parse_line(line: &str) -> CompressedTextIndexResult<Option<DiskObject>> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let parts: Vec<&str> = line.split('\t').collect();
    if parts.len() < 3 {
        return Err(CompressedTextIndexError::Parse(format!("expected at least 3 fields, got {}", parts.len())));
    }
    let path = parts[0];
    let kind_byte = parts[1].as_bytes().first().copied();
    let size: u64 = parts[2].parse().map_err(|_| {
        CompressedTextIndexError::Parse(format!("invalid size: {}", parts[2]))
    })?;
    let kind = match kind_byte {
        Some(KIND_FILE) => DiskObjectKind::File,
        Some(KIND_FOLDER) => DiskObjectKind::Folder,
        _ => return Err(CompressedTextIndexError::Parse(format!("invalid kind: {:?}", kind_byte))),
    };
    let path_lower = path.to_ascii_lowercase();
    let parent = parent_dir(path);
    let name = basename(path).to_string();
    let name_lower = name.to_ascii_lowercase();
    let ext = match kind {
        DiskObjectKind::File => std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase()),
        DiskObjectKind::Folder => None,
    };
    let (size_opt, recursive_size_opt, dev, ino, mtime) = match kind {
        DiskObjectKind::File => {
            let dev = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            let ino = parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
            let mtime = parts.get(5).and_then(|s| s.parse().ok());
            (Some(size), None, Some(dev), Some(ino), mtime)
        }
        DiskObjectKind::Folder => (None, Some(size), None, None, None),
    };
    Ok(Some(DiskObject {
        path: path.to_string(),
        path_lower,
        parent_path: if parent.is_empty() { None } else { Some(parent) },
        name,
        name_lower,
        ext,
        kind,
        size: size_opt,
        recursive_size: recursive_size_opt,
        dev,
        ino,
        mtime,
    }))
}

fn passes_filter(obj: &DiskObject, filter: &SearchFilter) -> bool {
    match filter {
        SearchFilter::None => true,
        SearchFilter::FoldersOnly => matches!(obj.kind, DiskObjectKind::Folder),
        SearchFilter::Other => {
            if matches!(obj.kind, DiskObjectKind::Folder) {
                return false;
            }
            let known = crate::core::search_category::all_known_extensions();
            match &obj.ext {
                None => true,
                Some(ext) => !known.iter().any(|e| e.eq_ignore_ascii_case(ext)),
            }
        }
        SearchFilter::Extensions(exts) => {
            if matches!(obj.kind, DiskObjectKind::Folder) {
                return true;
            }
            obj.ext.as_ref().map(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e))).unwrap_or(false)
        }
    }
}

pub fn write_compressed_text_index(
    index_path: &Path,
    files: &[FileEntry],
    folder_sizes: &std::collections::HashMap<std::path::PathBuf, u64>,
) -> CompressedTextIndexResult<()> {
    let mut entries: Vec<(String, u8, u64, Option<u64>, Option<u64>, Option<i64>)> = Vec::with_capacity(files.len() + folder_sizes.len());
    for f in files {
        let path_str = f.path.to_string_lossy().to_string();
        entries.push((
            path_str,
            KIND_FILE,
            f.size,
            Some(f.file_key.dev),
            Some(f.file_key.ino),
            f.mtime,
        ));
    }
    for (path, &size) in folder_sizes {
        let path_str = path.to_string_lossy().to_string();
        entries.push((path_str, KIND_FOLDER, size, None, None, None));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let file = File::create(index_path)?;
    let writer = BufWriter::new(file);
    let mut encoder = FrameEncoder::new(writer);

    for (path, kind, size, dev, ino, mtime) in entries {
        let line = match (kind, dev, ino, mtime) {
            (KIND_FILE, Some(d), Some(i), m) => {
                format!("{}\tf\t{}\t{}\t{}\t{}\n", path, size, d, i, m.unwrap_or(0))
            }
            _ => format!("{}\td\t{}\n", path, size),
        };
        encoder.write_all(line.as_bytes())?;
    }
    encoder.finish()?;
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
    filter: &SearchFilter,
    limit: usize,
    offset: usize,
) -> CompressedTextIndexResult<(Vec<DiskObject>, bool, CompressedTextIndexSearchTimings)> {
    use std::time::Instant;

    let open_start = Instant::now();
    let file = File::open(index_path)?;
    let decoder = FrameDecoder::new(file);
    let reader = BufReader::new(decoder);
    let open_ms = open_start.elapsed().as_millis();

    let query_lower = query.to_ascii_lowercase();
    let mut results: Vec<DiskObject> = Vec::with_capacity(limit + 1);
    let mut skipped = 0usize;

    let scan_start = Instant::now();
    for line in reader.lines() {
        let line = line?;
        let obj = match parse_line(&line)? {
            Some(o) => o,
            None => continue,
        };
        if !obj.name_lower.contains(&query_lower) {
            continue;
        }
        if !passes_filter(&obj, filter) {
            continue;
        }
        if skipped < offset {
            skipped += 1;
            continue;
        }
        results.push(obj);
        if results.len() > limit {
            break;
        }
    }
    let scan_ms = scan_start.elapsed().as_millis();

    let has_more = results.len() > limit;
    let truncated = if has_more {
        results.into_iter().take(limit).collect()
    } else {
        results
    };

    let timings = CompressedTextIndexSearchTimings {
        open_ms,
        scan_ms,
        filter_ms: 0,
    };

    Ok((truncated, has_more, timings))
}

pub fn compressed_text_index_exists(index_path: &Path) -> bool {
    index_path.is_file()
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
    let file = File::open(index_path)?;
    let decoder = FrameDecoder::new(file);
    let reader = BufReader::new(decoder);

    let mut files: Vec<crate::FileEntrySer> = Vec::new();
    let mut folder_sizes: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    for line in reader.lines() {
        let line = line?;
        let obj = match parse_line(&line)? {
            Some(o) => o,
            None => continue,
        };
        match obj.kind {
            crate::DiskObjectKind::File => {
                files.push(crate::FileEntrySer {
                    path: obj.path,
                    size: obj.size.unwrap_or(0),
                    file_key: crate::FileKey {
                        dev: obj.dev.unwrap_or(0),
                        ino: obj.ino.unwrap_or(0),
                    },
                    mtime: obj.mtime,
                });
            }
            crate::DiskObjectKind::Folder => {
                if let Some(s) = obj.recursive_size {
                    folder_sizes.insert(obj.path, s);
                }
            }
        }
    }

    let roots: Vec<String> = folder_sizes
        .keys()
        .filter(|path| {
            let p = std::path::Path::new(path);
            p.parent().is_none() || p.parent() == Some(std::path::Path::new("")) || path.len() <= 3
        })
        .cloned()
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
