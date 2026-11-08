//! Panels of a comic page, for the reading order: a page is cut along its
//! gutters (bands of plain background running right across), first into rows,
//! each row into panels, and so on a few levels deep. Manga reads the panels of
//! a row right to left, everything else left to right; rows top to bottom.
//! A page with no gutters (a splash page, a webtoon strip, borderless art) is
//! one panel, and the order stays the plain top-to-bottom one.

use crate::image::{luma, Frame, Rect};

/// Panels in reading order (frame coordinates).
pub fn find(frame: &Frame, rtl: bool) -> Vec<Rect> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    if w < 64 || h < 64 {
        return vec![Rect { x: 0.0, y: 0.0, w: w as f32, h: h as f32 }];
    }
    let lum: Vec<u8> = frame.bgra.chunks_exact(4).map(|p| luma(p[0], p[1], p[2])).collect();
    // The page background: the commonest luminance along the border (white
    // paper usually, sometimes black).
    let mut hist = [0u32; 256];
    for x in 0..w {
        hist[lum[x] as usize] += 1;
        hist[lum[(h - 1) * w + x] as usize] += 1;
    }
    for y in 0..h {
        hist[lum[y * w] as usize] += 1;
        hist[lum[y * w + w - 1] as usize] += 1;
    }
    let bg = hist.iter().enumerate().max_by_key(|(_, n)| **n).map(|(v, _)| v as u8).unwrap_or(255);
    let page = Cut { lum: &lum, w, bg };
    let mut out = Vec::new();
    page.split(0, 0, w, h, true, 0, rtl, &mut out);
    out
}

struct Cut<'a> {
    lum: &'a [u8],
    w: usize,
    bg: u8,
}

impl Cut<'_> {
    fn is_bg(&self, v: u8) -> bool {
        (v as i32 - self.bg as i32).abs() <= 18
    }

    /// Is row `y` (horizontal = true) or column `x` plain background across the region?
    fn plain(&self, horizontal: bool, at: usize, from: usize, to: usize) -> bool {
        let n = to - from;
        let mut off = 0;
        for k in (from..to).step_by(2) {
            let v = if horizontal { self.lum[at * self.w + k] } else { self.lum[k * self.w + at] };
            if !self.is_bg(v) {
                off += 1;
                if off * 2 * 100 > n * 2 {
                    return false; // more than ~2 % is not background
                }
            }
        }
        true
    }

    /// Gutters across the region: (start, end) of each band of plain lines,
    /// leaving out the bands at the region's own edges.
    fn gutters(&self, horizontal: bool, x0: usize, y0: usize, x1: usize, y1: usize) -> Vec<(usize, usize)> {
        let (along0, along1, across0, across1) = if horizontal { (y0, y1, x0, x1) } else { (x0, x1, y0, y1) };
        let min = ((along1 - along0) as f32 * 0.006).max(4.0) as usize;
        let mut bands = Vec::new();
        let mut start: Option<usize> = None;
        for a in along0..along1 {
            if self.plain(horizontal, a, across0, across1) {
                start.get_or_insert(a);
            } else if let Some(s) = start.take() {
                if a - s >= min && s > along0 {
                    bands.push((s, a));
                }
            }
        }
        bands
    }

    /// Cut the region along its gutters (alternating rows and columns) into
    /// panels, appended in reading order.
    #[allow(clippy::too_many_arguments)]
    fn split(&self, x0: usize, y0: usize, x1: usize, y1: usize, horizontal: bool, depth: u32, rtl: bool, out: &mut Vec<Rect>) {
        let leaf = |out: &mut Vec<Rect>| out.push(Rect { x: x0 as f32, y: y0 as f32, w: (x1 - x0) as f32, h: (y1 - y0) as f32 });
        if depth >= 6 || x1 - x0 < 48 || y1 - y0 < 48 {
            leaf(out);
            return;
        }
        let mut gutters = self.gutters(horizontal, x0, y0, x1, y1);
        let mut horizontal = horizontal;
        if gutters.is_empty() {
            // Nothing this way: try the other direction once.
            horizontal = !horizontal;
            gutters = self.gutters(horizontal, x0, y0, x1, y1);
            if gutters.is_empty() {
                leaf(out);
                return;
            }
        }
        // The pieces between the gutters, without the empty margins.
        let (lo, hi) = if horizontal { (y0, y1) } else { (x0, x1) };
        let mut pieces = Vec::new();
        let mut from = lo;
        for (a, b) in gutters {
            if a > from {
                pieces.push((from, a));
            }
            from = b;
        }
        if hi > from {
            pieces.push((from, hi));
        }
        if !horizontal && rtl {
            pieces.reverse();
        }
        for (a, b) in pieces {
            if b - a < 8 {
                continue;
            }
            if horizontal {
                self.split(x0, a, x1, b, false, depth + 1, rtl, out);
            } else {
                self.split(a, y0, b, y1, true, depth + 1, rtl, out);
            }
        }
    }
}

