//! Windows OCR (Windows.Media.Ocr) on a dedicated COM/WinRT thread.
//!
//! MVP engine: built into Windows 10/11, no extra download besides the OCR
//! language components (Korean, Japanese, Chinese), returns per-word boxes.
//! When the neural OCR is chosen and downloaded (`crate::neural`, PaddleOCR),
//! `recognize` reads with it instead, behind the same `OcrLine` interface.

use anyhow::{anyhow, Context, Result};
use std::sync::mpsc;
use tokio::sync::oneshot;
use manga_core::image::{Frame, Rect};
use std::collections::HashMap;
use manga_core::lang::{self, Lang};
use manga_core::text;

use windows::core::HSTRING;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

#[derive(Debug, Clone)]
pub struct OcrLine {
    /// Normalized text (spaces between ideographs removed).
    pub text: String,
    /// Bounding box in the coordinates of the *original* (un-upscaled) frame.
    pub rect: Rect,
}

enum Job {
    Recognize {
        frame: Frame,
        /// The frame was upscaled by this factor before OCR; boxes are divided back.
        scale: f32,
        lang: Lang,
        reply: oneshot::Sender<Result<Vec<OcrLine>>>,
    },
    Check {
        lang: Lang,
        reply: oneshot::Sender<Result<()>>,
    },
}

#[derive(Clone)]
pub struct Ocr {
    tx: mpsc::Sender<Job>,
    pub max_dim: u32,
}

impl Ocr {
    /// Start the OCR thread. Engines are created per language on first use,
    /// so switching the game language needs no restart.
    pub fn spawn() -> Result<Ocr> {
        let (tx, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<u32>();
        std::thread::Builder::new().name("ocr".into()).spawn(move || {
            unsafe {
                let _ = RoInitialize(RO_INIT_MULTITHREADED);
            }
            let _ = ready_tx.send(OcrEngine::MaxImageDimension().unwrap_or(2600));
            let mut engines: HashMap<&'static str, OcrEngine> = HashMap::new();
            let mut engine_for = |lang: Lang| -> Result<OcrEngine> {
                if let Some(e) = engines.get(lang.code()) {
                    return Ok(e.clone());
                }
                let e = create_engine(lang)?;
                engines.insert(lang.code(), e.clone());
                Ok(e)
            };
            while let Ok(job) = rx.recv() {
                match job {
                    Job::Recognize { frame, scale, lang, reply } => {
                        let res = engine_for(lang).and_then(|e| recognize(&e, &frame, scale, lang));
                        let _ = reply.send(res);
                    }
                    Job::Check { lang, reply } => {
                        let _ = reply.send(engine_for(lang).map(|_| ()));
                    }
                }
            }
        })?;
        let max_dim = ready_rx.recv().map_err(|_| anyhow!("поток OCR завершился"))?;
        Ok(Ocr { tx, max_dim })
    }

    /// Is Windows OCR for this language installed?
    pub fn check_blocking(&self, lang: Lang) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(Job::Check { lang, reply }).map_err(|_| anyhow!("поток OCR недоступен"))?;
        rx.blocking_recv().map_err(|_| anyhow!("поток OCR не ответил"))?
    }

    pub async fn check(&self, lang: Lang) -> Result<()> {
        if crate::neural::active(lang) {
            return Ok(()); // no Windows language component needed
        }
        let (reply, rx) = oneshot::channel();
        self.tx.send(Job::Check { lang, reply }).map_err(|_| anyhow!("поток OCR недоступен"))?;
        rx.await.map_err(|_| anyhow!("поток OCR не ответил"))?
    }

