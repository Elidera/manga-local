//! Lettering: the translation drawn into the picture itself (chapter files),
//! with the same rules as the overlay — wrapped inside the bubble, the size
//! picked so it fits, centred, an outline over artwork.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use wuxia_core::image::{Frame, Rect};

pub struct Letterer {
    font: FontVec,
}

/// One block to letter: where the original text was and how it looked.
#[derive(Debug, Clone)]
pub struct Block {
    /// The original text block (frame pixels).
    pub rect: Rect,
    /// Height of one original line, px.
    pub line_h: f32,
    pub text: String,
    /// Text colour (R, G, B).
    pub fg: [u8; 3],
    /// Outline colour, for text over artwork.
    pub outline: Option<[u8; 3]>,
}

impl Letterer {
    pub fn new(font_bytes: Vec<u8>) -> Result<Self, String> {
        let font = FontVec::try_from_vec(font_bytes).map_err(|e| format!("шрифт: {e}"))?;
        Ok(Self { font })
    }

    /// `px` is the em size (as Windows fonts are sized, lfHeight < 0), so the
    /// lettering in files is as big as the overlay's.
    fn scale(&self, px: f32) -> PxScale {
        let em = self.font.units_per_em().unwrap_or(1000.0);
        PxScale::from(px * self.font.height_unscaled() / em)
    }

    fn width(&self, s: &str, px: f32) -> f32 {
        let f = self.font.as_scaled(self.scale(px));
        let mut w = 0.0;
        let mut prev = None;
        for c in s.chars() {
            let g = f.glyph_id(c);
            if let Some(p) = prev {
                w += f.kern(p, g);
            }
            w += f.h_advance(g);
            prev = Some(g);
        }
        w
    }

    fn line_height(&self, px: f32) -> f32 {
        let f = self.font.as_scaled(self.scale(px));
        f.ascent() - f.descent() + f.line_gap()
    }

    /// Break `text` into lines no wider than `max_w`; a word longer than the
    /// line is cut with a hyphen.
    pub fn wrap(&self, text: &str, px: f32, max_w: f32) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let mut cur = String::new();
        for word in text.split_whitespace() {
            let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
            if self.width(&cand, px) <= max_w || cur.is_empty() && self.width(word, px) <= max_w {
                cur = cand;
                continue;
            }
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            if self.width(word, px) <= max_w {
                cur = word.to_string();
                continue;
            }
            // Too long for any line: cut it.
            let mut piece = String::new();
            for c in word.chars() {
                let next = format!("{piece}{c}-");
                if self.width(&next, px) > max_w && piece.chars().count() >= 2 {
                    lines.push(format!("{piece}-"));
                    piece.clear();
                }
                piece.push(c);
            }
            cur = piece;
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        lines
    }

    /// Size, box width and lines for a block — the overlay's rules: start from
    /// the original line height × `font_scale`, a narrow bubble may grow
    /// sideways to 3.5 lines' height, shrink until it fits (down to 45 %).
    pub fn fit(&self, b: &Block, font_scale: f32) -> (f32, f32, Vec<String>) {
        let lh = b.line_h.max(6.0);
        let pad = (lh * 0.18).clamp(2.0, 10.0);
        let box_w = b.rect.w.max(lh * 3.5) + pad;
        let max_h = (b.rect.h + pad).max(lh * 1.2);
        let start = (lh * font_scale).clamp(9.0, 96.0) as i32;
        let min = ((start as f32 * 0.45).max(9.0)) as i32;
        let px = wuxia_core::layout::fit_font(start, min, max_h as i32, |px| {
            let lines = self.wrap(&b.text, px as f32, box_w);
            let w = lines.iter().map(|l| self.width(l, px as f32)).fold(0.0, f32::max);
            let h = self.line_height(px as f32) * lines.len() as f32;
            (w as i32, if w > box_w + 2.0 { i32::MAX } else { h as i32 })
        }) as f32;
        let lines = self.wrap(&b.text, px, box_w);
        (px, box_w, lines)
    }

