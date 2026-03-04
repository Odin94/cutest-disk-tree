use super::*;
use std::collections::HashMap;

#[test]
fn build_disk_tree_creates_expected_structure() {
    let files_ser = vec![
        FileEntrySer {
            path: "/root/sub/file1".to_string(),
            size: 10,
            file_key: FileKey { dev: 1, ino: 1 },
            mtime: None,
        },
        FileEntrySer {
            path: "/root/file2".to_string(),
            size: 5,
            file_key: FileKey { dev: 1, ino: 2 },
            mtime: None,
        },
    ];

    let mut folder_sizes: HashMap<String, u64> = HashMap::new();
    folder_sizes.insert("/root".to_string(), 15);
    folder_sizes.insert("/root/sub".to_string(), 10);

    let scan = ScanResult {
        roots: vec!["/root".to_string()],
        files: files_ser,
        folder_sizes,
    };

    let (tree_opt, _timings) = build_disk_tree_timed(&scan, "/root", 10, 10);
    let tree = tree_opt.expect("tree should be built");

    fn collect_paths(node: &DiskTreeNode, out: &mut Vec<(String, u64, bool)>) {
        out.push((node.path.clone(), node.size, node.children.is_some()));
        if let Some(children) = &node.children {
            for child in children {
                collect_paths(child, out);
            }
        }
    }

    let mut all: Vec<(String, u64, bool)> = Vec::new();
    collect_paths(&tree, &mut all);

    assert!(all.iter().any(|(p, s, _)| p == "/root" && *s == 15));
    assert!(all.iter().any(|(p, s, _)| p == "/root/sub" && *s == 10));
}
