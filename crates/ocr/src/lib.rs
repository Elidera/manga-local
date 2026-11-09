//! Neural OCR for comic pages: PaddleOCR models in ONNX form (Apache-2.0,
//! taken from RapidOCR). A detector finds every text line on the page —
//! speech, captions, titles with outlines or glow, sound effects, slanted or
//! vertical text — and a recogniser reads each line on its own, so a huge
//! title next to small speech is no problem (unlike Windows OCR, which picks
//! one letter size per picture).
//!
//! Platform-independent: Windows builds link ONNX Runtime in (feature
//! `download`), tests and tools load it at run time (feature `dynamic`).

pub mod ctc;
pub mod db;
pub mod prep;

use anyhow::{anyhow, Context, Result};
use manga_core::image::{Frame, Rect};
use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;
use std::sync::Mutex;

/// One line read on the page (frame coordinates).
#[derive(Debug, Clone)]
pub struct Line {
    pub text: String,
    pub rect: Rect,
    /// Mean probability of the symbols (0–1).
    pub score: f32,
}

/// Built with the video card path (DirectML, Windows)?
pub fn gpu_compiled() -> bool {
    cfg!(feature = "gpu")
}

/// `gpu`: run on the video card through DirectML when the build has it; a
/// card or driver that cannot leaves the work to the processor by itself.
fn load(path: &Path, threads: usize, gpu: bool) -> Result<Session> {
    let b = Session::builder().map_err(|e| anyhow!("{e}"))?;
    let mut b = b.with_intra_threads(threads.max(1)).map_err(|e| anyhow!("{e}"))?;
    #[cfg(feature = "gpu")]
    if gpu {
        // DirectML needs these two off.
        b = b.with_memory_pattern(false).map_err(|e| anyhow!("{e}"))?;
        b = b.with_parallel_execution(false).map_err(|e| anyhow!("{e}"))?;
        b = b
            .with_execution_providers([ort::ep::DirectML::default().with_performance_preference(ort::ep::directml::PerformancePreference::HighPerformance).build()])
            .map_err(|e| anyhow!("{e}"))?;
    }
    #[cfg(not(feature = "gpu"))]
    let _ = gpu;
    b.commit_from_file(path).map_err(|e| anyhow!("{}: {e}", path.display()))
}

/// Finds text lines.
pub struct Detector {
    session: Mutex<Session>,
}

impl Detector {
    pub fn load(path: &Path, threads: usize, gpu: bool) -> Result<Self> {
        Ok(Self { session: Mutex::new(load(path, threads, gpu)?) })
    }

    /// Text line boxes in frame coordinates with their probability.
    pub fn find(&self, f: &Frame) -> Result<Vec<(Rect, f32)>> {
        if f.width < 8 || f.height < 8 {
            return Ok(vec![]);
        }
        let (dw, dh) = prep::det_size(f.width, f.height, 736, 2400);
        let mut data = vec![0f32; 3 * dw * dh];
        prep::write_region(f, 0.0, 0.0, f.width as f32, f.height as f32, dw, dh, dw, false, &mut data);
        let input = Tensor::from_array(([1usize, 3, dh, dw], data)).map_err(|e| anyhow!("{e}"))?;
        let mut s = self.session.lock().unwrap();
        let out = s.run(ort::inputs![input]).map_err(|e| anyhow!("поиск текста: {e}"))?;
        let (shape, prob) = out[0].try_extract_tensor::<f32>().map_err(|e| anyhow!("{e}"))?;
        let (mh, mw) = (shape[2] as usize, shape[3] as usize);
        let (sx, sy) = (f.width as f32 / mw as f32, f.height as f32 / mh as f32);
        Ok(db::boxes(prob, mw, mh, &db::DbParams::default())
            .into_iter()
            .map(|(r, p)| (Rect { x: r.x * sx, y: r.y * sy, w: r.w * sx, h: r.h * sy }, p))
            .collect())
    }
}

/// Reads one line of text.
pub struct Recognizer {
    session: Mutex<Session>,
    chars: Vec<String>,
    gpu: bool,
}

const REC_H: usize = 48;

impl Recognizer {
    pub fn load(path: &Path, threads: usize, gpu: bool) -> Result<Self> {
        let session = load(path, threads, gpu)?;
        let embedded = session
            .metadata()
            .map_err(|e| anyhow!("{e}"))?
            .custom("character")
            .with_context(|| format!("{}: в модели нет алфавита", path.display()))?;
        Ok(Self { session: Mutex::new(session), chars: ctc::alphabet(&embedded), gpu })
    }

