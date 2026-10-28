//! Artist / title / album / year guessed from a file name.

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Guess {
    pub artist: String,
    pub title: String,
    pub album: String,
    pub year: String,
}

fn is_year(s: &str) -> bool {
    s.len() == 4 && s.chars().all(|c| c.is_ascii_digit())
}

/// `Artist - Title [Album|Year]`, `Artist - Title (Album|Year)`, `Artist - Title - Album|Year`,
/// `Artist - Title`, `Artist_Title`, or just the title.
pub fn guess_from_filename(file_name: &str) -> Guess {
    let stem = std::path::Path::new(file_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = stem.trim();
    let mut g = Guess::default();
    let extra = |g: &mut Guess, third: &str| {
        let third = third.trim();
        if is_year(third) {
            g.year = third.to_string();
        } else if !third.is_empty() {
            g.album = third.to_string();
        }
    };
    if let Some((artist, rest)) = stem.split_once(" - ") {
        g.artist = artist.trim().to_string();
        let rest = rest.trim();
        // "Title [x]" / "Title (x)"
        for (open, close) in [('[', ']'), ('(', ')')] {
            if let (Some(o), true) = (rest.rfind(open), rest.ends_with(close)) {
                let inner = rest.get(o + 1..rest.len() - 1).unwrap_or("");
                let title = rest.get(..o).unwrap_or("").trim();
                if !title.is_empty() {
                    g.title = title.to_string();
                    extra(&mut g, inner);
                    return g;
                }
            }
        }
        if let Some((title, third)) = rest.split_once(" - ") {
            g.title = title.trim().to_string();
            extra(&mut g, third);
        } else {
            g.title = rest.to_string();
        }
        return g;
    }
    if let Some((artist, title)) = stem.split_once('_') {
        g.artist = artist.trim().to_string();
        g.title = title.trim().to_string();
        return g;
    }
    g.title = stem.to_string();
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses() {
        let g = guess_from_filename("Queen - Bohemian Rhapsody [1975].mp3");
        assert_eq!((g.artist.as_str(), g.title.as_str(), g.year.as_str(), g.album.as_str()), ("Queen", "Bohemian Rhapsody", "1975", ""));
        let g = guess_from_filename("Queen - Bohemian Rhapsody (A Night at the Opera).mp3");
        assert_eq!(g.album, "A Night at the Opera");
        let g = guess_from_filename("Queen - Bohemian Rhapsody - 1975.mp3");
        assert_eq!((g.title.as_str(), g.year.as_str()), ("Bohemian Rhapsody", "1975"));
        let g = guess_from_filename("Queen - Bohemian Rhapsody.flac");
        assert_eq!((g.artist.as_str(), g.title.as_str()), ("Queen", "Bohemian Rhapsody"));
        let g = guess_from_filename("Queen_Bohemian.mp3");
        assert_eq!((g.artist.as_str(), g.title.as_str()), ("Queen", "Bohemian"));
        let g = guess_from_filename("solo titolo.mp3");
        assert_eq!((g.artist.as_str(), g.title.as_str()), ("", "solo titolo"));
        // odd input never panics
        for s in ["", " - ", "[", "a - [", "a - ]x[", "é - ü (ö", "a - b (c"] {
            let _ = guess_from_filename(s);
        }
    }
}
