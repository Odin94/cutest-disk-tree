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
