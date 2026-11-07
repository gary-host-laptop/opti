use std::path::PathBuf;

/// File extensions recognized as viewable media (mirrors opti's MEDIA_EXTENSIONS).
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jif", "jfif", "jfi", "jxr", "hdp", "wdp", "jp2", "j2k", "jpf", "jpx",
    "jpm", "mj2", "png", "apng", "gif", "agif", "webp", "mng", "tif", "tiff", "svg", "svgz", "pdf",
    "bmp", "dib", "xbm", "ico", "heif", "heic", "flac", "mp4", "mkv", "m4a", "mp3", "ogv", "ogm",
    "ogg", "oga", "opus", "webm", "wav", "amr", "avi", "3gp",
];

fn mod_idx(i: isize, n: usize) -> usize {
    ((i % n as isize) + n as isize) as usize % n
}

fn ext_of(name: &std::ffi::OsStr) -> Option<String> {
    name.to_str().map(|s| s.to_lowercase())
}

/// Populate the local file list from the directory containing `dir_or_file`.
/// Returns true on success with the sorted file list and the index of `current`.
pub fn load_directory(dir_or_file: &PathBuf) -> Option<(Vec<PathBuf>, usize)> {
    let meta = std::fs::metadata(dir_or_file).ok()?;
    let (dir, current) = if meta.is_dir() {
        (dir_or_file.clone(), None)
    } else {
        (dir_or_file.parent()?.to_path_buf(), dir_or_file.file_name().map(|f| f.to_os_string()))
    };

    let mut list: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    list.sort();

    let index = current
        .and_then(|c| list.iter().position(|p| p.file_name() == Some(&c)))
        .unwrap_or(0);

    Some((list, index))
}

/// Move to the next/previous file in the current directory.
pub fn step_file(state: &mut crate::state::ViewState, dir: isize) -> Option<PathBuf> {
    if state.local_files.is_empty() {
        return None;
    }
    let n = state.local_files.len();
    let next = mod_idx(state.file_index as isize + dir, n);
    state.file_index = next;
    Some(state.local_files[next].clone())
}

/// Move to the next/previous media file (by extension) in the current directory,
/// wrapping around.
pub fn step_media(state: &mut crate::state::ViewState, dir: isize) -> Option<PathBuf> {
    if state.local_files.len() < 2 {
        return None;
    }
    let n = state.local_files.len() as isize;
    let mut i = 1;
    while i < n {
        let candidate = mod_idx(state.file_index as isize + dir * i, state.local_files.len());
        if is_media(&state.local_files[candidate]) {
            state.file_index = candidate;
            return Some(state.local_files[candidate].clone());
        }
        i += 1;
    }
    None
}

pub fn is_media(path: &PathBuf) -> bool {
    path.extension()
        .and_then(|e| ext_of(e))
        .map(|e| MEDIA_EXTENSIONS.contains(&e.as_str()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_dir(tag: &str, files: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "opti_nav_test_{}_{}",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in files {
            std::fs::write(dir.join(f), b"data").unwrap();
        }
        dir
    }

    #[test]
    fn loads_directory_listing() {
        let dir = make_dir("listing", &["b.png", "a.png", "c.jpg", "d.txt"]);
        let (list, idx) = load_directory(&dir.join("a.png")).unwrap();
        assert_eq!(
            list.iter().map(|p| p.file_name().unwrap().to_str().unwrap().to_string()).collect::<Vec<_>>(),
            vec!["a.png", "b.png", "c.jpg", "d.txt"]
        );
        assert_eq!(idx, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn steps_to_sibling() {
        let dir = make_dir("sibling", &["a.png", "b.png", "c.png"]);
        let (list, mut idx) = load_directory(&dir.join("a.png")).unwrap();
        let mut state = crate::state::ViewState::default();
        state.local_files = list;
        state.file_index = idx;
        let next = step_file(&mut state, 1).unwrap();
        assert_eq!(next.file_name().unwrap().to_str().unwrap(), "b.png");
        let _ = idx;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn steps_to_media_skipping_nonmedia() {
        let dir = make_dir("media", &["a.png", "x.txt", "b.jpg", "c.webm"]);
        let (list, mut _idx) = load_directory(&dir.join("a.png")).unwrap();
        let mut state = crate::state::ViewState::default();
        state.local_files = list;
        state.file_index = 0;
        let next = step_media(&mut state, 1).unwrap();
        assert_eq!(next.file_name().unwrap().to_str().unwrap(), "b.jpg");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
