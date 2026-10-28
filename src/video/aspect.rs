//! 16:9 cropping: fitting a 16:9 window into the picture, and keeping it 16:9 when one side
//! of the manual crop is moved.
//!
//! Windows are `32k × 18k` pixels (16:9 exactly, even sizes) and sit at even offsets, which
//! is what yuv420p needs.

use super::filters::Crop;

/// The smallest window: 160 × 90.
pub const MIN_K: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

fn even_down(v: u32) -> u32 {
    v & !1
}

/// The crop that leaves the 16:9 window `k` (32k × 18k) at `x`, `y` in a `w`×`h` frame.
fn window(w: u32, h: u32, k: u32, x: u32, y: u32) -> Crop {
    let (cw, ch) = (32 * k, 18 * k);
    let x = even_down(x.min(w.saturating_sub(cw)));
    let y = even_down(y.min(h.saturating_sub(ch)));
    Crop {
        left: x,
        right: w.saturating_sub(cw).saturating_sub(x),
        top: y,
        bottom: h.saturating_sub(ch).saturating_sub(y),
    }
}

/// Largest `k` that fits a `w`×`h` area, at least 1 when the area is not tiny.
fn max_k(w: u32, h: u32) -> u32 {
    (w / 32).min(h / 18)
}

/// The biggest 16:9 window inside the rectangle `rw`×`rh` at `rx`,`ry` (centred in it), as a crop
/// of the `w`×`h` frame. A rectangle too small for a window gives the whole frame's window.
pub fn fit(w: u32, h: u32, rx: u32, ry: u32, rw: u32, rh: u32) -> Crop {
    let rw = rw.min(w.saturating_sub(rx));
    let rh = rh.min(h.saturating_sub(ry));
    let k = max_k(rw, rh);
    if k < MIN_K {
        return fit_frame(w, h);
    }
    let x = rx + (rw - 32 * k) / 2;
    let y = ry + (rh - 18 * k) / 2;
    window(w, h, k, x, y)
}

/// The biggest centred 16:9 window of the whole frame.
pub fn fit_frame(w: u32, h: u32) -> Crop {
    let k = max_k(w, h).max(1);
    window(
        w,
        h,
        k,
        w.saturating_sub(32 * k) / 2,
        h.saturating_sub(18 * k) / 2,
    )
}

/// From a `cropdetect` result (the picture without its black bars) to a 16:9 crop.
pub fn from_detect(w: u32, h: u32, dw: u32, dh: u32, dx: u32, dy: u32) -> Crop {
    fit(w, h, dx, dy, dw, dh)
}

/// The size of the current window, as `(width, height)`.
pub fn size(w: u32, h: u32, c: Crop) -> (u32, u32) {
    (
        w.saturating_sub(c.left).saturating_sub(c.right),
        h.saturating_sub(c.top).saturating_sub(c.bottom),
    )
}

/// `new` is `old` with the `changed` side moved; returns a crop that is 16:9 again: the
/// opposite side stays where it is, and the two other sides move equally around the old centre.
pub fn adapt(w: u32, h: u32, old: Crop, new: Crop, changed: Side) -> Crop {
    let (ow, oh) = size(w, h, old);
    let (ocx, ocy) = (old.left + ow / 2, old.top + oh / 2);
    match changed {
        Side::Left | Side::Right => {
            let (fixed_edge, want) = if changed == Side::Left {
                let x1 = w.saturating_sub(old.right);
                (x1, x1.saturating_sub(new.left))
            } else {
                (
                    old.left,
                    w.saturating_sub(new.right).saturating_sub(old.left),
                )
            };
            let cap = max_k(w, h);
            let k = ((want + 16) / 32).clamp(MIN_K.min(cap.max(1)), cap.max(1));
            let cw = 32 * k;
            let x = if changed == Side::Left {
                fixed_edge.saturating_sub(cw)
            } else {
                fixed_edge
            };
            let y = ocy.saturating_sub(9 * k);
            window(w, h, k, x, y)
        }
        Side::Top | Side::Bottom => {
            let (fixed_edge, want) = if changed == Side::Top {
                let y1 = h.saturating_sub(old.bottom);
                (y1, y1.saturating_sub(new.top))
            } else {
                (
                    old.top,
                    h.saturating_sub(new.bottom).saturating_sub(old.top),
                )
            };
            let cap = max_k(w, h);
            let k = ((want + 9) / 18).clamp(MIN_K.min(cap.max(1)), cap.max(1));
            let ch = 18 * k;
            let y = if changed == Side::Top {
                fixed_edge.saturating_sub(ch)
            } else {
                fixed_edge
            };
            let x = ocx.saturating_sub(16 * k);
            window(w, h, k, x, y)
        }
    }
}

