//! Translating chapter files (button «Перевести файлы главы»): a folder of
//! pictures or a ZIP / CBZ archive. Each page: Windows OCR (tall webtoon strips
//! in overlapping pieces), the same bubbles and translation as F8, the original
//! text cleaned away and the translation lettered into the picture. The result
//! is written next to the original («<глава> (перевод)» or «… (перевод).cbz»).

use crate::interface;
use crate::ocr::{Ocr, OcrLine};
use crate::state::Shared;
use crate::translate::Translator;
use manga_pages::chapter::{self, Chapter, Output};
use manga_pages::letter::Letterer;
use manga_pages::{slice, Bubble};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use manga_core::prompt::TextKind;

static RUNNING: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);

/// Ask a running chapter translation to stop after the current page.
pub fn stop() {
    STOP.store(true, Ordering::SeqCst);
}

/// The bundled font for lettering: the one chosen for the translation if it is
/// one of ours, otherwise Balsamiq Sans.
fn font_file(shared: &Shared) -> PathBuf {
    let dir = shared.paths.bundled_fonts_dir();
    let face = shared.cfg().dialogue.overlay_font;
    let name = if face.eq_ignore_ascii_case("Pangolin") { "Pangolin-Regular.ttf" } else { "BalsamiqSans-Regular.ttf" };
    dir.join(name)
}

pub async fn translate_chapter(shared: Arc<Shared>, tr: Arc<Translator>, ocr: Ocr, path: PathBuf) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        shared.ui(|u| u.notice("Глава уже переводится — дождитесь или остановите"));
        return;
    }
    STOP.store(false, Ordering::SeqCst);
    let r = run(&shared, &tr, &ocr, &path).await;
    RUNNING.store(false, Ordering::SeqCst);
    shared.ui(|u| {
        u.files_progress = None;
        match &r {
            Ok((out, n, stopped)) => {
                let what = manga_core::text::count_ru(*n, "страница", "страницы", "страниц");
                if *stopped {
                    u.notice(format!("Перевод главы остановлен: готово {what} — {}", out.display()));
                } else {
                    u.notice(format!("Глава переведена ({what}): {}", out.display()));
                }
            }
            Err(e) => u.notice(format!("Перевод файлов: {e}")),
        }
    });
    if let Ok((out, n, _)) = &r {
        tracing::info!("глава: {n} стр. → {}", out.display());
        if *n > 0 {
            shared.send(crate::state::Command::Open(out.parent().map(Path::to_path_buf).unwrap_or_else(|| out.clone())));
        }
    }
}

