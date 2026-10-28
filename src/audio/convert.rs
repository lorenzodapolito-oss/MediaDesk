//! Audio → MP3 conversion through ffmpeg (loudness-normalised).

use crate::util::unique_path;
use std::path::{Path, PathBuf};

pub const BITRATE: &str = "160k";
/// EBU R128 loudness normalisation.
pub const LOUDNORM: &str = "loudnorm=I=-16:TP=-1.5:LRA=11";

/// ffmpeg arguments: `input` → `output` as MP3, cover and tags kept, ID3v2.3 for wide support.
pub fn args(input: &Path, output: &Path) -> Vec<String> {
    let s = |p: &Path| p.to_string_lossy().into_owned();
    [
        "-nostdin", "-hide_banner", "-nostats", "-progress", "pipe:1", "-y", "-i", &s(input), "-map", "0:a:0", "-map", "0:v?", "-c:v", "copy",
        "-disposition:v", "attached_pic", "-af", LOUDNORM, "-c:a", "libmp3lame", "-b:a", BITRATE, "-map_metadata", "0", "-id3v2_version", "3",
        &s(output),
    ]
    .iter()
    .map(|x| x.to_string())
    .collect()
}

/// `folder/converted/<stem>.mp3`, never overwriting.
pub fn output_path(input: &Path, out_folder: &Path) -> PathBuf {
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "audio".into());
    unique_path(out_folder, &stem, "mp3")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::tags::tests::{make_audio, temp_dir};
    use crate::ffmpeg;

    #[test]
    fn converts_to_mp3_and_keeps_tags() {
        let Some(ff) = ffmpeg::find_tool("ffmpeg", "") else { return };
        let dir = temp_dir("conv");
        let Some(src) = make_audio(&dir, "in.flac") else { return };
        crate::audio::tags::write(&src, &crate::audio::tags::TagEdit { title: "T".into(), artist: "A".into(), ..Default::default() }).unwrap();
        let out = output_path(&src, &dir);
        let r = ffmpeg::run(&ff, &args(&src, &out), &ffmpeg::new_cancel(), |_| {}).unwrap();
        assert!(r.success, "{}", r.stderr_tail);
        let info = crate::audio::tags::read(&out).unwrap();
        assert_eq!((info.title.as_str(), info.artist.as_str()), ("T", "A"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
