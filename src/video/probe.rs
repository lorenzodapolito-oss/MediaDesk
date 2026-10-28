//! ffprobe output → the streams the Video tab lets you pick, and ffmpeg `cropdetect` parsing.

use serde_json::Value;

/// Subtitle codecs that are pictures: they cannot be burnt in with the `subtitles` filter.
pub const IMAGE_SUB_CODECS: [&str; 5] = [
    "hdmv_pgs_subtitle",
    "dvd_subtitle",
    "dvb_subtitle",
    "xsub",
    "vobsub",
];

#[derive(Debug, Clone, PartialEq)]
pub struct AudioStream {
    /// Position among the audio streams (`0:a:N`).
    pub order: usize,
    pub index: u64,
    pub lang: String,
    pub title: String,
    pub codec: String,
    pub channels: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubStream {
    /// Position among the subtitle streams (`0:s:N`, and `si=N` of the `subtitles` filter).
    pub order: usize,
    pub index: u64,
    pub lang: String,
    pub title: String,
    pub codec: String,
    pub is_text: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub audio: Vec<AudioStream>,
    pub subs: Vec<SubStream>,
}

impl AudioStream {
    pub fn label(&self) -> String {
        let ch = if self.channels > 0 {
            format!(" {}ch", self.channels)
        } else {
            String::new()
        };
        format!(
            "#{} ({}) {} [{}{}]",
            self.index,
            self.lang.to_uppercase(),
            self.title,
            self.codec.to_uppercase(),
            ch
        )
    }
}

impl SubStream {
    pub fn label(&self) -> String {
        let kind = if self.is_text {
            "text"
        } else {
            "image: no hardsub"
        };
        format!(
            "#{} ({}) {} [{} – {}]",
            self.index,
            self.lang.to_uppercase(),
            self.title,
            self.codec.to_uppercase(),
            kind
        )
    }
}

/// ffprobe args for `path`.
pub fn args(path: &std::path::Path) -> Vec<String> {
    ["-v", "error", "-show_entries", "stream=index,codec_name,codec_type,channels,width,height:stream_tags=language,title:format=duration", "-of", "json"]
        .iter()
        .map(|s| s.to_string())
        .chain(std::iter::once(path.to_string_lossy().into_owned()))
        .collect()
}

/// Parses ffprobe's JSON (tolerating text around it).
pub fn parse(output: &str) -> Result<Probe, String> {
    let (start, end) = (output.find('{'), output.rfind('}'));
    let (Some(a), Some(b)) = (start, end) else {
        return Err("no JSON in ffprobe output".into());
    };
    let v: Value =
        serde_json::from_str(output.get(a..=b).unwrap_or("")).map_err(|e| e.to_string())?;
    let duration = v
        .pointer("/format/duration")
        .and_then(Value::as_str)
        .and_then(|d| d.parse().ok())
        .unwrap_or(0.0);
    let mut p = Probe {
        duration,
        ..Default::default()
    };
    let (mut a_order, mut s_order) = (0usize, 0usize);
    for st in v
        .get("streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |k: &str| st.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let tag = |k: &str| {
            st.pointer(&format!("/tags/{k}"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let index = st.get("index").and_then(Value::as_u64).unwrap_or(0);
        let codec = {
            let c = text("codec_name");
            if c.is_empty() {
                "unknown".to_string()
            } else {
                c
            }
        };
        let lang = {
            let l = tag("language");
            if l.is_empty() {
                "und".to_string()
            } else {
                l
            }
        };
        match text("codec_type").as_str() {
            "video" if p.width == 0 => {
                p.width = st.get("width").and_then(Value::as_u64).unwrap_or(0) as u32;
                p.height = st.get("height").and_then(Value::as_u64).unwrap_or(0) as u32;
            }
            "audio" => {
                p.audio.push(AudioStream {
                    order: a_order,
                    index,
                    lang,
                    title: tag("title"),
                    channels: st.get("channels").and_then(Value::as_u64).unwrap_or(0),
                    codec,
                });
                a_order += 1;
            }
            "subtitle" => {
                let is_text = !IMAGE_SUB_CODECS.contains(&codec.as_str());
                p.subs.push(SubStream {
                    order: s_order,
                    index,
                    lang,
                    title: tag("title"),
                    codec,
                    is_text,
                });
                s_order += 1;
            }
            _ => {}
        }
    }
    Ok(p)
}

/// ffmpeg args that only print the file's streams (for when there is no ffprobe).
pub fn info_args(path: &std::path::Path) -> Vec<String> {
    ["-hide_banner", "-nostdin", "-i"]
        .iter()
        .map(|s| s.to_string())
        .chain(std::iter::once(path.to_string_lossy().into_owned()))
        .collect()
}

/// `HH:MM:SS.xx` to seconds.
fn clock_seconds(text: &str) -> Option<f64> {
    let mut total = 0.0;
    for part in text.trim().split(':') {
        total = total * 60.0 + part.trim().parse::<f64>().ok()?;
    }
    (total.is_finite() && total >= 0.0).then_some(total)
}

fn channels_of(layout: &str) -> u64 {
    let l = layout.trim().to_ascii_lowercase();
    if l == "mono" {
        return 1;
    }
    if l == "stereo" {
        return 2;
    }
    if let Some(n) = l
        .strip_suffix(" channels")
        .and_then(|n| n.trim().parse().ok())
    {
        return n;
    }
    // "5.1", "7.1(side)": front + lfe
    let head = l.split('(').next().unwrap_or("");
    match head.split_once('.') {
        Some((a, b)) => a.parse::<u64>().unwrap_or(0) + b.parse::<u64>().unwrap_or(0),
        None => 0,
    }
}

/// Parses what `ffmpeg -i file` prints about the streams (the fallback when ffprobe is missing).
pub fn parse_ffmpeg_info(stderr: &str) -> Result<Probe, String> {
    let mut p = Probe::default();
    let (mut a_order, mut s_order) = (0usize, 0usize);
    // what the metadata lines that follow belong to: None = the file itself
    enum Cur {
        File,
        Audio,
        Sub,
        Other,
    }
    let mut cur = Cur::File;
    let mut found = false;
    for line in stderr.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Duration:") {
            if let Some(d) = rest.split(',').next().and_then(clock_seconds) {
                p.duration = d;
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("Stream #") {
            found = true;
            let (id, desc) = rest.split_once(": ").unwrap_or((rest, ""));
            let index = id
                .split([':', '(', '['])
                .nth(1)
                .and_then(|n| n.trim().parse::<u64>().ok())
                .unwrap_or(0);
            let lang = id
                .split_once('(')
                .and_then(|(_, r)| r.split_once(')'))
                .map(|(l, _)| l.to_string())
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| "und".into());
            let (kind, detail) = desc.split_once(": ").unwrap_or((desc, ""));
            let codec = detail
                .split([' ', ','])
                .next()
                .filter(|c| !c.is_empty())
                .unwrap_or("unknown")
                .to_ascii_lowercase();
            match kind.trim() {
                "Video" => {
                    cur = Cur::Other;
                    if p.width == 0 {
                        for tok in detail.split([' ', ',', '[', ']']) {
                            if let Some((w, h)) = tok.split_once('x') {
                                if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                                    if w >= 16 && h >= 16 {
                                        p.width = w;
                                        p.height = h;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                "Audio" => {
                    let layout = detail.split(',').nth(2).unwrap_or("");
                    p.audio.push(AudioStream {
                        order: a_order,
                        index,
                        lang,
                        title: String::new(),
                        channels: channels_of(layout),
                        codec,
                    });
                    a_order += 1;
                    cur = Cur::Audio;
                }
                "Subtitle" => {
                    let is_text = !IMAGE_SUB_CODECS.contains(&codec.as_str());
                    p.subs.push(SubStream {
                        order: s_order,
                        index,
                        lang,
                        title: String::new(),
                        codec,
                        is_text,
                    });
                    s_order += 1;
                    cur = Cur::Sub;
                }
                _ => cur = Cur::Other,
            }
            continue;
        }
        if let Some(title) = t
            .strip_prefix("title")
            .and_then(|r| r.trim_start().strip_prefix(':'))
        {
            let title = title.trim().to_string();
            match cur {
                Cur::Audio => {
                    if let Some(a) = p.audio.last_mut() {
                        a.title = title;
                    }
                }
                Cur::Sub => {
                    if let Some(s) = p.subs.last_mut() {
                        s.title = title;
                    }
                }
                _ => {}
            }
        }
    }
    if !found {
        return Err("ffmpeg did not list any stream (not a video file?)".into());
    }
    Ok(p)
}

/// Reads a file's streams with ffprobe when there is one, otherwise with `ffmpeg -i`.
pub fn read(
    ffprobe: Option<&std::path::Path>,
    ffmpeg: Option<&std::path::Path>,
    path: &std::path::Path,
) -> Result<Probe, String> {
    use std::time::Duration;
    if let Some(fp) = ffprobe {
        let (out, _) = crate::ffmpeg::capture(fp, &args(path), Duration::from_secs(30))?;
        if let Ok(p) = parse(&out) {
            if p.width > 0 {
                return Ok(p);
            }
        }
    }
    let Some(ff) = ffmpeg else {
        return Err("ffmpeg e ffprobe non trovati".into());
    };
    let (_, err) = crate::ffmpeg::capture(ff, &info_args(path), Duration::from_secs(30))?;
    parse_ffmpeg_info(&err)
}

/// The last `crop=w:h:x:y` of a `cropdetect` run (ffmpeg's stderr).
pub fn parse_cropdetect(stderr: &str) -> Option<(u32, u32, u32, u32)> {
    let line = stderr.lines().rev().find(|l| l.contains("crop="))?;
    let spec = line.split("crop=").nth(1)?.split_whitespace().next()?;
    let n: Vec<u32> = spec.split(':').filter_map(|x| x.parse().ok()).collect();
    match n.as_slice() {
        [w, h, x, y] => Some((*w, *h, *x, *y)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"streams":[
      {"index":0,"codec_name":"h264","codec_type":"video","width":1920,"height":1080},
      {"index":1,"codec_name":"aac","codec_type":"audio","channels":6,"tags":{"language":"ita","title":"Italiano"}},
      {"index":2,"codec_name":"ac3","codec_type":"audio","channels":2,"tags":{"language":"eng"}},
      {"index":3,"codec_name":"subrip","codec_type":"subtitle","tags":{"language":"ita"}},
      {"index":4,"codec_name":"hdmv_pgs_subtitle","codec_type":"subtitle"}],
      "format":{"duration":"125.5"}}"#;

    #[test]
    fn parses_streams_with_per_type_order() {
        let p = parse(&format!("warning\n{SAMPLE}\n")).unwrap();
        assert_eq!((p.width, p.height, p.duration), (1920, 1080, 125.5));
        assert_eq!(p.audio.len(), 2);
        assert_eq!(
            (p.audio[1].order, p.audio[1].index, p.audio[1].lang.as_str()),
            (1, 2, "eng")
        );
        assert_eq!((p.subs[0].order, p.subs[0].is_text), (0, true));
        assert_eq!((p.subs[1].order, p.subs[1].is_text), (1, false));
        assert!(p.audio[0].label().contains("6ch"));
    }

    #[test]
    fn ffmpeg_only_info_gives_the_same_streams() {
        let text = "Input #0, matroska,webm, from 'a.mkv':\n  Metadata:\n    title           : Whole file\n  Duration: 01:02:03.50, start: 0.000000, bitrate: 1234 kb/s\n  Stream #0:0(eng): Video: h264 (High), yuv420p(progressive), 1920x1080 [SAR 1:1 DAR 16:9], 25 fps, 25 tbr (default)\n  Stream #0:1(ita): Audio: ac3, 48000 Hz, 5.1(side), fltp, 384 kb/s (default)\n    Metadata:\n      title           : Italiano\n  Stream #0:2(eng): Audio: aac (LC), 44100 Hz, stereo, fltp\n  Stream #0:3(ita): Subtitle: subrip\n    Metadata:\n      title           : Forced\n  Stream #0:4: Subtitle: hdmv_pgs_subtitle\n";
        let p = parse_ffmpeg_info(text).unwrap();
        assert_eq!((p.width, p.height), (1920, 1080));
        assert!((p.duration - 3723.5).abs() < 0.01);
        assert_eq!(p.audio.len(), 2);
        assert_eq!(
            (
                p.audio[0].channels,
                p.audio[0].title.as_str(),
                p.audio[0].lang.as_str()
            ),
            (6, "Italiano", "ita")
        );
        assert_eq!(
            (p.audio[1].order, p.audio[1].index, p.audio[1].channels),
            (1, 2, 2)
        );
        assert_eq!(
            (p.subs[0].title.as_str(), p.subs[0].is_text),
            ("Forced", true)
        );
        assert!(!p.subs[1].is_text);
        assert!(parse_ffmpeg_info("nothing here").is_err());
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(parse("").is_err());
        assert!(parse("{ not json }").is_err());
        assert_eq!(parse("{}").unwrap(), Probe::default());
    }

    #[test]
    fn cropdetect_takes_the_last_suggestion() {
        let s = "[Parsed_cropdetect_0 @ 0x1] x1:0 crop=1920:800:0:140\n[Parsed_cropdetect_0 @ 0x1] x1:0 crop=1920:816:0:132\n";
        assert_eq!(parse_cropdetect(s), Some((1920, 816, 0, 132)));
        assert_eq!(parse_cropdetect("nothing"), None);
        assert_eq!(parse_cropdetect("crop=1:2"), None);
    }
}
