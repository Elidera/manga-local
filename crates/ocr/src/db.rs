//! Text boxes from the detector's probability map (Differentiable
//! Binarization, as PaddleOCR does it, with upright boxes): threshold, close
//! small holes, connected pieces, drop the unsure ones, grow each box back to
//! the letters' full size (the network marks a shrunk core of each line).

use manga_core::image::Rect;

pub struct DbParams {
    /// A pixel is text above this probability.
    pub thresh: f32,
    /// A piece is kept when its mean probability is above this.
    pub box_thresh: f32,
    /// How much a piece grows back: area × ratio / perimeter on each side.
    pub unclip_ratio: f32,
}

impl Default for DbParams {
    fn default() -> Self {
        Self { thresh: 0.3, box_thresh: 0.5, unclip_ratio: 1.6 }
    }
}

/// `prob`: w × h map. Boxes in map coordinates with their mean probability.
pub fn boxes(prob: &[f32], w: usize, h: usize, p: &DbParams) -> Vec<(Rect, f32)> {
    // Threshold, then dilate with a 2 × 2 kernel (joins letters split by one pixel).
    let raw: Vec<bool> = prob.iter().map(|&v| v > p.thresh).collect();
    let mut mask = raw.clone();
    for y in 0..h {
        for x in 0..w {
            if raw[y * w + x] {
                continue;
            }
            let near = (x > 0 && raw[y * w + x - 1]) || (y > 0 && raw[(y - 1) * w + x]) || (x > 0 && y > 0 && raw[(y - 1) * w + x - 1]);
            if near {
                mask[y * w + x] = true;
            }
        }
    }
    // Connected pieces (8-neighbourhood), each as its bounding box and mean probability.
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !mask[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        let (mut sum, mut n) = (0.0f32, 0usize);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            sum += prob[i];
            n += 1;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if mask[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        let (bw, bh) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
        if bw.min(bh) < 3.0 {
            continue;
        }
        let score = sum / n as f32;
        if score < p.box_thresh {
            continue;
        }
        let d = bw * bh * p.unclip_ratio / (2.0 * (bw + bh));
        let r = Rect { x: x0 as f32 - d, y: y0 as f32 - d, w: bw + 2.0 * d, h: bh + 2.0 * d };
        let r = clip(r, w as f32, h as f32);
        if r.w.min(r.h) < 5.0 {
            continue;
        }
        out.push((r, score));
    }
    out
}

fn clip(r: Rect, w: f32, h: f32) -> Rect {
    let (x0, y0) = (r.x.max(0.0), r.y.max(0.0));
    let (x1, y1) = ((r.x + r.w).min(w), (r.y + r.h).min(h));
    Rect { x: x0, y: y0, w: (x1 - x0).max(0.0), h: (y1 - y0).max(0.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_lines_two_boxes_grown_back() {
        let (w, h) = (100, 40);
        let mut prob = vec![0.0f32; w * h];
        // Two shrunk line cores: x 10..60 at rows 5..9, x 10..80 at rows 25..29.
        for y in 5..9 {
            for x in 10..60 {
                prob[y * w + x] = 0.9;
            }
        }
        for y in 25..29 {
            for x in 10..80 {
                prob[y * w + x] = 0.8;
            }
        }
        // A faint speck: dropped.
        prob[35 * w + 90] = 0.4;
        let b = boxes(&prob, w, h, &DbParams::default());
        assert_eq!(b.len(), 2, "{:?}", b.iter().map(|x| x.0).collect::<Vec<_>>());
        let first = b.iter().find(|(r, _)| r.y < 10.0).unwrap().0;
        // Grown beyond the 4 px core on both sides.
        assert!(first.y < 5.0 && first.y + first.h > 9.0 && first.x < 10.0, "{first:?}");
    }
}
