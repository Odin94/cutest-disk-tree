use std::path::Path;

use crate::FileKey;

pub const PROGRESS_INTERVAL: u64 = 5000;

#[cfg(unix)]
pub fn file_key_from_path(path: &Path) -> Option<FileKey> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| FileKey {
        dev: m.dev(),
        ino: m.ino(),
    })
}

#[cfg(windows)]
pub fn file_key_from_path(path: &Path) -> Option<FileKey> {
    win_file_id::get_file_id(path).ok().map(|id| {
        let (dev, ino) = match id {
            win_file_id::FileId::LowRes {
                volume_serial_number,
                file_index,
            } => (volume_serial_number as u64, file_index),
            win_file_id::FileId::HighRes {
                volume_serial_number,
                file_id,
            } => {
                let ino64 = (file_id as u64) ^ ((file_id >> 64) as u64);
                (volume_serial_number, ino64)
            }
        };
        FileKey { dev, ino }
    })
}

#[cfg(not(any(unix, windows)))]
pub fn file_key_from_path(_path: &Path) -> Option<FileKey> {
    None
}

