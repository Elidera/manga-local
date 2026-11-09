//! Link with the browser extension (folder `extension/`): a small web server
//! on 127.0.0.1:47811 that only the extension may talk to (requests from web
//! pages carry their site as Origin and are refused).
//!
//! The extension tells where the page's pictures are on screen and sends the
//! pictures themselves as the site gave them (full quality, not a photo of the
//! screen). F8 then reads the text from those pictures and puts it at the
//! place where the screen shows it; the rest of the page (bubble outlines,
//! paper colour, cleaning) still comes from the screen. While the reading
//! mode is on, the next pictures are read and translated ahead.
//! Nothing is downloaded beyond the pictures of the page being read, and
//! nothing is saved: the pictures stay in memory only.
//!
//! Requests: GET /hello · POST /page (JSON, answer: pictures wanted) ·
//! POST /image?id=… (bytes) · POST /mark (JSON: the reader's «здесь ошибка»).

use crate::capture::PxRect;
use crate::ocr::{Ocr, OcrLine};
use crate::state::Shared;
use crate::translate::Translator;
use manga_core::image::Frame;
use manga_core::lang::Lang;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const PORT: u16 = 47811;
/// Pieces of tall pictures (webtoon strips), as in chapter files.
const PIECE: u32 = 2400;
const OVERLAP: u32 = 400;
/// Pictures kept in memory, megapixels (4 bytes each: 60 Mpx ≈ 240 MB, a
/// few long webtoon strips).
const KEEP_MPX: u64 = 60;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Img {
    pub id: String,
    pub src: String,
    /// Where it is on the page now: x, y, width, height in CSS px of the viewport.
    pub rect: [f32; 4],
    /// The picture's own size, px.
    pub natural: [u32; 2],
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct View {
    pub url: String,
    pub title: String,
    /// Device pixels per CSS px (screen scaling × page zoom).
    pub dpr: f32,
    /// Viewport size, CSS px.
    pub inner: [f32; 2],
    /// Pictures on screen.
    pub images: Vec<Img>,
    /// The next pictures below (read ahead).
    #[serde(default)]
    pub ahead: Vec<Img>,
}

#[derive(Default)]
struct State {
    /// The last view and when it came (hotkeys clock, ms).
    view: Option<(u64, View)>,
    /// Decoded pictures, newest last.
    images: Vec<(String, Arc<Frame>)>,
    /// OCR lines of picture pieces (picture px).
    lines: HashMap<(String, usize, Lang), Vec<OcrLine>>,
    /// The extension's last request (hotkeys clock, ms).
    seen: u64,
    /// Pictures already being read ahead.
    ahead_done: HashSet<(String, Lang)>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State::default()));

/// The extension spoke within the last 20 s.
pub fn connected() -> bool {
    let seen = STATE.lock().unwrap().seen;
    seen > 0 && crate::hotkeys::now_ms().saturating_sub(seen) < 20_000
}

pub async fn serve(shared: Arc<Shared>, tr: Arc<Translator>, ocr: Ocr) {
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("связь с расширением браузера: порт {PORT} занят ({e})");
            return;
        }
    };
    tracing::info!("связь с расширением браузера: 127.0.0.1:{PORT}");
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let (sh, tr, o) = (shared.clone(), tr.clone(), ocr.clone());
        tokio::spawn(async move {
            if let Err(e) = handle(stream, &sh, &tr, &o).await {
                tracing::debug!("расширение: {e}");
            }
        });
    }
}

