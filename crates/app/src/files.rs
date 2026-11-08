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
use wuxia_core::prompt::TextKind;

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
                let what = wuxia_core::text::count_ru(*n, "страница", "страницы", "страниц");
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
    for (i, page) in ch.pages.iter().enumerate() {
        if STOP.load(Ordering::SeqCst) {
            out.finish()?;
            return Ok((out_path, done, true));
        }
        let progress = |stage: &str| {
            let text = format!("«{}»: страница {} из {total} — {stage}", ch.name, i + 1);
            shared.ui(|u| u.files_progress = Some(text));
        };
        progress("распознаю");
        let bytes = ch.read(i)?;
        let (out_name, jpeg) = chapter::output_name(page);
        let mut frame = match chapter::decode(&bytes) {
            Ok(f) => f,
            Err(e) => {
                // Not a picture after all: copied as it is.
                tracing::warn!("{page}: {e}");
                out.add(page.rsplit(['/', '\\']).next().unwrap_or(page), &bytes)?;
                continue;
            }
        };
        let lines = recognize(ocr, &frame, lang).await?;
        let ja = crate::jaocr::engine(shared).await;
        let items = interface::page_items(&lines, lang, tr, &frame, ja).await;
        tracing::info!(target: "files", "{page}: {} строк → {} пузырей", lines.len(), items.len());
        if items.is_empty() {
            // No text: the page is kept unchanged (no re-encoding).
            out.add(page.rsplit(['/', '\\']).next().unwrap_or(page), &bytes)?;
            done += 1;
            continue;
        }
        progress(&format!("перевожу {}", wuxia_core::text::count_ru(items.len(), "пузырь", "пузыря", "пузырей")));
        let mut result: HashMap<String, String> = HashMap::new();
        let mut unique: Vec<String> = Vec::new();
        for it in &items {
            if result.contains_key(&it.src) || unique.contains(&it.src) {
                continue;
            }
            match tr.lookup(&preset.id, &it.src, TextKind::Dialogue) {
                Some(t) => {
                    result.insert(it.src.clone(), t);
                }
                None => unique.push(it.src.clone()),
            }
        }
        let context = if cfg.interface.page_context { prev.clone() } else { Vec::new() };
        interface::translate_texts(shared, tr, &preset, &game, unique, &context, || !STOP.load(Ordering::SeqCst), |src, dst| {
            result.insert(src.to_string(), dst);
        })
        .await;
        let pairs: Vec<(String, String)> = items.iter().filter_map(|it| result.get(&it.src).filter(|d| **d != it.src).map(|d| (it.src.clone(), d.clone()))).collect();
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
            .filter(|b| !b.translation.is_empty())
            .collect();
        manga_pages::render(&mut frame, &bubbles, &letterer, cfg.interface.font_scale);
        let encoded = chapter::encode(&frame, jpeg)?;
        out.add(&out_name, &encoded)?;
        done += 1;
    }
    out.finish()?;
    Ok((out_path, done, false))
}

/// OCR of a whole page; tall webtoon strips in overlapping pieces of normal
/// page height (each line is taken from the piece where it is not cut).
async fn recognize(ocr: &Ocr, frame: &wuxia_core::image::Frame, lang: wuxia_core::lang::Lang) -> Result<Vec<OcrLine>, String> {
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
