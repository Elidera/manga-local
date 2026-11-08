//! Removing the original lettering before the translation is drawn.
//!
//! The overlay cannot erase pixels of the page, but it can lay a picture over
//! them. Two kinds of pictures ("patches"):
//!
//! * inside a speech bubble — only the letters are covered with the paper
//!   colour (a mask of the ink, a little wider), so no rectangle shows on the
//!   bubble and its outline stays untouched;
//! * text over artwork — the area is filled from the colours around it
//!   (push–pull inpainting: averages on ever coarser grids, then smoothly
//!   back), so gradients and tones continue under the translation instead of
//!   a dark plate.

use crate::image::{Frame, Rect};

/// A piece of picture laid over the page: straight-alpha BGRA8, `w * h * 4`
/// bytes; alpha 0 leaves the page visible.
#[derive(Debug, Clone, PartialEq)]
pub struct Patch {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub bgra: Vec<u8>,
}

impl Patch {
    pub fn offset(mut self, dx: i32, dy: i32) -> Patch {
        self.x += dx;
        self.y += dy;
        self
    }

    /// Mean colour of the covered pixels (R, G, B).
    pub fn mean_rgb(&self) -> [u8; 3] {
        let (mut s, mut n) = ([0u64; 3], 0u64);
        for p in self.bgra.chunks_exact(4).filter(|p| p[3] > 0) {
            s[0] += p[2] as u64;
            s[1] += p[1] as u64;
            s[2] += p[0] as u64;
            n += 1;
        }
        if n == 0 {
            return [255, 255, 255];
        }
        [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]
    }
}

/// Clip `r` grown by `pad` to the frame: (x0, y0, x1, y1), end exclusive.
fn clip(frame: &Frame, r: &Rect, pad: f32) -> Option<(i32, i32, i32, i32)> {
    let x0 = (r.x - pad).floor().max(0.0) as i32;
    let y0 = (r.y - pad).floor().max(0.0) as i32;
    let x1 = ((r.x + r.w + pad).ceil() as i32).min(frame.width as i32);
    let y1 = ((r.y + r.h + pad).ceil() as i32).min(frame.height as i32);
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// Remove the text of one block. `rect` is the text block, `line_h` its line
/// height, `around` the colour around it (the bubble paper, or the median of
/// the artwork). The letters are the pixels that differ from `around`, widened
/// by a pixel or two (anti-aliased edges, outlines of the letters); only they
/// are replaced, each from its neighbours (push–pull). In a bubble that gives
/// clean paper; over artwork tones, gradients and lines between the letters
/// stay and continue under them.
pub fn clean_text(frame: &Frame, rect: &Rect, line_h: f32, around: [u8; 3]) -> Option<Patch> {
    let pad = (line_h * 0.2).clamp(2.0, 8.0);
    let (x0, y0, x1, y1) = clip(frame, rect, pad)?;
    let (w, h) = (x1 - x0, y1 - y0);
    let [ar, ag, ab] = around;
    let mut ink = vec![false; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let p = frame.px((x0 + x) as u32, (y0 + y) as u32);
            let d = (p[2] as i32 - ar as i32).abs() + (p[1] as i32 - ag as i32).abs() + (p[0] as i32 - ab as i32).abs();
            ink[(y * w + x) as usize] = d > 45;
        }
    }
    let grow = ((line_h / 11.0).round() as i32).clamp(1, 5);
    let hole = dilate(&ink, w, h, grow);
    // Known pixels: everything in the block that is not a letter, plus a ring around it.
    let ring = 4;
    let (ox0, oy0) = ((x0 - ring).max(0), (y0 - ring).max(0));
    let (ox1, oy1) = ((x1 + ring).min(frame.width as i32), (y1 + ring).min(frame.height as i32));
    let (ow, oh) = ((ox1 - ox0) as usize, (oy1 - oy0) as usize);
    let mut col = vec![[0f32; 3]; ow * oh];
    let mut wt = vec![0f32; ow * oh];
    for y in 0..oh {
        for x in 0..ow {
            let (fx, fy) = (ox0 + x as i32, oy0 + y as i32);
            let inside = fx >= x0 && fx < x1 && fy >= y0 && fy < y1;
            if inside && hole[((fy - y0) * w + (fx - x0)) as usize] {
                continue;
            }
            let p = frame.px(fx as u32, fy as u32);
            col[y * ow + x] = [p[0] as f32, p[1] as f32, p[2] as f32];
            wt[y * ow + x] = 1.0;
        }
    }
    let filled = push_pull(&col, &wt, ow, oh);
    let mut bgra = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            if !hole[(y * w + x) as usize] {
                continue;
            }
            let c = filled[(y + y0 - oy0) as usize * ow + (x + x0 - ox0) as usize];
            let i = ((y * w + x) * 4) as usize;
            bgra[i..i + 4].copy_from_slice(&[c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, 255]);
        }
    }
    Some(Patch { x: x0, y: y0, w, h, bgra })
}

fn dilate(m: &[bool], w: i32, h: i32, r: i32) -> Vec<bool> {
    let mut out = vec![false; m.len()];
    for y in 0..h {
        for x in 0..w {
            if !m[(y * w + x) as usize] {
                continue;
            }
            for yy in (y - r).max(0)..=(y + r).min(h - 1) {
                for xx in (x - r).max(0)..=(x + r).min(w - 1) {
                    out[(yy * w + xx) as usize] = true;
                }
            }
        }
    }
    out
}

