//! Collision names for file-browser copy / paste (`name - Copy`).

/// `attempt` 0 keeps `name`. Later attempts match Explorer: `stem - Copy`, then `stem - Copy (2)`.
pub fn collision_name(name: &str, is_dir: bool, attempt: u32) -> String {
    if attempt == 0 {
        return name.to_string();
    }
    let (stem, ext) = split_stem(name, is_dir);
    if attempt == 1 {
        format!("{stem} - Copy{ext}")
    } else {
        format!("{stem} - Copy ({attempt}){ext}")
    }
}

fn split_stem(name: &str, is_dir: bool) -> (String, String) {
    if is_dir {
        return (name.to_string(), String::new());
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => {
            (stem.to_string(), format!(".{ext}"))
        }
        _ => (name.to_string(), String::new()),
    }
}

/// True when `path` is `root` or a descendant. Separators are normalized to `/`.
pub fn is_same_or_descendant(root: &str, path: &str, case_insensitive: bool) -> bool {
    let root = normalize_path_key(root, case_insensitive);
    let path = normalize_path_key(path, case_insensitive);
    if root.is_empty() || path.is_empty() {
        return false;
    }
    path == root || path.starts_with(&format!("{root}/"))
}

fn normalize_path_key(path: &str, case_insensitive: bool) -> String {
    let mut out = path.replace('\\', "/");
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    if case_insensitive {
        out.to_ascii_lowercase()
    } else {
        out
    }
}