    /// Recognize a frame. If it is larger than the engine limit it is split
    /// into overlapping tiles and the results are merged.
    pub async fn recognize(&self, frame: Frame, upscale: u32, lang: Lang) -> Result<Vec<OcrLine>> {
        // The neural OCR when it is chosen and downloaded: it finds the text
        // lines itself at any size, so no upscale and no tiles.
        let frame = if crate::neural::active(lang) {
            let (frame_back, read) = tokio::task::spawn_blocking(move || {
                let attempt = || crate::neural::engine(lang).ok_or_else(|| anyhow!("распознавание-нейросеть не загрузилось")).and_then(|e| crate::neural::read(&e, &frame, lang));
                let mut read = attempt();
                // The video card failed: once more on the processor.
                if read.is_err() && crate::neural::leave_gpu() {
                    read = attempt();
                }
                (frame, read)
            })
            .await
            .map_err(|e| anyhow!("распознавание-нейросеть: {e}"))?;
            match read {
                Ok(lines) => return Ok(lines),
                // Something wrong with the model files or the runtime: Windows
                // OCR from now on (the setting stays; a restart tries again).
                Err(e) => {
                    tracing::warn!("распознавание-нейросеть: {e:#}; дальше читаю распознаванием Windows");
                    crate::neural::set_on(false);
                }
            }
            frame_back
        } else {
            frame
        };
        let k = upscale.max(1);
        let max = self.max_dim.max(512);
        // Fast path: fits after upscaling.
        if frame.width * k <= max && frame.height * k <= max {
            let f = if k > 1 { frame.upscale(k) } else { frame };
            return self.run(f, k as f32, lang).await;
        }
        // Tile in original coordinates; each tile after upscale must fit `max`.
        let tile = (max / k).saturating_sub(16).max(256);
        let overlap = 64u32.min(tile / 4);
        let mut all: Vec<OcrLine> = Vec::new();
        let mut y = 0;
        while y < frame.height {
            let mut x = 0;
            while x < frame.width {
                let part = frame.crop(x, y, tile, tile);
                let f = if k > 1 { part.upscale(k) } else { part };
                for mut l in self.run(f, k as f32, lang).await? {
                    l.rect = l.rect.offset(x as f32, y as f32);
                    let dup = all.iter().any(|o| o.text == l.text && overlap_ratio(&o.rect, &l.rect) > 0.5);
                    if !dup {
                        all.push(l);
                    }
                }
                if x + tile >= frame.width {
                    break;
                }
                x += tile - overlap;
            }
            if y + tile >= frame.height {
                break;
            }
            y += tile - overlap;
        }
        Ok(all)
    }

    async fn run(&self, frame: Frame, scale: f32, lang: Lang) -> Result<Vec<OcrLine>> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(Job::Recognize { frame, scale, lang, reply }).map_err(|_| anyhow!("поток OCR недоступен"))?;
        rx.await.map_err(|_| anyhow!("поток OCR не ответил"))?
    }
}

fn overlap_ratio(a: &Rect, b: &Rect) -> f32 {
    let ix = ((a.x + a.w).min(b.x + b.w) - a.x.max(b.x)).max(0.0);
    let iy = ((a.y + a.h).min(b.y + b.h) - a.y.max(b.y)).max(0.0);
    let inter = ix * iy;
    let min_area = (a.w * a.h).min(b.w * b.h).max(1.0);
    inter / min_area
}

fn create_engine(lang: Lang) -> Result<OcrEngine> {
    // Windows reports languages under slightly different tags depending on build.
    let tags: &[&str] = match lang {
        Lang::Zh => &["zh-Hans-CN", "zh-CN", "zh-Hans"],
        Lang::En => &["en-US", "en-GB", "en"],
        Lang::Ko => &["ko-KR", "ko"],
        Lang::Ja => &["ja-JP", "ja"],
    };
    for t in tags {
        let l = Language::CreateLanguage(&HSTRING::from(*t)).context("неверный код языка OCR")?;
        if OcrEngine::IsLanguageSupported(&l).unwrap_or(false) {
            tracing::info!("OCR язык: {t}");
            return OcrEngine::TryCreateFromLanguage(&l).context("не удалось создать OcrEngine");
        }
    }
    let (what, add) = match lang {
        Lang::Zh => ("китайского", "«中文(中华人民共和国)»"),
        Lang::En => ("английского", "«English (United States)»"),
        Lang::Ko => ("корейского", "«한국어»"),
        Lang::Ja => ("японского", "«日本語»"),
    };
    Err(anyhow!(
        "Windows OCR для {what} не установлен. Переустановите Manga Local с галочкой «модуль распознавания» \
         или откройте Параметры → Время и язык → Язык и регион → Добавить язык → {add} \
         и включите компонент «Оптическое распознавание символов»."
    ))
}

