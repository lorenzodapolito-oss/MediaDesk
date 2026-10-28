//! The Video tab's logic: ffprobe parsing, filters, crop, conversion.

pub mod convert;
pub mod filters;
pub mod probe;

pub const EXTENSIONS: [&str; 10] = ["mkv", "mp4", "avi", "mov", "wmv", "flv", "webm", "m4v", "ts", "mpg"];

pub fn is_video(path: &std::path::Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Video files directly inside `folder`, sorted by name.
pub fn list_folder(folder: &std::path::Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut files: Vec<_> = std::fs::read_dir(folder)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_video(p))
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    Ok(files)
}
