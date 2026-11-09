//! Journal for the developer («Собирать журнал для разработчика»): every page
//! translated with F8 is kept with its picture, the recognised lines, the
//! bubbles with their translations and the timings; doubtful places are
//! marked by themselves (nothing read, no translation, letters of the
//! original left, the model refused, slow). Ctrl+F11 marks the page as badly
//! translated. «Собрать отчёт» packs it all with the logs into one ZIP that the
//! reader sends to the developer: nothing leaves the computer by itself.
//!
//! On disk: `<data>\journal\<date time>\` — `0001.jpg`…, `journal.jsonl` (one
//! page per line) and `index.md` (the same, readable). At most ~300 MB: the
//! oldest sessions go first.

use crate::state::Shared;
use manga_core::image::{Frame, Rect};
use manga_core::lang::Lang;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

static ON: AtomicBool = AtomicBool::new(false);
/// This run's folder (made at the first page).
static SESSION: OnceLock<PathBuf> = OnceLock::new();
static PAGES: AtomicU32 = AtomicU32::new(0);
/// Pages being translated, by view generation.
static PENDING: Mutex<Option<HashMap<u64, Entry>>> = Mutex::new(None);
/// What the browser extension said about the page in front (address, title).
static PAGE_INFO: Mutex<Option<(String, String)>> = Mutex::new(None);

const LIMIT_BYTES: u64 = 300 * 1024 * 1024;

pub fn set_on(on: bool) {
    ON.store(on, Ordering::SeqCst);
}

pub fn on() -> bool {
    ON.load(Ordering::SeqCst)
}

/// The page the browser shows now (from the extension).
pub fn set_page_info(url: &str, title: &str) {
    *PAGE_INFO.lock().unwrap() = Some((url.to_string(), title.to_string()));
}

pub fn dir(shared: &Shared) -> PathBuf {
    shared.paths.data.join("journal")
}

struct Entry {
    started: Instant,
    frame: Arc<Frame>,
    lang: Lang,
    engine: &'static str,
    source: String,
    ocr_ms: u128,
    outlines: usize,
    lines: Vec<(String, Rect)>,
    bubbles: Vec<Bubble>,
}

struct Bubble {
    src: String,
    dst: Option<String>,
    rect: Rect,
    sfx: bool,
}

/// A page was read: keep it until its translation ends.
#[allow(clippy::too_many_arguments)]
/// `area`: where the picture is on screen (bubble rectangles are in screen pixels).
pub fn begin(gen: u64, frame: &Frame, lang: Lang, source: &str, ocr_ms: u128, outlines: usize, lines: &[crate::ocr::OcrLine], items: &[crate::state::InterfaceItem], area: crate::capture::PxRect) {
    if !on() {
        return;
    }
    let engine = if crate::neural::active(lang) { "нейросеть" } else { "Windows" };
    let e = Entry {
        started: Instant::now(),
        frame: Arc::new(frame.clone()),
        lang,
        engine,
        source: source.to_string(),
        ocr_ms,
        outlines,
        lines: lines.iter().map(|l| (l.text.clone(), l.rect)).collect(),
        bubbles: items.iter().map(|it| Bubble { src: it.src.clone(), dst: it.dst.clone(), rect: it.rect.offset(-area.x as f32, -area.y as f32), sfx: it.sfx }).collect(),
    };
    let mut p = PENDING.lock().unwrap();
    let map = p.get_or_insert_with(HashMap::new);
    // Pages that never finished (the reader scrolled on) are not waited for forever.
    if map.len() > 8 {
        let oldest = map.iter().min_by_key(|(_, e)| e.started).map(|(g, _)| *g);
        if let Some(g) = oldest {
            map.remove(&g);
        }
    }
    map.insert(gen, e);
}

/// A bubble of page `gen` got its translation.
pub fn result(gen: u64, src: &str, dst: &str) {
    if !on() {
        return;
    }
    if let Some(e) = PENDING.lock().unwrap().as_mut().and_then(|m| m.get_mut(&gen)) {
        for b in e.bubbles.iter_mut().filter(|b| b.src == src) {
            b.dst = Some(dst.to_string());
        }
    }
}

/// Page `gen` is done (`left`: the reader scrolled on before the end): written in the background.
pub fn finish(shared: &Arc<Shared>, gen: u64, left: bool) {
    let Some(e) = PENDING.lock().unwrap().as_mut().and_then(|m| m.remove(&gen)) else { return };
    let shared = shared.clone();
    std::thread::spawn(move || {
        if let Err(err) = write(&shared, e, left) {
            tracing::warn!("журнал: {err}");
        }
    });
}

/// Ctrl+F11: the last page of the journal is marked as badly translated.
pub fn mark_bad() {
    if !on() {
        return;
    }
    let Some(dir) = SESSION.get() else { return };
    let n = PAGES.load(Ordering::SeqCst);
    if n == 0 {
        return;
    }
    let line = serde_json::json!({ "page": n, "mark": "плохой перевод (Ctrl+F11)" });
    let _ = append(&dir.join("journal.jsonl"), &format!("{line}\n"));
    let _ = append(&dir.join("index.md"), &format!("\n**Страница {n:04}: читатель нажал «плохой перевод» (Ctrl+F11)**\n"));
}

