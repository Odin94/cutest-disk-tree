use super::*;

#[test]
fn search_entry_from_disk_object_maps_file_and_folder() {
    let file = DiskObject {
        path: "C:/root/file.txt".to_string(),
        path_lower: "c:/root/file.txt".to_string(),
        parent_path: Some("C:/root".to_string()),
        name: "file.txt".to_string(),
        name_lower: "file.txt".to_string(),
        ext: Some("txt".to_string()),
        kind: DiskObjectKind::File,
        size: Some(10),
        recursive_size: None,
        dev: Some(1),
        ino: Some(2),
        mtime: None,
    };
    let folder = DiskObject {
        path: "C:/root/folder".to_string(),
        path_lower: "c:/root/folder".to_string(),
        parent_path: Some("C:/root".to_string()),
        name: "folder".to_string(),
        name_lower: "folder".to_string(),
        ext: None,
        kind: DiskObjectKind::Folder,
        size: None,
        recursive_size: Some(20),
        dev: None,
        ino: None,
        mtime: None,
    };

    let file_entry = search_entry_from_disk_object(&file);
    assert_eq!(file_entry.path, "C:/root/file.txt");
    assert_eq!(file_entry.size, 10);
    assert_eq!(file_entry.kind, "file");
    assert!(file_entry.file_key.is_some());
    let fk = file_entry.file_key.unwrap();
    assert_eq!(fk.dev, 1);
    assert_eq!(fk.ino, 2);

    let folder_entry = search_entry_from_disk_object(&folder);
    assert_eq!(folder_entry.path, "C:/root/folder");
    assert_eq!(folder_entry.size, 20);
    assert_eq!(folder_entry.kind, "folder");
    assert!(folder_entry.file_key.is_none());
}

fn make_file(name: &str, ino: u64) -> DiskObject {
    DiskObject {
        path: format!("C:/root/{}", name),
        path_lower: format!("c:/root/{}", name.to_ascii_lowercase()),
        parent_path: Some("C:/root".to_string()),
        name: name.to_string(),
        name_lower: name.to_ascii_lowercase(),
        ext: name.rsplit('.').next().map(|e| e.to_ascii_lowercase()),
        kind: DiskObjectKind::File,
        size: Some(0),
        recursive_size: None,
        dev: Some(1),
        ino: Some(ino),
        mtime: None,
    }
}

#[test]
fn suffix_index_finds_both_names_containing_query() {
    let objs = vec![
        make_file("AbstractButton.qml", 1),
        make_file("Button.txt", 2),
    ];

    let (index, ..) = build_suffix_index(&objs);
    let candidates = search_suffix_index(&index, "button")
        .expect("should return some candidates");

    assert!(candidates.contains(&0));
    assert!(candidates.contains(&1));
}

#[test]
fn suffix_index_returns_empty_set_for_no_match() {
    let objs = vec![
        make_file("readme.md", 1),
        make_file("main.rs", 2),
    ];

    let (index, ..) = build_suffix_index(&objs);
    let candidates = search_suffix_index(&index, "zzznomatch")
        .expect("should return Some (empty set)");

    assert!(candidates.is_empty());
}

#[test]
fn suffix_index_no_bleed_across_name_boundary() {
    let objs = vec![
        make_file("file.txt", 1),
        make_file("exe.bin", 2),
    ];

    let (index, ..) = build_suffix_index(&objs);
    let candidates = search_suffix_index(&index, "lee");
    if let Some(c) = candidates {
        assert!(c.is_empty());
    }
}

#[test]
fn suffix_index_skips_folders() {
    let mut objs = vec![
        make_file("notes.txt", 1),
    ];
    objs.push(DiskObject {
        path: "C:/root/notes_folder".to_string(),
        path_lower: "c:/root/notes_folder".to_string(),
        parent_path: Some("C:/root".to_string()),
        name: "notes_folder".to_string(),
        name_lower: "notes_folder".to_string(),
        ext: None,
        kind: DiskObjectKind::Folder,
        size: None,
        recursive_size: Some(0),
        dev: None,
        ino: None,
        mtime: None,
    });

    let (index, ..) = build_suffix_index(&objs);
    let candidates = search_suffix_index(&index, "notes")
        .expect("should match the file");

    assert!(candidates.contains(&0));
    assert!(!candidates.contains(&1));
}

#[test]
fn suffix_index_empty_objects_returns_none() {
    let objs: Vec<DiskObject> = Vec::new();
    let (index, ..) = build_suffix_index(&objs);
    let result = search_suffix_index(&index, "anything");
    assert!(result.is_none());
}

#[test]
fn get_filesystem_roots_returns_at_least_one() {
    let roots = cutest_disk_tree::get_filesystem_roots();
    assert!(!roots.is_empty());
    for root in &roots {
        assert!(root.exists(), "root {:?} should exist on disk", root);
    }
}
