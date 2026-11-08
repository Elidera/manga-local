//! Translating chapter files (Stage 3): a folder of pictures or a ZIP / CBZ
//! archive in, the same chapter with the translation lettered into the
//! pictures out. Recognition and translation are the app's (Windows OCR,
//! llama-server); this crate does everything around them and builds on any OS.

pub mod chapter;
pub mod letter;
pub mod slice;

use manga_core::clean;
use manga_core::image::{contrast_text, Frame, Rect};

/// A bubble found on a page, ready to be cleaned and lettered.
#[derive(Debug, Clone)]
pub struct Bubble {
    pub rect: Rect,
    pub line_h: f32,
    /// Colour around the text (paper of the bubble, or the artwork).
    pub around: [u8; 3],
    /// Text over artwork (not a flat bubble).
    pub busy: bool,
    pub translation: String,
}

/// Remove the original text of every bubble, then letter the translations.
pub fn render(frame: &mut Frame, bubbles: &[Bubble], letterer: &letter::Letterer, font_scale: f32) {
    // Clean everything first from the untouched picture, then letter.
    let original = frame.clone();
    let mut fg_outline: Vec<([u8; 3], Option<[u8; 3]>)> = Vec::new();
    for b in bubbles {
        let paper = !b.busy && manga_core::image::luma(b.around[2], b.around[1], b.around[0]) > 200;
        let patch = clean::clean_text(&original, &b.rect, b.line_h, b.around, paper);
        let under = match &patch {
            Some(p) if b.busy => p.mean_rgb(),
            _ => b.around,
        };
        if let Some(p) = patch {
            for y in 0..p.h {
                for x in 0..p.w {
                    let i = ((y * p.w + x) * 4) as usize;
                    if p.bgra[i + 3] == 0 {
                        continue;
                    }
                    let o = (((p.y + y) as u32 * frame.width + (p.x + x) as u32) * 4) as usize;
                    frame.bgra[o..o + 3].copy_from_slice(&p.bgra[i..i + 3]);
                }
            }
        }
        let fg = contrast_text(under);
        let outline = b.busy.then(|| contrast_text(fg));
        fg_outline.push((fg, outline));
    }
    for (b, (fg, outline)) in bubbles.iter().zip(fg_outline) {
        if b.translation.trim().is_empty() {
            continue;
        }
        let block = letter::Block { rect: b.rect, line_h: b.line_h, text: b.translation.clone(), fg, outline };
        letterer.draw(frame, &block, font_scale);
    }
}
