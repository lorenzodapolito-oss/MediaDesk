//! The Audio tab's logic: tags, file-name guessing, online lookup, conversion, playback.

pub mod convert;
pub mod guess;
pub mod online;
pub mod player;
pub mod tags;

/// File extensions the list shows.
pub const EXTENSIONS: [&str; 5] = ["mp3", "flac", "wav", "m4a", "ogg"];

pub fn is_audio(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Audio files directly inside `folder`, sorted by name (case-insensitive).
pub fn list_folder(folder: &std::path::Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut files: Vec<_> = std::fs::read_dir(folder)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_audio(p))
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    Ok(files)
}