    /// Read each box of `f`: (text, probability), in the order of `boxes`.
    /// A box taller than 1.5 × its width is a vertical column: turned first.
    pub fn read(&self, f: &Frame, boxes: &[Rect]) -> Result<Vec<(String, f32)>> {
        let mut out = vec![(String::new(), 0.0); boxes.len()];
        // Similar shapes together: less padding in a batch.
        let ratio = |r: &Rect| {
            let rot = r.h >= r.w * 1.5;
            (rot, if rot { r.h / r.w.max(1.0) } else { r.w / r.h.max(1.0) })
        };
        let mut order: Vec<usize> = (0..boxes.len()).filter(|&i| boxes[i].w >= 2.0 && boxes[i].h >= 2.0).collect();
        order.sort_by(|&a, &b| ratio(&boxes[a]).1.total_cmp(&ratio(&boxes[b]).1));
        for chunk in order.chunks(8) {
            let max_ratio = chunk.iter().map(|&i| ratio(&boxes[i]).1).fold(320.0 / 48.0, f32::max).min(40.0);
            let mut width = (REC_H as f32 * max_ratio).ceil() as usize;
            // The video card prepares itself for every new input size: a few
            // sizes only (width in steps of 4 heights, always 8 rows).
            let rows = if self.gpu { 8 } else { chunk.len() };
            if self.gpu {
                width = width.div_ceil(REC_H * 4) * REC_H * 4;
            }
            let per = 3 * REC_H * width;
            let mut data = vec![0f32; rows * per];
            for (k, &i) in chunk.iter().enumerate() {
                let r = &boxes[i];
                let (rot, q) = ratio(r);
                let rw = ((REC_H as f32 * q).ceil() as usize).clamp(1, width);
                prep::write_region(f, r.x, r.y, r.w, r.h, rw, REC_H, width, rot, &mut data[k * per..(k + 1) * per]);
            }
            let input = Tensor::from_array(([rows, 3, REC_H, width], data)).map_err(|e| anyhow!("{e}"))?;
            let mut s = self.session.lock().unwrap();
            let res = s.run(ort::inputs![input]).map_err(|e| anyhow!("чтение текста: {e}"))?;
            let (shape, logits) = res[0].try_extract_tensor::<f32>().map_err(|e| anyhow!("{e}"))?;
            let (t, c) = (shape[1] as usize, shape[2] as usize);
            // Blank + alphabet + space; one less when the list already ends in a space.
            if c != self.chars.len() && c + 1 != self.chars.len() {
                return Err(anyhow!("алфавит модели не совпадает: {} знаков, выходов {c}", self.chars.len()));
            }
            for (k, &i) in chunk.iter().enumerate() {
                out[i] = ctc::decode(&logits[k * t * c..(k + 1) * t * c], t, c, &self.chars);
            }
        }
        Ok(out)
    }
}

/// Detector + recogniser.
pub struct Engine {
    pub det: Detector,
    pub rec: Recognizer,
}

/// Large single symbols one under another («あ» over «っ») are found as
/// separate boxes: joined into one column, they are read together.
fn stack_columns(mut rects: Vec<Rect>) -> Vec<Rect> {
    // Not a horizontal line: a symbol or a column.
    let upright = |r: &Rect| r.w <= r.h * 1.5;
    loop {
        let mut joined = None;
        'search: for i in 0..rects.len() {
            for j in 0..rects.len() {
                let (a, b) = (rects[i], rects[j]);
                // a above b, both symbols or columns already.
                if i == j || !upright(&a) || !upright(&b) || b.y < a.y + a.h * 0.5 {
                    continue;
                }
                let narrow = a.w.min(b.w);
                let overlap = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
                let gap = b.y - (a.y + a.h);
                if overlap >= narrow * 0.7 && a.w.min(b.w) >= a.w.max(b.w) * 0.6 && gap < narrow * 0.6 {
                    joined = Some((i, j));
                    break 'search;
                }
            }
        }
        let Some((i, j)) = joined else { return rects };
        let u = rects[i].union(&rects[j]);
        rects[i] = u;
        rects.remove(j);
    }
}

impl Engine {
    /// `gpu`: on the video card when the build has it (see `gpu_compiled`).
    pub fn load(det: &Path, rec: &Path, threads: usize, gpu: bool) -> Result<Self> {
        let gpu = gpu && gpu_compiled();
        Ok(Self { det: Detector::load(det, threads, gpu)?, rec: Recognizer::load(rec, threads, gpu)? })
    }

    /// All lines of the page read with at least `min_score`.
    pub fn read(&self, f: &Frame, min_score: f32) -> Result<Vec<Line>> {
        let found = self.det.find(f)?;
        let rects = stack_columns(found.iter().map(|(r, _)| *r).collect());
        let texts = self.rec.read(f, &rects)?;
        Ok(rects
            .into_iter()
            .zip(texts)
            // One or two symbols need more certainty: drawn circles and
            // strokes are read as «O», «1», «一».
            .filter(|(_, (t, s))| {
                let n = t.trim().chars().count();
                let need = match n {
                    1 => min_score.max(0.9),
                    2 => min_score.max(0.8),
                    _ => min_score,
                };
                n > 0 && *s >= need
            })
            .map(|(rect, (text, score))| Line { text: text.trim().to_string(), rect, score })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn big_symbols_one_under_another_make_a_column() {
        let got = stack_columns(vec![r(266.0, 576.0, 79.0, 87.0), r(274.0, 668.0, 65.0, 58.0)]);
        assert_eq!(got.len(), 1);
        assert!(got[0].h > got[0].w * 1.5);
    }

    #[test]
    fn lines_of_text_stay_apart() {
        // Two horizontal lines, and two symbols side by side.
        assert_eq!(stack_columns(vec![r(100.0, 100.0, 300.0, 30.0), r(100.0, 135.0, 280.0, 30.0)]).len(), 2);
        assert_eq!(stack_columns(vec![r(100.0, 100.0, 40.0, 40.0), r(150.0, 100.0, 40.0, 40.0)]).len(), 2);
    }
}