    /// Draw the block centred on its original text.
    pub fn draw(&self, frame: &mut Frame, b: &Block, font_scale: f32) {
        let (px, _, lines) = self.fit(b, font_scale);
        let lh = self.line_height(px);
        let f = self.font.as_scaled(self.scale(px));
        let (cx, cy) = (b.rect.x + b.rect.w / 2.0, b.rect.y + b.rect.h / 2.0);
        let total = lh * lines.len() as f32;
        let stroke = (px / 12.0).round().max(1.0) as i32;
        for pass in 0..2 {
            let (color, offsets): ([u8; 3], Vec<(i32, i32)>) = match (pass, b.outline) {
                (0, Some(o)) => (o, ring(stroke)),
                (0, None) => continue,
                _ => (b.fg, vec![(0, 0)]),
            };
            for (i, line) in lines.iter().enumerate() {
                let w = self.width(line, px);
                let mut x = cx - w / 2.0;
                let base = cy - total / 2.0 + lh * i as f32 + f.ascent();
                let mut prev = None;
                for c in line.chars() {
                    let gid = f.glyph_id(c);
                    if let Some(p) = prev {
                        x += f.kern(p, gid);
                    }
                    let g = gid.with_scale_and_position(self.scale(px), ab_glyph::point(x, base));
                    x += f.h_advance(gid);
                    prev = Some(gid);
                    let Some(outlined) = self.font.outline_glyph(g) else { continue };
                    let bb = outlined.px_bounds();
                    outlined.draw(|gx, gy, cov| {
                        for &(dx, dy) in &offsets {
                            blend(frame, bb.min.x as i32 + gx as i32 + dx, bb.min.y as i32 + gy as i32 + dy, color, cov);
                        }
                    });
                }
            }
        }
    }
}

fn ring(r: i32) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r * r + r && (dx, dy) != (0, 0) {
                v.push((dx, dy));
            }
        }
    }
    v
}

fn blend(f: &mut Frame, x: i32, y: i32, c: [u8; 3], cov: f32) {
    if x < 0 || y < 0 || x >= f.width as i32 || y >= f.height as i32 || cov <= 0.0 {
        return;
    }
    let a = cov.min(1.0);
    let i = ((y as u32 * f.width + x as u32) * 4) as usize;
    let p = &mut f.bgra[i..i + 3];
    for (k, ch) in [c[2], c[1], c[0]].into_iter().enumerate() {
        p[k] = (ch as f32 * a + p[k] as f32 * (1.0 - a)).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letterer() -> Letterer {
        Letterer::new(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/BalsamiqSans-Regular.ttf")).unwrap()).unwrap()
    }

    #[test]
    fn wraps_and_cuts_long_words() {
        let l = letterer();
        let lines = l.wrap("Прости, что опоздал. Дороги забиты.", 30.0, 200.0);
        assert!(lines.len() >= 2, "{lines:?}");
        assert!(lines.iter().all(|s| l.width(s, 30.0) <= 200.0), "{lines:?}");
        let cut = l.wrap("Достопримечательность", 30.0, 120.0);
        assert!(cut.len() >= 2 && cut[0].ends_with('-'), "{cut:?}");
    }

    #[test]
    fn longer_russian_gets_smaller_but_fits() {
        let l = letterer();
        let b = Block { rect: Rect { x: 100.0, y: 100.0, w: 160.0, h: 70.0 }, line_h: 33.0, text: "Сегодня я угощаю, заказывай всё, что хочешь!".into(), fg: [0, 0, 0], outline: None };
        let (px, box_w, lines) = l.fit(&b, 0.82);
        assert!(px < 33.0 * 0.82, "shrunk: {px}");
        assert!(lines.iter().all(|s| l.width(s, px) <= box_w + 1.0));
        assert!(l.line_height(px) * lines.len() as f32 <= 70.0 + 10.0 + 1.0);
    }

    #[test]
    fn draws_dark_text_on_white() {
        let l = letterer();
        let mut f = Frame::new(300, 120, vec![255; 300 * 120 * 4]);
        let b = Block { rect: Rect { x: 50.0, y: 40.0, w: 200.0, h: 40.0 }, line_h: 36.0, text: "Привет!".into(), fg: [0, 0, 0], outline: Some([255, 255, 255]) };
        l.draw(&mut f, &b, 0.82);
        let dark = f.bgra.chunks_exact(4).filter(|p| p[0] < 100).count();
        assert!(dark > 200, "{dark} dark pixels");
        // Nothing drawn far outside the block.
        assert!((0..300).all(|x| f.px(x, 2)[0] == 255));
    }
}
