//! Page translation (F8): OCR the reader window, group the lines into speech
//! bubbles, translate all bubbles of the page in one request and paint the
//! Russian text inside each bubble over the original (fill + wrapped text).
//!
//! With «Следить за прокруткой» on, F8 starts a reading mode: the translation
//! hides the moment the page starts moving (mouse wheel, scrolling keys, or the
//! picture changing) and the newly visible bubbles are translated by themselves
//! once the page stands still. F8 again turns it off.

use crate::capture::{self, PxRect};
use crate::ocr::Ocr;
use crate::state::{InterfaceItem, InterfaceView, Shared};
use crate::translate::{Opts, Translator};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use crate::ocr::OcrLine;
use wuxia_core::image::{contrast_text, luma, Frame, Rect};
use wuxia_core::lang::Lang;
use wuxia_core::layout;
use wuxia_core::prompt::TextKind;

/// Coarse 32×18 luminance grid of the whole screen, for auto-hide.
fn coarse(frame: &Frame) -> Vec<f32> {
    const GW: u32 = 32;
    const GH: u32 = 18;
    let mut out = vec![0f32; (GW * GH) as usize];
    for gy in 0..GH {
        for gx in 0..GW {
            let x = gx * frame.width / GW + frame.width / GW / 2;
            let y = gy * frame.height / GH + frame.height / GH / 2;
            let mut s = 0f32;
            for (dx, dy) in [(0, 0), (3, 0), (0, 3), (3, 3)] {
                let p = frame.px((x + dx).min(frame.width - 1), (y + dy).min(frame.height - 1));
                s += luma(p[0], p[1], p[2]) as f32;
            }
            out[(gy * GW + gx) as usize] = s / 4.0 / 255.0;
        }
    }
    out
}

fn coarse_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32
}

/// OCR upscale for F8: small fonts below 1440p read much better at 2×.
pub fn upscale_for(configured: u32, screen_h: i32) -> u32 {
    match configured {
        0 if screen_h < 1440 => 2,
        0 => 1,
        k => k,
    }
}

/// Is this OCR line a label F8 translates? (Shared with `--selftest`.)
pub fn is_label_line(text: &str, lang: wuxia_core::lang::Lang, tr: &Translator) -> bool {
    wuxia_core::lang::is_label(text, lang) && !tr.should_skip(text)
}

/// Group the OCR lines of a page into bubbles, in reading order. Each bubble
/// keeps its text joined the way the language is written, its box (`frame`
/// coordinates) and the colour around it. Shared with `--selftest`.
pub fn bubbles(lines: &[OcrLine], lang: Lang, tr: &Translator, frame: &Frame) -> Vec<InterfaceItem> {
    let kept: Vec<&OcrLine> = lines.iter().filter(|l| is_label_line(&l.text, lang, tr)).collect();
    let rects: Vec<Rect> = kept.iter().map(|l| l.rect).collect();
    let groups = layout::group_lines(&rects);
    let items: Vec<InterfaceItem> = groups
        .iter()
        .map(|g| {
            let rect = layout::block_rect(&rects, g);
            let texts: Vec<&str> = g.iter().map(|&i| kept[i].text.as_str()).collect();
            let mut src = tr.fix_ocr(&wuxia_core::text::join_lines_for(&texts, lang));
            if lang == Lang::En {
                src = wuxia_core::text::sentence_case_en(&src);
            }
            let line_h = g.iter().map(|&i| rects[i].h).sum::<f32>() / g.len() as f32;
            // Look at the paper a little outside the text: flat = a bubble (fill
            // with its colour), varied = text over artwork (dark plate).
            let (bg, spread) = frame.ring_stats(&rect, (line_h * 0.35).clamp(3.0, 14.0));
            // More than a quarter of the ring is not the paper colour: artwork.
            let busy = spread > 25.0;
            // Clean the original: the letters filled from their neighbours.
            // A speech bubble: flat and light paper (a flat grey or dark background is
            // the artwork, where letters may touch lines of the picture).
            let paper = !busy && wuxia_core::image::luma(bg[2], bg[1], bg[0]) > 200;
            let patch = wuxia_core::clean::clean_text(frame, &rect, line_h, bg, paper);
            let fg = match &patch {
                Some(p) if busy => contrast_text(p.mean_rgb()),
                _ => contrast_text(bg),
            };
            InterfaceItem { fg, busy, bg, rect, src, dst: None, line_h, lines: g.len(), patch: patch.map(Arc::new) }
        })
        .filter(|it| !it.src.is_empty())
        .collect();
    let order = layout::reading_order(&items.iter().map(|it| it.rect).collect::<Vec<_>>(), lang == Lang::Ja);
    order.into_iter().map(|i| items[i].clone()).collect()
}

