use super::*;

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
fn suffix_index_empty_objects_returns_none() {
    let objs: Vec<DiskObject> = Vec::new();
    let (index, ..) = build_suffix_index(&objs);
    let result = search_suffix_index(&index, "anything");
    assert!(result.is_none());
}
