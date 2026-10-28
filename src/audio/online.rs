//! Online lookup of tags, covers and lyrics: MusicBrainz, iTunes, Deezer, Cover Art Archive and
//! lyrics.ovh (none needs an API key). The JSON parsers are separate from the network calls.

use super::guess::{guess_from_filename, Guess};
use serde_json::Value;
use std::time::{Duration, Instant};

/// One source's answer.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Candidate {
    pub source: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    pub cover_url: String,
    pub mb_release_id: String,
    pub cover: Option<Vec<u8>>,
}

/// Similarity of two strings in 0..=1 (Dice coefficient on character pairs, case-insensitive).
pub fn similar(a: &str, b: &str) -> f64 {
    let (a, b) = (a.trim().to_lowercase(), b.trim().to_lowercase());
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let pairs = |s: &str| -> Vec<(char, char)> {
        let c: Vec<char> = s.chars().collect();
        c.windows(2).map(|w| (w[0], w[1])).collect()
    };
    let (pa, mut pb) = (pairs(&a), pairs(&b));
    if pa.is_empty() || pb.is_empty() {
        return 0.0;
    }
    let total = (pa.len() + pb.len()) as f64;
    let mut hits = 0usize;
    for p in &pa {
        if let Some(i) = pb.iter().position(|q| q == p) {
            pb.swap_remove(i);
            hits += 1;
        }
    }
    2.0 * hits as f64 / total
}

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Best MusicBrainz recording with score ≥ `min_score`.
pub fn parse_musicbrainz(
    json: &Value,
    min_score: u32,
    fallback_artist: &str,
    fallback_title: &str,
) -> Option<Candidate> {
    let rec =
        json.get("recordings")?.as_array()?.iter().find(|r| {
            r.get("score").and_then(Value::as_u64).unwrap_or(0) >= u64::from(min_score)
        })?;
    let artist = {
        let phrase = s(rec, "artist-credit-phrase");
        if !phrase.is_empty() {
            phrase
        } else {
            rec.get("artist-credit")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .map(|c| {
                    let n = s(c, "name");
                    if n.is_empty() {
                        c.get("artist").map(|a| s(a, "name")).unwrap_or_default()
                    } else {
                        n
                    }
                })
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| fallback_artist.to_string())
        }
    };
    let title = {
        let t = s(rec, "title");
        if t.is_empty() {
            fallback_title.to_string()
        } else {
            t
        }
    };
    let mut c = Candidate {
        source: "MusicBrainz".into(),
        title,
        artist,
        ..Default::default()
    };
    if let Some(rel) = rec
        .get("releases")
        .and_then(Value::as_array)
        .and_then(|r| r.first())
    {
        c.album = s(rel, "title");
        c.mb_release_id = s(rel, "id");
        c.year = s(rel, "date").chars().take(4).collect();
    }
    Some(c)
}

