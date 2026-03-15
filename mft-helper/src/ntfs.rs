use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct FileKey {
    pub dev: u64,
    pub ino: u64,
}

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub file_key: FileKey,
    pub mtime: Option<i64>,
}

pub struct ScanProgress {
    pub files_count: u64,
    pub current_path: Option<String>,
    pub status: Option<String>,
}

const PROGRESS_INTERVAL: u64 = 5000;

#[cfg(windows)]
fn file_key_from_path(path: &Path) -> Option<FileKey> {
    win_file_id::get_file_id(path).ok().map(|id| {
        let (dev, ino) = match id {
            win_file_id::FileId::LowRes { volume_serial_number, file_index } => {
                (volume_serial_number as u64, file_index)
            }
            win_file_id::FileId::HighRes { volume_serial_number, file_id } => {
                let ino = (file_id as u64) ^ ((file_id >> 64) as u64);
                (volume_serial_number, ino)
            }
        };
        FileKey { dev, ino }
    })
}

fn aggregate_folder_sizes(root: &Path, files: &[FileEntry]) -> HashMap<PathBuf, u64> {
    let root_len = root.as_os_str().len();
    let mut seen: HashSet<FileKey> = HashSet::with_capacity(files.len());
    let unique: Vec<&FileEntry> = files.iter().filter(|e| seen.insert(e.file_key)).collect();

    let mut root_size: u64 = 0;
    let mut folder_sizes: HashMap<PathBuf, u64> = HashMap::new();

    for entry in &unique {
        root_size += entry.size;
        let mut a = entry.path.parent();
        while let Some(anc) = a {
            if anc.as_os_str().len() <= root_len {
                break;
            }
            *folder_sizes.entry(anc.to_path_buf()).or_insert(0) += entry.size;
            a = anc.parent();
        }
    }

    folder_sizes.insert(root.to_path_buf(), root_size);
    folder_sizes
}

