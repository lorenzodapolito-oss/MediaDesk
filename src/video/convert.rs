//! Batch conversion: ffmpeg arguments for one file, and the preview frame.

use super::filters::{video_filter, Color, Crop};
use super::probe::Probe;
use crate::util::unique_path;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mkv,
    Mp4,
}

impl Container {
    pub fn ext(self) -> &'static str {
        match self {
            Container::Mkv => "mkv",
            Container::Mp4 => "mp4",
        }
    }
}

pub const PRESETS: [&str; 5] = ["faster", "fast", "medium", "slow", "slower"];

#[derive(Debug, Clone, PartialEq)]
pub enum AudioChoice {
    All,
    /// Position among the audio streams.
    Track(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub container: Container,
    /// 15–45
    pub crf: u32,
    pub preset: String,
    pub audio: AudioChoice,
    /// Subtitle order to burn in.
    pub hardsub: Option<usize>,
    pub crop: Option<Crop>,
    pub color: Color,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            container: Container::Mkv,
            crf: 20,
            preset: "medium".into(),
            audio: AudioChoice::All,
            hardsub: None,
            crop: None,
            color: Color::default(),
        }
    }
}

/// `<out folder>/<stem>_converted.<ext>`, never overwriting.
pub fn output_path(input: &Path, out_folder: &Path, container: Container) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "video".into());
    unique_path(out_folder, &format!("{stem}_converted"), container.ext())
}

