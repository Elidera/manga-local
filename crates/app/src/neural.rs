//! Neural OCR (PaddleOCR models, `manga-neural-ocr`): finds every text line on
//! the page and reads it — titles with outlines or glow, a big bold first
//! line, slanted text — where Windows OCR misses them. Chosen in the settings
//! («Распознавание текста»); the model files (~30–45 MB per language) are
//! downloaded on demand into `%LOCALAPPDATA%\MangaLocal\models\paddle` from
//! ModelScope (RapidOCR's ONNX exports, Apache-2.0).

use crate::state::Shared;
use manga_core::lang::Lang;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const BASE: &str = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx";

/// (file, path under BASE, exact size in bytes).
type Model = (&'static str, &'static str, u64);
const DET: Model = ("PP-OCRv6_det_small.onnx", "PP-OCRv6/det/PP-OCRv6_det_small.onnx", 9_929_594);
/// Chinese, Japanese and English in one.
const REC_V6: Model = ("PP-OCRv6_rec_small.onnx", "PP-OCRv6/rec/PP-OCRv6_rec_small.onnx", 21_234_383);
const REC_KO: Model = ("korean_PP-OCRv5_rec_mobile.onnx", "PP-OCRv5/rec/korean_PP-OCRv5_rec_mobile.onnx", 13_488_748);
const REC_EN: Model = ("en_PP-OCRv5_rec_mobile.onnx", "PP-OCRv5/rec/en_PP-OCRv5_rec_mobile.onnx", 7_872_351);

fn rec_for(lang: Lang) -> Model {
    match lang {
        Lang::Ko => REC_KO,
        Lang::En => REC_EN,
        Lang::Zh | Lang::Ja => REC_V6,
    }
}

/// Files a language needs.
fn files(lang: Lang) -> [Model; 2] {
    [DET, rec_for(lang)]
}

/// Compiled with the neural OCR?
pub fn available() -> bool {
    cfg!(feature = "neural-ocr")
}

static DIR: OnceLock<PathBuf> = OnceLock::new();
static ON: AtomicBool = AtomicBool::new(false);

pub fn model_dir(shared: &Shared) -> PathBuf {
    shared.paths.data.join("models").join("paddle")
}

/// Where the models are (set once at start, or by `--selftest --neural <dir>`).
pub fn init(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// Follow the setting «Распознавание текста».
pub fn set_on(on: bool) {
    ON.store(on && available(), Ordering::Relaxed);
}

pub fn wanted() -> bool {
    ON.load(Ordering::Relaxed)
}

/// All files of the language are on disk, whole.
pub fn ready(dir: &Path, lang: Lang) -> bool {
    files(lang).iter().all(|(f, _, size)| std::fs::metadata(dir.join(f)).is_ok_and(|m| m.len() == *size))
}

/// Neural OCR is on and can read `lang` right now.
pub fn active(lang: Lang) -> bool {
    wanted() && DIR.get().is_some_and(|d| ready(d, lang))
}

/// Download what `lang` needs (shows progress in the panel).
pub async fn download(shared: Arc<Shared>, lang: Lang) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let dir = model_dir(&shared);
    tokio::fs::create_dir_all(&dir).await?;
    let client = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(20)).build()?;
    let todo: Vec<Model> = files(lang).into_iter().filter(|(f, _, size)| !std::fs::metadata(dir.join(f)).is_ok_and(|m| m.len() == *size)).collect();
    for (i, (file, path, size)) in todo.iter().enumerate() {
        let resp = client.get(format!("{BASE}/{path}")).send().await?.error_for_status()?;
        let part = dir.join(format!("{file}.part"));
        let mut out = tokio::fs::File::create(&part).await?;
        let (mut got, mut shown) = (0u64, 0u64);
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            out.write_all(&chunk).await?;
            got += chunk.len() as u64;
            if got - shown > 2_000_000 || got == *size {
                shown = got;
                let text = format!("Скачиваю распознавание: файл {} из {} — {} из {} МБ", i + 1, todo.len(), got / 1_000_000, size / 1_000_000);
                shared.ui(|u| u.neural_status = Some(text));
            }
        }
        out.flush().await?;
        drop(out);
        if got != *size {
            let _ = tokio::fs::remove_file(&part).await;
            anyhow::bail!("{file}: скачано {got} байт вместо {size}");
        }
        tokio::fs::rename(&part, dir.join(file)).await?;
    }
    Ok(())
}

static CACHE: Mutex<Option<HashMap<&'static str, Arc<Engine>>>> = Mutex::new(None);

#[cfg(feature = "neural-ocr")]
pub type Engine = neural_ocr::Engine;
#[cfg(not(feature = "neural-ocr"))]
pub struct Engine;

/// The engine for `lang`, loaded on first use (blocking, ~0.3 s). None when
/// off, not compiled in or not downloaded.
pub fn engine(lang: Lang) -> Option<Arc<Engine>> {
    if !active(lang) {
        return None;
    }
    let dir = DIR.get()?;
    let key = rec_for(lang).0;
    let mut cache = CACHE.lock().unwrap();
    let map = cache.get_or_insert_with(HashMap::new);
    if let Some(e) = map.get(key) {
        return Some(e.clone());
    }
    match load(dir, lang) {
        Ok(e) => {
            tracing::info!("распознавание-нейросеть: загружено для языка «{}»", lang.name_ru());
            let e = Arc::new(e);
            map.insert(key, e.clone());
            Some(e)
        }
        Err(e) => {
            tracing::warn!("распознавание-нейросеть: {e:#}");
            None
        }
    }
}

#[cfg(feature = "neural-ocr")]
fn load(dir: &Path, lang: Lang) -> anyhow::Result<Engine> {
    // Half the cores: the translation model and the browser need the rest.
    let threads = std::thread::available_parallelism().map(|n| n.get() / 2).unwrap_or(2).clamp(2, 8);
    neural_ocr::Engine::load(&dir.join(DET.0), &dir.join(rec_for(lang).0), threads)
}

#[cfg(not(feature = "neural-ocr"))]
fn load(_dir: &Path, _lang: Lang) -> anyhow::Result<Engine> {
    anyhow::bail!("эта сборка без распознавания-нейросети")
}

/// Read a picture: lines in frame coordinates, texts normalised for `lang`.
#[cfg(feature = "neural-ocr")]
pub fn read(engine: &Engine, frame: &manga_core::image::Frame, lang: Lang) -> anyhow::Result<Vec<crate::ocr::OcrLine>> {
    Ok(engine
        .read(frame, 0.5)?
        .into_iter()
        .map(|l| crate::ocr::OcrLine { text: manga_core::text::normalize_for(&l.text, lang), rect: l.rect })
        // A lone Korean letter («ㅇ») is a stroke of the drawing; «ㅋㅋ», «ㅠㅠ» stay.
        .filter(|l| !l.text.is_empty() && !(l.text.chars().count() == 1 && l.text.chars().all(|c| ('\u{3131}'..='\u{318E}').contains(&c))))
        .collect())
}

#[cfg(not(feature = "neural-ocr"))]
pub fn read(_engine: &Engine, _frame: &manga_core::image::Frame, _lang: Lang) -> anyhow::Result<Vec<crate::ocr::OcrLine>> {
    Ok(vec![])
}

/// Megabytes still to download for `lang` (for the settings hint).
pub fn missing_mb(dir: &Path, lang: Lang) -> u64 {
    files(lang).iter().filter(|(f, _, size)| !std::fs::metadata(dir.join(f)).is_ok_and(|m| m.len() == *size)).map(|(_, _, s)| s / 1_000_000).sum()
}
