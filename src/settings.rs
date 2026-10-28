//! Settings: portable when `portable.txt` sits next to the program (data in `MediaDeskData`),
//! otherwise in the user's config folder.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub audio_folder: String,
    pub video_in: String,
    pub video_out: String,
    pub ffmpeg_path: String,
    pub musicbrainz_agent: String,
    pub mb_min_score: u32,
    /// The tab shown last ("audio" or "video").
    pub tab: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            audio_folder: String::new(),
            video_in: String::new(),
            video_out: String::new(),
            ffmpeg_path: String::new(),
            musicbrainz_agent: "MediaDesk/0.1 (https://github.com/lorenzodapolito-oss)".into(),
            mb_min_score: 75,
            tab: "audio".into(),
        }
    }
}

/// The folder holding settings, or `None` when none can be found.
pub fn data_dir() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("portable.txt").exists() {
                return Some(dir.join("MediaDeskData"));
            }
        }
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        Some(PathBuf::from(x))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
    };
    base.map(|b| b.join("MediaDesk"))
}

impl Settings {
    pub fn load() -> Self {
        data_dir()
            .and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = data_dir().ok_or("no settings folder")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("settings.json"), text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_partial_settings_fall_back_to_defaults() {
        let s: Settings = serde_json::from_str(r#"{"audio_folder":"X"}"#).unwrap();
        assert_eq!(s.audio_folder, "X");
        assert_eq!(s.mb_min_score, 75);
        assert!(serde_json::from_str::<Settings>("not json").is_err());
    }
}
