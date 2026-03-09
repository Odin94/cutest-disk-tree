// Stub module for n-gram indexing. In-memory-n-grams mode uses suffix index as fallback until implemented.

use crate::DiskObject;

pub fn build_ngram_index(_objects: &[DiskObject]) -> NgramIndex {
    NgramIndex
}

pub fn search_ngram_index(_index: &NgramIndex, _query: &str) -> Option<std::collections::HashSet<usize>> {
    None
}

#[derive(Clone)]
pub struct NgramIndex;
