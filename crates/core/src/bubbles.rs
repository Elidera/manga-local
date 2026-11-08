//! Finding speech bubbles in a picture without a neural network: a bubble is
//! a light, closed area (paper inside an outline) that does not touch the edge
//! of the picture and has dark letters inside. Windows OCR cannot read
//! vertical Japanese; with the bubbles found here the text can be cut out and
//! read by a manga OCR model whatever its direction.

use crate::image::{luma, Frame, Rect};

/// A bubble: its paper area and the block of letters inside.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// Bounding box of the paper (inside the outline).
    pub outer: Rect,
    /// Bounding box of the letters.
    pub text: Rect,
    /// Paper colour (R, G, B).
    pub paper: [u8; 3],
    /// Taller than wide letters block: most likely vertical Japanese.
    pub vertical: bool,
    /// Columns (vertical) or lines (horizontal) of letters.
    pub lines: usize,
    /// Size of one letter, px (column width or line height).
    pub char_px: f32,
}

/// Runs of non-empty cells separated by gaps of at least `gap` empty ones.
fn runs(profile: &[u32], gap: usize) -> usize {
    let (mut n, mut empty, mut inside) = (0, gap, false);
    for &v in profile {
        if v > 0 {
            if !inside && empty >= gap {
                n += 1;
            }
            inside = true;
            empty = 0;
        } else {
            inside = false;
            empty += 1;
        }
    }
    n.max(1)
}

/// Light enough to be paper.
const PAPER: u8 = 200;
/// Dark enough to be a letter.
const INK: u8 = 140;

pub fn find(frame: &Frame) -> Vec<Found> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    if w < 40 || h < 40 {
        return Vec::new();
    }
    let lum: Vec<u8> = frame.bgra.chunks_exact(4).map(|p| luma(p[0], p[1], p[2])).collect();
    // Label the light areas (4-neighbours).
    let mut label = vec![0u32; w * h];
    let mut comps: Vec<Comp> = vec![Comp::default()]; // 0 = none
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..w * h {
        if label[start] != 0 || lum[start] < PAPER {
            continue;
        }
        let id = comps.len() as u32;
        let mut c = Comp { x0: usize::MAX, y0: usize::MAX, ..Default::default() };
        label[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            c.add(x, y, w, h);
            let mut push = |j: usize| {
                if label[j] == 0 && lum[j] >= PAPER {
                    label[j] = id;
                    stack.push(j);
                }
            };
            if x > 0 {
                push(i - 1);
            }
            if x + 1 < w {
                push(i + 1);
            }
            if y > 0 {
                push(i - w);
            }
            if y + 1 < h {
                push(i + w);
            }
        }
        comps.push(c);
    }

    let page = (w * h) as f32;
    let mut out = Vec::new();
    for (id, c) in comps.iter().enumerate().skip(1) {
        let (bw, bh) = (c.x1 - c.x0 + 1, c.y1 - c.y0 + 1);
        let area = c.area as f32;
        // Size: a bubble, not a speck inside a letter nor the page background.
        if c.touches_edge || bw < 30 || bh < 30 || c.area < 1200 || area > page * 0.3 || bw as f32 > w as f32 * 0.9 {
            continue;
        }
        // Round or boxy, not a thin stripe.
        if area / ((bw * bh) as f32) < 0.45 {
            continue;
        }
        // Letters: dark pixels enclosed by this paper in their row.
        let (mut ink, mut ix0, mut iy0, mut ix1, mut iy1) = (0usize, usize::MAX, usize::MAX, 0usize, 0usize);
        let mut by_x = vec![0u32; bw];
        let mut by_y = vec![0u32; bh];
        for y in c.y0..=c.y1 {
            let row = &label[y * w..(y + 1) * w];
            let Some(l) = (c.x0..=c.x1).find(|&x| row[x] == id as u32) else { continue };
            let r = (c.x0..=c.x1).rev().find(|&x| row[x] == id as u32).unwrap();
            for x in l + 1..r {
                if row[x] != id as u32 && lum[y * w + x] < INK {
                    ink += 1;
                    by_x[x - c.x0] += 1;
                    by_y[y - c.y0] += 1;
                    ix0 = ix0.min(x);
                    iy0 = iy0.min(y);
                    ix1 = ix1.max(x);
                    iy1 = iy1.max(y);
                }
            }
        }
        if ink == 0 {
            continue;
        }
        let share = ink as f32 / (area + ink as f32);
        let (tw, th) = (ix1 - ix0 + 1, iy1 - iy0 + 1);
        if !(0.01..=0.45).contains(&share) || tw < 8 || th < 8 {
            continue;
        }
        // The letters sit inside, away from the outline on at least two sides.
        let paper = {
            let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
            for y in (c.y0..=c.y1).step_by(3) {
                for x in (c.x0..=c.x1).step_by(3) {
                    if label[y * w + x] == id as u32 {
                        let p = frame.px(x as u32, y as u32);
                        b += p[0] as u64;
                        g += p[1] as u64;
                        r += p[2] as u64;
                        n += 1;
                    }
                }
            }
            let n = n.max(1);
            [(r / n) as u8, (g / n) as u8, (b / n) as u8]
        };
        let vertical = th as f32 > tw as f32 * 1.3;
        let lines = if vertical { runs(&by_x[ix0 - c.x0..=ix1 - c.x0], 3) } else { runs(&by_y[iy0 - c.y0..=iy1 - c.y0], 3) };
        let char_px = if vertical { tw as f32 / lines as f32 } else { th as f32 / lines as f32 };
        out.push(Found {
            outer: Rect { x: c.x0 as f32, y: c.y0 as f32, w: bw as f32, h: bh as f32 },
            text: Rect { x: ix0 as f32, y: iy0 as f32, w: tw as f32, h: th as f32 },
            paper,
            vertical,
            lines,
            char_px,
        });
    }
    out
}

