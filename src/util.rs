//! Small file-name helpers shared by both tabs.

use std::path::{Path, PathBuf};

/// Removes characters that are not valid in Windows/Linux/macOS file names.
pub fn sanitize_filename(value: &str) -> String {
    let cleaned: String = value
        .trim()
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') && !c.is_control())
        .map(|c| match c {
            '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            c => c,
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim().trim_matches('.').trim();
    // Reserved Windows device names would make the rename fail.
    let upper = trimmed.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4 && (upper.starts_with("COM") || upper.starts_with("LPT")) && upper.ends_with(|c: char| c.is_ascii_digit()));
    if trimmed.is_empty() || reserved {
        "audio".to_string()
    } else {
        trimmed.to_string()
    }
}

/// A path in `folder` that does not exist yet: `name.ext`, `name_1.ext`, `name_2.ext`, …
pub fn unique_path(folder: &Path, stem: &str, ext: &str) -> PathBuf {
    let make = |n: u32| {
        let name = if n == 0 { stem.to_string() } else { format!("{stem}_{n}") };
        if ext.is_empty() {
            folder.join(name)
        } else {
            folder.join(format!("{name}.{ext}"))
        }
    };
    let mut n = 0u32;
    loop {
        let p = make(n);
        if !p.exists() || n >= 10_000 {
            return p;
        }
        n += 1;
    }
}

/// Opens a folder in the system file manager.
pub fn open_folder(path: &Path) -> Result<(), String> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program).arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// `seconds` as `h:mm:ss` or `m:ss`.
pub fn clock(seconds: f64) -> String {
    let s = if seconds.is_finite() && seconds > 0.0 { seconds as u64 } else { 0 };
    let (h, m, sec) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_filename("AC/DC: Back  In Black?"), "ACDC Back In Black");
        assert_eq!(sanitize_filename("  ...  "), "audio");
        assert_eq!(sanitize_filename("CON"), "audio");
        assert_eq!(sanitize_filename("l\u{2019}amore"), "l'amore");
        assert_eq!(sanitize_filename("è così – già"), "è così – già");
    }

    #[test]
    fn unique_paths_do_not_overwrite() {
        let d = std::env::temp_dir().join(format!("mediadesk-unique-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.mp3"), b"x").unwrap();
        std::fs::write(d.join("a_1.mp3"), b"x").unwrap();
        assert_eq!(unique_path(&d, "a", "mp3"), d.join("a_2.mp3"));
        assert_eq!(unique_path(&d, "b", "mp3"), d.join("b.mp3"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn clock_formats() {
        assert_eq!(clock(65.4), "1:05");
        assert_eq!(clock(3725.0), "1:02:05");
        assert_eq!(clock(f64::NAN), "0:00");
        assert_eq!(clock(-3.0), "0:00");
    }
}
