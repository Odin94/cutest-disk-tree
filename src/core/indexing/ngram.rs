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
//! **Search (empty query)**: Return the first `limit` live objects, O(n) on deleted set size.

use std::collections::{HashMap, HashSet};

use nucleo::{Config, Matcher, Utf32String};
use nucleo::pattern::{Atom, AtomKind, CaseMatching, Normalization};

use crate::DiskObject;
use crate::DiskObjectKind;
use crate::core::indexing::sqlite::SearchFilter;
use crate::core::search_category;

// ── Internal helpers ────────────────────────────────────────────────────────

#[inline]
fn pack(a: u8, b: u8, c: u8) -> u32 {
    ((a as u32) << 16) | ((b as u32) << 8) | (c as u32)
}

/// Extract unique trigrams from a lowercased name into `out` (cleared first).
fn extract_trigrams(name_lower: &str, out: &mut Vec<u32>) {
    out.clear();
    let name = name_lower.as_bytes();
    if name.len() < 3 {
        return;
    }
    for i in 0..=(name.len() - 3) {
        let tri = pack(name[i], name[i + 1], name[i + 2]);
        if !out.contains(&tri) {
            out.push(tri);
        }
    }
}

/// Test whether an object passes the search filter, mirroring the SQL conditions in sqlite.rs.
fn passes_filter(obj: &DiskObject, filter: &SearchFilter) -> bool {
    match filter {
        SearchFilter::None => true,
        SearchFilter::FoldersOnly => obj.kind == DiskObjectKind::Folder,
        SearchFilter::Extensions(exts) => {
            if exts.is_empty() {
                return true;
            }
            obj.kind == DiskObjectKind::File
                && obj.ext.as_ref().map_or(false, |e| exts.contains(e))
        }
        SearchFilter::Other => {
            if obj.kind != DiskObjectKind::File {
                return false;
            }
            let known = search_category::all_known_extensions();
            match &obj.ext {
                None => true,
                Some(e) => !known.contains(&e.as_str()),
            }
        }
    }
}

// ── Public types ────────────────────────────────────────────────────────────

pub struct TrigramIndex {
    /// All indexed objects, in the order they were inserted.
    pub objects: Vec<DiskObject>,
    /// trigram → sorted list of object indices (u32 to halve storage vs usize on 64-bit).
    /// Sorted because objects are inserted in index order during build.
    pub map: HashMap<u32, Vec<u32>>,
    /// Tombstoned object indices (logically deleted but not yet compacted out).
    pub deleted: HashSet<u32>,
    /// path → object index for O(1) remove().
    pub path_to_idx: HashMap<String, u32>,
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
        let deleted_overhead = self.deleted.capacity() * 4;
        // avg 32-char path + u32 idx + pointer overhead
        let path_idx_overhead = self.path_to_idx.capacity() * (40 + 4 + 8);
        obj_fixed + obj_heap + map_overhead + map_data + deleted_overhead + path_idx_overhead
    }

    /// Add a single object to the index without a full rebuild.
    ///
    /// The new entry is appended, so posting lists stay sorted automatically.
    pub fn add(&mut self, obj: DiskObject) {
        let idx = self.objects.len() as u32;
        let mut trigrams: Vec<u32> = Vec::with_capacity(64);
        extract_trigrams(&obj.name_lower, &mut trigrams);
        for tri in &trigrams {
            self.map.entry(*tri).or_default().push(idx);
        }
        self.path_to_idx.insert(obj.path.clone(), idx);
        self.objects.push(obj);
    }

    /// Tombstone an object by path. Returns true if the object was found, false otherwise.
    ///
    /// The object's posting-list entries remain in `map` until `compact` is called; they are
    /// skipped in `find_files` and `find_files_fuzzy` via the `deleted` set.
    pub fn remove(&mut self, path: &str) -> bool {
        match self.path_to_idx.remove(path) {
            Some(idx) => {
                self.deleted.insert(idx);
                true
            }
            None => false,
        }
    }

    /// Rebuild the index from all live (non-tombstoned) objects, dropping dead entries.
    ///
    /// Call after a full rescan or when the tombstone ratio exceeds ~5%.
    pub fn compact(&mut self) {
        let live: Vec<DiskObject> = self.objects.iter().enumerate()
            .filter(|(i, _)| !self.deleted.contains(&(*i as u32)))
            .map(|(_, obj)| obj.clone())
            .collect();
        *self = build_index(&live);
    }

    /// Number of live (non-tombstoned) objects in the index.
    pub fn live_count(&self) -> usize {
        self.objects.len() - self.deleted.len()
    }
}

