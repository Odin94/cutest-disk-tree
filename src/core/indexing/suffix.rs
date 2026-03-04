use std::collections::HashSet;
use std::time::Instant;

use suffix::SuffixTable;

use crate::{DiskObject, DiskObjectKind};

#[derive(Clone)]
pub struct SuffixIndex {
    pub st: SuffixTable<'static, 'static>,
    pub offsets: Vec<usize>,
    pub disk_object_indices: Vec<usize>,
    pub buffer: String,
}

pub fn build_suffix_index(objects: &[DiskObject]) -> (SuffixIndex, u128, u128) {
    let concat_start = Instant::now();
    let mut buffer = String::with_capacity(objects.len() * 16);
    let mut offsets: Vec<usize> = Vec::with_capacity(objects.len());
    let mut disk_object_indices: Vec<usize> = Vec::with_capacity(objects.len());

    for (i, o) in objects.iter().enumerate() {
        if !matches!(o.kind, DiskObjectKind::File) {
            continue;
        }
        offsets.push(buffer.len());
        disk_object_indices.push(i);
        buffer.push_str(&o.name_lower);
        buffer.push('\0');
    }
    let concat_ms = concat_start.elapsed().as_millis();

    let table_start = Instant::now();
    let st = SuffixTable::new(buffer.clone());
    let table_ms = table_start.elapsed().as_millis();

    (
        SuffixIndex {
            st,
            offsets,
            disk_object_indices,
            buffer,
        },
        concat_ms,
        table_ms,
    )
}

pub fn search_suffix_index(index: &SuffixIndex, q_lower: &str) -> Option<HashSet<usize>> {
    if q_lower.is_empty() || index.offsets.is_empty() {
        return None;
    }

    let positions = index.st.positions(q_lower);

    if positions.is_empty() {
        return Some(HashSet::new());
    }

    let mut result: HashSet<usize> = HashSet::new();
    for &pos in positions {
        let pos = pos as usize;
        let local_idx = index.offsets.partition_point(|&x| x <= pos) - 1;
        result.insert(index.disk_object_indices[local_idx]);
    }
    Some(result)
}

#[cfg(test)]
mod tests {
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
}

