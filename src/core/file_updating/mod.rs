pub mod reconciler;
pub mod watcher;

#[cfg(test)]
mod tests;

pub use reconciler::IndexReconciler;
pub use watcher::IndexWatcher;

use std::path::Path;
use crate::{DiskObject, DiskObjectKind};
use crate::core::scanning::ignore_scanner::{is_virtual_fs, is_dependencies_dir};

/// Returns `true` if `path` itself, or any of its ancestor directories, should be
/// excluded from the index — matching the same rules as the main scanner.
pub(crate) fn should_skip(path: &Path) -> bool {
    path.ancestors().any(|p| is_virtual_fs(p) || is_dependencies_dir(p))
}

/// Build a [`DiskObject`] from a live filesystem path by reading its metadata.
///
/// Returns `None` if metadata cannot be read (e.g. the file was deleted before we got here).
pub(crate) fn disk_object_from_path(path: &Path) -> Option<DiskObject> {
    let meta = std::fs::metadata(path).ok()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    let path_str = path.to_string_lossy().into_owned();
    let parent = path.parent().map(|p| p.to_string_lossy().into_owned());

    let kind = if meta.is_dir() {
        DiskObjectKind::Folder
    } else {
        DiskObjectKind::File
    };

    let ext = if kind == DiskObjectKind::File {
        path.extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
    } else {
        None
    };

    let size = if kind == DiskObjectKind::File { Some(meta.len()) } else { None };

    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    let name_lower = name.to_lowercase();
    let path_lower = path_str.to_lowercase();

    Some(DiskObject {
        path: path_str,
        path_lower,
        parent_path: parent,
        name,
        name_lower,
        ext,
        kind,
        size,
        recursive_size: None,
        // dev/ino are populated on a full rescan via file_key_from_path; not needed here.
        dev: None,
        ino: None,
        mtime,
    })
}
