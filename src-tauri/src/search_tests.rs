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

    let index = suffix_build_index(&objs);
    let candidates = suffix_find_files(&index, "notes")
        .expect("should match the file");

    assert!(candidates.contains(&0));
    assert!(!candidates.contains(&1));
}

#[test]
fn get_filesystem_roots_returns_at_least_one() {
    let roots = cutest_disk_tree::get_filesystem_roots();
    assert!(!roots.is_empty());
    for root in &roots {
        assert!(root.exists(), "root {:?} should exist on disk", root);
    }
}

// ── nucleo_rank_objects tests ────────────────────────────────────────────────

#[test]
fn nucleo_rank_objects_matches_subsequence_not_substring() {
    // "ntes" is a subsequence of "notes" but not a substring — this is the kind
    // of query that the trigram pre-filter would drop (0 candidates), but nucleo
    // should still find the match.
    let objs = vec![
        make_file("notes.txt", 1),
        make_file("random.txt", 2),
        make_file("unrelated.txt", 3),
    ];
    let candidates: Vec<&DiskObject> = objs.iter().collect();
    let results = nucleo_rank_objects("ntes", candidates, 10);
    let paths: Vec<&str> = results.iter().map(|e| e.path.as_str()).collect();
    assert!(
        paths.contains(&"C:/root/notes.txt"),
        "nucleo should fuzzy-match 'ntes' to 'notes.txt', got: {:?}",
        paths
    );
}

#[test]
fn nucleo_rank_objects_better_match_ranks_higher() {
    // "note" is a stronger match for "notes.txt" than for "annotate.txt"
    // (prefix match vs mid-word). Both contain "note" as a substring, but
    // nucleo should score the closer match higher.
    let objs = vec![
        make_file("annotate.txt", 1),
        make_file("notes.txt", 2),
    ];
    let candidates: Vec<&DiskObject> = objs.iter().collect();
    let results = nucleo_rank_objects("notes", candidates, 10);
    assert!(!results.is_empty(), "should have results");
    assert_eq!(
        results[0].path, "C:/root/notes.txt",
        "exact name match should rank first, got: {:?}",
        results.iter().map(|e| &e.path).collect::<Vec<_>>()
    );
}

#[test]
fn nucleo_rank_objects_respects_limit() {
    let objs: Vec<DiskObject> = (0..20).map(|i| make_file(&format!("file{}.txt", i), i)).collect();
    let candidates: Vec<&DiskObject> = objs.iter().collect();
    let results = nucleo_rank_objects("file", candidates, 5);
    assert_eq!(results.len(), 5);
}

#[test]
fn nucleo_rank_objects_empty_candidates_returns_empty() {
    let results = nucleo_rank_objects("anything", vec![], 10);
    assert!(results.is_empty());
}

#[test]
fn nucleo_rank_objects_case_insensitive() {
    let objs = vec![
        make_file("README.md", 1),
        make_file("unrelated.txt", 2),
    ];
    let candidates: Vec<&DiskObject> = objs.iter().collect();
    let results = nucleo_rank_objects("readme", candidates, 10);
    let paths: Vec<&str> = results.iter().map(|e| e.path.as_str()).collect();
    assert!(
        paths.contains(&"C:/root/README.md"),
        "nucleo should match 'readme' to 'README.md' case-insensitively, got: {:?}",
        paths
    );
}
