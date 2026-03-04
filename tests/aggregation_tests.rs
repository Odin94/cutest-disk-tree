use cutest_disk_tree::{compute_folder_sizes, FileEntry, FileKey};
use std::path::PathBuf;

fn make_entry(path: &str, size: u64, dev: u64, ino: u64) -> FileEntry {
    FileEntry {
        path: PathBuf::from(path),
        size,
        file_key: FileKey { dev, ino },
        mtime: None,
    }
}

#[test]
fn root_folder_sums_all_files() {
    let root = PathBuf::from("/data");
    let files = vec![
        make_entry("/data/a.txt", 10, 1, 1),
        make_entry("/data/b.txt", 20, 1, 2),
    ];

    let sizes = compute_folder_sizes(&root, &files);
    let root_size = sizes.get(&root).copied().unwrap_or(0);
    assert_eq!(root_size, 30);
}

#[test]
fn subfolder_sizes_correct() {
    let root = PathBuf::from("/data");
    let files = vec![
        make_entry("/data/sub/a.txt", 10, 1, 1),
        make_entry("/data/sub/deep/b.txt", 5, 1, 2),
        make_entry("/data/c.txt", 3, 1, 3),
    ];

    let sizes = compute_folder_sizes(&root, &files);
    assert_eq!(sizes.get(&root).copied().unwrap_or(0), 18);
    assert_eq!(sizes.get(&PathBuf::from("/data/sub")).copied().unwrap_or(0), 15);
    assert_eq!(sizes.get(&PathBuf::from("/data/sub/deep")).copied().unwrap_or(0), 5);
}

#[test]
fn hardlink_deduplication_via_file_key() {
    let root = PathBuf::from("/data");
    let files = vec![
        make_entry("/data/link1.txt", 100, 1, 42),
        make_entry("/data/link2.txt", 100, 1, 42),
    ];

    let sizes = compute_folder_sizes(&root, &files);
    let root_size = sizes.get(&root).copied().unwrap_or(0);
    assert_eq!(root_size, 100, "hardlinks to same file_key should be counted once");
}

#[test]
fn empty_files_list() {
    let root = PathBuf::from("/empty");
    let files: Vec<FileEntry> = Vec::new();

    let sizes = compute_folder_sizes(&root, &files);
    let root_size = sizes.get(&root).copied().unwrap_or(0);
    assert_eq!(root_size, 0);
}
