//! Page translation (F8): OCR the reader window, group the lines into speech
//! bubbles, translate all bubbles of the page in one request and paint the
//! Russian text inside each bubble over the original (fill + wrapped text).

use crate::capture::{self, PxRect};
use crate::ocr::Ocr;
use crate::state::{InterfaceItem, InterfaceView, Shared};
use crate::translate::{Opts, Translator};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
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
            let src = tr.fix_ocr(&wuxia_core::text::join_lines_for(&texts, lang));
            let line_h = g.iter().map(|&i| rects[i].h).sum::<f32>() / g.len() as f32;
            // Look at the paper a little outside the text: flat = a bubble (fill
            // with its colour), varied = text over artwork (dark plate).
            let (bg, spread) = frame.ring_stats(&rect, (line_h * 0.35).clamp(3.0, 14.0));
            InterfaceItem { fg: contrast_text(bg), busy: spread > 28.0, bg, rect, src, dst: None, line_h, lines: g.len() }
        })
        .filter(|it| !it.src.is_empty())
        .collect();
    let order = layout::reading_order(&items.iter().map(|it| it.rect).collect::<Vec<_>>(), lang == Lang::Ja);
    order.into_iter().map(|i| items[i].clone()).collect()
}

pub async fn toggle(shared: Arc<Shared>, tr: Arc<Translator>, ocr: Ocr) {
    // Second press hides the current translation.
    let showing = shared.ui.lock().unwrap().interface.is_some();
    if showing {
        shared.interface_gen.fetch_add(1, Ordering::SeqCst);
        shared.ui(|u| u.interface = None);
        return;
    }
    let gen = shared.interface_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let cfg = shared.cfg();
    // The game's own picture when it is in front (no taskbar or other windows).
    let full: PxRect = capture::game_area(&cfg);
    // Game menus open with an animation (labels slide and grow): a picture taken
    // mid-animation gives boxes smaller than the final text, and the original
    // then shows around the translation. Wait until two pictures 250 ms apart
    // match (at most ~1.5 s).
    let mut frame = match capture::capture_clean(&shared, full) {
        Ok(f) => f,
        Err(e) => {
            shared.ui(|u| u.notice(format!("Захват экрана: {e}")));
            return;
        }
    };
    for _ in 0..6 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let Ok(next) = capture::capture_clean(&shared, full) else { break };
        let d = coarse_diff(&coarse(&frame), &coarse(&next));
        frame = next;
        if d < 0.012 {
            break;
        }
        tracing::debug!(target: "interface", "экран ещё меняется ({d:.3}), жду");
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
            return;
        }
    };
    let ocr_ms = t0.elapsed().as_millis();
    let n_lines = lines.len();
    let items: Vec<InterfaceItem> = bubbles(&lines, lang, &tr, &frame)
        .into_iter()
        .take(cfg.interface.max_lines)
        .map(|mut it| {
            it.rect = it.rect.offset(full.x as f32, full.y as f32); // to screen coordinates
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
    if items.is_empty() {
        shared.ui(|u| {
            u.interface = None;
            u.notice(format!("На экране не найден текст ({})", lang.name_ru()))
        });
        return;
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

    // Auto-hide watcher: the game screen changed (menu closed / scrolled).
    // Everything of ours that was ever on screen (translation labels, the panel
    // while it was closing) is blanked in both pictures, so only a real change
    // of the game screen counts. Menus have animated backgrounds (characters,
    // particles), so the change must be large and last ~1.5 s.
    if cfg.interface.auto_hide {
        let sh = shared.clone();
        let mut base = frame.clone();
        let mut masks: Vec<[i32; 4]> = capture::own_rects(&shared);
        tokio::spawn(async move {
            let mut changed = 0;
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                if sh.interface_gen.load(Ordering::SeqCst) != gen {
                    return;
                }
                let Ok(mut f) = capture::capture(full) else { continue };
                for m in capture::own_rects(&sh) {
                    if !masks.contains(&m) {
                        masks.push(m);
                    }
                }
                capture::apply_masks(&mut f, full, &masks);
                capture::apply_masks(&mut base, full, &masks);
                let d = coarse_diff(&coarse(&base), &coarse(&f));
                changed = if d > 0.12 { changed + 1 } else { 0 };
                if changed >= 3 {
                    tracing::info!(target: "interface", "экран изменился ({d:.3}) — прячу перевод");
                    if sh.interface_gen.load(Ordering::SeqCst) == gen {
                        sh.interface_gen.fetch_add(1, Ordering::SeqCst);
                        sh.ui(|u| u.interface = None);
                    }
                    return;
                }
            }
        });
    }

    let game = cfg.general.game_title.clone();
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
    // The whole page in one call when it fits (up to 24 bubbles / ~900 chars):
    // the model then sees the conversation and keeps names the same.
    let mut chunks: Vec<Vec<String>> = Vec::new();
    for src in unique {
        let fits = chunks.last().map_or(false, |c: &Vec<String>| {
            c.len() < 24 && c.iter().map(|s| s.chars().count()).sum::<usize>() + src.chars().count() < 900
        });
        if fits {
            chunks.last_mut().unwrap().push(src);
        } else {
            chunks.push(vec![src]);
        }
    }
    for chunk in chunks {
        if shared.interface_gen.load(Ordering::SeqCst) != gen {
            return; // hidden or replaced
        }
        let mut rest: Vec<String> = chunk.clone();
        if chunk.len() > 1 {
            let t0 = std::time::Instant::now();
            match tr.translate_batch(&preset, &game, &chunk, TextKind::Dialogue).await {
                Ok(out) => {
                    shared.ui(|u| u.stats.add_llm(t0.elapsed().as_millis() / chunk.len() as u128, false));
                    rest.clear();
                    for (src, dst) in chunk.iter().zip(out) {
                        match dst {
                            Some(d) => set_result(src, d),
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
            if shared.interface_gen.load(Ordering::SeqCst) != gen {
                return;
            }
            let res = tr.translate(&preset, &game, "", &src, TextKind::Dialogue, Opts { use_cache: true, use_history: false }, |_| {}).await;
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
            set_result(&src, dst);
        }
    }
}