/// Reading-mode session: 0 = off. A new F8 session ends the previous one.
static READING: AtomicU64 = AtomicU64::new(0);
static SESSIONS: AtomicU64 = AtomicU64::new(0);

fn reading(session: u64) -> bool {
    READING.load(Ordering::SeqCst) == session
}

pub async fn toggle(shared: Arc<Shared>, tr: Arc<Translator>, ocr: Ocr) {
    // Second press hides the translation and ends the reading mode.
    let showing = shared.ui.lock().unwrap().interface.is_some();
    if showing {
        READING.store(0, Ordering::SeqCst);
        shared.interface_gen.fetch_add(1, Ordering::SeqCst);
        shared.ui(|u| u.interface = None);
        return;
    }
    let session = SESSIONS.fetch_add(1, Ordering::SeqCst) + 1;
    READING.store(session, Ordering::SeqCst);
    PAGE_CONTEXT.lock().unwrap().clear();
    let follow = shared.cfg().interface.auto_hide;
    let mut shown_at = crate::hotkeys::now_ms();
    // The first page now, while the panel is hidden for the picture.
    let Some(mut page) = translate_page(&shared, &tr, &ocr, session, follow, true).await else {
        if reading(session) {
            READING.store(0, Ordering::SeqCst);
        }
        return;
    };
    if !follow {
        return; // stays until F8
    }
    tokio::spawn(async move {
        let mut masks: Vec<[i32; 4]> = capture::own_rects(&shared);
        let mut tick = 0u32;
        let mut changed = 0;
        loop {
            tokio::time::sleep(Duration::from_millis(60)).await;
            if !reading(session) {
                return;
            }
            // Someone else cleared the translation (region pick, F8 in the panel…).
            if shared.ui.lock().unwrap().interface.is_none() {
                READING.store(0, Ordering::SeqCst);
                return;
            }
            tick += 1;
            let mut moved = crate::hotkeys::last_scroll_ms() > shown_at;
            // Scrollbar dragging, touchpads, page loads: the picture itself changed.
            // Our own drawings (and the panel) are blanked in both pictures.
            if !moved && tick.is_multiple_of(3) {
                if let Ok(mut f) = capture::capture(page.area) {
                    for m in capture::own_rects(&shared) {
                        if !masks.contains(&m) {
                            masks.push(m);
                        }
                    }
                    capture::apply_masks(&mut f, page.area, &masks);
                    capture::apply_masks(&mut page.frame, page.area, &masks);
                    let d = coarse_diff(&coarse(&page.frame), &coarse(&f));
                    changed = if d > 0.04 { changed + 1 } else { 0 };
                    moved = changed >= 2;
                }
            }
            if !moved {
                continue;
            }
            // Hide at once, keep the mode on.
            let gen = shared.interface_gen.fetch_add(1, Ordering::SeqCst) + 1;
            shared.ui(|u| {
                if u.interface.is_some() {
                    u.interface = Some(InterfaceView { items: vec![], pending: 0, generation: gen });
                }
            });
            // Wait until the page stands still: no wheel for 0.35 s and two
            // pictures 150 ms apart match.
            let mut prev: Option<Vec<f32>> = None;
            loop {
                tokio::time::sleep(Duration::from_millis(150)).await;
                if !reading(session) {
                    return;
                }
                if crate::hotkeys::now_ms().saturating_sub(crate::hotkeys::last_scroll_ms()) < 350 {
                    prev = None;
                    continue;
                }
                let area = capture::game_area(&shared.cfg());
                let Ok(f) = capture::capture_clean(&shared, area) else { continue };
                let c = coarse(&f);
                if prev.as_ref().is_some_and(|p| p.len() == c.len() && coarse_diff(p, &c) < 0.01) {
                    break;
                }
                prev = Some(c);
            }
            if shared.ui.lock().unwrap().interface.is_none() {
                READING.store(0, Ordering::SeqCst);
                return;
            }
            shown_at = crate::hotkeys::now_ms();
            match translate_page(&shared, &tr, &ocr, session, true, false).await {
                Some(p) => page = p,
                None => {
                    if reading(session) {
                        READING.store(0, Ordering::SeqCst);
                    }
                    return;
                }
            }
            masks = capture::own_rects(&shared);
            changed = 0;
        }
    });
}

/// The picture a page was read from (for noticing that it moved).
struct Page {
    frame: Frame,
    area: PxRect,
}