async fn reply(s: &mut tokio::net::TcpStream, code: &str, body: &str) -> anyhow::Result<()> {
    let head = format!("HTTP/1.1 {code}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    s.write_all(head.as_bytes()).await?;
    s.write_all(body.as_bytes()).await?;
    Ok(())
}

async fn handle(mut s: tokio::net::TcpStream, shared: &Arc<Shared>, tr: &Arc<Translator>, ocr: &Ocr) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 16 * 1024];
    let head_end = loop {
        let n = s.read(&mut tmp).await?;
        anyhow::ensure!(n > 0, "соединение закрыто");
        buf.extend_from_slice(&tmp[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        anyhow::ensure!(buf.len() < 64 * 1024, "слишком длинный заголовок");
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut hl = head.split("\r\n");
    let mut first = hl.next().unwrap_or("").split(' ');
    let (method, target) = (first.next().unwrap_or("").to_string(), first.next().unwrap_or("").to_string());
    let mut length = 0usize;
    let mut origin: Option<String> = None;
    for h in hl {
        if let Some((k, v)) = h.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = v.trim().parse().unwrap_or(0),
                "origin" => origin = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    // Only the extension: a web page's request carries the page's site as Origin.
    let from_extension = origin.as_deref().is_some_and(|o| o.starts_with("chrome-extension://") || o.starts_with("moz-extension://"));
    if (origin.is_some() || method == "POST") && !from_extension {
        return reply(&mut s, "403 Forbidden", "{\"error\":\"только расширение Manga Local\"}").await;
    }
    anyhow::ensure!(length <= 80 * 1024 * 1024, "слишком большой запрос");
    let mut body = buf[head_end..].to_vec();
    while body.len() < length {
        let n = s.read(&mut tmp).await?;
        anyhow::ensure!(n > 0, "запрос оборвался");
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(length);
    STATE.lock().unwrap().seen = crate::hotkeys::now_ms();
    let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    let lang = shared.cfg().source_lang();
    match (method.as_str(), path) {
        ("GET", "/hello") => {
            let reading = crate::interface::reading_on();
            let journal = crate::journal::on();
            let body = serde_json::json!({ "app": "Manga Local", "version": env!("CARGO_PKG_VERSION"), "reading": reading, "journal": journal, "lang": lang.name_ru() });
            reply(&mut s, "200 OK", &body.to_string()).await
        }
        ("POST", "/page") => {
            let view: View = serde_json::from_slice(&body)?;
            crate::journal::set_page_info(&view.url, &view.title);
            let want: Vec<String> = {
                let st = STATE.lock().unwrap();
                // Pictures ahead only while the reading mode is on.
                let ahead: &[Img] = if crate::interface::reading_on() { &view.ahead } else { &[] };
                view.images.iter().chain(ahead.iter()).filter(|i| !st.images.iter().any(|(id, _)| id == &i.id)).map(|i| i.id.clone()).collect()
            };
            {
                let mut st = STATE.lock().unwrap();
                // Not reading: only the pictures on screen stay in memory (the
                // extension describes the page every few seconds and sends them again).
                if !crate::interface::reading_on() {
                    let shown: HashSet<&str> = view.images.iter().map(|i| i.id.as_str()).collect();
                    st.images.retain(|(id, _)| shown.contains(id.as_str()));
                    let left: HashSet<String> = st.images.iter().map(|(id, _)| id.clone()).collect();
                    st.lines.retain(|(id, _, _), _| left.contains(id));
                    st.ahead_done.retain(|(id, _)| left.contains(id));
                }
                st.view = Some((crate::hotkeys::now_ms(), view));
            }
            reply(&mut s, "200 OK", &serde_json::json!({ "want": want }).to_string()).await
        }
        ("POST", "/image") => {
            let id = query.split('&').find_map(|kv| kv.strip_prefix("id=")).unwrap_or("").to_string();
            anyhow::ensure!(!id.is_empty() && id.len() < 100, "нет id картинки");
            let frame = tokio::task::spawn_blocking(move || manga_pages::chapter::decode(&body)).await?.map_err(anyhow::Error::msg)?;
            tracing::info!(target: "link", "картинка со страницы {}×{}", frame.width, frame.height);
            keep_image(&id, frame);
            reply(&mut s, "200 OK", "{}").await?;
            if crate::interface::reading_on() {
                let (sh, tr, o) = (shared.clone(), tr.clone(), ocr.clone());
                tokio::spawn(async move { read_ahead(&sh, &tr, &o, &id, lang).await });
            }
            Ok(())
        }
        ("POST", "/mark") => {
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
            let note = v.get("note").and_then(|n| n.as_str()).unwrap_or("").chars().take(500).collect::<String>();
            let journal = crate::journal::on();
            crate::journal::mark_error(&note);
            shared.ui(|u| u.notice(if journal { "Ошибка на странице отмечена в журнале" } else { "Ошибка не записана: включите «Журнал для разработчика» в настройках" }));
            reply(&mut s, "200 OK", &serde_json::json!({ "journal": journal }).to_string()).await
        }
        _ => reply(&mut s, "404 Not Found", "{}").await,
    }
}

fn keep_image(id: &str, frame: Frame) {
    let mut st = STATE.lock().unwrap();
    st.images.retain(|(i, _)| i != id);
    st.images.push((id.to_string(), Arc::new(frame)));
    let mpx = |st: &State| st.images.iter().map(|(_, f)| f.width as u64 * f.height as u64).sum::<u64>() / 1_000_000;
    while st.images.len() > 1 && mpx(&st) > KEEP_MPX {
        let (gone, _) = st.images.remove(0);
        st.lines.retain(|(i, _, _), _| i != &gone);
        st.ahead_done.retain(|(i, _)| i != &gone);
    }
}

fn image(id: &str) -> Option<Arc<Frame>> {
    STATE.lock().unwrap().images.iter().find(|(i, _)| i == id).map(|(_, f)| f.clone())
}

/// OCR lines of piece `k` of picture `id` (picture px), read once.
async fn piece_lines(ocr: &Ocr, id: &str, frame: &Frame, k: usize, lang: Lang) -> anyhow::Result<Vec<OcrLine>> {
    let key = (id.to_string(), k, lang);
    if let Some(l) = STATE.lock().unwrap().lines.get(&key) {
        return Ok(l.clone());
    }
    let pieces = manga_pages::slice::pieces(frame.height, PIECE, OVERLAP);
    let (y0, y1) = pieces[k];
    let part = if pieces.len() == 1 { frame.clone() } else { frame.crop(0, y0, frame.width, y1 - y0) };
    let up = if frame.width < 2000 { 2 } else { 1 };
    let mut lines = ocr.recognize(part, up, lang).await?;
    for l in lines.iter_mut() {
        l.rect = l.rect.offset(0.0, y0 as f32);
    }
    lines.retain(|l| manga_pages::slice::owns(&pieces, k, l.rect.y + l.rect.h / 2.0));
    STATE.lock().unwrap().lines.insert(key, lines.clone());
    Ok(lines)
}

/// F8 on a browser page the extension described: the text lines read from
/// the page's own pictures, placed where the screen shows them (coordinates
/// of the frame captured at `area`). None: no usable description (the page
/// moved since, the window does not match, a picture is missing) — then the
/// screen is read as usual.
pub async fn screen_lines(area: PxRect, screen: &Frame, lang: Lang, ocr: &Ocr) -> Option<Vec<OcrLine>> {
    let view = {
        let st = STATE.lock().unwrap();
        let (at, v) = st.view.as_ref()?;
        // Described after the last scrolling, so it is what the screen shows.
        if *at < crate::hotkeys::last_scroll_ms() {
            return None;
        }
        v.clone()
    };
    if view.images.is_empty() || view.dpr <= 0.0 {
        return None;
    }
    // Where the page's viewport is on screen.
    let (vw, vh) = (view.inner[0] * view.dpr, view.inner[1] * view.dpr);
    if (vw - area.w as f32).abs() > 24.0 {
        return None;
    }
    let (ox, oy) = if (vh - area.h as f32).abs() <= 24.0 {
        (0.0, 0.0)
    } else if vh < area.h as f32 {
        (0.0, area.h as f32 - vh) // tabs and the address bar above
    } else {
        return None;
    };
    let mut out = Vec::new();
    let mut used = 0;
    for img in &view.images {
        let [x, y, w, h] = img.rect;
        let (nw, nh) = (img.natural[0] as f32, img.natural[1] as f32);
        if w < 1.0 || h < 1.0 {
            continue;
        }
        // Still loading: its text would be missed, so the screen is read.
        if nw < 1.0 || nh < 1.0 {
            return None;
        }
        // The site shows a small picture enlarged: the screen is as good.
        if nw < w * view.dpr * 0.9 {
            return None;
        }
        let frame = image(&img.id)?;
        if frame.width as f32 != nw || frame.height as f32 != nh {
            return None;
        }
        // The part of the picture inside the viewport, picture px.
        let vis0 = ((-y).max(0.0) / h * nh).max(0.0);
        let vis1 = ((view.inner[1] - y).min(h) / h * nh).min(nh);
        if vis1 <= vis0 {
            continue;
        }
        let pieces = manga_pages::slice::pieces(frame.height, PIECE, OVERLAP);
        let (sx, sy) = (w * view.dpr / nw, h * view.dpr / nh);
        let (bx, by) = (ox + x * view.dpr, oy + y * view.dpr);
        // The screen must show this picture at this place (the page could
        // have moved after the extension described it).
        if !same_picture(&frame, screen, vis0, vis1, bx, by, sx, sy) {
            tracing::info!(target: "link", "картинка страницы не совпала с экраном: читаю экран");
            return None;
        }
        for (k, &(p0, p1)) in pieces.iter().enumerate() {
            if (p1 as f32) < vis0 || (p0 as f32) > vis1 {
                continue;
            }
            let lines = piece_lines(ocr, &img.id, &frame, k, lang).await.ok()?;
            for l in lines {
                let cy = l.rect.y + l.rect.h / 2.0;
                if cy < vis0 || cy > vis1 {
                    continue;
                }
                let rect = manga_core::image::Rect { x: bx + l.rect.x * sx, y: by + l.rect.y * sy, w: l.rect.w * sx, h: l.rect.h * sy };
                out.push(OcrLine { text: l.text, rect });
            }
        }
        used += 1;
    }
    if used == 0 {
        return None;
    }
    tracing::info!(target: "link", "текст взят с {used} картинок страницы в полном качестве: {} строк", out.len());
    Some(out)
}

/// Brightness of a picture pixel (BGRA).
fn luma_at(f: &Frame, x: f32, y: f32) -> Option<f32> {
    let (x, y) = (x as i64, y as i64);
    if x < 0 || y < 0 || x >= f.width as i64 || y >= f.height as i64 {
        return None;
    }
    let i = ((y as u32 * f.width + x as u32) * 4) as usize;
    let p = &f.bgra[i..i + 4];
    Some(manga_core::image::luma(p[0], p[1], p[2]) as f32)
}

/// The visible rows `vis0..vis1` of `pic`, scaled by (sx, sy) and placed at
/// (bx, by) of `screen`, look like the screen there (13 × 13 points compared).
#[allow(clippy::too_many_arguments)]
fn same_picture(pic: &Frame, screen: &Frame, vis0: f32, vis1: f32, bx: f32, by: f32, sx: f32, sy: f32) -> bool {
    let (mut diff, mut n, mut spread) = (0.0f32, 0u32, (255.0f32, 0.0f32));
    for i in 1..14 {
        for j in 1..14 {
            let px = pic.width as f32 * i as f32 / 14.0;
            let py = vis0 + (vis1 - vis0) * j as f32 / 14.0;
            let (Some(a), Some(b)) = (luma_at(pic, px, py), luma_at(screen, bx + px * sx, by + py * sy)) else { continue };
            diff += (a - b).abs();
            n += 1;
            spread = (spread.0.min(a), spread.1.max(a));
        }
    }
    // Too few points on screen, or a flat picture (white page): nothing to tell by.
    n >= 40 && (diff / (n as f32) < 28.0 || spread.1 - spread.0 < 30.0)
}

/// Read a picture ahead and translate its text into the cache, piece by
/// piece, while the reading mode is on; the page on screen always goes first.
async fn read_ahead(shared: &Arc<Shared>, tr: &Arc<Translator>, ocr: &Ocr, id: &str, lang: Lang) {
    if !STATE.lock().unwrap().ahead_done.insert((id.to_string(), lang)) {
        return;
    }
    let Some(frame) = image(id) else { return };
    let pieces = manga_pages::slice::pieces(frame.height, PIECE, OVERLAP);
    let cfg = shared.cfg();
    let preset = cfg.active_model().clone();
    let game = cfg.general.game_title.clone();
    for k in 0..pieces.len() {
        if !crate::interface::reading_on() {
            return;
        }
        let Ok(lines) = piece_lines(ocr, id, &frame, k, lang).await else { return };
        if lines.is_empty() {
            continue;
        }
        let (y0, y1) = pieces[k];
        let part = frame.crop(0, y0, frame.width, y1 - y0);
        let local: Vec<OcrLine> = lines.iter().map(|l| OcrLine { text: l.text.clone(), rect: l.rect.offset(0.0, -(y0 as f32)) }).collect();
        let ja = crate::jaocr::engine(shared).await;
        let found = manga_core::bubbles::find(&part);
        let items = crate::interface::page_items(&local, lang, tr, &part, ocr, found, ja).await;
        let mut done = HashMap::new();
        let (talk, sfx) = crate::interface::split_unique(&items, tr, &preset.id, &mut done);
        for chunk in talk.chunks(6) {
            // The page on screen first.
            while shared.ui.lock().unwrap().interface.as_ref().is_some_and(|v| v.pending > 0) {
                if !crate::interface::reading_on() {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            crate::interface::translate_texts(shared, tr, &preset, &game, chunk.to_vec(), manga_core::prompt::TextKind::Dialogue, &[], crate::interface::reading_on, |_, _| {}).await;
        }
        if !sfx.is_empty() && cfg.interface.sfx != "keep" {
            crate::interface::translate_texts(shared, tr, &preset, &game, sfx, manga_core::prompt::TextKind::Interface, &[], crate::interface::reading_on, |_, _| {}).await;
        }
    }
    tracing::info!(target: "link", "картинка прочитана и переведена заранее ({} кусков)", pieces.len());
}
