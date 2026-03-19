use super::*;
use crate::DiskObjectKind;

fn make_file(name: &str) -> DiskObject {
    DiskObject {
        path: format!("C:/root/{name}"),
        path_lower: format!("c:/root/{}", name.to_ascii_lowercase()),
        parent_path: Some("C:/root".to_string()),
        name: name.to_string(),
        name_lower: name.to_ascii_lowercase(),
        ext: name.rsplit('.').next().map(|e| e.to_ascii_lowercase()),
        kind: DiskObjectKind::File,
        size: Some(0),
        recursive_size: None,
        dev: None,
        ino: None,
        mtime: None,
    }
}

fn names(idx: &TrigramIndex, indices: &[u32]) -> Vec<String> {
    indices.iter().map(|&i| idx.objects[i as usize].name.clone()).collect()
}

// ── existing tests ───────────────────────────────────────────────────────

#[test]
fn exact_substring_match() {
    let objs = vec![make_file("readme.md"), make_file("main.rs"), make_file("Cargo.toml")];
    let idx = build_index(&objs);
    let (results, _) = find_files(&idx, "main", &SearchFilter::None, 10, 0);
    assert_eq!(results.len(), 1);
    assert!(idx.objects[results[0] as usize].name.contains("main"));
}

#[test]
fn no_false_positives() {
    // "cargo" appears as individual trigrams in "xcarxgoxo" but not as the full substring
    let objs = vec![make_file("readme.md"), make_file("main.rs")];
    let idx = build_index(&objs);
    let (results, _) = find_files(&idx, "cargo", &SearchFilter::None, 10, 0);
    assert!(results.is_empty());
}

#[test]
fn short_query_falls_back_to_linear_scan() {
    let objs = vec![make_file("rs_utils.txt"), make_file("main.rs"), make_file("other.py")];
    let idx = build_index(&objs);
    let (results, _) = find_files(&idx, "rs", &SearchFilter::None, 10, 0);
    assert_eq!(results.len(), 2);
}

#[test]
fn empty_query_returns_limit_objects() {
    let objs: Vec<_> = (0..10).map(|i| make_file(&format!("file{i}.txt"))).collect();
    let idx = build_index(&objs);
    let (results, has_more) = find_files(&idx, "", &SearchFilter::None, 3, 0);
    assert_eq!(results.len(), 3);
    assert!(has_more);
}

#[test]
fn case_insensitive_match() {
    let objs = vec![make_file("README.md"), make_file("main.rs")];
    let idx = build_index(&objs);
    let (results, _) = find_files(&idx, "readme", &SearchFilter::None, 10, 0);
    assert_eq!(results.len(), 1);
}

#[test]
fn pagination_offset() {
    let objs: Vec<_> = (0..5).map(|i| make_file(&format!("config{i}.toml"))).collect();
    let idx = build_index(&objs);
    let (page1, _) = find_files(&idx, "config", &SearchFilter::None, 2, 0);
    let (page2, _) = find_files(&idx, "config", &SearchFilter::None, 2, 2);
    assert_eq!(page1.len(), 2);
    assert_eq!(page2.len(), 2);
    assert_ne!(names(&idx, &page1)[0], names(&idx, &page2)[0]);
}

// ── tombstone / dynamic-update tests ────────────────────────────────────

#[test]
fn tombstoned_entry_not_in_find_files() {
    let objs = vec![make_file("alpha.txt"), make_file("beta.txt"), make_file("gamma.txt")];
    let mut idx = build_index(&objs);
    idx.remove("C:/root/beta.txt");
    let (results, _) = find_files(&idx, "beta", &SearchFilter::None, 10, 0);
    assert!(results.is_empty());
    assert_eq!(idx.live_count(), 2);
}

#[test]
fn tombstoned_entry_not_in_empty_query() {
    let objs: Vec<_> = (0..5).map(|i| make_file(&format!("file{i}.txt"))).collect();
    let mut idx = build_index(&objs);
    idx.remove("C:/root/file2.txt");
    let (results, _) = find_files(&idx, "", &SearchFilter::None, 10, 0);
    assert_eq!(results.len(), 4);
    for r in &results {
        assert!(!idx.deleted.contains(r));
    }
}

#[test]
fn tombstoned_entry_not_in_short_query() {
    let objs = vec![make_file("ax.txt"), make_file("ay.txt"), make_file("az.txt")];
    let mut idx = build_index(&objs);
    idx.remove("C:/root/ay.txt");
    let (results, _) = find_files(&idx, "a", &SearchFilter::None, 10, 0);
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|&r| idx.objects[r as usize].name != "ay.txt"));
}

#[test]
fn add_and_find() {
    let objs = vec![make_file("alpha.txt")];
    let mut idx = build_index(&objs);
    idx.add(make_file("newfile.rs"));
    let (results, _) = find_files(&idx, "newfile", &SearchFilter::None, 10, 0);
    assert_eq!(results.len(), 1);
    assert_eq!(idx.live_count(), 2);
}

#[test]
fn compact_removes_tombstones() {
    let objs = vec![make_file("alpha.txt"), make_file("beta.txt"), make_file("gamma.txt")];
    let mut idx = build_index(&objs);
    idx.remove("C:/root/beta.txt");
    idx.compact();
    assert_eq!(idx.objects.len(), 2);
    assert!(idx.deleted.is_empty());
    let (results, _) = find_files(&idx, "gamma", &SearchFilter::None, 10, 0);
    assert_eq!(results.len(), 1);
}

#[test]
fn remove_returns_false_for_unknown_path() {
    let objs = vec![make_file("alpha.txt")];
    let mut idx = build_index(&objs);
    assert!(!idx.remove("C:/root/nonexistent.txt"));
}

// ── fuzzy search tests ───────────────────────────────────────────────────

#[test]
fn fuzzy_matches_documents() {
    let objs = vec![make_file("Documents"), make_file("foobar.txt")];
    let idx = build_index(&objs);
    let results = find_files_fuzzy(&idx, "dcm", &SearchFilter::None, 10);
    let names: Vec<&str> = results.iter().map(|(o, _)| o.name.as_str()).collect();
    // "Documents" should fuzzy-match "dcm"; "foobar" likely won't
    assert!(names.contains(&"Documents"), "expected Documents in results, got {names:?}");
    // If both appear, Documents should rank first
    if results.len() > 1 {
        assert_eq!(results[0].0.name, "Documents");
    }
}

#[test]
fn fuzzy_empty_query_returns_empty() {
    let objs = vec![make_file("anything.txt")];
    let idx = build_index(&objs);
    let results = find_files_fuzzy(&idx, "", &SearchFilter::None, 10);
    assert!(results.is_empty());
}

#[test]
fn fuzzy_tombstoned_not_returned() {
    let objs = vec![make_file("alpha.txt"), make_file("alright.txt")];
    let mut idx = build_index(&objs);
    idx.remove("C:/root/alpha.txt");
    let results = find_files_fuzzy(&idx, "alp", &SearchFilter::None, 10);
    assert!(results.iter().all(|(o, _)| o.name != "alpha.txt"));
}

#[test]
fn fuzzy_limit_respected() {
    let objs: Vec<_> = (0..20).map(|i| make_file(&format!("readme{i}.md"))).collect();
    let idx = build_index(&objs);
    let results = find_files_fuzzy(&idx, "readme", &SearchFilter::None, 5);
    assert!(results.len() <= 5);
}