#[derive(Debug, Default, Clone)]
struct Comp {
    area: usize,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    touches_edge: bool,
}

impl Comp {
    fn add(&mut self, x: usize, y: usize, w: usize, h: usize) {
        self.area += 1;
        self.x0 = self.x0.min(x);
        self.y0 = self.y0.min(y);
        self.x1 = self.x1.max(x);
        self.y1 = self.y1.max(y);
        if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
            self.touches_edge = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Grey artwork with a white ellipse bubble (black outline) holding two
    /// vertical "columns of letters" (dark dashes), a white page margin.
    fn page() -> Frame {
        let (w, h) = (400u32, 300u32);
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let margin = x < 10 || y < 10 || x >= w - 10 || y >= h - 10;
                let (dx, dy) = ((x as f32 - 200.0) / 70.0, (y as f32 - 150.0) / 100.0);
                let r = dx * dx + dy * dy;
                let column = (x == 185 || x == 186 || x == 187 || x == 210 || x == 211 || x == 212) && (100..200).contains(&y) && y % 12 < 8;
                let v = if margin {
                    255
                } else if r < 0.94 {
                    if column { 20 } else { 252 }
                } else if r < 1.0 {
                    0
                } else {
                    120 + ((x * 7 + y * 3) % 40) as u8
                };
                let i = ((y * w + x) * 4) as usize;
                bgra[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        Frame::new(w, h, bgra)
    }

    #[test]
    fn finds_the_bubble_and_its_vertical_text() {
        let found = find(&page());
        assert_eq!(found.len(), 1, "{found:?}");
        let b = &found[0];
        assert!(b.vertical);
        assert!((b.text.x - 185.0).abs() < 1.0 && (b.text.w - 28.0).abs() < 1.5, "{:?}", b.text);
        assert!((b.text.y - 100.0).abs() < 1.0 && b.text.h > 90.0, "{:?}", b.text);
        assert!(b.outer.w > 120.0 && b.outer.h > 180.0);
        assert_eq!(b.paper, [252, 252, 252]);
        assert_eq!(b.lines, 2, "two columns");
        assert!((b.char_px - 14.0).abs() < 1.0, "{}", b.char_px);
    }

    #[test]
    fn profile_runs() {
        assert_eq!(runs(&[0, 1, 1, 0, 0, 0, 1, 0, 1, 1], 3), 2);
        assert_eq!(runs(&[1, 1, 1], 3), 1);
    }

    #[test]
    fn empty_bubble_and_page_margin_are_not_bubbles() {
        let mut f = page();
        // Wipe the letters: an empty white ellipse is not a speech bubble.
        for p in f.bgra.chunks_exact_mut(4) {
            if p[0] == 20 {
                p[..3].copy_from_slice(&[252, 252, 252]);
            }
        }
        assert!(find(&f).is_empty());
    }
}