/// Reading order of text blocks: panel by panel, and inside a panel the plain
/// row order (`layout::reading_order`). A block belongs to the panel holding
/// its centre; blocks outside all panels come last.
pub fn reading_order(rects: &[Rect], panels: &[Rect], rtl: bool) -> Vec<usize> {
    let panel_of = |r: &Rect| {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        panels.iter().position(|p| cx >= p.x && cx <= p.x + p.w && cy >= p.y && cy <= p.y + p.h).unwrap_or(panels.len())
    };
    let mut by_panel: Vec<Vec<usize>> = vec![Vec::new(); panels.len() + 1];
    for (i, r) in rects.iter().enumerate() {
        by_panel[panel_of(r)].push(i);
    }
    let mut out = Vec::new();
    for group in by_panel {
        let sub: Vec<Rect> = group.iter().map(|&i| rects[i]).collect();
        for j in crate::layout::reading_order(&sub, rtl) {
            out.push(group[j]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// White page, grey panels at the given rectangles (x, y, w, h).
    fn page(w: u32, h: u32, panels: &[(u32, u32, u32, u32)]) -> Frame {
        let mut bgra = vec![255u8; (w * h * 4) as usize];
        for &(px, py, pw, ph) in panels {
            for y in py..py + ph {
                for x in px..px + pw {
                    let i = ((y * w + x) * 4) as usize;
                    let v = 120 + ((x * 3 + y * 5) % 60) as u8;
                    bgra[i..i + 3].copy_from_slice(&[v, v, v]);
                }
            }
        }
        Frame::new(w, h, bgra)
    }

    #[test]
    fn manga_page_reads_right_to_left() {
        // A tall panel on the right, two stacked on the left, a wide one below.
        let f = page(600, 800, &[(320, 20, 260, 460), (20, 20, 280, 220), (20, 260, 280, 220), (20, 500, 560, 280)]);
        let p = find(&f, true);
        assert_eq!(p.len(), 4, "{p:?}");
        assert!(p[0].x > 300.0, "the right panel first: {p:?}");
        assert!(p[1].x < 300.0 && p[1].y < 200.0 && p[2].y > 200.0 && p[2].y < 500.0);
        assert!(p[3].y > 480.0);
        // A bubble low in the right panel comes before one high in the left panel.
        let bubbles = [Rect { x: 60.0, y: 40.0, w: 100.0, h: 40.0 }, Rect { x: 400.0, y: 400.0, w: 100.0, h: 40.0 }];
        assert_eq!(reading_order(&bubbles, &p, true), vec![1, 0]);
        // The plain order would have read the left one first.
        assert_eq!(crate::layout::reading_order(&bubbles, true), vec![0, 1]);
    }

    #[test]
    fn left_to_right_for_manhwa_and_one_panel_without_gutters() {
        let f = page(600, 400, &[(20, 20, 270, 360), (310, 20, 270, 360)]);
        let p = find(&f, false);
        assert_eq!(p.len(), 2);
        assert!(p[0].x < p[1].x);
        // Artwork right across (no gutters): one panel, the whole page.
        let full = page(400, 400, &[(0, 0, 400, 400)]);
        assert_eq!(find(&full, true).len(), 1);
    }
}