/// The extension's «здесь ошибка» with what the reader wrote about it.
pub fn mark_error(note: &str) {
    if !on() {
        return;
    }
    let Some(dir) = SESSION.get() else { return };
    let n = PAGES.load(Ordering::SeqCst);
    let line = serde_json::json!({ "page": n, "mark": "ошибка (из браузера)", "note": note });
    let _ = append(&dir.join("journal.jsonl"), &format!("{line}\n"));
    let _ = append(&dir.join("index.md"), &format!("\n**Страница {n:04}: читатель отметил ошибку** {note}\n"));
}

fn append(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::OpenOptions::new().create(true).append(true).open(path)?.write_all(text.as_bytes())
}

fn session_dir(shared: &Shared) -> std::io::Result<&'static PathBuf> {
    if let Some(d) = SESSION.get() {
        return Ok(d);
    }
    let (date, time) = crate::win::local_date_time();
    let d = dir(shared).join(format!("{date}_{}", time.replace(':', "-")));
    std::fs::create_dir_all(&d)?;
    prune(&dir(shared), &d);
    Ok(SESSION.get_or_init(|| d))
}

/// Letters of the original language left in a translation.
fn has_original_letters(s: &str, lang: Lang) -> bool {
    s.chars().any(|c| match lang {
        Lang::Ko => ('\u{AC00}'..='\u{D7A3}').contains(&c) || ('\u{3131}'..='\u{318E}').contains(&c),
        Lang::Ja => ('\u{3040}'..='\u{30FF}').contains(&c) || ('\u{4E00}'..='\u{9FFF}').contains(&c),
        Lang::Zh => ('\u{4E00}'..='\u{9FFF}').contains(&c),
        Lang::En => c.is_ascii_alphabetic() && !s.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)),
    })
}

/// The doubtful places of a page, in plain words.
fn flags(e: &Entry, translate_ms: u128, left: bool) -> Vec<String> {
    let mut f = Vec::new();
    if e.bubbles.is_empty() && e.outlines >= 2 {
        f.push(format!("пузырей на картинке {}, текст не прочитан", e.outlines));
    }
    if e.ocr_ms > 2500 {
        f.push(format!("распознавание долго: {} мс", e.ocr_ms));
    }
    if translate_ms > 12_000 {
        f.push(format!("перевод долго: {:.1} с", translate_ms as f64 / 1000.0));
    }
    if left {
        f.push("страницу пролистали до конца перевода".into());
    }
    for b in &e.bubbles {
        match &b.dst {
            None => f.push(format!("нет перевода: «{}»", b.src)),
            Some(d) if d == &b.src => f.push(format!("перевод не удался: «{}»", b.src)),
            Some(d) if manga_core::prompt::is_refusal(d) => f.push(format!("нейросеть отказалась: «{}» → «{d}»", b.src)),
            Some(d) if has_original_letters(d, e.lang) => f.push(format!("в переводе остался оригинал: «{}» → «{d}»", b.src)),
            _ => {}
        }
        if !b.sfx && b.src.chars().filter(|c| c.is_alphanumeric()).count() <= 1 {
            f.push(format!("подозрительно короткий текст: «{}»", b.src));
        }
    }
    f
}

fn write(shared: &Shared, e: Entry, left: bool) -> anyhow::Result<()> {
    let dir = session_dir(shared)?;
    let n = PAGES.fetch_add(1, Ordering::SeqCst) + 1;
    // A long session: older ones leave while the journal is over its limit.
    if n % 50 == 0 {
        prune(&self::dir(shared), dir);
    }
    let translate_ms = e.started.elapsed().as_millis();
    let flags = flags(&e, translate_ms, left);
    let jpg = manga_pages::chapter::encode(&e.frame, true).map_err(anyhow::Error::msg)?;
    std::fs::write(dir.join(format!("{n:04}.jpg")), jpg)?;
    let (_, time) = crate::win::local_date_time();
    let info = PAGE_INFO.lock().unwrap().clone();
    let r = |r: &Rect| [r.x as i32, r.y as i32, r.w as i32, r.h as i32];
    let line = serde_json::json!({
        "page": n,
        "time": time,
        "lang": e.lang.code(),
        "ocr": e.engine,
        "source": e.source,
        "url": info.as_ref().map(|i| i.0.clone()),
        "title": info.as_ref().map(|i| i.1.clone()),
        "size": [e.frame.width, e.frame.height],
        "ocr_ms": e.ocr_ms,
        "translate_ms": translate_ms,
        "outlines": e.outlines,
        "lines": e.lines.iter().map(|(t, rc)| serde_json::json!({ "text": t, "rect": r(rc) })).collect::<Vec<_>>(),
        "bubbles": e.bubbles.iter().map(|b| serde_json::json!({ "src": b.src, "dst": b.dst, "rect": r(&b.rect), "sfx": b.sfx })).collect::<Vec<_>>(),
        "flags": flags,
    });
    append(&dir.join("journal.jsonl"), &format!("{line}\n"))?;
    let mut md = format!(
        "\n## {n:04} · {time} · {} · распознавание {} {} мс, перевод {:.1} с{}\n\n![]({n:04}.jpg)\n\n",
        e.lang.name_ru(),
        e.engine,
        e.ocr_ms,
        translate_ms as f64 / 1000.0,
        info.as_ref().map(|i| format!(" · {}", i.1)).unwrap_or_default()
    );
    for b in &e.bubbles {
        md.push_str(&format!("- {}«{}» → «{}»\n", if b.sfx { "звук: " } else { "" }, b.src, b.dst.as_deref().unwrap_or("—")));
    }
    for f in &flags {
        md.push_str(&format!("- ⚠ {f}\n"));
    }
    append(&dir.join("index.md"), &md)?;
    shared.ui(|u| u.journal_pages = n);
    Ok(())
}