/// OCR the reader, show the bubbles (cached ones at once) and translate the
/// rest in the background. `follow`: an empty page keeps the reading mode on.
/// `first`: the F8 press itself (wait for the page to settle, report an empty
/// page); later pages come after the reading mode saw the page stand still.
/// None = the mode ended (F8 pressed meanwhile, or a capture/OCR error).
async fn translate_page(shared: &Arc<Shared>, tr: &Arc<Translator>, ocr: &Ocr, session: u64, follow: bool, first: bool) -> Option<Page> {
    let gen = shared.interface_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let cfg = shared.cfg();
    tr.ensure_title(&cfg.general.profile, &cfg.general.game_title);
    // The reader's own picture when it is in front (no taskbar or other windows).
    let full: PxRect = capture::game_area(&cfg);
    // Wait until two pictures 250 ms apart match (at most ~1.5 s): a page that
    // is still loading or sliding gives half-drawn bubbles.
    let mut frame = match capture::capture_clean(shared, full) {
        Ok(f) => f,
        Err(e) => {
            shared.ui(|u| {
                u.interface = None;
                u.notice(format!("Захват экрана: {e}"))
            });
            return None;
        }
    };
    for _ in 0..(if first { 6 } else { 0 }) {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let Ok(next) = capture::capture_clean(shared, full) else { break };
        let d = coarse_diff(&coarse(&frame), &coarse(&next));
        frame = next;
        if d < 0.012 {
            break;
        }
        tracing::debug!(target: "interface", "экран ещё меняется ({d:.3}), жду");
    }
    if !reading(session) {
        return None;
    }
    let up = upscale_for(cfg.interface.upscale, full.h);
    shared.ui(|u| u.interface = Some(InterfaceView { items: vec![], pending: 1, generation: gen }));
    let t0 = Instant::now();
    let lang = cfg.source_lang();
    let lines = match ocr.recognize(frame.clone(), up, lang).await {
        Ok(l) => l,
        Err(e) => {
            shared.ui(|u| {
                u.interface = None;
                u.notice(format!("OCR: {e}"))
            });
            return None;
        }
    };
    if !reading(session) || shared.interface_gen.load(Ordering::SeqCst) != gen {
        return None;
    }
    let ocr_ms = t0.elapsed().as_millis();
    let n_lines = lines.len();
    let items: Vec<InterfaceItem> = bubbles(&lines, lang, tr, &frame)
        .into_iter()
        .take(cfg.interface.max_lines)
        .map(|mut it| {
            it.rect = it.rect.offset(full.x as f32, full.y as f32); // to screen coordinates
            it.patch = it.patch.take().map(|p| Arc::new((*p).clone().offset(full.x, full.y)));
            it
        })
        .collect();
    shared.ui(|u| {
        u.stats.add_ocr(ocr_ms);
        let id = u.ocr_debug.as_ref().map_or(0, |d| d.id) + 1;
        let text = items.iter().map(|it| it.src.as_str()).collect::<Vec<_>>().join("\n");
        u.ocr_debug = Some(crate::state::OcrDebug { frame: Arc::new(frame.clone()), text, ms: ocr_ms, id });
    });
    tracing::info!(target: "interface", "OCR страницы: {n_lines} строк → {} пузырей за {ocr_ms} мс", items.len());
    for it in &items {
        tracing::debug!(target: "interface", "пузырь {}×{} строк: {}", it.rect.w as i32, it.lines, it.src);
    }
    let page = Page { frame: frame.clone(), area: full };
    if items.is_empty() {
        shared.ui(|u| {
            if follow {
                u.interface = Some(InterfaceView { items: vec![], pending: 0, generation: gen });
            } else {
                u.interface = None;
            }
            if first && follow {
                u.notice(format!("Пузырей с текстом ({}) пока не видно — листайте, перевод появится сам. F8 — выключить", lang.name_ru()))
            } else if first {
                u.notice(format!("На экране не найден текст ({})", lang.name_ru()))
            }
        });
        return if follow { Some(page) } else { None };
    }

    let preset = cfg.active_model().clone();
    // Instant answers first (cache/glossary/corrections), then the model.
    let mut unique: Vec<String> = Vec::new();
    let mut done: HashMap<String, String> = HashMap::new();
    for it in &items {
        if done.contains_key(&it.src) || unique.contains(&it.src) {
            continue;
        }
        match tr.lookup(&preset.id, &it.src, TextKind::Dialogue) {
            Some(t) => {
                done.insert(it.src.clone(), t);
            }
            None => unique.push(it.src.clone()),
        }
    }
    let mut items = items;
    for it in items.iter_mut() {
        it.dst = done.get(&it.src).cloned();
    }
    let n_pending = unique.len();
    shared.ui(|u| u.interface = Some(InterfaceView { items, pending: n_pending, generation: gen }));
    if unique.is_empty() {
        remember_page(shared, gen);
    } else {
        let (shared, tr) = (shared.clone(), tr.clone());
        let game = cfg.general.game_title.clone();
        tokio::spawn(async move { translate_bubbles(&shared, &tr, &preset, &game, gen, unique).await });
    }
    Some(page)
}

