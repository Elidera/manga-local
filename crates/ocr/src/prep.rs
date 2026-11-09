//! Pictures to network inputs: bilinear resize of a BGR(A) frame and the
//! PaddleOCR normalisation ((v / 255 − 0.5) / 0.5, channels B, G, R, planar).

use manga_core::image::Frame;

/// Sample a pixel (B, G, R) with bilinear interpolation at (x, y) in `f`.
#[inline]
fn sample(f: &Frame, x: f32, y: f32) -> [f32; 3] {
    let (w, h) = (f.width as i32, f.height as i32);
    let x0 = (x.floor() as i32).clamp(0, w - 1);
    let y0 = (y.floor() as i32).clamp(0, h - 1);
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let (fx, fy) = ((x - x0 as f32).clamp(0.0, 1.0), (y - y0 as f32).clamp(0.0, 1.0));
    let px = |xx: i32, yy: i32| {
        let i = ((yy * w + xx) * 4) as usize;
        [f.bgra[i] as f32, f.bgra[i + 1] as f32, f.bgra[i + 2] as f32]
    };
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    let mut out = [0.0; 3];
    for k in 0..3 {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bot = c[k] + (d[k] - c[k]) * fx;
        out[k] = top + (bot - top) * fy;
    }
    out
}

/// The region (x, y, w, h) of `f`, resized to `ow` × `oh`, normalised, planar
/// (3 × oh × ow), written into `out` at row stride `stride` (≥ ow; the rest of
/// each row is left as it is — zeros for padding). `rot`: turn the region a
/// quarter left first (a vertical column becomes a line read left to right).
#[allow(clippy::too_many_arguments)]
pub fn write_region(f: &Frame, x: f32, y: f32, w: f32, h: f32, ow: usize, oh: usize, stride: usize, rot: bool, out: &mut [f32]) {
    let plane = oh * stride;
    for oy in 0..oh {
        for ox in 0..ow {
            // Centre of the output pixel in the region's own coordinates.
            let (u, v) = ((ox as f32 + 0.5) / ow as f32, (oy as f32 + 0.5) / oh as f32);
            // Quarter turn left: output column ← source row from the top,
            // output row ← source column from the right.
            let (sx, sy) = if rot { (x + (1.0 - v) * w, y + u * h) } else { (x + u * w, y + v * h) };
            let p = sample(f, sx - 0.5, sy - 0.5);
            for (k, value) in p.iter().enumerate() {
                out[k * plane + oy * stride + ox] = (value / 255.0 - 0.5) / 0.5;
            }
        }
    }
}

/// Detector input size: the shorter side at least `min_side` (small pictures
/// are enlarged), the longer at most `max_side`, both multiples of 32.
pub fn det_size(w: u32, h: u32, min_side: u32, max_side: u32) -> (usize, usize) {
    let (w, h) = (w as f32, h as f32);
    let mut r = if w.min(h) < min_side as f32 { min_side as f32 / w.min(h) } else { 1.0 };
    if w.max(h) * r > max_side as f32 {
        r = max_side as f32 / w.max(h);
    }
    let round32 = |v: f32| (((v / 32.0).round() as usize) * 32).max(32);
    (round32(w * r), round32(h * r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_multiples_of_32() {
        assert_eq!(det_size(1000, 1400, 736, 2400), (992, 1408));
        // Small: the short side grows to 736.
        assert_eq!(det_size(400, 300, 736, 2400), (992, 736));
        // Huge: the long side shrinks to 2400.
        let (w, h) = det_size(4096, 3053, 736, 2400);
        assert!(w <= 2400 && w % 32 == 0 && h % 32 == 0, "{w} {h}");
    }

    #[test]
    fn quarter_turn_puts_the_top_on_the_left() {
        // 2 × 4 column: top half white, bottom half black.
        let mut bgra = vec![0u8; 2 * 4 * 4];
        for i in 0..4 {
            bgra[i * 4..i * 4 + 3].copy_from_slice(&[255, 255, 255]);
        }
        let f = Frame::new(2, 4, bgra);
        let mut out = vec![0.0; 3 * 2 * 4];
        write_region(&f, 0.0, 0.0, 2.0, 4.0, 4, 2, 4, true, &mut out);
        // First output column (left) = top of the column = white (1.0).
        assert!(out[0] > 0.9 && out[3] < -0.9, "{out:?}");
    }
}
