pub mod ntfs;

/// mft-helper: stand-alone binary that reads the NTFS Master File Table for a
/// given root directory and writes the result as JSON to a temp file.
///
/// Usage:  mft-helper <root-path> <output-file>
///
/// Exit codes:
///   0  success – output file written
///   1  wrong argument count
///   2  MFT scan failed (non-NTFS, insufficient privileges, …)
///   3  failed to serialise / write output file

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct MftFileEntry {
    pub path: String,
    pub size: u64,
    pub dev: u64,
    pub ino: u64,
    pub mtime: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub struct MftScanOutput {
    pub files: Vec<MftFileEntry>,
    pub folders: Vec<String>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: mft-helper <root-path> <output-file>");
        std::process::exit(1);
    }

    let root = std::path::Path::new(&args[1]);
    let output_path = std::path::Path::new(&args[2]);

    match run_mft_scan(root) {
        Some(output) => {
            let json = match serde_json::to_string(&output) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("JSON serialisation failed: {}", e);
                    std::process::exit(3);
                }
            };
            if let Err(e) = std::fs::write(output_path, json.as_bytes()) {
                eprintln!("Failed to write output file: {}", e);
                std::process::exit(3);
            }
            std::process::exit(0);
        }
        None => {
            eprintln!("MFT scan failed");
            std::process::exit(2);
        }
    }
}

#[cfg(windows)]
fn run_mft_scan(root: &std::path::Path) -> Option<MftScanOutput> {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use usn_journal_rs::{mft::Mft, volume::Volume};

    let root_str = root.to_string_lossy();
    let drive_letter = root_str.chars().next().unwrap_or_default();
    if !drive_letter.is_ascii_alphabetic() {
        return None;
    }
    let drive_letter = drive_letter.to_ascii_uppercase();

    let volume = Volume::from_drive_letter(drive_letter).ok()?;
    let mft = Mft::new(&volume);

    let drive_prefix = format!("{}:\\", drive_letter);
    let root_lower = root.to_string_lossy().to_lowercase();

    // Pass 1: enumerate every MFT record.
    // Directories: fid -> (parent_fid, name)
    // Files: (parent_fid, name)
    let mut dir_map: HashMap<u64, (u64, OsString)> = HashMap::new();
    let mut raw_files: Vec<(u64, OsString)> = Vec::new();

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

    // Collect all folder paths under root.
    let mut folders: Vec<String> = Vec::new();
    folders.push(root.to_string_lossy().into_owned());
    for (&fid, (parent_fid, name)) in &dir_map {
        if *parent_fid == fid {
            continue; // volume root, self-referential
        }
        if let Some(path) = reconstruct_path(&dir_map, &drive_prefix, name, *parent_fid) {
            if path.to_string_lossy().to_lowercase().starts_with(&root_lower) {
                folders.push(path.to_string_lossy().into_owned());
            }
        }
    }

    // Pass 2: reconstruct full paths for files and collect metadata.
    let mut files: Vec<MftFileEntry> = Vec::new();
    for (parent_fid, name) in &raw_files {
        let full_path = match reconstruct_path(&dir_map, &drive_prefix, name, *parent_fid) {
            Some(p) => p,
            None => continue,
        };

        if !full_path.to_string_lossy().to_lowercase().starts_with(&root_lower) {
            continue;
        }

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

        let (dev, ino) = file_id(&full_path).unwrap_or((0, 0));

        files.push(MftFileEntry {
            path: full_path.to_string_lossy().into_owned(),
            size,
            dev,
            ino,
            mtime,
        });
    }

    Some(MftScanOutput { files, folders })
}

#[cfg(windows)]
fn reconstruct_path(
    dir_map: &std::collections::HashMap<u64, (u64, std::ffi::OsString)>,
    drive_prefix: &str,
    name: &std::ffi::OsStr,
    parent_fid: u64,
) -> Option<std::path::PathBuf> {
    use std::ffi::OsString;
    use std::path::PathBuf;

    let mut components: Vec<OsString> = Vec::new();
    let mut current_fid = parent_fid;

    for _ in 0..256 {
        match dir_map.get(&current_fid) {
            Some((p_fid, dir_name)) => {
                if *p_fid == current_fid {
                    break; // volume root
                }
                components.push(dir_name.clone());
                current_fid = *p_fid;
            }
            None => return None,
        }
    }

    let mut path = PathBuf::from(drive_prefix);
    for component in components.iter().rev() {
        path.push(component);
    }
    path.push(name);
    Some(path)
}

/// Get a (dev, ino) pair for a file on Windows via GetFileInformationByHandle.
#[cfg(windows)]
fn file_id(path: &std::path::Path) -> Option<(u64, u64)> {
    win_file_id::get_file_id(path).ok().map(|id| match id {
        win_file_id::FileId::LowRes { volume_serial_number, file_index } => {
            (volume_serial_number as u64, file_index)
        }
        win_file_id::FileId::HighRes { volume_serial_number, file_id } => {
            let ino = (file_id as u64) ^ ((file_id >> 64) as u64);
            (volume_serial_number, ino)
        }
    })
}

#[cfg(not(windows))]
fn run_mft_scan(_root: &std::path::Path) -> Option<MftScanOutput> {
    eprintln!("MFT is only available on Windows");
    std::process::exit(2);
}
