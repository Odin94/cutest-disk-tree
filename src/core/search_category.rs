pub const AUDIO: &[&str] = &["mp3", "wav", "flac", "m4a", "ogg", "aac", "opus"];
pub const DOCUMENT: &[&str] = &[
    "pdf", "txt", "md", "rtf", "doc", "docx", "odt", "xls", "xlsx", "csv",
    "ppt", "pptx",
];
pub const VIDEO: &[&str] = &["mp4", "mkv", "mov", "avi", "webm", "m4v"];
pub const IMAGE: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "heic", "bmp", "tiff", "svg"];
pub const EXECUTABLE: &[&str] = &[
    "exe", "dll", "so", "dylib", "bin", "sh", "bat", "cmd", "appimage",
];
pub const COMPRESSED: &[&str] = &["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz"];
pub const CONFIG: &[&str] = &[
    "cfg", "conf", "ini", "json", "yaml", "yml", "toml", "xml", "props",
    "properties", "rc", "config", "env",
];

pub fn extension_set(category: &str) -> Option<&'static [&'static str]> {
    let set: &[&str] = match category {
        "audio" => AUDIO,
        "document" => DOCUMENT,
        "video" => VIDEO,
        "image" => IMAGE,
        "executable" => EXECUTABLE,
        "compressed" => COMPRESSED,
        "config" => CONFIG,
        _ => return None,
    };
    Some(set)
}

pub fn all_known_extensions() -> Vec<&'static str> {
    AUDIO
        .iter()
        .chain(DOCUMENT)
        .chain(VIDEO)
        .chain(IMAGE)
        .chain(EXECUTABLE)
        .chain(COMPRESSED)
        .chain(CONFIG)
        .copied()
        .collect()
}