/// Returns (where the result is, pages done, stopped by the reader).
async fn run(shared: &Arc<Shared>, tr: &Arc<Translator>, ocr: &Ocr, path: &Path) -> Result<(PathBuf, usize, bool), String> {
    let ch = Chapter::open(path)?;
    if ch.pages.is_empty() {
        return Err(format!("в «{}» нет картинок (png, jpg, webp)", ch.name));
    }
    let cfg = shared.cfg();
    let lang = cfg.source_lang();
    ocr.check(lang).await.map_err(|e| e.to_string())?;
    tr.ensure_title(&cfg.general.profile, &cfg.general.game_title);
    let letterer = Letterer::new(std::fs::read(font_file(shared)).map_err(|e| format!("шрифт: {e}"))?)?;
    let out_path = ch.output_path();
    let mut out = Output::create(&out_path, ch.is_archive())?;
    let preset = cfg.active_model().clone();
    let game = cfg.general.game_title.clone();
    let total = ch.pages.len();
    tracing::info!("глава «{}»: {total} стр., язык {}", ch.name, lang.code());
    let mut done = 0;
    // The last bubbles of the previous page, for the model's context.
    let mut prev: Vec<(String, String)> = Vec::new();
    // Conveyor: while the model translates page i, page i + 1 is decoded and
    // read by OCR (the two use different hardware).
    let keep_sfx = cfg.interface.sfx == "keep";
    let start = |i: usize| -> Result<tokio::task::JoinHandle<Result<Prepared, String>>, String> {
        let bytes = ch.read(i)?;
        let (sh, tr, ocr) = (shared.clone(), tr.clone(), ocr.clone());
        Ok(tokio::spawn(async move { prepare(&sh, &tr, &ocr, lang, keep_sfx, bytes).await }))
    };
    progress_line(shared, &ch.name, 0, total, "распознаю");
    let mut next = Some(start(0)?);
    for (i, page) in ch.pages.iter().enumerate() {
        let Some(job) = next.take() else { break };
        let prepared = job.await.map_err(|e| format!("распознавание: {e}"))??;
        if i + 1 < total && !STOP.load(Ordering::SeqCst) {
            next = Some(start(i + 1)?);
        }
        if STOP.load(Ordering::SeqCst) {
            out.finish()?;
            return Ok((out_path, done, true));
        }
        let progress = |stage: &str| progress_line(shared, &ch.name, i, total, stage);
        let (out_name, jpeg) = chapter::output_name(page);
        let Prepared { bytes, frame, items, lines } = prepared;
        let Some(mut frame) = frame else {
            // Not a picture after all: copied as it is.
            out.add(&chapter::flat_name(page), &bytes)?;
            continue;
        };
        tracing::info!(target: "files", "{page}: {lines} строк → {} пузырей", items.len());
        if items.is_empty() {
            // No text: the page is kept unchanged (no re-encoding).
            out.add(&chapter::flat_name(page), &bytes)?;
            done += 1;
            continue;
        }
        progress(&format!("перевожу {}", manga_core::text::count_ru(items.len(), "пузырь", "пузыря", "пузырей")));
        let mut result: HashMap<String, String> = HashMap::new();
        let (talk, sfx) = interface::split_unique(&items, tr, &preset.id, &mut result);
        let context = if cfg.interface.page_context { prev.clone() } else { Vec::new() };
        let alive = || !STOP.load(Ordering::SeqCst);
        interface::translate_texts(shared, tr, &preset, &game, talk, TextKind::Dialogue, &context, alive, |src, dst| {
            result.insert(src.to_string(), dst);
        })
        .await;
        interface::translate_texts(shared, tr, &preset, &game, sfx, TextKind::Interface, &[], alive, |src, dst| {
            result.insert(src.to_string(), dst);
        })
        .await;
        for (src, dst) in &result {
            let names = tr.observe_names(lang, src, dst);
            if !names.is_empty() {
                shared.ui(|u| u.offer_names(names));
            }
        }
        let pairs: Vec<(String, String)> = items.iter().filter(|it| !it.sfx).filter_map(|it| result.get(&it.src).filter(|d| **d != it.src).map(|d| (it.src.clone(), d.clone()))).collect();
        prev = pairs.into_iter().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect();
        progress("вписываю перевод");
        let bubbles: Vec<Bubble> = items
            .iter()
            .map(|it| Bubble {
                rect: it.rect,
                line_h: it.line_h,
                around: it.bg,
                busy: it.busy,
                translation: result.get(&it.src).cloned().unwrap_or_default(),
            })
            // No translation (or the original back after a failure): the bubble is left as drawn.
            .filter(|b| !b.translation.is_empty())
            .collect::<Vec<_>>()
            .into_iter()
            .zip(items.iter())
            .filter(|(b, it)| b.translation != it.src)
            .map(|(b, _)| b)
            .collect();
        manga_pages::render(&mut frame, &bubbles, &letterer, cfg.interface.font_scale);
        let encoded = chapter::encode(&frame, jpeg)?;
        out.add(&out_name, &encoded)?;
        done += 1;
    }
    out.finish()?;
    Ok((out_path, done, false))
}

fn progress_line(shared: &Shared, name: &str, i: usize, total: usize, stage: &str) {
    let text = format!("«{name}»: страница {} из {total} — {stage}", i + 1);
    shared.ui(|u| u.files_progress = Some(text));
}

/// A page read and recognised, ready for translation.
struct Prepared {
    bytes: Vec<u8>,
    /// None: not a picture (copied as it is).
    frame: Option<manga_core::image::Frame>,
    items: Vec<crate::state::InterfaceItem>,
    lines: usize,
}

async fn prepare(shared: &Arc<Shared>, tr: &Arc<Translator>, ocr: &Ocr, lang: manga_core::lang::Lang, keep_sfx: bool, bytes: Vec<u8>) -> Result<Prepared, String> {
    let decoded = {
        let b = bytes.clone();
        tokio::task::spawn_blocking(move || chapter::decode(&b)).await.map_err(|e| e.to_string())?
    };
    let frame = match decoded {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("{e}");
            return Ok(Prepared { bytes, frame: None, items: vec![], lines: 0 });
        }
    };
    let lines = recognize(ocr, &frame, lang).await?;
    let ja = crate::jaocr::engine(shared).await;
    let mut items = interface::page_items(&lines, lang, tr, &frame, ja).await;
    if keep_sfx {
        items.retain(|it| !it.sfx);
    }
    Ok(Prepared { bytes, frame: Some(frame), items, lines: lines.len() })
}

/// OCR of a whole page; tall webtoon strips in overlapping pieces of normal
/// page height (each line is taken from the piece where it is not cut).
async fn recognize(ocr: &Ocr, frame: &manga_core::image::Frame, lang: manga_core::lang::Lang) -> Result<Vec<OcrLine>, String> {
    // Small text of narrow manhwa pages (720–1000 px wide) reads better at 2×.
    let up = if frame.width < 1300 { 2 } else { 1 };
    let pieces = slice::pieces(frame.height, 2400, 400);
    let mut all = Vec::new();
    for (i, &(y0, y1)) in pieces.iter().enumerate() {
        let part = if pieces.len() == 1 { frame.clone() } else { frame.crop(0, y0, frame.width, y1 - y0) };
        let lines = ocr.recognize(part, up, lang).await.map_err(|e| format!("OCR: {e}"))?;
        for mut l in lines {
            l.rect = l.rect.offset(0.0, y0 as f32);
            if slice::owns(&pieces, i, l.rect.y + l.rect.h / 2.0) {
                all.push(l);
            }
        }
    }
    Ok(all)
}