/// Scan a directory (or whole volume) by reading the NTFS Master File Table directly,
/// returning files, pre-computed folder sizes, and the set of all folder paths found.
///
/// Requires the volume to be NTFS and the process to have sufficient privileges to open
/// the volume for reading (typically requires running as Administrator).
///
/// Returns `None` if the MFT cannot be opened (non-NTFS volume, insufficient permissions,
/// non-drive-letter path, etc.), so the caller can fall back to a normal directory walk.
#[cfg(windows)]
pub fn index_directory_ntfs_with_progress<F>(
    root: &Path,
    mut progress: F,
) -> Option<(Vec<FileEntry>, HashMap<PathBuf, u64>, HashSet<PathBuf>)>
where
    F: FnMut(ScanProgress),
{
    use std::ffi::OsString;
    use usn_journal_rs::{mft::Mft, volume::Volume};

    let root_str = root.to_string_lossy();
    let drive_letter = root_str.chars().next().unwrap_or_default();
    if !drive_letter.is_ascii_alphabetic() {
        return None;
    }
    let drive_letter = drive_letter.to_ascii_uppercase();

    let volume = match Volume::from_drive_letter(drive_letter) {
        Ok(v) => v,
        Err(e) => {
            progress(ScanProgress {
                files_count: 0,
                current_path: None,
                status: Some(format!("MFT unavailable ({}), falling back to directory walk…", e)),
            });
            return None;
        }
    };

    progress(ScanProgress {
        files_count: 0,
        current_path: None,
        status: Some("Using MFT scan…".into()),
    });

    let mft = Mft::new(&volume);

    // Pass 1: enumerate every MFT record.
    //
    // For directories, store fid -> (parent_fid, name) so we can reconstruct full
    // paths without any further I/O (no OpenFileById calls needed).
    //
    // For files, just store (parent_fid, name) – we resolve the path in pass 2.
    let mut dir_map: HashMap<u64, (u64, OsString)> = HashMap::new();
    let mut raw_files: Vec<(u64, OsString)> = Vec::new(); // (parent_fid, file_name)

    for entry_res in mft.iter() {
        let entry = match entry_res {
            Ok(e) => e,
            Err(_) => continue,
        };

        if entry.is_dir() {
            dir_map.insert(entry.fid, (entry.parent_fid, entry.file_name.clone()));
        } else {
            raw_files.push((entry.parent_fid, entry.file_name.clone()));
        }
    }

    let drive_prefix = format!("{}:\\", drive_letter);
    // Normalise root for case-insensitive prefix matching (Windows is case-insensitive).
    let root_lower = root.to_string_lossy().to_lowercase();

    // Reconstruct paths for all directories under root and collect them.
    let mut folder_paths: HashSet<PathBuf> =
        reconstruct_dir_paths(&dir_map, &drive_prefix, &root_lower);

    // Always include root itself.
    folder_paths.insert(root.to_path_buf());

    progress(ScanProgress {
        files_count: 0,
        current_path: None,
        status: Some("Scanning files…".into()),
    });

    let mut files: Vec<FileEntry> = Vec::new();

    // Pass 2: reconstruct full paths for files and collect metadata.
    for (parent_fid, name) in &raw_files {
        let mut components: Vec<OsString> = Vec::new();
        let mut current_fid = *parent_fid;
        let mut path_ok = true;

        // Depth limit guards against cycles in a corrupt MFT.
        for _ in 0..256 {
            match dir_map.get(&current_fid) {
                Some((p_fid, dir_name)) => {
                    if *p_fid == current_fid {
                        // Self-referential parent: this is the volume root.
                        break;
                    }
                    components.push(dir_name.clone());
                    current_fid = *p_fid;
                }
                None => {
                    path_ok = false;
                    break;
                }
            }
        }

        if !path_ok {
            continue;
        }

        let mut full_path = PathBuf::from(&drive_prefix);
        for component in components.iter().rev() {
            full_path.push(component);
        }
        full_path.push(name);

        if !full_path
            .to_string_lossy()
            .to_lowercase()
            .starts_with(&root_lower)
        {
            continue;
        }

        // Fetch size and mtime via the OS (MFT USN records don't carry file size).
        let meta = match std::fs::metadata(&full_path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);

        let key = match file_key_from_path(&full_path) {
            Some(k) => k,
            None => continue,
        };

        files.push(FileEntry {
            path: full_path.clone(),
            size,
            file_key: key,
            mtime,
        });

        if files.len() as u64 % PROGRESS_INTERVAL == 0 {
            progress(ScanProgress {
                files_count: files.len() as u64,
                current_path: Some(full_path.to_string_lossy().to_string()),
                status: None,
            });
        }
    }

    progress(ScanProgress {
        files_count: files.len() as u64,
        current_path: None,
        status: None,
    });

    progress(ScanProgress {
        files_count: files.len() as u64,
        current_path: None,
        status: Some("Computing folder sizes…".into()),
    });

    let folder_sizes = aggregate_folder_sizes(root, &files);

    Some((files, folder_sizes, folder_paths))
}

/// Reconstruct the full path for every directory in `dir_map` that falls under
/// the given root (identified by `root_lower`, a lower-cased path prefix).
#[cfg(windows)]
fn reconstruct_dir_paths(
    dir_map: &HashMap<u64, (u64, std::ffi::OsString)>,
    drive_prefix: &str,
    root_lower: &str,
) -> HashSet<PathBuf> {
    use std::ffi::OsString;

    let mut result = HashSet::new();

    for (&fid, (parent_fid, name)) in dir_map {
        // Skip the root directory entry itself (self-referential).
        if *parent_fid == fid {
            continue;
        }

        let mut components: Vec<OsString> = vec![name.clone()];
        let mut current_fid = *parent_fid;
        let mut path_ok = true;

        for _ in 0..256 {
            match dir_map.get(&current_fid) {
                Some((p_fid, dir_name)) => {
                    if *p_fid == current_fid {
                        break; // reached volume root
                    }
                    components.push(dir_name.clone());
                    current_fid = *p_fid;
                }
                None => {
                    path_ok = false;
                    break;
                }
            }
        }

        if !path_ok {
            continue;
        }

        let mut path = PathBuf::from(drive_prefix);
        for component in components.iter().rev() {
            path.push(component);
        }

        if path
            .to_string_lossy()
            .to_lowercase()
            .starts_with(root_lower)
        {
            result.insert(path);
        }
    }

    result
}