fn recognize(engine: &OcrEngine, frame: &Frame, scale: f32, lang: Lang) -> Result<Vec<OcrLine>> {
    let writer = DataWriter::new()?;
    writer.WriteBytes(&frame.bgra)?;
    let buffer = writer.DetachBuffer()?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        frame.width as i32,
        frame.height as i32,
        BitmapAlphaMode::Premultiplied,
    )?;
    let result = engine.RecognizeAsync(&bitmap)?.get()?;
    let mut out = Vec::new();
    for line in result.Lines()? {
        let mut words: Vec<(String, Rect)> = Vec::new();
        for w in line.Words()? {
            let r = w.BoundingRect()?;
            words.push((w.Text()?.to_string_lossy(), Rect { x: r.X, y: r.Y, w: r.Width, h: r.Height }));
        }
        // Windows OCR joins words standing at one height right across the
        // page: captions of three panels side by side become one «line». A
        // gap wider than two letter heights splits it.
        for piece in split_at_gaps(&words) {
            let raw: Vec<&str> = piece.iter().map(|(t, _)| t.as_str()).collect();
            let norm = text::normalize_for(&raw.join(" "), lang);
            if norm.is_empty() {
                continue;
            }
            let rect = piece.iter().skip(1).fold(piece[0].1, |a, (_, r)| a.union(r));
            out.push(OcrLine { text: norm, rect: rect.scale(1.0 / scale) });
        }
    }
    Ok(out)
}

/// Split a line's words where the gap between two neighbours is wider than
/// two letter heights (the median word height).
fn split_at_gaps(words: &[(String, Rect)]) -> Vec<&[(String, Rect)]> {
    if words.len() < 2 {
        return if words.is_empty() { vec![] } else { vec![words] };
    }
    let mut hs: Vec<f32> = words.iter().map(|(_, r)| r.h).collect();
    hs.sort_by(|a, b| a.total_cmp(b));
    let h = hs[hs.len() / 2].max(1.0);
    let mut out = Vec::new();
    let mut start = 0;
    for i in 1..words.len() {
        let (a, b) = (&words[i - 1].1, &words[i].1);
        if b.x - (a.x + a.w) > 2.0 * h {
            out.push(&words[start..i]);
            start = i;
        }
    }
    out.push(&words[start..]);
    out
}

/// Group OCR lines of a subtitle region into one text (top-to-bottom).
/// For English, a short Title-Case first line is the speaker name shown above
/// the line; it is returned as "Name: text" so `split_speaker` picks it up.
pub fn subtitle_text(mut lines: Vec<OcrLine>, lang: Lang) -> String {
    lines.sort_by(|a, b| a.rect.y.partial_cmp(&b.rect.y).unwrap_or(std::cmp::Ordering::Equal));
    let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).filter(|t| lang::is_label(t, lang)).collect();
    if lang == Lang::En && texts.len() >= 2 && text::looks_like_speaker_en(texts[0]) && texts[0].split_whitespace().count() <= 3 {
        let body = text::join_lines_en(&texts[1..]);
        if !body.is_empty() && lang::sentence_like_en(&body) {
            return format!("{}: {body}", texts[0]);
        }
    }
    text::join_lines_for(&texts, lang)
}