/// Best iTunes result by similarity to the wanted artist and title.
pub fn parse_itunes(json: &Value, artist: &str, title: &str) -> Option<Candidate> {
    let best = json.get("results")?.as_array()?.iter().max_by(|a, b| {
        let sc =
            |x: &Value| similar(&s(x, "artistName"), artist) + similar(&s(x, "trackName"), title);
        sc(a)
            .partial_cmp(&sc(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    Some(Candidate {
        source: "iTunes".into(),
        title: s(best, "trackName"),
        artist: s(best, "artistName"),
        album: s(best, "collectionName"),
        genre: s(best, "primaryGenreName"),
        year: s(best, "releaseDate").chars().take(4).collect(),
        cover_url: s(best, "artworkUrl100").replace("100x100", "600x600"),
        ..Default::default()
    })
}

/// Best Deezer result by similarity.
pub fn parse_deezer(json: &Value, artist: &str, title: &str) -> Option<Candidate> {
    let best = json.get("data")?.as_array()?.iter().max_by(|a, b| {
        let sc = |x: &Value| {
            similar(
                &x.get("artist").map(|a| s(a, "name")).unwrap_or_default(),
                artist,
            ) + similar(&s(x, "title"), title)
        };
        sc(a)
            .partial_cmp(&sc(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    let album = best.get("album").cloned().unwrap_or(Value::Null);
    let cover = {
        let big = s(&album, "cover_big");
        if big.is_empty() {
            s(&album, "cover_medium")
        } else {
            big
        }
    };
    Some(Candidate {
        source: "Deezer".into(),
        title: s(best, "title"),
        artist: best.get("artist").map(|a| s(a, "name")).unwrap_or_default(),
        album: s(&album, "title"),
        cover_url: cover,
        ..Default::default()
    })
}

/// Everything found for one file.
#[derive(Debug, Default, Clone)]
pub struct Identified {
    pub guess: Guess,
    pub candidates: Vec<Candidate>,
}

/// One tag set from the candidates: MusicBrainz first, then Deezer, then iTunes, then the file name.
pub fn merge(found: &Identified) -> Candidate {
    let mut out = Candidate::default();
    let mut used: Vec<&str> = Vec::new();
    for source in ["MusicBrainz", "Deezer", "iTunes"] {
        let Some(c) = found.candidates.iter().find(|c| c.source == source) else {
            continue;
        };
        let mut took = false;
        macro_rules! fill {
            ($f:ident) => {
                if out.$f.is_empty() && !c.$f.is_empty() {
                    out.$f = c.$f.clone();
                    took = true;
                }
            };
        }
        fill!(title);
        fill!(artist);
        fill!(album);
        fill!(year);
        fill!(genre);
        fill!(cover_url);
        fill!(mb_release_id);
        if out.cover.is_none() && c.cover.is_some() {
            out.cover = c.cover.clone();
        }
        if took {
            used.push(source);
        }
    }
    let g = &found.guess;
    for (field, value) in [
        (&mut out.title, &g.title),
        (&mut out.artist, &g.artist),
        (&mut out.album, &g.album),
        (&mut out.year, &g.year),
    ] {
        if field.is_empty() {
            *field = value.clone();
        }
    }
    out.source = if used.is_empty() {
        "File name".into()
    } else {
        used.join(" + ")
    };
    out
}

/// Percent-encodes a URL path segment.
pub fn encode_segment(text: &str) -> String {
    let mut out = String::new();
    for b in text.trim().bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// An HTTP client that keeps MusicBrainz to about one request per second.
pub struct Client {
    agent: ureq::Agent,
    mb_last: Option<Instant>,
    pub min_score: u32,
}

impl Client {
    pub fn new(user_agent: &str, min_score: u32) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(12)))
            .user_agent(user_agent.to_string())
            .build()
            .into();
        Self {
            agent,
            mb_last: None,
            min_score,
        }
    }

    fn json(&self, url: &str, query: &[(&str, &str)]) -> Option<Value> {
        let mut req = self.agent.get(url);
        for (k, v) in query {
            req = req.query(*k, *v);
        }
        req.call().ok()?.body_mut().read_json::<Value>().ok()
    }

    fn bytes(&self, url: &str) -> Option<Vec<u8>> {
        let data = self
            .agent
            .get(url)
            .call()
            .ok()?
            .body_mut()
            .with_config()
            .limit(15 * 1024 * 1024)
            .read_to_vec()
            .ok()?;
        (!data.is_empty()).then_some(data)
    }

    fn mb_wait(&mut self) {
        if let Some(t) = self.mb_last {
            let el = t.elapsed();
            if el < Duration::from_millis(1100) {
                std::thread::sleep(Duration::from_millis(1100) - el);
            }
        }
        self.mb_last = Some(Instant::now());
    }

    pub fn musicbrainz(&mut self, artist: &str, title: &str) -> Option<Candidate> {
        if artist.trim().is_empty() && title.trim().is_empty() {
            return None;
        }
        self.mb_wait();
        let clean = |x: &str| x.replace('"', "").trim().to_string();
        let query = format!(
            "artist:\"{}\" AND recording:\"{}\"",
            clean(artist),
            clean(title)
        );
        let json = self.json(
            "https://musicbrainz.org/ws/2/recording/",
            &[("query", &query), ("fmt", "json"), ("limit", "5")],
        )?;
        parse_musicbrainz(&json, self.min_score, artist, title)
    }

    pub fn itunes(&self, artist: &str, title: &str) -> Option<Candidate> {
        let term = format!("{artist} {title}");
        if term.trim().is_empty() {
            return None;
        }
        let json = self.json(
            "https://itunes.apple.com/search",
            &[
                ("term", term.trim()),
                ("media", "music"),
                ("entity", "song"),
                ("limit", "5"),
            ],
        )?;
        parse_itunes(&json, artist, title)
    }

    pub fn deezer(&self, artist: &str, title: &str) -> Option<Candidate> {
        if artist.trim().is_empty() && title.trim().is_empty() {
            return None;
        }
        let term = if artist.trim().is_empty() {
            title.to_string()
        } else if title.trim().is_empty() {
            format!("artist:\"{artist}\"")
        } else {
            format!("artist:\"{artist}\" track:\"{title}\"")
        };
        let json = self.json(
            "https://api.deezer.com/search",
            &[("q", &term), ("limit", "5")],
        )?;
        parse_deezer(&json, artist, title)
    }

    /// Cover from the Cover Art Archive, else from the source's own artwork URL.
    pub fn cover(&self, c: &Candidate) -> Option<Vec<u8>> {
        if !c.mb_release_id.is_empty() {
            if let Some(d) = self.bytes(&format!(
                "https://coverartarchive.org/release/{}/front",
                c.mb_release_id
            )) {
                return Some(d);
            }
        }
        if c.cover_url.is_empty() {
            None
        } else {
            self.bytes(&c.cover_url)
        }
    }

    pub fn lyrics(&self, artist: &str, title: &str) -> Option<String> {
        if artist.trim().is_empty() || title.trim().is_empty() {
            return None;
        }
        let url = format!(
            "https://api.lyrics.ovh/v1/{}/{}",
            encode_segment(artist),
            encode_segment(title)
        );
        let json = self.json(&url, &[])?;
        let l = s(&json, "lyrics");
        (!l.is_empty()).then_some(l)
    }

    /// Asks every source (reporting progress through `status`), covers included when asked.
    pub fn identify(
        &mut self,
        file_name: &str,
        with_covers: bool,
        mut status: impl FnMut(&str),
    ) -> Identified {
        let guess = guess_from_filename(file_name);
        let mut candidates = Vec::new();
        status("MusicBrainz…");
        let mb = self.musicbrainz(&guess.artist, &guess.title);
        let (mut ra, mut rt) = (guess.artist.clone(), guess.title.clone());
        if let Some(m) = &mb {
            if similar(&m.title, &guess.title) > 0.4 || guess.title.is_empty() {
                ra = m.artist.clone();
                rt = m.title.clone();
            }
        }
        candidates.extend(mb);
        status("iTunes…");
        candidates.extend(self.itunes(&ra, &rt));
        status("Deezer…");
        candidates.extend(self.deezer(&ra, &rt));
        if with_covers {
            status("Cover…");
            for c in &mut candidates {
                c.cover = self.cover(c);
            }
        }
        Identified { guess, candidates }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn similarity_is_sane() {
        assert_eq!(similar("Queen", "queen"), 1.0);
        assert_eq!(similar("", "x"), 0.0);
        assert!(similar("Bohemian Rhapsody", "Bohemian Rhapsody - Remastered") > 0.7);
        assert!(similar("Queen", "Metallica") < 0.3);
    }

    #[test]
    fn musicbrainz_picks_first_above_threshold() {
        let j = json!({"recordings":[
            {"score": 60, "title": "Low", "artist-credit-phrase": "X"},
            {"score": 98, "title": "Bohemian Rhapsody", "artist-credit-phrase": "Queen",
             "releases":[{"id":"abc","title":"A Night at the Opera","date":"1975-11-21"}]}]});
        let c = parse_musicbrainz(&j, 75, "", "").unwrap();
        assert_eq!(
            (
                c.title.as_str(),
                c.artist.as_str(),
                c.album.as_str(),
                c.year.as_str(),
                c.mb_release_id.as_str()
            ),
            (
                "Bohemian Rhapsody",
                "Queen",
                "A Night at the Opera",
                "1975",
                "abc"
            )
        );
        assert!(parse_musicbrainz(&j, 99, "", "").is_none());
        assert!(parse_musicbrainz(&json!({}), 75, "", "").is_none());
        // artist from the credit list when there is no phrase
        let j = json!({"recordings":[{"score":90,"title":"T","artist-credit":[{"artist":{"name":"Z"}}]}]});
        assert_eq!(parse_musicbrainz(&j, 75, "", "").unwrap().artist, "Z");
    }

    #[test]
    fn itunes_and_deezer_choose_the_closest() {
        let j = json!({"results":[
            {"artistName":"Other","trackName":"Other song"},
            {"artistName":"Queen","trackName":"Bohemian Rhapsody","collectionName":"Opera","primaryGenreName":"Rock","releaseDate":"1975-10-31T08:00:00Z","artworkUrl100":"http://x/100x100bb.jpg"}]});
        let c = parse_itunes(&j, "Queen", "Bohemian Rhapsody").unwrap();
        assert_eq!(
            (c.genre.as_str(), c.year.as_str(), c.cover_url.as_str()),
            ("Rock", "1975", "http://x/600x600bb.jpg")
        );
        let j = json!({"data":[
            {"title":"Nope","artist":{"name":"Nobody"}},
            {"title":"Bohemian Rhapsody","artist":{"name":"Queen"},"album":{"title":"Opera","cover_big":"http://c/big.jpg"}}]});
        let c = parse_deezer(&j, "Queen", "Bohemian Rhapsody").unwrap();
        assert_eq!(
            (c.album.as_str(), c.cover_url.as_str()),
            ("Opera", "http://c/big.jpg")
        );
        assert!(parse_deezer(&json!({"data":[]}), "a", "b").is_none());
    }

    #[test]
    fn merge_prefers_musicbrainz_then_fills_from_others_and_the_file_name() {
        let found = Identified {
            guess: Guess {
                artist: "Queen".into(),
                title: "Bohemian".into(),
                year: "1975".into(),
                ..Default::default()
            },
            candidates: vec![
                Candidate {
                    source: "iTunes".into(),
                    title: "Bohemian Rhapsody (iTunes)".into(),
                    genre: "Rock".into(),
                    ..Default::default()
                },
                Candidate {
                    source: "MusicBrainz".into(),
                    title: "Bohemian Rhapsody".into(),
                    artist: "Queen".into(),
                    ..Default::default()
                },
            ],
        };
        let m = merge(&found);
        assert_eq!(
            (
                m.title.as_str(),
                m.genre.as_str(),
                m.year.as_str(),
                m.source.as_str()
            ),
            ("Bohemian Rhapsody", "Rock", "1975", "MusicBrainz + iTunes")
        );
        assert_eq!(merge(&Identified::default()).source, "File name");
    }

    #[test]
    fn url_segments_are_encoded() {
        assert_eq!(encode_segment("AC/DC & è"), "AC%2FDC%20%26%20%C3%A8");
    }
}