/// Translate the page's new bubbles and put each result into the view `gen`.
/// Stops when that view is gone (the page moved or F8 was pressed); finished
/// translations stay in the cache, so the page comes back at once.
async fn translate_bubbles(shared: &Arc<Shared>, tr: &Arc<Translator>, preset: &crate::config::ModelPreset, game: &str, gen: u64, unique: Vec<String>) {
    let alive = || shared.interface_gen.load(Ordering::SeqCst) == gen;
    let set_result = |src: &str, dst: String| {
        shared.ui(|u| {
            if let Some(v) = u.interface.as_mut().filter(|v| v.generation == gen) {
                for it in v.items.iter_mut().filter(|it| it.src == src) {
                    it.dst = Some(dst.clone());
                }
                v.pending = v.pending.saturating_sub(1);
            }
        })
    };
    let context = if shared.cfg().interface.page_context { PAGE_CONTEXT.lock().unwrap().clone() } else { Vec::new() };
    translate_texts(shared, tr, preset, game, unique, &context, alive, set_result).await;
    if alive() {
        remember_page(shared, gen);
    }
}

/// The last bubbles of the page read before (original, translation), in
/// reading order: context for the next page of the same reading session.
static PAGE_CONTEXT: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// Keep the last 6 translated bubbles of view `gen` as context for the next page.
fn remember_page(shared: &Arc<Shared>, gen: u64) {
    let pairs: Vec<(String, String)> = {
        let u = shared.ui.lock().unwrap();
        let Some(v) = u.interface.as_ref().filter(|v| v.generation == gen) else { return };
        v.items.iter().filter_map(|it| it.dst.as_ref().filter(|d| **d != it.src).map(|d| (it.src.clone(), d.clone()))).collect()
    };
    if pairs.is_empty() {
        return;
    }
    let skip = pairs.len().saturating_sub(6);
    *PAGE_CONTEXT.lock().unwrap() = pairs.into_iter().skip(skip).collect();
}

/// Translate the bubbles of one page: the whole page in one call when it fits
/// (up to 24 bubbles / ~900 chars) — the model then sees the conversation and
/// keeps names the same; what a batch could not give back, one by one.
/// `alive() == false` stops early; `done` gets every result (or the original
/// text when translation failed). Also used for chapter files.
pub async fn translate_texts(
    shared: &Arc<Shared>,
    tr: &Arc<Translator>,
    preset: &crate::config::ModelPreset,
    game: &str,
    unique: Vec<String>,
    context: &[(String, String)],
    alive: impl Fn() -> bool,
    mut done: impl FnMut(&str, String),
) {
    let mut chunks: Vec<Vec<String>> = Vec::new();
    for src in unique {
        let fits = chunks.last().is_some_and(|c: &Vec<String>| c.len() < 24 && c.iter().map(|s| s.chars().count()).sum::<usize>() + src.chars().count() < 900);
        if fits {
            chunks.last_mut().unwrap().push(src);
        } else {
            chunks.push(vec![src]);
        }
    }
    for chunk in chunks {
        if !alive() {
            return; // hidden or replaced
        }
        let mut rest: Vec<String> = chunk.clone();
        if chunk.len() > 1 {
            let t0 = std::time::Instant::now();
            match tr.translate_batch(preset, game, &chunk, TextKind::Dialogue, context).await {
                Ok(out) => {
                    shared.ui(|u| u.stats.add_llm(t0.elapsed().as_millis() / chunk.len() as u128, false));
                    rest.clear();
                    for (src, dst) in chunk.iter().zip(out) {
                        match dst {
                            Some(d) => done(src, d),
                            None => rest.push(src.clone()),
                        }
                    }
                    if !rest.is_empty() {
                        tracing::info!("пакет: {} переведу по одному", wuxia_core::text::count_ru(rest.len(), "пузырь", "пузыря", "пузырей"));
                    }
                }
                Err(e) => tracing::warn!("пакетный перевод страницы: {e}; перевожу по одному"),
            }
        }
        for src in rest {
            if !alive() {
                return;
            }
            let res = tr.translate(preset, game, "", &src, TextKind::Dialogue, Opts { use_cache: true, use_history: false }, |_| {}).await;
            let dst = match res {
                Ok(o) => {
                    shared.ui(|u| u.stats.add_llm(o.ms, o.cached));
                    o.text
                }
                Err(e) => {
                    tracing::warn!("перевод пузыря «{src}»: {e}");
                    src.clone()
                }
            };
            done(&src, dst);
        }
    }
}
