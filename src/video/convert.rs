//! Batch conversion: ffmpeg arguments for one file, and the preview frame.

use super::filters::{color_filter, video_filter, Color, Crop};
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
    args_with_live(input, output, o, probe, None)
}

/// Like [`args`]; with `live` ffmpeg also writes the frame it is encoding (once per second of
/// video, 480 px wide) over and over to that JPEG file, so the program can show it.
pub fn args_with_live(
    input: &Path,
    output: &Path,
    o: &Options,
    probe: &Probe,
    live: Option<&Path>,
) -> Vec<String> {
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
    if live.is_some() {
        a.extend(["-map", "[main]"].map(String::from));
    } else {
        a.extend(["-map", "0:v:0"].map(String::from));
    }
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
            "3.1",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-ac",
            "2",
        ]
        .map(String::from),
    );
    if live.is_some() {
        a.push("-filter_complex".into());
        a.push(format!(
            "[0:v:0]{vf},split=2[main][pv];[pv]fps=1,scale=480:-2[pvo]"
        ));
    } else {
        a.push("-vf".into());
        a.push(vf);
    }
    a.extend(["-movflags", "+faststart"].map(String::from));
    a.push(output.to_string_lossy().into_owned());
    if let Some(live) = live {
        a.extend(
            [
                "-map", "[pvo]", "-an", "-sn", "-c:v", "mjpeg", "-q:v", "6", "-f", "image2",
                "-update", "1",
            ]
            .map(String::from),
        );
        a.push(live.to_string_lossy().into_owned());
    }
    a
}

/// Average x264 video bitrate (kbit/s) for 720p at `crf` with the medium preset: about 1500 at
/// CRF 23, halving every 6 steps. A rough rule of thumb, not a promise.
pub fn video_kbps(crf: u32) -> f64 {
    1500.0 * 2f64.powf((23.0 - f64::from(crf.clamp(15, 45))) / 6.0)
}

/// The expected size of the converted file in bytes (video estimate plus 128 kbit/s per audio track).
pub fn estimated_bytes(duration_secs: f64, crf: u32, audio_tracks: usize) -> u64 {
    if !duration_secs.is_finite() || duration_secs <= 0.0 {
        return 0;
    }
    let kbps = video_kbps(crf) + 128.0 * audio_tracks as f64;
    (duration_secs * kbps * 1000.0 / 8.0) as u64
}

/// `1.4 GB`, `312 MB`, `850 KB`.
pub fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else {
        format!("{:.0} KB", b / 1e3)
    }
}

/// One frame (at `at` seconds) as a PNG at `out`, 960 px wide at most, with the colour
/// correction applied so the preview shows what the conversion will produce.
pub fn preview_args(input: &Path, at: f64, out: &Path, color: &Color) -> Vec<String> {
    let mut a: Vec<String> = ["-nostdin", "-hide_banner", "-y", "-ss"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    a.push(format!("{:.2}", at.max(0.0)));
    a.push("-i".into());
    a.push(input.to_string_lossy().into_owned());
    let mut vf = String::from("scale='min(960,iw)':-2");
    let col = color_filter(color);
    if !col.is_empty() {
        vf.push(',');
        vf.push_str(&col);
    }
    a.extend(["-frames:v".to_string(), "1".into(), "-vf".into(), vf]);
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

/// The default moment for preview and crop analysis: minute 10, or the middle of a shorter video.
pub fn sample_time(duration: f64) -> f64 {
    if duration.is_finite() && duration > 0.0 {
        600f64.min(duration / 2.0)
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
        assert_eq!(sample_time(600.0), 300.0);
        assert_eq!(sample_time(7200.0), 600.0);
        assert_eq!(sample_time(f64::NAN), 0.0);
    }

    #[test]
    fn audio_is_128k_stereo_and_the_level_is_3_1() {
        let s = joined(&Options::default());
        assert!(
            s.contains("-b:a 128k -ac 2") && s.contains("-level:v 3.1"),
            "{s}"
        );
        assert!(s.contains("-vf ") && !s.contains("filter_complex"), "{s}");
    }

    #[test]
    fn the_live_preview_adds_a_second_output() {
        let a = args_with_live(
            Path::new("in.mkv"),
            Path::new("out.mkv"),
            &Options::default(),
            &probe(),
            Some(Path::new("live.jpg")),
        );
        let s = a.join(" ");
        assert!(
            s.contains("-map [main]") && s.contains("-filter_complex"),
            "{s}"
        );
        assert!(!s.contains("-vf "), "{s}");
        // the main output comes first, the preview image last
        let out = s.find("out.mkv").unwrap();
        assert!(
            out < s.find("-map [pvo]").unwrap() && s.ends_with("live.jpg"),
            "{s}"
        );
    }

    #[test]
    fn the_preview_frame_carries_the_colour_correction() {
        let out = Path::new("p.png");
        let plain = preview_args(Path::new("a.mkv"), 5.0, out, &Color::default()).join(" ");
        assert!(plain.ends_with("scale='min(960,iw)':-2 p.png"), "{plain}");
        let c = Color { enabled: true, saturation: 2.0, ..Default::default() };
        let tinted = preview_args(Path::new("a.mkv"), 5.0, out, &c).join(" ");
        assert!(tinted.contains("scale='min(960,iw)':-2,eq=saturation=2.00"), "{tinted}");
    }

    #[test]
    fn size_estimate_follows_the_quality() {
        let small = estimated_bytes(3600.0, 35, 1);
        let mid = estimated_bytes(3600.0, 23, 1);
        let big = estimated_bytes(3600.0, 15, 1);
        assert!(small < mid && mid < big);
        // CRF 23, one hour: 1500 + 128 kbit/s
        assert_eq!(mid, (3600.0 * 1628.0 * 1000.0 / 8.0) as u64);
        assert_eq!(estimated_bytes(f64::NAN, 20, 1), 0);
        assert_eq!(human_size(312_000_000), "312 MB");
        assert_eq!(human_size(1_460_000_000), "1.5 GB");
        // out-of-range quality is clamped like the encoder does
        assert_eq!(estimated_bytes(100.0, 99, 0), estimated_bytes(100.0, 45, 0));
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
        // always 1280x720, whatever the source or crop
        assert_eq!((q.width, q.height), (1280, 720));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