// ── Entry points ────────────────────────────────────────────────────────────

pub fn build_index(objects: &[DiskObject]) -> TrigramIndex {
    let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut path_to_idx: HashMap<String, u32> = HashMap::with_capacity(objects.len());

    // Per-name dedup buffer: avoids pushing the same object index twice into one posting list
    // when a trigram appears more than once in the same filename.  Filenames are short enough
    // that a small Vec + linear search is cheaper than a HashSet.
    let mut seen: Vec<u32> = Vec::with_capacity(64);

    for (idx, obj) in objects.iter().enumerate() {
        let name = obj.name_lower.as_bytes();
        if name.len() >= 3 {
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
        path_to_idx.insert(obj.path.clone(), idx as u32);
    }

    TrigramIndex {
        objects: objects.to_vec(),
        map,
        deleted: HashSet::new(),
        path_to_idx,
    }
}

/// Returns matching object indices (into `index.objects`) and whether more results exist.
/// Callers build their result type directly from the index to avoid cloning full objects.
pub fn find_files(
    index: &TrigramIndex,
    query: &str,
    _filter: &SearchFilter,
    limit: usize,
    offset: usize,
) -> (Vec<u32>, bool) {
    let query_lower = query.to_ascii_lowercase();

    // Empty query: return live objects in insertion order
    if query_lower.is_empty() {
        let live: Vec<u32> = (0..index.objects.len() as u32)
            .filter(|i| !index.deleted.contains(i))
            .collect();
        let has_more = live.len() > offset + limit;
        let s = offset.min(live.len());
        let e = (s + limit).min(live.len());
        return (live[s..e].to_vec(), has_more);
    }

    let qb = query_lower.as_bytes();
    let global_needed = limit.saturating_add(offset).saturating_add(1);

    // Short query (< 3 chars): trigrams don't apply — linear scan with early termination
    if qb.len() < 3 {
        let mut candidates: Vec<u32> = Vec::new();
        for (i, obj) in index.objects.iter().enumerate() {
            if index.deleted.contains(&(i as u32)) {
                continue;
            }
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
        return (candidates[s..e].to_vec(), has_more);
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
        .filter(|&idx| !index.deleted.contains(&idx))
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
    (candidates[s..e].to_vec(), has_more)
}

/// Fuzzy search using nucleo's matcher. Returns `(object, score)` pairs sorted by score desc.
///
/// No `offset` parameter — fuzzy results are score-ranked so offset-based pagination is not
/// meaningful. Returns an empty Vec immediately if `query` is empty.
pub fn find_files_fuzzy(
    index: &TrigramIndex,
    query: &str,
    filter: &SearchFilter,
    limit: usize,
) -> Vec<(DiskObject, u32)> {
    if query.is_empty() {
        return Vec::new();
    }

    let atom = Atom::new(query, CaseMatching::Ignore, Normalization::Smart, AtomKind::Fuzzy, false);
    let mut matcher = Matcher::new(Config::DEFAULT);

    let mut scored: Vec<(u32, u32)> = Vec::new(); // (score, idx)
    for (i, obj) in index.objects.iter().enumerate() {
        let idx = i as u32;
        if index.deleted.contains(&idx) {
            continue;
        }
        if !passes_filter(obj, filter) {
            continue;
        }
        // Build the haystack once per candidate; score is the expensive step
        let haystack = Utf32String::from(obj.name_lower.as_str());
        if let Some(score) = atom.score(haystack.slice(..), &mut matcher) {
            scored.push((score as u32, idx));
        }
    }

    // Sort by score desc, then name length asc as tiebreaker (shorter names rank higher)
    scored.sort_unstable_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| {
            index.objects[a.1 as usize].name_lower.len()
                .cmp(&index.objects[b.1 as usize].name_lower.len())
        })
    });
    scored.truncate(limit);

    scored.into_iter()
        .map(|(score, idx)| (index.objects[idx as usize].clone(), score))
        .collect()
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
}
