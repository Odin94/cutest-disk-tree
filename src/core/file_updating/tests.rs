use super::*;
use tempfile::TempDir;

#[test]
fn file_attributes_are_read_correctly() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("hello.txt");
    std::fs::write(&path, b"contents").unwrap();

    let obj = disk_object_from_path(&path).expect("should read file");
    assert_eq!(obj.name, "hello.txt");
    assert_eq!(obj.ext.as_deref(), Some("txt"));
    assert_eq!(obj.kind, crate::DiskObjectKind::File);
    assert_eq!(obj.size, Some(8));
    assert!(obj.path.ends_with("hello.txt"));
    assert_eq!(obj.path_lower, obj.path.to_lowercase());
    assert_eq!(obj.name_lower, "hello.txt");
}

#[test]
fn folder_has_no_ext_and_no_size() {
    let dir = TempDir::new().unwrap();
    let sub = dir.path().join("mydir");
    std::fs::create_dir(&sub).unwrap();

    let obj = disk_object_from_path(&sub).expect("should read folder");
    assert_eq!(obj.kind, crate::DiskObjectKind::Folder);
    assert!(obj.ext.is_none());
    assert!(obj.size.is_none());
}

#[test]
fn returns_none_for_nonexistent_path() {
    let path = std::path::Path::new("/this/path/does/not/exist/ever.txt");
    assert!(disk_object_from_path(path).is_none());
}

#[test]
fn parent_path_is_set() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("child.rs");
    std::fs::write(&path, b"").unwrap();

    let obj = disk_object_from_path(&path).unwrap();
    assert_eq!(obj.parent_path.as_deref(), Some(dir.path().to_string_lossy().as_ref()));
}
