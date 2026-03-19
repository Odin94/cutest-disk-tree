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

pub fn build_index(objects: &[DiskObject]) -> SuffixIndex {
    let (index, _, _) = build_suffix_index(objects);
    index
}

pub fn find_files(index: &SuffixIndex, query: &str) -> Option<HashSet<usize>> {
    search_suffix_index(index, query)
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
mod tests;