/// Push–pull: average the known pixels on halved grids down to 1×1, then on
/// the way back fill what is unknown from the coarser level (bilinear).
fn push_pull(col: &[[f32; 3]], wt: &[f32], w: usize, h: usize) -> Vec<[f32; 3]> {
    if w <= 1 && h <= 1 {
        return col.to_vec();
    }
    let (cw, chh) = (w.div_ceil(2), h.div_ceil(2));
    let mut ccol = vec![[0f32; 3]; cw * chh];
    let mut cwt = vec![0f32; cw * chh];
    for y in 0..h {
        for x in 0..w {
            let k = wt[y * w + x];
            if k > 0.0 {
                let c = (y / 2) * cw + x / 2;
                for (i, v) in ccol[c].iter_mut().enumerate() {
                    *v += col[y * w + x][i] * k;
                }
                cwt[c] += k;
            }
        }
    }
    for (c, k) in ccol.iter_mut().zip(cwt.iter_mut()) {
        if *k > 0.0 {
            for v in c.iter_mut() {
                *v /= *k;
            }
            *k = k.min(1.0);
        }
    }
    let coarse = push_pull(&ccol, &cwt, cw, chh);
    let sample = |fx: f32, fy: f32| -> [f32; 3] {
        let fx = fx.clamp(0.0, (cw - 1) as f32);
        let fy = fy.clamp(0.0, (chh - 1) as f32);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(cw - 1), (y0 + 1).min(chh - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let mut out = [0f32; 3];
        for (i, o) in out.iter_mut().enumerate() {
            let a = coarse[y0 * cw + x0][i] * (1.0 - tx) + coarse[y0 * cw + x1][i] * tx;
            let b = coarse[y1 * cw + x0][i] * (1.0 - tx) + coarse[y1 * cw + x1][i] * tx;
            *o = a * (1.0 - ty) + b * ty;
        }
        out
    };
    let mut out = col.to_vec();
    for y in 0..h {
        for x in 0..w {
            let k = wt[y * w + x].min(1.0);
            if k >= 1.0 {
                continue;
            }
            let up = sample((x as f32 - 0.5) / 2.0, (y as f32 - 0.5) / 2.0);
            for (i, o) in out[y * w + x].iter_mut().enumerate() {
                *o = *o * k + up[i] * (1.0 - k);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Frame {
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let [r, g, b] = f(x, y);
                bgra.extend_from_slice(&[b, g, r, 255]);
            }
        }
        Frame::new(w, h, bgra)
    }

    fn at(p: &Patch, x: i32, y: i32) -> [u8; 4] {
        let i = (((y - p.y) * p.w + (x - p.x)) * 4) as usize;
        [p.bgra[i], p.bgra[i + 1], p.bgra[i + 2], p.bgra[i + 3]]
    }

    #[test]
    fn bubble_cleaning_covers_only_the_ink() {
        // White paper, a black stroke in the middle, a black bubble outline far away.
        let f = frame(120, 60, |x, y| if (40..80).contains(&x) && (28..32).contains(&y) || x == 2 { [0, 0, 0] } else { [255, 255, 255] });
        let r = Rect { x: 38.0, y: 24.0, w: 44.0, h: 12.0 };
        let p = clean_text(&f, &r, 12.0, [255, 255, 255]).unwrap();
        assert_eq!(at(&p, 60, 30), [255, 255, 255, 255], "the stroke is covered with paper");
        assert_eq!(at(&p, 60, 27)[3], 255, "and a pixel around it");
        assert_eq!(at(&p, 45, 25)[3], 0, "plain paper stays the page itself");
        assert!(p.x > 2, "the bubble outline is outside the patch");
    }

    #[test]
    fn letters_over_a_gradient_take_its_colour() {
        // A left-to-right gradient with white letters (every third column) in the middle.
        let grad = |x: u32| {
            let v = (x * 255 / 199) as u8;
            [v, v / 2, 255 - v]
        };
        let f = frame(200, 80, |x, y| if (60..140).contains(&x) && (30..50).contains(&y) && x % 3 == 0 { [255, 255, 255] } else { grad(x) });
        let r = Rect { x: 60.0, y: 30.0, w: 80.0, h: 20.0 };
        let p = clean_text(&f, &r, 20.0, grad(100)).unwrap();
        for y in 0..p.h {
            for x in 0..p.w {
                let i = ((y * p.w + x) * 4) as usize;
                if p.bgra[i + 3] == 0 {
                    continue;
                }
                let want = grad((p.x + x) as u32)[0] as i32;
                let got = p.bgra[i + 2] as i32; // red channel
                assert!((got - want).abs() <= 30, "({}, {}): {got} vs {want}", p.x + x, p.y + y);
            }
        }
        // Every white letter pixel is covered.
        assert_eq!(at(&p, 99, 40)[3], 255);
    }

    #[test]
    fn flat_background_stays_flat() {
        let f = frame(100, 50, |x, y| if (40..60).contains(&x) && (15..35).contains(&y) { [0, 0, 0] } else { [90, 120, 200] });
        let p = clean_text(&f, &Rect { x: 40.0, y: 15.0, w: 20.0, h: 20.0 }, 20.0, [90, 120, 200]).unwrap();
        assert_eq!(p.mean_rgb(), [90, 120, 200]);
    }
}
