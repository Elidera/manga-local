//! Long webtoon strips (often 10 000–60 000 px tall) are read in overlapping
//! pieces: Windows OCR takes at most 10 000 px and reads small text better in
//! a piece of normal page height. Each recognised line is kept from the piece
//! where it lies away from the cut, so a line cut in one piece is taken whole
//! from the next one.

/// Pieces (y0, y1) covering `height`: at most `max_h` tall, `overlap` px shared.
pub fn pieces(height: u32, max_h: u32, overlap: u32) -> Vec<(u32, u32)> {
    let max_h = max_h.max(overlap * 2 + 1);
    if height <= max_h {
        return vec![(0, height)];
    }
    let step = max_h - overlap;
    let mut v = Vec::new();
    let mut y = 0;
    loop {
        let end = (y + max_h).min(height);
        v.push((y, end));
        if end == height {
            break;
        }
        y += step;
    }
    v
}

/// Is a line whose centre is at `center_y` (strip coordinates) this piece's own?
/// The shared stripe is split in the middle.
pub fn owns(pieces: &[(u32, u32)], i: usize, center_y: f32) -> bool {
    let (y0, y1) = pieces[i];
    let lo = if i == 0 { 0.0 } else { (y0 as f32 + pieces[i - 1].1 as f32) / 2.0 };
    let hi = if i + 1 == pieces.len() { f32::MAX } else { (pieces[i + 1].0 as f32 + y1 as f32) / 2.0 };
    center_y >= lo && center_y < hi
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_page_is_one_piece() {
        assert_eq!(pieces(1400, 2400, 400), vec![(0, 1400)]);
    }

    #[test]
    fn tall_strip_is_cut_with_overlap() {
        let p = pieces(6000, 2400, 400);
        assert_eq!(p, vec![(0, 2400), (2000, 4400), (4000, 6000)]);
        // Every line belongs to exactly one piece.
        for y in (0..6000).step_by(7) {
            let n = (0..p.len()).filter(|&i| owns(&p, i, y as f32)).count();
            assert_eq!(n, 1, "y = {y}");
        }
        // A line across the first cut (2380..2420, centre 2400) is not the first
        // piece's: there it is cut off, the second piece has it whole.
        assert!(!owns(&p, 0, 2400.0) && owns(&p, 1, 2400.0));
    }
}
