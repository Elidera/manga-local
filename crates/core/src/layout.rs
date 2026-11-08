//! Group OCR lines of a whole screen into text blocks (a two-line subtitle,
//! a speaker name above its line, a tooltip paragraph).

use crate::image::Rect;

/// Indices of `lines` grouped into blocks, each block top-to-bottom.
/// Two lines join when the second starts just below the first (gap under ~0.9
/// line height), they overlap horizontally or share a centre, and their heights
/// are similar (a big title is not part of the small text under it).
pub fn group_lines(lines: &[Rect]) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..lines.len()).collect();
    order.sort_by(|&a, &b| lines[a].y.partial_cmp(&lines[b].y).unwrap_or(std::cmp::Ordering::Equal));
    let mut blocks: Vec<Vec<usize>> = Vec::new();
    for i in order {
        let l = &lines[i];
        let mut joined = false;
        for b in blocks.iter_mut().rev() {
            let last = &lines[*b.last().unwrap()];
            let h = last.h.max(l.h).max(1.0);
            let gap = l.y - (last.y + last.h);
            let similar_height = (l.h / last.h.max(1.0)).clamp(0.0, 10.0);
            let x_overlap = (l.x + l.w).min(last.x + last.w) - l.x.max(last.x);
            let centres = ((l.x + l.w / 2.0) - (last.x + last.w / 2.0)).abs() < l.w.max(last.w) / 2.0;
            if gap > -0.4 * h && gap < 0.9 * h && (0.6..=1.7).contains(&similar_height) && (x_overlap > 0.0 || centres) {
                b.push(i);
                joined = true;
                break;
            }
        }
        if !joined {
            blocks.push(vec![i]);
        }
    }
    blocks
}

/// Bounding box of a block.
pub fn block_rect(lines: &[Rect], idx: &[usize]) -> Rect {
    let mut r = lines[idx[0]];
    for &i in &idx[1..] {
        r = r.union(&lines[i]);
    }
    r
}

/// Order blocks the way a reader goes through a page: top to bottom, and
/// within one row left to right (Korean webtoons, Chinese manhua) or right to
/// left (Japanese manga, `rtl`). Two blocks are in one row when their vertical
/// spans overlap by at least half of the shorter one.
pub fn reading_order(rects: &[Rect], rtl: bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..rects.len()).collect();
    order.sort_by(|&a, &b| rects[a].y.partial_cmp(&rects[b].y).unwrap_or(std::cmp::Ordering::Equal));
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for i in order {
        let r = &rects[i];
        let joined = rows.last_mut().filter(|row| {
            row.iter().any(|&j| {
                let o = &rects[j];
                let overlap = (r.y + r.h).min(o.y + o.h) - r.y.max(o.y);
                overlap >= 0.5 * r.h.min(o.h)
            })
        });
        match joined {
            Some(row) => row.push(i),
            None => rows.push(vec![i]),
        }
    }
    let mut out = Vec::with_capacity(rects.len());
    for mut row in rows {
        row.sort_by(|&a, &b| {
            let (xa, xb) = (rects[a].x + rects[a].w / 2.0, rects[b].x + rects[b].w / 2.0);
            let o = xa.partial_cmp(&xb).unwrap_or(std::cmp::Ordering::Equal);
            if rtl {
                o.reverse()
            } else {
                o
            }
        });
        out.extend(row);
    }
    out
}

/// The largest font size (px) in `min_px..=start_px` at which `measure(px)`
/// (text width and height when wrapped to the bubble width) fits `max_h`.
/// Returns `min_px` when even that does not fit: the caller then lets the text
/// grow past the bubble.
pub fn fit_font(start_px: i32, min_px: i32, max_h: i32, mut measure: impl FnMut(i32) -> (i32, i32)) -> i32 {
    let (mut lo, mut hi) = (min_px.max(1), start_px.max(min_px.max(1)));
    if measure(hi).1 <= max_h {
        return hi;
    }
    // Binary search on the largest size that fits.
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if measure(mid).1 <= max_h {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn subtitle_with_speaker_and_separate_menu() {
        let lines = [
            r(1100.0, 1150.0, 120.0, 30.0), // speaker name
            r(700.0, 1190.0, 1000.0, 34.0), // line 1
            r(760.0, 1232.0, 880.0, 34.0),  // line 2
            r(60.0, 100.0, 200.0, 30.0),    // menu item far away
            r(60.0, 300.0, 200.0, 30.0),    // another menu item, big gap
        ];
        let g = group_lines(&lines);
        assert_eq!(g.len(), 3);
        assert!(g.contains(&vec![0, 1, 2]));
        let b = block_rect(&lines, &[0, 1, 2]);
        assert_eq!((b.y, b.y + b.h), (1150.0, 1266.0));
    }

    #[test]
    fn reading_order_rows() {
        // Two bubbles side by side, one below.
        let rects = [r(600.0, 100.0, 200.0, 80.0), r(100.0, 110.0, 200.0, 60.0), r(300.0, 400.0, 200.0, 60.0)];
        assert_eq!(reading_order(&rects, false), vec![1, 0, 2]);
        assert_eq!(reading_order(&rects, true), vec![0, 1, 2]);
    }

    #[test]
    fn font_fits_bubble() {
        // Fake measure: text of 100 chars, char width = px/2, wrapped at 200 px.
        let measure = |px: i32| {
            let per_line = (200 / (px / 2).max(1)).max(1);
            let lines = (100 + per_line - 1) / per_line;
            (200, lines * px)
        };
        let px = fit_font(40, 8, 150, measure);
        assert!(measure(px).1 <= 150);
        assert!(measure(px + 1).1 > 150);
        assert_eq!(fit_font(12, 8, 1000, measure), 12);
        assert_eq!(fit_font(40, 30, 10, measure), 30);
    }

    #[test]
    fn title_not_glued_to_body() {
        let lines = [r(100.0, 100.0, 600.0, 80.0), r(100.0, 190.0, 600.0, 30.0)];
        assert_eq!(group_lines(&lines).len(), 2);
    }
}
