//! Vertical and handwritten Japanese. Windows OCR reads only horizontal lines;
//! here the bubbles are found by the picture itself (`wuxia_core::bubbles`)
//! and their text is read by manga-ocr (kha-white/manga-ocr-base exported to
//! ONNX, ~441 MB, Apache-2.0), which reads columns and handwriting alike.
//!
//! The model is downloaded on demand (setting «Вертикальный японский») into
//! `%LOCALAPPDATA%\MangaLocal\models\manga-ocr`. The engine itself is compiled
//! in only with the `vertical-ja` feature (the release build has it).

use crate::state::Shared;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use wuxia_core::image::{Frame, Rect};

const FILES: &[(&str, u64)] = &[("vocab.txt", 20_000), ("decoder_model.onnx", 100_000_000), ("encoder_model.onnx", 300_000_000)];
const BASE: &str = "https://huggingface.co/mayocream/manga-ocr-onnx/resolve/main";

pub fn model_dir(shared: &Shared) -> PathBuf {
    shared.paths.data.join("models").join("manga-ocr")
}

/// Compiled with the manga-ocr engine?
pub fn available() -> bool {
    cfg!(feature = "vertical-ja")
}

/// All model files are on disk (and not cut short).
pub fn models_ready(dir: &Path) -> bool {
    FILES.iter().all(|(f, min)| std::fs::metadata(dir.join(f)).is_ok_and(|m| m.len() >= *min))
}

/// Download the model files (resumes nothing: a cut file is downloaded again).
pub async fn download(shared: Arc<Shared>) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let dir = model_dir(&shared);
    tokio::fs::create_dir_all(&dir).await?;
    let client = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(20)).build()?;
    for (i, (file, min)) in FILES.iter().enumerate() {
        let dest = dir.join(file);
        if std::fs::metadata(&dest).is_ok_and(|m| m.len() >= *min) {
            continue;
        }
        let resp = client.get(format!("{BASE}/{file}")).send().await?.error_for_status()?;
        let total = resp.content_length().unwrap_or(0);
        let part = dir.join(format!("{file}.part"));
        let mut out = tokio::fs::File::create(&part).await?;
        let mut got: u64 = 0;
        let mut last_shown = 0u64;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            out.write_all(&chunk).await?;
            got += chunk.len() as u64;
            if got - last_shown > 4_000_000 || got == total {
                last_shown = got;
                let text = if total > 0 {
                    format!("Скачиваю распознавание японского: файл {} из {} — {} из {} МБ", i + 1, FILES.len(), got / 1_000_000, total / 1_000_000)
                } else {
                    format!("Скачиваю распознавание японского: файл {} из {} — {} МБ", i + 1, FILES.len(), got / 1_000_000)
                };
                shared.ui(|u| u.ja_ocr_status = Some(text));
            }
        }
        out.flush().await?;
        drop(out);
        tokio::fs::rename(&part, &dest).await?;
    }
    Ok(())
}

/// Read the text of one block (frame coordinates): `None` when the model is
/// not sure or the answer is not Japanese.
pub struct JaOcr {
    #[cfg(feature = "vertical-ja")]
    inner: manga_ocr_rs::MangaOcr,
}

impl JaOcr {
    #[cfg(feature = "vertical-ja")]
    fn load(dir: &Path) -> anyhow::Result<JaOcr> {
        Ok(JaOcr { inner: manga_ocr_rs::MangaOcr::new(dir)? })
    }

    #[cfg(not(feature = "vertical-ja"))]
    fn load(_dir: &Path) -> anyhow::Result<JaOcr> {
        anyhow::bail!("эта сборка без распознавания вертикального японского")
    }

    pub fn read(&self, frame: &Frame, r: &Rect) -> Option<String> {
        let pad = 6.0;
        let x = (r.x - pad).max(0.0) as u32;
        let y = (r.y - pad).max(0.0) as u32;
        let w = ((r.w + 2.0 * pad) as u32).min(frame.width.saturating_sub(x));
        let h = ((r.h + 2.0 * pad) as u32).min(frame.height.saturating_sub(y));
        if w < 8 || h < 8 {
            return None;
        }
        let crop = frame.crop(x, y, w, h);
        self.read_crop(&crop)
    }

    #[cfg(feature = "vertical-ja")]
    fn read_crop(&self, crop: &Frame) -> Option<String> {
        let mut rgba = crop.bgra.clone();
        for p in rgba.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(crop.width, crop.height, rgba)?);
        let r = match self.inner.recognize_with_score(&img) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("manga-ocr: {e}");
                return None;
            }
        };
        let text = r.text.trim().to_string();
        let japanese = text.chars().filter(|&c| wuxia_core::text::is_kana(c) || wuxia_core::text::is_han(c)).count();
        tracing::debug!(target: "jaocr", "{text} ({:.2})", r.confidence);
        (r.confidence >= 0.45 && japanese >= 1 && japanese * 2 >= text.chars().count()).then_some(text)
    }

    #[cfg(not(feature = "vertical-ja"))]
    fn read_crop(&self, _crop: &Frame) -> Option<String> {
        None
    }
}

static ENGINE: Mutex<Option<Arc<JaOcr>>> = Mutex::new(None);

/// The engine, loaded on first use (a few seconds). None when it is off,
/// not compiled in, or the model is not downloaded.
pub async fn engine(shared: &Shared) -> Option<Arc<JaOcr>> {
    if !available() || !shared.cfg().interface.vertical_ja {
        return None;
    }
    if let Some(e) = ENGINE.lock().unwrap().clone() {
        return Some(e);
    }
    let dir = model_dir(shared);
    if !models_ready(&dir) {
        return None;
    }
    let loaded = tokio::task::spawn_blocking(move || JaOcr::load(&dir)).await.ok()?;
    match loaded {
        Ok(e) => {
            let e = Arc::new(e);
            *ENGINE.lock().unwrap() = Some(e.clone());
            tracing::info!("manga-ocr: модель загружена");
            Some(e)
        }
        Err(e) => {
            tracing::warn!("manga-ocr: {e:#}");
            shared.ui(|u| u.ja_ocr_status = Some(format!("Не загрузилось распознавание японского: {e}")));
            None
        }
    }
}

/// Load for `--selftest` (no settings): from `dir` if the files are there.
pub fn engine_from(dir: &Path) -> Option<Arc<JaOcr>> {
    if !available() || !models_ready(dir) {
        return None;
    }
    JaOcr::load(dir).map(Arc::new).map_err(|e| tracing::warn!("manga-ocr: {e:#}")).ok()
}
