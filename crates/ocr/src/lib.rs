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

fn load(path: &Path, threads: usize) -> Result<Session> {
    let b = Session::builder().map_err(|e| anyhow!("{e}"))?;
    let mut b = b.with_intra_threads(threads.max(1)).map_err(|e| anyhow!("{e}"))?;
    b.commit_from_file(path).map_err(|e| anyhow!("{}: {e}", path.display()))
}

/// Finds text lines.
pub struct Detector {
    session: Mutex<Session>,
}

impl Detector {
    pub fn load(path: &Path, threads: usize) -> Result<Self> {
        Ok(Self { session: Mutex::new(load(path, threads)?) })
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
}

const REC_H: usize = 48;

impl Recognizer {
    pub fn load(path: &Path, threads: usize) -> Result<Self> {
        let session = load(path, threads)?;
        let embedded = session
            .metadata()
            .map_err(|e| anyhow!("{e}"))?
            .custom("character")
            .with_context(|| format!("{}: в модели нет алфавита", path.display()))?;
        Ok(Self { session: Mutex::new(session), chars: ctc::alphabet(&embedded) })
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
            let width = (REC_H as f32 * max_ratio).ceil() as usize;
            let per = 3 * REC_H * width;
            let mut data = vec![0f32; chunk.len() * per];
            for (k, &i) in chunk.iter().enumerate() {
                let r = &boxes[i];
                let (rot, q) = ratio(r);
                let rw = ((REC_H as f32 * q).ceil() as usize).clamp(1, width);
                prep::write_region(f, r.x, r.y, r.w, r.h, rw, REC_H, width, rot, &mut data[k * per..(k + 1) * per]);
            }
            let input = Tensor::from_array(([chunk.len(), 3, REC_H, width], data)).map_err(|e| anyhow!("{e}"))?;
            let mut s = self.session.lock().unwrap();
            let res = s.run(ort::inputs![input]).map_err(|e| anyhow!("чтение текста: {e}"))?;
            let (shape, logits) = res[0].try_extract_tensor::<f32>().map_err(|e| anyhow!("{e}"))?;
            let (t, c) = (shape[1] as usize, shape[2] as usize);
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

impl Engine {
    pub fn load(det: &Path, rec: &Path, threads: usize) -> Result<Self> {
        Ok(Self { det: Detector::load(det, threads)?, rec: Recognizer::load(rec, threads)? })
    }

    /// All lines of the page read with at least `min_score`.
    pub fn read(&self, f: &Frame, min_score: f32) -> Result<Vec<Line>> {
        let found = self.det.find(f)?;
        let rects: Vec<Rect> = found.iter().map(|(r, _)| *r).collect();
        let texts = self.rec.read(f, &rects)?;
        Ok(rects
            .into_iter()
            .zip(texts)
            // One or two symbols need more certainty: drawn circles and
            // strokes are read as «O», «1», «一».
            .filter(|(_, (t, s))| {
                let n = t.trim().chars().count();
                n > 0 && *s >= if n <= 2 { min_score.max(0.9) } else { min_score }
            })
            .map(|(rect, (text, score))| Line { text: text.trim().to_string(), rect, score })
            .collect())
    }
}
