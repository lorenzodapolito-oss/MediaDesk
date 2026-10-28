//! Reading and writing tags (ID3, Vorbis comments, MP4 atoms) with `lofty`, in every format.

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::items::popularimeter::{Popularimeter, StarRating};
use lofty::tag::{Accessor, ItemKey, Tag};
use std::path::Path;

/// The e-mail field of the rating frame this program writes.
pub const RATING_EMAIL: &str = "MediaDesk";

#[derive(Debug, Default, Clone, PartialEq)]
pub struct AudioInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    pub lyrics: String,
    pub cover: Option<Vec<u8>>,
    /// 0–5 stars.
    pub rating: u8,
    pub duration_secs: f64,
}

/// What to write: empty strings are left untouched, `None` leaves the cover/rating/lyrics alone.
#[derive(Debug, Default, Clone)]
pub struct TagEdit {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    pub lyrics: Option<String>,
    pub cover: Option<Vec<u8>>,
    pub rating: Option<u8>,
}

fn stars(r: StarRating) -> u8 {
    r as u8
}

fn star_rating(n: u8) -> Option<StarRating> {
    Some(match n {
        1 => StarRating::One,
        2 => StarRating::Two,
        3 => StarRating::Three,
        4 => StarRating::Four,
        5 => StarRating::Five,
        _ => return None,
    })
}

pub fn image_mime(data: &[u8]) -> MimeType {
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        MimeType::Png
    } else if data.starts_with(b"GIF8") {
        MimeType::Gif
    } else if data.starts_with(b"BM") {
        MimeType::Bmp
    } else {
        MimeType::Jpeg
    }
}

fn open(path: &Path) -> Result<TaggedFile, String> {
    lofty::read_from_path(path).map_err(|e| e.to_string())
}

fn first_tag(file: &TaggedFile) -> Option<&Tag> {
    file.primary_tag().or_else(|| file.first_tag())
}

pub fn read(path: &Path) -> Result<AudioInfo, String> {
    let file = open(path)?;
    let mut info = AudioInfo { duration_secs: file.properties().duration().as_secs_f64(), ..Default::default() };
    let Some(tag) = first_tag(&file) else { return Ok(info) };
    info.title = tag.title().map(|s| s.trim().to_string()).unwrap_or_default();
    info.artist = tag.artist().map(|s| s.trim().to_string()).unwrap_or_default();
    info.album = tag.album().map(|s| s.trim().to_string()).unwrap_or_default();
    info.genre = tag.genre().map(|s| s.trim().to_string()).unwrap_or_default();
    info.year = tag
        .get_string(ItemKey::Year)
        .or_else(|| tag.get_string(ItemKey::RecordingDate))
        .map(|s| s.chars().take(4).collect())
        .unwrap_or_default();
    info.lyrics = tag
        .get_string(ItemKey::UnsyncLyrics)
        .or_else(|| tag.get_string(ItemKey::Lyrics))
        .unwrap_or_default()
        .to_string();
    info.cover = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first())
        .map(|p| p.data().to_vec());
    info.rating = tag.ratings().next().map(|p| stars(p.rating())).unwrap_or(0);
    Ok(info)
}

/// Writes `edit` into the file's tag (creating the format's usual tag when it has none).
pub fn write(path: &Path, edit: &TagEdit) -> Result<(), String> {
    let mut file = open(path)?;
    if file.primary_tag().is_none() && file.first_tag().is_none() {
        file.insert_tag(Tag::new(file.primary_tag_type()));
    }
    let tag = match file.primary_tag_mut() {
        Some(t) => t,
        None => file.first_tag_mut().ok_or("no tag")?,
    };
    let set = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
    if let Some(v) = set(&edit.title) {
        tag.set_title(v);
    }
    if let Some(v) = set(&edit.artist) {
        tag.set_artist(v);
    }
    if let Some(v) = set(&edit.album) {
        tag.set_album(v);
    }
    if let Some(v) = set(&edit.genre) {
        tag.set_genre(v);
    }
    if let Some(v) = set(&edit.year) {
        tag.insert_text(ItemKey::RecordingDate, v);
    }
    if let Some(l) = &edit.lyrics {
        tag.remove_key(ItemKey::UnsyncLyrics);
        tag.remove_key(ItemKey::Lyrics);
        if !l.trim().is_empty() {
            // ID3v2 only has the unsynchronised kind; the others take either
            let _ = tag.insert_text(ItemKey::UnsyncLyrics, l.clone()) || tag.insert_text(ItemKey::Lyrics, l.clone());
        }
    }
    if let Some(data) = &edit.cover {
        tag.remove_picture_type(PictureType::CoverFront);
        let pic = Picture::unchecked(data.clone()).pic_type(PictureType::CoverFront).mime_type(image_mime(data)).build();
        tag.push_picture(pic);
    }
    if let Some(r) = edit.rating {
        tag.remove_key(ItemKey::Popularimeter);
        if let Some(s) = star_rating(r.min(5)) {
            tag.insert_text(ItemKey::Popularimeter, Popularimeter::custom(RATING_EMAIL, s, 0).to_string());
        }
    }
    file.save_to_path(path, WriteOptions::default()).map_err(|e| e.to_string())
}

