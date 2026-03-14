//! Trigram index for substring search.
//!
//! # How it works
//!
//! **Build**: For each object's lowercased filename, extract every consecutive 3-byte window
//! (a *trigram*) and record the object's index in that trigram's posting list.  Posting lists
//! are built in object-index order so they are naturally sorted — no sort pass needed.
//!
//! **Search (query ≥ 3 chars)**: Extract the unique trigrams in the lowercased query.  If *any*
//! trigram has an empty posting list, there are zero matches and we return early.  Otherwise
//! intersect all posting lists (iterating the shortest one, binary-searching in each other),
//! then verify each surviving candidate with a real `str::contains` call to eliminate the small
//! number of false positives that can arise when query trigrams appear individually in a name but
//! not in the required sequence.
//!
//! **Search (query < 3 chars)**: Trigrams don't cover sub-3-char patterns, so fall back to a
//! linear scan with early termination.
//!
//! **Search (empty query)**: Return the first `limit` objects directly, O(1) slice.

use std::collections::HashMap;

use crate::DiskObject;
use crate::core::indexing::sqlite::SearchFilter;

// ── Internal helpers ────────────────────────────────────────────────────────

#[inline]
fn pack(a: u8, b: u8, c: u8) -> u32 {
    ((a as u32) << 16) | ((b as u32) << 8) | (c as u32)
}

// ── Public types ────────────────────────────────────────────────────────────

pub struct TrigramIndex {
    /// All indexed objects, in the order they were inserted.
    pub objects: Vec<DiskObject>,
    /// trigram → sorted list of object indices (u32 to halve storage vs usize on 64-bit).
    /// Sorted because objects are inserted in index order during build.
    pub map: HashMap<u32, Vec<u32>>,
}

impl TrigramIndex {
    /// Approximate heap bytes used by this index.
    ///
    /// Accounts for the DiskObject vector (fixed struct size + all heap String data) and the
    /// HashMap (overhead buckets + Vec<u32> posting-list data).
    pub fn size_bytes(&self) -> usize {
        let obj_fixed = std::mem::size_of::<DiskObject>() * self.objects.len();
        let obj_heap: usize = self.objects.iter().map(|o| {
            o.path.len()
                + o.path_lower.len()
                + o.name.len()
                + o.name_lower.len()
                + o.parent_path.as_ref().map_or(0, |s| s.len())
                + o.ext.as_ref().map_or(0, |s| s.len())
        }).sum();
        // HashMap overhead: per-bucket cost (key + Vec header + hash/pointer).
        let map_overhead = self.map.capacity() * (4 + std::mem::size_of::<Vec<u32>>() + 8);
        let map_data: usize = self.map.values().map(|v| v.len() * 4).sum();
        obj_fixed + obj_heap + map_overhead + map_data
    }
}

// ── Entry points ────────────────────────────────────────────────────────────

pub fn build_index(objects: &[DiskObject]) -> TrigramIndex {
    let mut map: HashMap<u32, Vec<u32>> = HashMap::new();

    // Per-name dedup buffer: avoids pushing the same object index twice into one posting list
    // when a trigram appears more than once in the same filename.  Filenames are short enough
    // that a small Vec + linear search is cheaper than a HashSet.
    let mut seen: Vec<u32> = Vec::with_capacity(64);

    for (idx, obj) in objects.iter().enumerate() {
        let name = obj.name_lower.as_bytes();
        if name.len() < 3 {
            continue;
        }
        seen.clear();
        for i in 0..=(name.len() - 3) {
            let tri = pack(name[i], name[i + 1], name[i + 2]);
            if !seen.contains(&tri) {
                seen.push(tri);
                // Objects are enumerated in order → posting lists stay sorted automatically.
                map.entry(tri).or_default().push(idx as u32);
            }
        }
    }

    TrigramIndex { objects: objects.to_vec(), map }
}

pub fn find_files(
    index: &TrigramIndex,
    query: &str,
    _filter: &SearchFilter,
    limit: usize,
    offset: usize,
) -> (Vec<DiskObject>, bool) {
    let query_lower = query.to_ascii_lowercase();

    // Empty query: return a direct slice — no filtering needed
    if query_lower.is_empty() {
        let has_more = index.objects.len() > offset + limit;
        let s = offset.min(index.objects.len());
        let e = (s + limit).min(index.objects.len());
        return (index.objects[s..e].to_vec(), has_more);
    }

    let qb = query_lower.as_bytes();
    let global_needed = limit.saturating_add(offset).saturating_add(1);

    // Short query (< 3 chars): trigrams don't apply — linear scan with early termination
    if qb.len() < 3 {
        let mut candidates: Vec<u32> = Vec::new();
        for (i, obj) in index.objects.iter().enumerate() {
            if obj.name_lower.contains(query_lower.as_str()) {
                candidates.push(i as u32);
                if candidates.len() >= global_needed {
                    break;
                }
            }
        }
        let has_more = candidates.len() >= global_needed;
        let s = offset.min(candidates.len());
        let e = (s + limit).min(candidates.len());
        return (
            candidates[s..e].iter().map(|&i| index.objects[i as usize].clone()).collect(),
            has_more,
        );
    }

    // Trigram query: collect unique query trigrams, look up posting lists, intersect
    let mut query_trigrams: Vec<u32> = Vec::with_capacity(qb.len() - 2);
    for i in 0..=(qb.len() - 3) {
        let tri = pack(qb[i], qb[i + 1], qb[i + 2]);
        if !query_trigrams.contains(&tri) {
            query_trigrams.push(tri);
        }
    }

    // If any trigram has no posting list, there cannot be any matches
    let mut lists: Vec<&[u32]> = Vec::with_capacity(query_trigrams.len());
    for tri in &query_trigrams {
        match index.map.get(tri) {
            Some(v) => lists.push(v.as_slice()),
            None => return (vec![], false),
        }
    }

    // Sort by posting-list length so we iterate the shortest list (fewest candidates)
    lists.sort_unstable_by_key(|l| l.len());
    let (first, rest) = lists.split_first().unwrap();

    // Intersect and verify.
    // Verification with `str::contains` eliminates the false positives that arise when a name
    // contains all the query trigrams individually but not in the right order/sequence
    // (e.g. query "abc" trigram found in "xaxbxc" which has 'a','b','c' but not "abc").
    let candidates: Vec<u32> = first
        .iter()
        .copied()
        .filter(|&idx| rest.iter().all(|list| list.binary_search(&idx).is_ok()))
        .filter(|&idx| {
            index.objects[idx as usize]
                .name_lower
                .contains(query_lower.as_str())
        })
        .collect();

    let has_more = candidates.len() > offset + limit;
    let s = offset.min(candidates.len());
    let e = (s + limit).min(candidates.len());
    (
        candidates[s..e].iter().map(|&i| index.objects[i as usize].clone()).collect(),
        has_more,
    )
}

// ── Backward-compatible stubs (used by the Tauri app's InMemoryNgrams mode) ─

/// Alias kept for compatibility with the Tauri host crate.
pub type NgramIndex = TrigramIndex;

pub fn build_ngram_index(objects: &[DiskObject]) -> TrigramIndex {
    build_index(objects)
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn exact_substring_match() {
        let objs = vec![make_file("readme.md"), make_file("main.rs"), make_file("Cargo.toml")];
        let idx = build_index(&objs);
        let (results, _) = find_files(&idx, "main", &SearchFilter::None, 10, 0);
        assert_eq!(results.len(), 1);
        assert!(results[0].name.contains("main"));
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
        assert_ne!(page1[0].name, page2[0].name);
    }
}
