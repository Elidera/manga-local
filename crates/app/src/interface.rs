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
use manga_core::image::{contrast_text, luma, Frame, Rect};
use manga_core::lang::Lang;
use manga_core::layout;
use manga_core::prompt::TextKind;

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

/// OCR upscale for F8: small fonts (and the thin lettering drawn over
/// artwork) read much better at 2×. Windows OCR takes ~0.2 s a page, so 2× is
/// used up to 4K; only a 4K picture is big enough as it is.
pub fn upscale_for(configured: u32, screen_h: i32) -> u32 {
    match configured {
        0 if screen_h < 2000 => 2,
        0 => 1,
        k => k,
    }
}

/// Is this OCR line a label F8 translates? (Shared with `--selftest`.)
pub fn is_label_line(text: &str, lang: manga_core::lang::Lang, tr: &Translator) -> bool {
    manga_core::lang::is_label(text, lang) && !tr.should_skip(text)
}

/// Group the OCR lines of a page into bubbles, in reading order. Each bubble
/// keeps its text joined the way the language is written, its box (`frame`
/// coordinates) and the colour around it. `found`: the bubble outlines of the
/// page (`manga_core::bubbles::find`, looked for once per page). Not ordered.
fn bubbles(lines: &[OcrLine], lang: Lang, tr: &Translator, frame: &Frame, found: &[manga_core::bubbles::Found]) -> Vec<InterfaceItem> {
    let outlines: Vec<Rect> = found.iter().map(|f| f.outer).collect();
    let bubble_of = |r: &Rect| {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        outlines.iter().position(|o| cx >= o.x && cx <= o.x + o.w && cy >= o.y && cy <= o.y + o.h)
    };
    // A line that alone does not look like text («I», «…») stays when it is in
    // a bubble together with real text: it is part of the phrase.
    let good: Vec<bool> = lines.iter().map(|l| is_label_line(&l.text, lang, tr)).collect();
    let kept: Vec<&OcrLine> = lines
        .iter()
        .enumerate()
        .filter(|(i, l)| {
            let r = &l.rect;
            // Right above or below a real line, overlapping it sideways (the
            // «I'M» over «THANKFUL FOR ALL…» in a caption).
            let next_to_text = |o: &OcrLine| {
                let h = o.rect.h.max(r.h);
                let gap = (o.rect.y - (r.y + r.h)).max(r.y - (o.rect.y + o.rect.h));
                let overlap = (r.x + r.w).min(o.rect.x + o.rect.w) - r.x.max(o.rect.x);
                gap < h * 0.8 && overlap > 0.0 && r.h < o.rect.h * 1.8 && o.rect.h < r.h * 1.8
            };
            good[*i]
                || !tr.should_skip(&l.text)
                    && !manga_core::lang::is_screen_junk(&l.text)
                    && l.text.chars().any(char::is_alphanumeric)
                    && lines.iter().enumerate().any(|(j, o)| j != *i && good[j] && (bubble_of(&o.rect).is_some() && bubble_of(&o.rect) == bubble_of(r) || next_to_text(o)))
        })
        .map(|(_, l)| l)
        .collect();
    let rects: Vec<Rect> = kept.iter().map(|l| l.rect).collect();
    // Lines inside one bubble outline are one bubble, whatever the spacing.
    let groups = layout::group_in_outlines(&rects, &outlines);
    let items = groups
        .iter()
        .map(|g| {
            let mut rect = layout::block_rect(&rects, g);
            // In a light speech bubble, everything lettered there is cleaned
            // away, also a line OCR did not read (a bigger bold first line
            // like «HEY!» is often skipped): the letters box of the bubble.
            if let Some(f) = bubble_of(&rect).map(|b| &found[b]) {
                let light = manga_core::image::luma(f.paper[2], f.paper[1], f.paper[0]) > 200;
                let small = f.outer.w * f.outer.h < frame.width as f32 * frame.height as f32 * 0.25;
                if light && small && !f.vertical && f.text.h <= rect.h * 3.0 + f.char_px * 2.0 {
                    rect = rect.union(&f.text);
                }
            }
            let texts: Vec<&str> = g.iter().map(|&i| kept[i].text.as_str()).collect();
            let mut src = tr.fix_ocr(&manga_core::text::join_lines_for(&texts, lang));
            // As read, before the change of case: for the intonation below.
            let raw = src.clone();
            if lang == Lang::En {
                src = manga_core::text::sentence_case_en(&src);
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
            let paper = !busy && manga_core::image::luma(bg[2], bg[1], bg[0]) > 200;
            let patch = manga_core::clean::clean_text(frame, &rect, line_h, bg, paper);
            let fg = match &patch {
                Some(p) if busy => contrast_text(p.mean_rgb()),
                _ => contrast_text(bg),
            };
            // On the artwork, outside every bubble, and shaped like a sound: a sound effect.
            let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            let in_bubble = outlines.iter().any(|o| cx >= o.x && cx <= o.x + o.w && cy >= o.y && cy <= o.y + o.h);
            let sfx = !in_bubble && busy && manga_core::lang::is_sfx_like(&src, lang);
            // One or two small letters alone outside every bubble («름», «4수»):
            // bits of the drawing or of the site, not a phrase. Big ones are sounds.
            let letters = src.chars().filter(|c| c.is_alphanumeric()).count();
            let fragment = !in_bubble && g.len() == 1 && letters <= 2 && !src.contains(['!', '?', '！', '？']) && line_h < frame.height as f32 * 0.035;
            if fragment {
                tracing::debug!(target: "interface", "обрывок вне пузыря пропущен: «{src}»");
                src.clear();
            }
            (InterfaceItem { fg, busy, bg, rect, src, dst: None, line_h, lines: g.len(), patch: patch.map(Arc::new), sfx }, raw)
        })
        .filter(|(it, _)| !it.src.is_empty())
        .collect::<Vec<_>>();
    // How each bubble is said: its letters against the usual ones of the page.
    let mut heights: Vec<f32> = items.iter().filter(|(it, _)| !it.sfx).map(|(it, _)| it.line_h).collect();
    heights.sort_by(f32::total_cmp);
    let usual = heights.get(heights.len() / 2).copied().unwrap_or(0.0);
    items
        .into_iter()
        .map(|(it, raw)| {
            let size = if usual > 0.0 && heights.len() >= 3 { it.line_h / usual } else { 0.0 };
            if !it.sfx {
                tr.note_tone(&it.src, manga_core::tone::detect(&raw, size));
            }
            it
        })
        .collect()
}

/// Reading-mode session: 0 = off. A new F8 session ends the previous one.
static READING: AtomicU64 = AtomicU64::new(0);
static SESSIONS: AtomicU64 = AtomicU64::new(0);

fn reading(session: u64) -> bool {
    READING.load(Ordering::SeqCst) == session
}

/// The reading mode (F8) is on.
pub fn reading_on() -> bool {
    READING.load(Ordering::SeqCst) != 0
}

/// The texts to translate, split by kind: (dialogue, sound effects). Cached
/// answers go straight into `done`.
pub fn split_unique(items: &[InterfaceItem], tr: &Translator, model: &str, done: &mut HashMap<String, String>) -> (Vec<String>, Vec<String>) {
    let (mut talk, mut sfx) = (Vec::new(), Vec::new());
    for it in items {
        let list = if it.sfx { &mut sfx } else { &mut talk };
        if done.contains_key(&it.src) || list.contains(&it.src) {
            continue;
        }
        let kind = if it.sfx { TextKind::Interface } else { TextKind::Dialogue };
        match tr.lookup(model, &it.src, kind) {
            Some(t) => {
                done.insert(it.src.clone(), tr.toned(&it.src, t));
            }
            None => list.push(it.src.clone()),
        }
    }
    (talk, sfx)
}

/// Reading order of a page's bubbles: panel by panel (found by the gutters
/// between them; manga right to left), inside a panel row by row. A page with
/// no gutters keeps the plain row order.
fn page_order(items: Vec<InterfaceItem>, frame: &Frame, rtl: bool) -> Vec<InterfaceItem> {
    let rects: Vec<Rect> = items.iter().map(|it| it.rect).collect();
    let panels = manga_core::panels::find(frame, rtl);
    let order = manga_core::panels::reading_order(&rects, &panels, rtl);
    let mut slots: Vec<Option<InterfaceItem>> = items.into_iter().map(Some).collect();
    order.into_iter().filter_map(|i| slots[i].take()).collect()
}

/// The bubbles of a page: from Windows OCR lines (`bubbles`), and — for
/// Japanese with manga-ocr — bubbles found by the picture whose text Windows
/// OCR could not read (vertical columns, handwriting) read by manga-ocr. Frame
/// coordinates, reading order. Shared with chapter files and `--selftest`.
/// `found`: the page's bubbles (`manga_core::bubbles::find`), looked for once
/// per page (on a long strip each pass takes time and memory).
#[allow(clippy::too_many_arguments)]
pub async fn page_items(lines: &[OcrLine], lang: Lang, tr: &Translator, frame: &Frame, ocr: &Ocr, found: Vec<manga_core::bubbles::Found>, ja: Option<Arc<crate::jaocr::JaOcr>>) -> Vec<InterfaceItem> {
    let rtl = lang == Lang::Ja;
    let lines = reread_bubbles(lines, lang, tr, frame, ocr, &found).await;
    let lines = &lines[..];
    let mut items = bubbles(lines, lang, tr, frame, &found);
    let Some(ja) = ja.filter(|_| lang == Lang::Ja) else { return page_order(items, frame, rtl) };
    let inside = |r: &manga_core::image::Rect, o: &manga_core::image::Rect| {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        cx >= o.x && cx <= o.x + o.w && cy >= o.y && cy <= o.y + o.h
    };
    // Outside every bubble Windows OCR often reads artwork as letters («も。〇き朝。»
    // from drawn circles): such lines stay only when they look like real text.
    items.retain(|it| {
        found.iter().any(|f| inside(&it.rect, &f.outer)) || {
            let japanese = it.src.chars().filter(|&c| manga_core::text::is_kana(c) || manga_core::text::is_han(c)).count();
            japanese >= 4 && !it.src.contains(['〇', '○', '◯'])
        }
    });
    // Horizontal bubbles Windows OCR has read stay its own (faster, as good).
    let todo: Vec<manga_core::bubbles::Found> = found.into_iter().filter(|f| f.vertical || !items.iter().any(|it| inside(&it.rect, &f.outer))).collect();
    if todo.is_empty() {
        return page_order(items, frame, rtl);
    }
    let t0 = Instant::now();
    // Only the bubbles' pieces go to the reader, not a copy of the whole page.
    let crops: Vec<(manga_core::bubbles::Found, Frame)> = todo.into_iter().filter_map(|f| crate::jaocr::JaOcr::crop(frame, &f.text).map(|c| (f, c))).collect();
    let read: Vec<(manga_core::bubbles::Found, String)> = tokio::task::spawn_blocking(move || crops.into_iter().filter_map(|(f, c)| ja.read_crop(&c).map(|t| (f, t))).collect()).await.unwrap_or_default();
    tracing::info!(target: "interface", "manga-ocr: {} пузырей за {} мс", read.len(), t0.elapsed().as_millis());
    for (f, text) in read {
        items.retain(|it| !inside(&it.rect, &f.outer));
        let src = tr.fix_ocr(&manga_core::text::normalize_for(&text, lang));
        if src.is_empty() || tr.should_skip(&src) {
            continue;
        }
        // Russian is written across: give it most of the bubble's width.
        let cx = f.text.x + f.text.w / 2.0;
        let w = (f.outer.w * 0.78).max(f.text.w);
        let rect = manga_core::image::Rect { x: cx - w / 2.0, y: f.text.y, w, h: f.text.h };
        let patch = manga_core::clean::clean_text(frame, &f.text, f.char_px, f.paper, true);
        items.push(InterfaceItem { fg: contrast_text(f.paper), busy: false, bg: f.paper, rect, src, dst: None, line_h: f.char_px, lines: f.lines.max(2), patch: patch.map(Arc::new), sfx: false });
    }
    page_order(items, frame, rtl)
}

/// Recognize a screen picture: each piece of the page on its own (the blank
/// site margins around a webtoon strip left out), enlarged so the letters of
/// its bubbles come out about 40 px high (a strip shown narrow on a wide
/// screen has letters of 10 px). `found`: the page's bubbles.
pub async fn read_screen(ocr: &Ocr, frame: &Frame, configured: u32, lang: Lang, found: &[manga_core::bubbles::Found]) -> anyhow::Result<Vec<OcrLine>> {
    let base = upscale_for(configured, frame.height as i32);
    let mut out = Vec::new();
    for (x, w) in frame.content_columns() {
        let whole = (x, w) == (0, frame.width);
        // The letters decide only for a strip cut out of a wide screen (letters
        // of 10 px); a whole page keeps the usual 2×. Never more than ~8
        // million pixels for the recognizer (time grows with them).
        let k = if configured != 0 {
            configured
        } else if whole {
            base
        } else {
            let mut k = letters_upscale(base, found, x as f32, w as f32);
            while k > 1 && (w * k) as u64 * (frame.height * k) as u64 > 8_000_000 {
                k -= 1;
            }
            k.max(1)
        };
        let piece = if whole { frame.clone() } else { frame.crop(x, 0, w, frame.height) };
        if !whole || k != base {
            tracing::debug!(target: "interface", "распознаю полосу x {x}, ширина {w}, увеличение ×{k}");
        }
        for mut l in ocr.recognize(piece, k, lang).await? {
            l.rect = l.rect.offset(x as f32, 0.0);
            out.push(l);
        }
    }
    Ok(out)
}

/// Upscale for a piece of the page: by the letter size of its bubbles (median),
/// never less than `base`, at most 4×.
fn letters_upscale(base: u32, found: &[manga_core::bubbles::Found], x: f32, w: f32) -> u32 {
    let mut sizes: Vec<f32> = found
        .iter()
        // Real speech bubbles: light paper, a few letters wide (not an eye or a button).
        .filter(|f| !f.vertical && f.char_px >= 5.0 && f.text.w >= f.char_px * 2.5)
        .filter(|f| manga_core::image::luma(f.paper[2], f.paper[1], f.paper[0]) > 200)
        .filter(|f| {
            let cx = f.outer.x + f.outer.w / 2.0;
            cx >= x && cx < x + w
        })
        .map(|f| f.char_px)
        .collect();
    if sizes.is_empty() {
        return base;
    }
    sizes.sort_by(|a, b| a.total_cmp(b));
    let m = sizes[sizes.len() / 2];
    ((40.0 / m).round() as u32).clamp(1, 4).max(base)
}

/// Bubbles found by the picture where Windows OCR read nothing, or only part
/// of the lettering (the letters box reaches a line height and more beyond
/// the lines read): read once more, each on its own and enlarged. Windows OCR
/// picks one letter size per picture, so next to a huge «CRASH!» it can skip
/// ordinary bubbles, and a bigger bold first line («HEY!») next to normal
/// ones. Vertical Japanese is left to manga-ocr. Returns the lines to use.
async fn reread_bubbles(lines: &[OcrLine], lang: Lang, tr: &Translator, frame: &Frame, ocr: &Ocr, found: &[manga_core::bubbles::Found]) -> Vec<OcrLine> {
    // The neural OCR reads every line at its own size already.
    if crate::neural::active(lang) {
        return lines.to_vec();
    }
    let inside = |r: &Rect, o: &Rect| {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        cx >= o.x && cx <= o.x + o.w && cy >= o.y && cy <= o.y + o.h
    };
    let letters = |t: &str| {
        t.chars()
            .filter(|&c| match lang {
                Lang::Ko => manga_core::text::is_hangul(c),
                Lang::Ja => manga_core::text::is_kana(c) || manga_core::text::is_han(c),
                Lang::Zh => manga_core::text::is_han(c),
                Lang::En => c.is_ascii_alphabetic(),
            })
            .count()
    };
    let page = frame.width as f32 * frame.height as f32;
    let todo: Vec<&manga_core::bubbles::Found> = found
        .iter()
        .filter(|f| !(lang == Lang::Ja && f.vertical))
        .filter(|f| f.text.w >= 12.0 && f.text.h >= 8.0 && f.outer.w * f.outer.h < page * 0.4)
        // A real bubble: light paper and a few letters wide (eyes, buttons and
        // glints are found as «bubbles» too, and each re-read costs time).
        .filter(|f| manga_core::image::luma(f.paper[2], f.paper[1], f.paper[0]) > 200 && f.text.w >= f.char_px * 2.5 && f.lines >= 1)
        .filter(|f| {
            let read: Vec<&OcrLine> = lines.iter().filter(|l| inside(&l.rect, &f.outer)).collect();
            let Some(first) = read.first() else { return true };
            let u = read.iter().skip(1).fold(first.rect, |a, l| a.union(&l.rect));
            let h = read.iter().map(|l| l.rect.h).sum::<f32>() / read.len() as f32;
            let light = manga_core::image::luma(f.paper[2], f.paper[1], f.paper[0]) > 200;
            light && (u.y - f.text.y > h * 0.9 || (f.text.y + f.text.h) - (u.y + u.h) > h * 0.9)
        })
        .take(8)
        .collect();
    let mut out: Vec<OcrLine> = lines.to_vec();
    let mut added = 0;
    let t0 = Instant::now();
    for f in todo {
        // At most ~0.4 s on this per page: reading speed matters more.
        if t0.elapsed() > Duration::from_millis(400) {
            break;
        }
        let pad = 4.0;
        let x = (f.outer.x - pad).max(0.0) as u32;
        let y = (f.outer.y - pad).max(0.0) as u32;
        let w = ((f.outer.w + 2.0 * pad) as u32).min(frame.width.saturating_sub(x));
        let h = ((f.outer.h + 2.0 * pad) as u32).min(frame.height.saturating_sub(y));
        if w < 16 || h < 16 {
            continue;
        }
        // Letters about 40 px high read best.
        let k = (40.0 / f.char_px.max(4.0)).round().clamp(2.0, 4.0) as u32;
        let Ok(read) = ocr.recognize(frame.crop(x, y, w, h), k, lang).await else { continue };
        let new: Vec<OcrLine> = read
            .into_iter()
            .map(|mut l| {
                l.rect = l.rect.offset(x as f32, y as f32);
                l
            })
            .filter(|l| inside(&l.rect, &f.outer) && !tr.should_skip(&l.text))
            .collect();
        let had: usize = out.iter().filter(|l| inside(&l.rect, &f.outer)).map(|l| letters(&l.text)).sum();
        let got: usize = new.iter().map(|l| letters(&l.text)).sum();
        // Only real text, and more of it than before (eyes and hair read as «O», «ll»…).
        if got < if lang == Lang::En { 3 } else { 1 } || got <= had || !new.iter().any(|l| is_label_line(&l.text, lang, tr)) {
            continue;
        }
        out.retain(|l| !inside(&l.rect, &f.outer));
        added += new.len();
        out.extend(new);
    }
    if added > 0 {
        tracing::info!(target: "interface", "перечитаны пузыри: {added} строк");
    }
    out
}

pub async fn toggle(shared: Arc<Shared>, tr: Arc<Translator>, ocr: Ocr) {
    // Second press hides the translation and ends the reading mode.
    let showing = shared.ui.lock().unwrap().interface.is_some();
    if showing {
        READING.store(0, Ordering::SeqCst);
        shared.interface_gen.fetch_add(1, Ordering::SeqCst);
        shared.ui(|u| {
            u.interface = None;
            u.flash = Some(("Перевод выключен".into(), Instant::now() + Duration::from_millis(1200)));
        });
        return;
    }
    let session = SESSIONS.fetch_add(1, Ordering::SeqCst) + 1;
    READING.store(session, Ordering::SeqCst);
    shared.ui(|u| u.flash = Some(("Перевод включён".into(), Instant::now() + Duration::from_millis(1200))));
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
    if tr.ensure_title(&cfg.general.profile, &cfg.general.game_title) {
        shared.ui(|u| u.name_offers.clear());
    }
    // F8 pressed with the taskbar, our panel or another program in front: the
    // reader window comes back first, so its page (not the whole screen with
    // tabs and other windows) is read.
    if first && cfg.watches_window() && !cfg.window_in_front() {
        let class = crate::config::profile(&cfg.general.profile).window_class;
        if !tokio::task::block_in_place(|| crate::win::bring_reader_back(&cfg.general.window_match, class)) && cfg.general.profile == "browser" {
            READING.store(0, Ordering::SeqCst);
            shared.ui(|u| {
                u.interface = None;
                u.notice("Впереди не браузер: откройте главу в браузере и нажмите F8 ещё раз")
            });
            return None;
        }
    }
    // The reader's own picture when it is in front (no taskbar or other windows).
    let full: PxRect = capture::game_area(&cfg);
    crate::hotkeys::watch_foreground();
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
    let lang = cfg.source_lang();
    let found = manga_core::bubbles::find(&frame);
    // Neural OCR chosen but this language not downloaded yet: Windows OCR
    // reads meanwhile, the files come in the background (asked once).
    if crate::neural::wanted() && !crate::neural::active(lang) {
        static ASKED: std::sync::Mutex<Vec<Lang>> = std::sync::Mutex::new(Vec::new());
        let mut asked = ASKED.lock().unwrap();
        if !asked.contains(&lang) {
            asked.push(lang);
            shared.send(crate::state::Command::DownloadNeural);
        }
    }
    shared.ui(|u| u.interface = Some(InterfaceView { items: vec![], pending: 1, generation: gen }));
    let t0 = Instant::now();
    // The browser extension sent the page's pictures: their text in full quality.
    let from_page = crate::link::screen_lines(full, &frame, lang, ocr).await;
    let source = if from_page.is_some() { "картинки страницы (расширение)" } else { "экран" };
    let read = match from_page {
        Some(l) => Ok(l),
        None => read_screen(ocr, &frame, cfg.interface.upscale, lang, &found).await,
    };
    let lines = match read {
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
    // An English page while Korean is chosen (and the like): offer to switch.
    let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    if let Some(seen) = manga_core::lang::other_page_language(&texts, lang) {
        if shared.ui(|u| u.offer_language(lang, seen)) {
            crate::tray::balloon(
                &format!("На странице {} текст?", seen.name_ru()),
                &format!("Выбран {}. Откройте панель ({}) и нажмите «Переключить».", lang.name_ru(), cfg.hotkeys.panel),
            );
        }
    }
    let ja = crate::jaocr::engine(shared).await;
    let outlines = found.len();
    let items: Vec<InterfaceItem> = page_items(&lines, lang, tr, &frame, ocr, found, ja)
        .await
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
        if outlines >= 2 {
            crate::journal::begin(gen, &frame, lang, source, ocr_ms, outlines, &lines, &[], full);
            crate::journal::finish(shared, gen, false);
        }
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
    let mut items = items;
    if cfg.interface.sfx == "keep" {
        items.retain(|it| !it.sfx);
    }
    // Instant answers first (cache/glossary/corrections), then the model.
    let mut done: HashMap<String, String> = HashMap::new();
    let (talk, sfx) = split_unique(&items, tr, &preset.id, &mut done);
    for it in items.iter_mut() {
        it.dst = done.get(&it.src).cloned();
    }
    let n_pending = talk.len() + sfx.len();
    crate::journal::begin(gen, &frame, lang, source, ocr_ms, outlines, &lines, &items, full);
    shared.ui(|u| u.interface = Some(InterfaceView { items, pending: n_pending, generation: gen }));
    if n_pending == 0 {
        crate::journal::finish(shared, gen, false);
        remember_page(shared, gen);
    } else {
        let (shared, tr) = (shared.clone(), tr.clone());
        let game = cfg.general.game_title.clone();
        tokio::spawn(async move { translate_bubbles(&shared, &tr, &preset, &game, gen, talk, sfx).await });
    }
    Some(page)
}

/// Translate the page's new bubbles and put each result into the view `gen`.
/// Stops when that view is gone (the page moved or F8 was pressed); finished
/// translations stay in the cache, so the page comes back at once.
async fn translate_bubbles(shared: &Arc<Shared>, tr: &Arc<Translator>, preset: &crate::config::ModelPreset, game: &str, gen: u64, unique: Vec<String>, sfx: Vec<String>) {
    let alive = || shared.interface_gen.load(Ordering::SeqCst) == gen;
    let lang = shared.cfg().source_lang();
    let set_result = |src: &str, dst: String| {
        // The original back means the translation failed: the bubble stays as it is.
        let failed = dst == src;
        crate::journal::result(gen, src, &dst);
        let names = if failed { Vec::new() } else { tr.observe_names(lang, src, &dst) };
        shared.ui(|u| {
            u.offer_names(names);
            if let Some(v) = u.interface.as_mut().filter(|v| v.generation == gen) {
                if !failed {
                    for it in v.items.iter_mut().filter(|it| it.src == src) {
                        it.dst = Some(dst.clone());
                    }
                }
                v.pending = v.pending.saturating_sub(1);
            }
        })
    };
    let context = if shared.cfg().interface.page_context { PAGE_CONTEXT.lock().unwrap().clone() } else { Vec::new() };
    translate_texts(shared, tr, preset, game, unique, TextKind::Dialogue, &context, alive, set_result).await;
    // Sound effects: on their own, briefly, with no dialogue around them.
    if !sfx.is_empty() && alive() {
        translate_texts(shared, tr, preset, game, sfx, TextKind::Interface, &[], alive, set_result).await;
    }
    crate::journal::finish(shared, gen, !alive());
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
        v.items.iter().filter(|it| !it.sfx).filter_map(|it| it.dst.as_ref().filter(|d| **d != it.src).map(|d| (it.src.clone(), d.clone()))).collect()
    };
    if pairs.is_empty() {
        return;
    }
    let skip = pairs.len().saturating_sub(6);
    *PAGE_CONTEXT.lock().unwrap() = pairs.into_iter().skip(skip).collect();
}

/// Translation requests of pages, one at a time (see `translate_texts`).
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Wait until the model is loaded (the first start downloads it: minutes).
/// False when the page went away meanwhile or the model failed to start.
async fn wait_for_model(shared: &Arc<Shared>, alive: &impl Fn() -> bool) -> bool {
    let mut told = false;
    loop {
        let status = shared.ui.lock().unwrap().server.clone();
        match status {
            Some(s) if s.is_ready() => return true,
            Some(crate::state::ServerStatus::Failed(_)) => return false,
            _ => {}
        }
        if !alive() {
            return false;
        }
        if !told {
            told = true;
            shared.ui(|u| u.notice("Нейросеть ещё загружается — перевод появится сам, как только она будет готова"));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
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
    kind: TextKind,
    context: &[(String, String)],
    alive: impl Fn() -> bool,
    mut done: impl FnMut(&str, String),
) {
    if unique.is_empty() || !wait_for_model(shared, &alive).await {
        return;
    }
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
        // One page's request at a time; a page that went away while waiting
        // for its turn is skipped (no minutes-long queue of old pages).
        let _turn = TURN.lock().await;
        if !alive() {
            return; // hidden or replaced
        }
        let mut rest: Vec<String> = chunk.clone();
        if chunk.len() > 1 {
            let t0 = std::time::Instant::now();
            match tr.translate_batch(preset, game, &chunk, kind, context).await {
                Ok(out) => {
                    shared.ui(|u| u.stats.add_llm(t0.elapsed().as_millis() / chunk.len() as u128, false));
                    rest.clear();
                    for (src, dst) in chunk.iter().zip(out) {
                        match dst {
                            Some(d) => done(src, tr.toned(src, d)),
                            None => rest.push(src.clone()),
                        }
                    }
                    if !rest.is_empty() {
                        tracing::info!("пакет: {} переведу по одному", manga_core::text::count_ru(rest.len(), "пузырь", "пузыря", "пузырей"));
                    }
                }
                Err(e) => tracing::warn!("пакетный перевод страницы: {e}; перевожу по одному"),
            }
        }
        for src in rest {
            if !alive() {
                return;
            }
            let res = tr.translate(preset, game, "", &src, kind, Opts { use_cache: true, use_history: false }, |_| {}).await;
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
            done(&src, tr.toned(&src, dst));
        }
    }
}