/// Which side differs between two crops (the first one found), if any.
pub fn changed_side(old: Crop, new: Crop) -> Option<Side> {
    if new.left != old.left {
        Some(Side::Left)
    } else if new.right != old.right {
        Some(Side::Right)
    } else if new.top != old.top {
        Some(Side::Top)
    } else if new.bottom != old.bottom {
        Some(Side::Bottom)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ratio_ok(w: u32, h: u32, c: Crop) -> bool {
        let (cw, ch) = size(w, h, c);
        cw * 9 == ch * 16 && cw & 1 == 0 && ch & 1 == 0 && c.left & 1 == 0 && c.top & 1 == 0
    }

    #[test]
    fn a_full_hd_frame_needs_no_cut() {
        let c = fit_frame(1920, 1080);
        assert_eq!(c, Crop::default());
    }

    #[test]
    fn a_4_3_frame_loses_top_and_bottom() {
        let c = fit_frame(1440, 1080);
        assert!(ratio_ok(1440, 1080, c));
        assert_eq!((c.left, c.right), (0, 0));
        assert!(c.top > 0 && c.top.abs_diff(c.bottom) <= 2);
    }

    #[test]
    fn letterboxed_video_is_cropped_to_the_picture() {
        // 1920x1080 with 140 px bars: the picture is 1920x800 (2.4:1) -> a 16:9 window of it
        let c = from_detect(1920, 1080, 1920, 800, 0, 140);
        assert!(ratio_ok(1920, 1080, c));
        assert!(c.top >= 140 && c.bottom >= 140, "{c:?}");
    }

    #[test]
    fn moving_any_side_keeps_16_9() {
        let (w, h) = (1920, 1080);
        let old = fit(w, h, 100, 100, 1200, 800);
        assert!(ratio_ok(w, h, old));
        for (side, new) in [
            (
                Side::Left,
                Crop {
                    left: old.left + 80,
                    ..old
                },
            ),
            (
                Side::Right,
                Crop {
                    right: old.right + 120,
                    ..old
                },
            ),
            (
                Side::Top,
                Crop {
                    top: old.top + 50,
                    ..old
                },
            ),
            (
                Side::Bottom,
                Crop {
                    bottom: old.bottom + 70,
                    ..old
                },
            ),
            (Side::Left, Crop { left: 0, ..old }),
            (Side::Top, Crop { top: 0, ..old }),
        ] {
            let c = adapt(w, h, old, new, side);
            assert!(ratio_ok(w, h, c), "{side:?}: {c:?}");
            assert!(c.left + c.right < w && c.top + c.bottom < h);
        }
    }

    #[test]
    fn the_opposite_side_stays_put() {
        let (w, h) = (1920, 1080);
        let old = fit(w, h, 200, 100, 1280, 720);
        let c = adapt(
            w,
            h,
            old,
            Crop {
                left: old.left + 64,
                ..old
            },
            Side::Left,
        );
        assert_eq!(c.right, old.right);
        let c = adapt(
            w,
            h,
            old,
            Crop {
                top: old.top + 36,
                ..old
            },
            Side::Top,
        );
        assert_eq!(c.bottom, old.bottom);
    }

    #[test]
    fn nonsense_never_panics_or_leaves_the_frame() {
        for (w, h) in [(0, 0), (10, 10), (31, 17), (64, 36), (5000, 100)] {
            let c = fit_frame(w, h);
            assert!(
                c.left + c.right <= w && c.top + c.bottom <= h,
                "{w}x{h} {c:?}"
            );
            let c = adapt(
                w,
                h,
                Crop::default(),
                Crop {
                    left: 9999,
                    ..Default::default()
                },
                Side::Left,
            );
            assert!(c.left + c.right <= w && c.top + c.bottom <= h);
            let c = fit(w, h, 50, 50, 9999, 9999);
            assert!(c.left + c.right <= w && c.top + c.bottom <= h);
        }
    }

    #[test]
    fn changed_side_finds_the_edit() {
        let a = Crop::default();
        assert_eq!(changed_side(a, a), None);
        assert_eq!(changed_side(a, Crop { bottom: 4, ..a }), Some(Side::Bottom));
    }
}
