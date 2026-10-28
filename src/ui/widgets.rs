//! Small reusable widgets: multi-selection, star rating, cover textures, notices.

use egui::{Color32, Context, TextureHandle, TextureOptions};

/// A multi-selection over list rows (click, Ctrl-click, Shift-click, select all).
#[derive(Default, Debug, Clone)]
pub struct Selection {
    selected: Vec<bool>,
    anchor: Option<usize>,
}

impl Selection {
    pub fn resize(&mut self, len: usize) {
        self.selected = vec![false; len];
        self.anchor = None;
    }

    pub fn contains(&self, i: usize) -> bool {
        self.selected.get(i).copied().unwrap_or(false)
    }

    pub fn indices(&self) -> Vec<usize> {
        self.selected
            .iter()
            .enumerate()
            .filter(|(_, s)| **s)
            .map(|(i, _)| i)
            .collect()
    }

    pub fn first(&self) -> Option<usize> {
        self.selected.iter().position(|s| *s)
    }

    pub fn select_all(&mut self) {
        self.selected.iter_mut().for_each(|s| *s = true);
    }

    pub fn only(&mut self, i: usize) {
        self.selected.iter_mut().for_each(|s| *s = false);
        if let Some(s) = self.selected.get_mut(i) {
            *s = true;
            self.anchor = Some(i);
        }
    }

    pub fn clear(&mut self) {
        self.selected.iter_mut().for_each(|s| *s = false);
    }

    /// Applies a click on row `i`.
    pub fn click(&mut self, i: usize, ctrl: bool, shift: bool) {
        if i >= self.selected.len() {
            return;
        }
        if shift {
            let a = self.anchor.unwrap_or(i);
            let (lo, hi) = (a.min(i), a.max(i));
            if !ctrl {
                self.selected.iter_mut().for_each(|s| *s = false);
            }
            for s in self.selected.iter_mut().take(hi + 1).skip(lo) {
                *s = true;
            }
        } else if ctrl {
            if let Some(s) = self.selected.get_mut(i) {
                *s = !*s;
            }
            self.anchor = Some(i);
        } else {
            self.only(i);
        }
    }

    /// The row after/before `from` (wrapping), for next/previous track.
    pub fn step(&self, from: Option<usize>, delta: isize) -> Option<usize> {
        let n = self.selected.len();
        if n == 0 {
            return None;
        }
        let cur = from.unwrap_or(0) as isize;
        Some((cur + delta).rem_euclid(n as isize) as usize)
    }
}

/// "★★★☆☆" for 0–5 stars.
pub fn stars_text(n: u8) -> String {
    let n = n.min(5) as usize;
    format!("{}{}", "★".repeat(n), "☆".repeat(5 - n))
}

/// Decodes image bytes into a texture, or `None` when they are not an image.
pub fn texture_from_bytes(
    ctx: &Context,
    name: &str,
    bytes: &[u8],
    max_side: u32,
) -> Option<TextureHandle> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width().max(img.height()) > max_side {
        img.thumbnail(max_side, max_side)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
        TextureOptions::LINEAR,
    ))
}

/// A message shown at the bottom, newest last.
#[derive(Debug, Clone)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

pub fn notice_color(n: &Notice) -> Color32 {
    if n.error {
        crate::theme::ACCENT
    } else {
        crate::theme::TEAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_click_modes() {
        let mut s = Selection::default();
        s.resize(6);
        s.click(1, false, false);
        s.click(3, false, true);
        assert_eq!(s.indices(), vec![1, 2, 3]);
        s.click(5, true, false);
        assert_eq!(s.indices(), vec![1, 2, 3, 5]);
        s.click(2, true, false);
        assert_eq!(s.indices(), vec![1, 3, 5]);
        s.click(0, false, false);
        assert_eq!(s.indices(), vec![0]);
        s.select_all();
        assert_eq!(s.indices().len(), 6);
        s.click(99, false, false); // out of range: ignored
        assert_eq!(s.step(Some(5), 1), Some(0));
        assert_eq!(s.step(Some(0), -1), Some(5));
        assert_eq!(Selection::default().step(None, 1), None);
    }

    #[test]
    fn stars() {
        assert_eq!(stars_text(3), "★★★☆☆");
        assert_eq!(stars_text(9), "★★★★★");
    }
}
