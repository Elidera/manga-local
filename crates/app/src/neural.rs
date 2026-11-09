//! Neural OCR (PaddleOCR models, `manga-neural-ocr`): finds every text line on
//! the page and reads it — titles with outlines or glow, a big bold first
//! line, slanted text — where Windows OCR misses them. Chosen in the settings
//! («Распознавание текста»); the model files (~30–45 MB per language) are
//! downloaded on demand into `%LOCALAPPDATA%\MangaLocal\models\paddle` from
//! this project's GitHub release, ModelScope second (RapidOCR's ONNX exports,
//! Apache-2.0).

use crate::state::Shared;
use manga_core::lang::Lang;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Copies of the same files (the release «ocr-models-v1» of this repository):
/// GitHub opens where ModelScope does not.
const MIRROR: &str = "https://github.com/Elidera/manga-local/releases/download/ocr-models-v1";
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
/// On the video card (setting «Распознавание на видеокарте»).
static GPU: AtomicBool = AtomicBool::new(true);

/// Follow the setting «Распознавание на видеокарте»: the models load again.
pub fn set_gpu(on: bool) {
    if GPU.swap(on, Ordering::Relaxed) != on {
        *CACHE.lock().unwrap() = None;
        CHOSEN.store(0, Ordering::Relaxed);
    }
}

/// The build can run the recognition on the video card.
pub fn gpu_available() -> bool {
    #[cfg(feature = "neural-ocr")]
    return neural_ocr::gpu_compiled();
    #[cfg(not(feature = "neural-ocr"))]
    false
}

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
    if !on {
        // Windows OCR chosen: the models leave memory (a page being read keeps
        // its copy until it is done).
        *CACHE.lock().unwrap() = None;
    }
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
        let resp = match client.get(format!("{MIRROR}/{file}")).send().await.and_then(|r| r.error_for_status()) {
            Ok(r) => r,
            Err(first) => {
                tracing::warn!("{file}: GitHub не ответил ({first}), пробую ModelScope");
                client.get(format!("{BASE}/{path}")).send().await?.error_for_status()?
            }
        };
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
    // One language at a time: the reader of another one leaves memory.
    map.clear();
    match load(dir, lang) {
        Ok(e) => {
            tracing::info!(
                "распознавание-нейросеть: загружено для языка «{}» ({})",
                lang.name_ru(),
                if GPU.load(Ordering::Relaxed) && gpu_available() && CHOSEN.load(Ordering::Relaxed) == 1 { "видеокарта" } else { "процессор" }
            );
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

/// Reading on the video card failed (a driver that breaks mid-way): the
/// processor from now on. False when it was not on the video card.
pub fn leave_gpu() -> bool {
    if CHOSEN.compare_exchange(1, 2, Ordering::Relaxed, Ordering::Relaxed).is_err() {
        return false;
    }
    *CACHE.lock().unwrap() = None;
    tracing::warn!("распознавание на видеокарте дало сбой — дальше на процессоре");
    true
}

/// What the speed test chose: 0 not yet, 1 video card, 2 processor.
static CHOSEN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

#[cfg(feature = "neural-ocr")]
fn load(dir: &Path, lang: Lang) -> anyhow::Result<Engine> {
    // Half the cores: the translation model and the browser need the rest.
    let threads = std::thread::available_parallelism().map(|n| n.get() / 2).unwrap_or(2).clamp(2, 8);
    let (det, rec) = (dir.join(DET.0), dir.join(rec_for(lang).0));
    // A real video card only: the stand-in Windows gives without one
    // (Microsoft Basic Render, virtual machines) is slower than the processor.
    let real_card = crate::win::gpu().is_some_and(|(name, mem)| mem > 0 && !name.contains("Microsoft") && !name.contains("Hyper-V") && !name.contains("VMware"));
    if !(GPU.load(Ordering::Relaxed) && gpu_available() && real_card) {
        return neural_ocr::Engine::load(&det, &rec, threads, false);
    }
    match CHOSEN.load(Ordering::Relaxed) {
        1 => return neural_ocr::Engine::load(&det, &rec, threads, true),
        2 => return neural_ocr::Engine::load(&det, &rec, threads, false),
        _ => {}
    }
    // The first time: both tried on the same picture, the faster one stays (a
    // weak or missing video card can be slower than the processor).
    let cpu = neural_ocr::Engine::load(&det, &rec, threads, false)?;
    let gpu = match neural_ocr::Engine::load(&det, &rec, threads, true) {
        Ok(g) => g,
        Err(e) => {
            tracing::warn!("распознавание на видеокарте не запустилось: {e:#}");
            CHOSEN.store(2, Ordering::Relaxed);
            return Ok(cpu);
        }
    };
    let (t_cpu, t_gpu) = (speed(&cpu), speed(&gpu));
    let use_gpu = t_gpu.is_some_and(|g| t_cpu.is_none_or(|c| g < c));
    tracing::info!(
        "распознавание-нейросеть, проверка скорости: процессор {} мс, видеокарта {} мс → {}",
        t_cpu.map_or("—".into(), |t| t.to_string()),
        t_gpu.map_or("—".into(), |t| t.to_string()),
        if use_gpu { "видеокарта" } else { "процессор" }
    );
    CHOSEN.store(if use_gpu { 1 } else { 2 }, Ordering::Relaxed);
    Ok(if use_gpu { gpu } else { cpu })
}

/// Milliseconds for a screen-sized picture with 8 lines (the second of two
/// runs: the first prepares the video card).
#[cfg(feature = "neural-ocr")]
fn speed(e: &Engine) -> Option<u128> {
    // A wide screen and a tall page: the video card prepares itself for every
    // new size, so two sizes show that cost too.
    Some(speed_at(e, 1600, 900)? + speed_at(e, 1000, 1400)?)
}

#[cfg(feature = "neural-ocr")]
fn speed_at(e: &Engine, w: u32, h: u32) -> Option<u128> {
    let mut px = vec![255u8; (w * h * 4) as usize];
    // Dark bars like lines of text, so the reader has work to do.
    for row in 0..8u32 {
        for y in 100 + row * 90..130 + row * 90 {
            for x in (200..w.min(1300)).filter(|x| x % 23 < 15) {
                let i = ((y * w + x) * 4) as usize;
                px[i..i + 3].copy_from_slice(&[20, 20, 20]);
            }
        }
    }
    let f = manga_core::image::Frame::new(w, h, px);
    let boxes: Vec<manga_core::image::Rect> = (0..8).map(|r| manga_core::image::Rect { x: 190.0, y: 95.0 + r as f32 * 90.0, w: (w.min(1300) - 180) as f32, h: 40.0 }).collect();
    let mut best = None;
    for _ in 0..2 {
        let t = std::time::Instant::now();
        e.det.find(&f).ok()?;
        e.rec.read(&f, &boxes).ok()?;
        best = Some(t.elapsed().as_millis());
    }
    best
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