/// The ffmpeg command for one file. `probe` says which soft subtitles can be kept.
pub fn args(input: &Path, output: &Path, o: &Options, probe: &Probe) -> Vec<String> {
    let mut a: Vec<String> = [
        "-nostdin",
        "-hide_banner",
        "-nostats",
        "-progress",
        "pipe:1",
        "-y",
        "-i",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.push(input.to_string_lossy().into_owned());
    a.extend(["-map", "0:v:0"].map(String::from));
    match o.audio {
        AudioChoice::All => a.extend(["-map", "0:a?"].map(String::from)),
        AudioChoice::Track(n) => a.extend(["-map".to_string(), format!("0:a:{n}")]),
    }
    let hardsub = o
        .hardsub
        .and_then(|n| probe.subs.iter().find(|s| s.order == n && s.is_text))
        .map(|s| s.order);
    if hardsub.is_none() {
        // keep the soft subtitles the container can hold
        match o.container {
            Container::Mkv => {
                a.extend(["-map", "0:s?", "-c:s", "copy"].map(String::from));
            }
            Container::Mp4 => {
                for s in probe.subs.iter().filter(|s| s.is_text) {
                    a.extend(["-map".to_string(), format!("0:s:{}", s.order)]);
                }
                if probe.subs.iter().any(|s| s.is_text) {
                    a.extend(["-c:s", "mov_text"].map(String::from));
                }
            }
        }
    }
    let file = input.to_string_lossy();
    let vf = video_filter(o.crop, &o.color, hardsub.map(|n| (file.as_ref(), n)));
    let preset = if PRESETS.contains(&o.preset.as_str()) {
        o.preset.as_str()
    } else {
        "medium"
    };
    a.extend(["-c:v", "libx264", "-crf"].map(String::from));
    a.push(o.crf.clamp(15, 45).to_string());
    a.extend(
        [
            "-preset",
            preset,
            "-profile:v",
            "high",
            "-level:v",
            "4.1",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-vf",
        ]
        .map(String::from),
    );
    a.push(vf);
    a.extend(["-movflags", "+faststart"].map(String::from));
    a.push(output.to_string_lossy().into_owned());
    a
}

/// One frame (at `at` seconds) as a PNG at `out`, 960 px wide at most.
pub fn preview_args(input: &Path, at: f64, out: &Path) -> Vec<String> {
    let mut a: Vec<String> = ["-nostdin", "-hide_banner", "-y", "-ss"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    a.push(format!("{:.2}", at.max(0.0)));
    a.push("-i".into());
    a.push(input.to_string_lossy().into_owned());
    a.extend(["-frames:v", "1", "-vf", "scale='min(960,iw)':-2"].map(String::from));
    a.push(out.to_string_lossy().into_owned());
    a
}

/// ffmpeg args that print `cropdetect` suggestions for a few frames at `at` seconds.
pub fn cropdetect_args(input: &Path, at: f64) -> Vec<String> {
    let mut a: Vec<String> = ["-nostdin", "-hide_banner", "-ss"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    a.push(format!("{:.2}", at.max(0.0)));
    a.push("-i".into());
    a.push(input.to_string_lossy().into_owned());
    a.extend(
        [
            "-vf",
            "cropdetect=24:2:0",
            "-frames:v",
            "10",
            "-f",
            "null",
            "-",
        ]
        .map(String::from),
    );
    a
}

/// A good moment for preview/cropdetect: 30 s in, or the middle of a short video.
pub fn sample_time(duration: f64) -> f64 {
    if duration.is_finite() && duration > 0.0 {
        30f64.min(duration / 2.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::probe::{AudioStream, SubStream};

    fn probe() -> Probe {
        Probe {
            width: 1920,
            height: 1080,
            duration: 60.0,
            audio: vec![AudioStream {
                order: 0,
                index: 1,
                lang: "ita".into(),
                title: String::new(),
                codec: "aac".into(),
                channels: 2,
            }],
            subs: vec![
                SubStream {
                    order: 0,
                    index: 2,
                    lang: "ita".into(),
                    title: String::new(),
                    codec: "subrip".into(),
                    is_text: true,
                },
                SubStream {
                    order: 1,
                    index: 3,
                    lang: "eng".into(),
                    title: String::new(),
                    codec: "hdmv_pgs_subtitle".into(),
                    is_text: false,
                },
            ],
        }
    }

    fn joined(o: &Options) -> String {
        args(Path::new("in.mkv"), Path::new("out.mkv"), o, &probe()).join(" ")
    }

    #[test]
    fn mkv_copies_soft_subtitles_and_maps_audio_by_order() {
        let s = joined(&Options {
            audio: AudioChoice::Track(0),
            ..Default::default()
        });
        assert!(
            s.contains("-map 0:a:0") && s.contains("-map 0:s? -c:s copy"),
            "{s}"
        );
        assert!(
            s.contains("-crf 20") && s.contains("-preset medium") && s.contains("libx264"),
            "{s}"
        );
    }

    #[test]
    fn hardsub_replaces_soft_subtitles_and_image_subs_are_ignored() {
        let s = joined(&Options {
            hardsub: Some(0),
            ..Default::default()
        });
        assert!(
            s.contains("subtitles='in.mkv':si=0") && !s.contains("-c:s"),
            "{s}"
        );
        // an image subtitle cannot be burnt in: fall back to soft copy
        let s = joined(&Options {
            hardsub: Some(1),
            ..Default::default()
        });
        assert!(!s.contains("subtitles=") && s.contains("-c:s copy"), "{s}");
    }

    #[test]
    fn mp4_keeps_only_text_subtitles_as_mov_text() {
        let s = joined(&Options {
            container: Container::Mp4,
            ..Default::default()
        });
        assert!(
            s.contains("-map 0:s:0") && !s.contains("-map 0:s:1") && s.contains("mov_text"),
            "{s}"
        );
    }

    #[test]
    fn values_are_sanitised() {
        let s = joined(&Options {
            crf: 99,
            preset: "weird; rm -rf".into(),
            ..Default::default()
        });
        assert!(s.contains("-crf 45") && s.contains("-preset medium"), "{s}");
        assert_eq!(sample_time(10.0), 5.0);
        assert_eq!(sample_time(600.0), 30.0);
        assert_eq!(sample_time(f64::NAN), 0.0);
    }

    #[test]
    fn output_never_overwrites() {
        let d = std::env::temp_dir().join(format!("mediadesk-vout-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a_converted.mkv"), b"x").unwrap();
        assert_eq!(
            output_path(Path::new("/x/a.avi"), &d, Container::Mkv),
            d.join("a_converted_1.mkv")
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_real_conversion_runs_with_crop_colour_and_subtitles_off() {
        use crate::ffmpeg;
        let (Some(ff), Some(fp)) = (
            ffmpeg::find_tool("ffmpeg", ""),
            ffmpeg::find_tool("ffprobe", ""),
        ) else {
            return;
        };
        let dir = crate::audio::tags::tests::temp_dir("vid");
        let src = dir.join("in.mp4");
        let gen: Vec<String> = [
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=640x360:rate=10:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=2",
            "-shortest",
            src.to_str().unwrap_or(""),
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if !ffmpeg::run(&ff, &gen, &ffmpeg::new_cancel(), |_| {})
            .map(|o| o.success)
            .unwrap_or(false)
        {
            return;
        }
        let (out, _) = ffmpeg::capture(
            &fp,
            &crate::video::probe::args(&src),
            std::time::Duration::from_secs(20),
        )
        .unwrap();
        let p = crate::video::probe::parse(&out).unwrap();
        assert_eq!((p.width, p.height), (640, 360));
        assert_eq!(p.audio.len(), 1);
        let o = Options {
            container: Container::Mp4,
            crop: Some(Crop {
                left: 10,
                right: 11,
                top: 20,
                bottom: 21,
            }),
            color: Color {
                enabled: true,
                brightness: 0.05,
                temperature: 30,
                hue: 15.0,
                ..Default::default()
            },
            preset: "faster".into(),
            ..Default::default()
        };
        let dst = output_path(&src, &dir, o.container);
        let mut last = 0.0;
        let r = ffmpeg::run(&ff, &args(&src, &dst, &o, &p), &ffmpeg::new_cancel(), |s| {
            last = s
        })
        .unwrap();
        assert!(r.success, "{}", r.stderr_tail);
        assert!(last > 0.0, "progress was reported");
        let (out, _) = ffmpeg::capture(
            &fp,
            &crate::video::probe::args(&dst),
            std::time::Duration::from_secs(20),
        )
        .unwrap();
        let q = crate::video::probe::parse(&out).unwrap();
        // 640-10-11 = 619 → even 618; 360-20-21 = 319 → 318
        assert_eq!((q.width, q.height), (618, 318));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
