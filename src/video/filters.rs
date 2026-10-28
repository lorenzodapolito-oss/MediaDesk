//! Crop, scale, colour and subtitle filters as ffmpeg filter strings.

/// Colour correction, applied only when `enabled`.
#[derive(Debug, Clone, PartialEq)]
pub struct Color {
    pub enabled: bool,
    /// −1..1
    pub brightness: f64,
    /// 0..2
    pub contrast: f64,
    /// 0..3
    pub saturation: f64,
    /// 0.1..3
    pub gamma: f64,
    /// −100 (cooler) .. 100 (warmer)
    pub temperature: i32,
    /// −180..180 degrees
    pub hue: f64,
}

impl Default for Color {
    fn default() -> Self {
        Self {
            enabled: false,
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            gamma: 1.0,
            temperature: 0,
            hue: 0.0,
        }
    }
}

/// Pixels cut from each side of the source frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Crop {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
}

impl Crop {
    pub fn is_none(&self) -> bool {
        *self == Crop::default()
    }

    /// Keeps the cut inside a `w`×`h` frame and leaves at least 16×16 pixels.
    pub fn clamped(self, w: u32, h: u32) -> Crop {
        let side = |a: u32, b: u32, total: u32| {
            let max_total = total.saturating_sub(16);
            let a = a.min(max_total);
            let b = b.min(max_total.saturating_sub(a));
            (a, b)
        };
        let (left, right) = side(self.left, self.right, w);
        let (top, bottom) = side(self.top, self.bottom, h);
        Crop {
            left,
            right,
            top,
            bottom,
        }
    }

    /// From a `cropdetect` result `w:h:x:y` in a `src_w`×`src_h` frame.
    pub fn from_detect(w: u32, h: u32, x: u32, y: u32, src_w: u32, src_h: u32) -> Crop {
        Crop {
            left: x,
            top: y,
            right: src_w.saturating_sub(w).saturating_sub(x),
            bottom: src_h.saturating_sub(h).saturating_sub(y),
        }
        .clamped(src_w, src_h)
    }
}

/// `colorbalance` + `eq` + `hue`, or an empty string when disabled or neutral.
pub fn color_filter(c: &Color) -> String {
    if !c.enabled {
        return String::new();
    }
    let mut f: Vec<String> = Vec::new();
    let t = c.temperature.clamp(-100, 100);
    if t != 0 {
        let k = 0.1 * f64::from(t) / 100.0;
        f.push(format!("colorbalance=rs={k:.3}:bs={:.3}", -k));
    }
    let mut eq: Vec<String> = Vec::new();
    if c.brightness.abs() > 1e-9 {
        eq.push(format!("brightness={:.2}", c.brightness.clamp(-1.0, 1.0)));
    }
    if (c.contrast - 1.0).abs() > 1e-9 {
        eq.push(format!("contrast={:.2}", c.contrast.clamp(0.0, 2.0)));
    }
    if (c.saturation - 1.0).abs() > 1e-9 {
        eq.push(format!("saturation={:.2}", c.saturation.clamp(0.0, 3.0)));
    }
    if (c.gamma - 1.0).abs() > 1e-9 {
        eq.push(format!("gamma={:.2}", c.gamma.clamp(0.1, 3.0)));
    }
    if !eq.is_empty() {
        f.push(format!("eq={}", eq.join(":")));
    }
    if c.hue.abs() > 1e-9 {
        f.push(format!("hue=h={:.1}", c.hue.clamp(-180.0, 180.0)));
    }
    f.join(",")
}

/// Escapes a file path for use inside `subtitles='…'`.
pub fn escape_filter_path(path: &str) -> String {
    path.replace('\\', "/")
        .replace(':', "\\:")
        .replace('\'', "\\'")
        .replace(',', "\\,")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

/// The whole `-vf` chain: crop → scale (max 1280 wide) → colour → burnt-in subtitles → yuv420p.
/// `hardsub` is `(file, subtitle order)`.
pub fn video_filter(crop: Option<Crop>, color: &Color, hardsub: Option<(&str, usize)>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(c) = crop.filter(|c| !c.is_none()) {
        // even sizes, as yuv420p needs
        parts.push(format!(
            "crop=trunc((in_w-{l}-{r})/2)*2:trunc((in_h-{t}-{b})/2)*2:{l}:{t}",
            l = c.left,
            r = c.right,
            t = c.top,
            b = c.bottom
        ));
        parts.push("setsar=1".into());
    }
    parts.push("scale='min(1280,iw)':-2".into());
    let col = color_filter(color);
    if !col.is_empty() {
        parts.push(col);
    }
    if let Some((file, si)) = hardsub {
        parts.push(format!("subtitles='{}':si={si}:force_style='OutlineColour=&H00000000,ShadowColour=&H00000000,MarginV=50'", escape_filter_path(file)));
    }
    parts.push("format=yuv420p".into());
    parts.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_filter_only_lists_what_changed() {
        assert_eq!(color_filter(&Color::default()), "");
        let c = Color {
            enabled: true,
            ..Default::default()
        };
        assert_eq!(color_filter(&c), "");
        let c = Color {
            enabled: true,
            brightness: 0.1,
            saturation: 1.5,
            temperature: 50,
            hue: 10.0,
            ..Default::default()
        };
        assert_eq!(
            color_filter(&c),
            "colorbalance=rs=0.050:bs=-0.050,eq=brightness=0.10:saturation=1.50,hue=h=10.0"
        );
        let off = Color {
            enabled: false,
            brightness: 0.5,
            ..Default::default()
        };
        assert_eq!(color_filter(&off), "");
        // out-of-range values are clamped, NaN does not panic
        let wild = Color {
            enabled: true,
            contrast: 9.0,
            hue: f64::NAN,
            ..Default::default()
        };
        assert!(color_filter(&wild).contains("contrast=2.00"));
    }

    #[test]
    fn crop_is_clamped_and_detected() {
        let c = Crop {
            left: 2000,
            right: 2000,
            top: 0,
            bottom: 5000,
        }
        .clamped(1920, 1080);
        assert!(c.left + c.right <= 1920 - 16 && c.top + c.bottom <= 1080 - 16);
        let d = Crop::from_detect(1920, 800, 0, 140, 1920, 1080);
        assert_eq!(
            d,
            Crop {
                left: 0,
                right: 0,
                top: 140,
                bottom: 140
            }
        );
        // detection bigger than the frame never underflows
        let d = Crop::from_detect(4000, 4000, 10, 10, 1920, 1080);
        assert_eq!((d.right, d.bottom), (0, 0));
    }

    #[test]
    fn chain_order_and_escaping() {
        let vf = video_filter(None, &Color::default(), None);
        assert_eq!(vf, "scale='min(1280,iw)':-2,format=yuv420p");
        let crop = Crop {
            left: 0,
            right: 0,
            top: 140,
            bottom: 140,
        };
        let vf = video_filter(
            Some(crop),
            &Color {
                enabled: true,
                gamma: 1.2,
                ..Default::default()
            },
            Some(("C:\\v\\a [1].mkv", 0)),
        );
        let order: Vec<usize> = ["crop=", "scale=", "eq=", "subtitles=", "format="]
            .iter()
            .map(|k| vf.find(k).unwrap())
            .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{vf}");
        assert!(vf.contains("C\\:/v/a \\[1\\].mkv"), "{vf}");
    }
}
