//! ffprobe output → the streams the Video tab lets you pick, and ffmpeg `cropdetect` parsing.

use serde_json::Value;

/// Subtitle codecs that are pictures: they cannot be burnt in with the `subtitles` filter.
pub const IMAGE_SUB_CODECS: [&str; 5] = ["hdmv_pgs_subtitle", "dvd_subtitle", "dvb_subtitle", "xsub", "vobsub"];

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
        let ch = if self.channels > 0 { format!(" {}ch", self.channels) } else { String::new() };
        format!("#{} ({}) {} [{}{}]", self.index, self.lang.to_uppercase(), self.title, self.codec.to_uppercase(), ch)
    }
}

impl SubStream {
    pub fn label(&self) -> String {
        let kind = if self.is_text { "text" } else { "image: no hardsub" };
        format!("#{} ({}) {} [{} – {}]", self.index, self.lang.to_uppercase(), self.title, self.codec.to_uppercase(), kind)
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
    let (Some(a), Some(b)) = (start, end) else { return Err("no JSON in ffprobe output".into()) };
    let v: Value = serde_json::from_str(output.get(a..=b).unwrap_or("")).map_err(|e| e.to_string())?;
    let mut p = Probe::default();
    p.duration = v.pointer("/format/duration").and_then(Value::as_str).and_then(|d| d.parse().ok()).unwrap_or(0.0);
    let (mut a_order, mut s_order) = (0usize, 0usize);
    for st in v.get("streams").and_then(Value::as_array).into_iter().flatten() {
        let text = |k: &str| st.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let tag = |k: &str| st.pointer(&format!("/tags/{k}")).and_then(Value::as_str).unwrap_or("").to_string();
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
                p.audio.push(AudioStream { order: a_order, index, lang, title: tag("title"), channels: st.get("channels").and_then(Value::as_u64).unwrap_or(0), codec });
                a_order += 1;
            }
            "subtitle" => {
                let is_text = !IMAGE_SUB_CODECS.contains(&codec.as_str());
                p.subs.push(SubStream { order: s_order, index, lang, title: tag("title"), codec, is_text });
                s_order += 1;
            }
            _ => {}
        }
    }
    Ok(p)
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
        assert_eq!((p.audio[1].order, p.audio[1].index, p.audio[1].lang.as_str()), (1, 2, "eng"));
        assert_eq!((p.subs[0].order, p.subs[0].is_text), (0, true));
        assert_eq!((p.subs[1].order, p.subs[1].is_text), (1, false));
        assert!(p.audio[0].label().contains("6ch"));
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
