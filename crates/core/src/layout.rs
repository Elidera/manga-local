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

/// Group lines using the outlines of speech bubbles found in the picture
/// (`bubbles::find`): every line whose centre is inside one outline belongs to
/// that bubble, however far apart the lines are; two bubbles side by side are
/// never merged. Lines outside every outline are grouped by `group_lines`.
/// Lines of a bubble are ordered top to bottom, then left to right.
pub fn group_in_outlines(lines: &[Rect], outlines: &[Rect]) -> Vec<Vec<usize>> {
    let centre_in = |r: &Rect, o: &Rect| {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        cx >= o.x && cx <= o.x + o.w && cy >= o.y && cy <= o.y + o.h
    };
    let mut per_outline: Vec<Vec<usize>> = vec![Vec::new(); outlines.len()];
    let mut loose: Vec<usize> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        // The smallest outline holding the line (a bubble drawn inside a panel box).
        let best = outlines
            .iter()
            .enumerate()
            .filter(|(_, o)| centre_in(l, o))
            .min_by(|a, b| (a.1.w * a.1.h).partial_cmp(&(b.1.w * b.1.h)).unwrap_or(std::cmp::Ordering::Equal));
        match best {
            Some((k, _)) => per_outline[k].push(i),
            None => loose.push(i),
        }
    }
    let mut out: Vec<Vec<usize>> = Vec::new();
    for mut g in per_outline.into_iter().filter(|g| !g.is_empty()) {
        g.sort_by(|&a, &b| {
            let (ra, rb) = (&lines[a], &lines[b]);
            // Same line when they overlap vertically by half: then left to right.
            let overlap = (ra.y + ra.h).min(rb.y + rb.h) - ra.y.max(rb.y);
            if overlap > ra.h.min(rb.h) * 0.5 {
                ra.x.partial_cmp(&rb.x).unwrap_or(std::cmp::Ordering::Equal)
            } else {
                ra.y.partial_cmp(&rb.y).unwrap_or(std::cmp::Ordering::Equal)
            }
        });
        out.push(g);
    }
    let loose_rects: Vec<Rect> = loose.iter().map(|&i| lines[i]).collect();
    for g in group_lines(&loose_rects) {
        out.push(g.into_iter().map(|j| loose[j]).collect());
    }
    out
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

    #[test]
    fn outlines_keep_bubbles_apart_and_together() {
        // Two bubbles side by side with lines at the same height: proximity would
        // merge them; the outlines keep them apart. A bubble with a big gap
        // between its lines stays one. A caption outside stays alone.
        let lines = [
            r(100.0, 100.0, 120.0, 30.0), // bubble A, line 1
            r(250.0, 100.0, 120.0, 30.0), // bubble B, line 1 (right next to A)
            r(100.0, 135.0, 110.0, 30.0), // bubble A, line 2
            r(255.0, 180.0, 100.0, 30.0), // bubble B, line 2, far below (gap 50)
            r(600.0, 600.0, 200.0, 30.0), // caption, no outline
        ];
        let outlines = [r(80.0, 80.0, 160.0, 110.0), r(240.0, 80.0, 150.0, 150.0)];
        let mut g = group_in_outlines(&lines, &outlines);
        g.sort();
        assert_eq!(g, vec![vec![0, 2], vec![1, 3], vec![4]]);
        // Without outlines, proximity alone would split bubble B (gap 50 > 0.9 line).
        assert!(group_lines(&lines).iter().any(|b| b == &vec![3] || b == &vec![1]));
    }

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
