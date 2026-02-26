use cutest_disk_tree::index_directory;
use std::path::PathBuf;
use std::process::exit;

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    if !root.is_dir() {
        eprintln!("Not a directory: {}", root.display());
        exit(1);
    }

    let (files, folder_sizes) = index_directory(&root);

    let (unique_count, unique_size): (usize, u64) = {
        let mut seen = std::collections::HashSet::new();
        files.iter().fold((0, 0u64), |(n, s), (_, size, k)| {
            if seen.insert(*k) {
                (n + 1, s + size)
            } else {
                (n, s)
            }
        })
    };

    println!("Root: {}", root.display());
    println!("Total file entries: {}", files.len());
    println!("Unique files (hard links deduped): {} (total size: {})", unique_count, human_size(unique_size));
    println!();

    let mut folders: Vec<_> = folder_sizes.into_iter().collect();
    folders.sort_by(|a, b| b.1.cmp(&a.1));

    println!("Top 20 folders by recursive size:");
    for (path, size) in folders.into_iter().take(20) {
        println!("  {:>12}  {}", human_size(size), path.display());
    }
}

fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;
    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
