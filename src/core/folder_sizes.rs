use std::collections::HashMap;
use std::path::PathBuf;

use rayon::prelude::*;

use crate::{FileEntry, FileKey};

pub fn aggregate_folder_sizes(
    root: &std::path::Path,
    files: &[FileEntry],
) -> HashMap<PathBuf, u64> {
    let root_len = root.as_os_str().len();

    let mut seen: std::collections::HashSet<FileKey> = std::collections::HashSet::with_capacity(files.len());
    let unique: Vec<&FileEntry> = files
        .iter()
        .filter(|e| seen.insert(e.file_key))
        .collect();

    let (root_size, mut folder_sizes) = unique
        .par_iter()
        .fold(
            || (0u64, HashMap::<PathBuf, u64>::new()),
            |mut acc, entry| {
                let (rs, map) = &mut acc;
                *rs += entry.size;
                let mut a = entry.path.parent();
                while let Some(anc) = a {
                    if anc.as_os_str().len() <= root_len {
                        break;
                    }
                    *map.entry(anc.to_path_buf()).or_insert(0) += entry.size;
                    a = anc.parent();
                }
                acc
            },
        )
        .reduce(
            || (0u64, HashMap::new()),
            |(r1, mut m1), (r2, m2)| {
                for (k, v) in m2 {
                    *m1.entry(k).or_insert(0) += v;
                }
                (r1 + r2, m1)
            },
        );

    folder_sizes.insert(root.to_path_buf(), root_size);
    folder_sizes
}

pub fn compute_folder_sizes(
    root: &std::path::Path,
    files: &[FileEntry],
) -> HashMap<PathBuf, u64> {
    aggregate_folder_sizes(root, files)
}