fn size_of(dir: &Path) -> u64 {
    std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| if e.path().is_dir() { size_of(&e.path()) } else { e.metadata().map_or(0, |m| m.len()) }).sum()).unwrap_or(0)
}

/// Oldest sessions out while the journal is over its limit (the current one stays).
fn prune(root: &Path, keep: &Path) {
    let mut sessions: Vec<PathBuf> = std::fs::read_dir(root).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && p != keep).collect()).unwrap_or_default();
    sessions.sort();
    let mut total = size_of(root);
    for s in sessions {
        if total <= LIMIT_BYTES {
            break;
        }
        total = total.saturating_sub(size_of(&s));
        let _ = std::fs::remove_dir_all(&s);
    }
}

/// How many pages the journal holds and its size in MB (for the panel; the
/// panel asks every frame, the disk is looked at every few seconds).
pub fn summary(shared: &Shared) -> (usize, u64) {
    static LAST: Mutex<Option<(Instant, (usize, u64))>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap();
    if let Some((at, v)) = *last {
        if at.elapsed().as_secs() < 3 {
            return v;
        }
    }
    let v = count(shared);
    *last = Some((Instant::now(), v));
    v
}

fn count(shared: &Shared) -> (usize, u64) {
    let root = dir(shared);
    let pages = std::fs::read_dir(&root)
        .map(|rd| rd.flatten().filter(|e| e.path().is_dir()).map(|e| std::fs::read_dir(e.path()).map(|r| r.flatten().filter(|f| f.path().extension().is_some_and(|x| x == "jpg")).count()).unwrap_or(0)).sum())
        .unwrap_or(0);
    (pages, size_of(&root) / 1_000_000)
}

/// Delete the whole journal.
pub fn clear(shared: &Shared) {
    let root = dir(shared);
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            if Some(&e.path()) != SESSION.get() {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    if let Some(s) = SESSION.get() {
        if let Ok(rd) = std::fs::read_dir(s) {
            for e in rd.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        PAGES.store(0, Ordering::SeqCst);
    }
}

/// «Собрать отчёт»: the journal, the logs and the settings in one ZIP on the
/// desktop (or in the data folder). Returns its path.
pub fn report(shared: &Shared) -> anyhow::Result<PathBuf> {
    let (date, time) = crate::win::local_date_time();
    let name = format!("MangaLocal-отчёт-{date}_{}.zip", time.replace(':', "-"));
    let desktop = std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Desktop")).filter(|d| d.is_dir());
    let out = desktop.unwrap_or_else(|| shared.paths.data.clone()).join(name);
    let mut zip = manga_pages::chapter::Output::create(&out, true).map_err(anyhow::Error::msg)?;
    let add_dir = |zip: &mut manga_pages::chapter::Output, root: &Path, prefix: &str, filter: &dyn Fn(&Path) -> bool| -> anyhow::Result<()> {
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d)?.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if filter(&p) {
                    let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
                    zip.add(&format!("{prefix}/{rel}"), &std::fs::read(&p)?).map_err(anyhow::Error::msg)?;
                }
            }
        }
        Ok(())
    };
    let root = dir(shared);
    if root.is_dir() {
        add_dir(&mut zip, &root, "journal", &|_| true)?;
    }
    // The three newest logs.
    let mut logs: Vec<PathBuf> = std::fs::read_dir(shared.paths.logs()).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect()).unwrap_or_default();
    logs.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    let newest: Vec<PathBuf> = logs.into_iter().rev().take(3).collect();
    add_dir(&mut zip, &shared.paths.logs(), "logs", &|p| newest.iter().any(|n| n == p))?;
    if let Ok(cfg) = std::fs::read(shared.paths.config()) {
        zip.add("config.toml", &cfg).map_err(anyhow::Error::msg)?;
    }
    zip.finish().map_err(anyhow::Error::msg)?;
    Ok(out)
}