/// Stars only (0–5), keeping every other tag as it is.
pub fn write_rating(path: &Path, stars: u8) -> Result<(), String> {
    write(path, &TagEdit { rating: Some(stars), ..Default::default() })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ffmpeg;

    /// A one-second tone encoded as `ext`, or `None` when ffmpeg is missing.
    pub fn make_audio(dir: &Path, name: &str) -> Option<std::path::PathBuf> {
        let ffmpeg = ffmpeg::find_tool("ffmpeg", "")?;
        let out = dir.join(name);
        let args: Vec<String> =
            ["-nostdin", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=1", out.to_str()?].iter().map(|s| s.to_string()).collect();
        let r = ffmpeg::run(&ffmpeg, &args, &ffmpeg::new_cancel(), |_| {}).ok()?;
        r.success.then_some(out)
    }

    pub fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mediadesk-{tag}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    #[test]
    fn tags_cover_lyrics_and_rating_round_trip_in_every_format() {
        let dir = temp_dir("tags");
        // a tiny PNG as cover
        let cover = {
            let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 30, 30, 255]));
            let mut buf = std::io::Cursor::new(Vec::new());
            img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
            buf.into_inner()
        };
        let mut tested = 0;
        for name in ["t.mp3", "t.flac", "t.ogg", "t.m4a"] {
            let Some(p) = make_audio(&dir, name) else { continue };
            let edit = TagEdit {
                title: "Bohemian Rhapsody".into(),
                artist: "Queen".into(),
                album: "A Night at the Opera".into(),
                year: "1975".into(),
                genre: "Rock".into(),
                lyrics: Some("Is this the real life?".into()),
                cover: Some(cover.clone()),
                rating: Some(4),
            };
            if let Err(e) = write(&p, &edit) {
                panic!("{name}: {e}");
            }
            let i = read(&p).unwrap();
            assert_eq!(i.title, "Bohemian Rhapsody", "{name}");
            assert_eq!(i.artist, "Queen", "{name}");
            assert_eq!(i.album, "A Night at the Opera", "{name}");
            assert_eq!(i.year, "1975", "{name}");
            assert_eq!(i.genre, "Rock", "{name}");
            assert_eq!(i.lyrics, "Is this the real life?", "{name}");
            assert!(i.cover.is_some(), "{name} cover");
            assert_eq!(i.rating, 4, "{name} rating");
            assert!(i.duration_secs > 0.5, "{name}");
            // rating change keeps the rest
            write_rating(&p, 1).unwrap();
            let i = read(&p).unwrap();
            assert_eq!((i.rating, i.title.as_str()), (1, "Bohemian Rhapsody"), "{name}");
            write_rating(&p, 0).unwrap();
            assert_eq!(read(&p).unwrap().rating, 0, "{name}");
            tested += 1;
        }
        let _ = std::fs::remove_dir_all(&dir);
        if tested == 0 {
            eprintln!("skipped: no ffmpeg");
        }
    }

    #[test]
    fn broken_files_are_errors() {
        let dir = temp_dir("broken");
        let p = dir.join("x.mp3");
        std::fs::write(&p, b"this is not audio").unwrap();
        assert!(read(&p).is_err() || read(&p).unwrap().title.is_empty());
        assert!(read(&dir.join("missing.mp3")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
