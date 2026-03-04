use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{FileEntry, ScanProgress};
use crate::core::folder_sizes::aggregate_folder_sizes;
use crate::core::scanning::utils::{file_key_from_path, PROGRESS_INTERVAL};

#[cfg(windows)]
pub fn index_directory_ntfs_with_progress<F>(
    root: &Path,
    mut progress: F,
) -> Option<(Vec<FileEntry>, HashMap<PathBuf, u64>)>
where
    F: FnMut(ScanProgress),
{
    use std::ffi::OsString;
    use usn_journal_rs::{mft::Mft, volume::Volume};
    use usn_journal_rs::path::PathResolvableEntry;

    let root_str = root.to_string_lossy();
    let drive_letter = root_str.chars().next().unwrap_or_default();
    if !drive_letter.is_ascii_alphabetic() {
        return None;
    }

    let volume = match Volume::from_drive_letter(drive_letter) {
        Ok(v) => v,
        Err(_) => return None,
    };

    let mft = Mft::new(&volume);
    let mut files: Vec<FileEntry> = Vec::new();

    for entry_res in mft.iter() {
        let entry = match entry_res {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.is_dir() {
            continue;
        }

        let name: &OsString = entry.file_name();
        let name_str = name.to_string_lossy().to_string();

        let candidate_path = PathBuf::from(format!("{}:\\{}", drive_letter, name_str));
        let meta = std::fs::metadata(&candidate_path).ok();
        let (size, mtime) = if let Some(m) = meta {
            let sz = m.len();
            let mt = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            (sz, mt)
        } else {
            (0, None)
        };

        if let Some(key) = file_key_from_path(&candidate_path) {
            files.push(FileEntry {
                path: candidate_path.clone(),
                size,
                file_key: key,
                mtime,
            });
            if files.len() as u64 % PROGRESS_INTERVAL == 0 {
                progress(ScanProgress {
                    files_count: files.len() as u64,
                    current_path: Some(candidate_path.to_string_lossy().to_string()),
                    status: None,
                });
            }
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

    Some((files, folder_sizes))
}

