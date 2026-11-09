//! State shared between the async workers and the egui UI thread.

use crate::config::Config;
use crate::paths::Paths;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use tokio::sync::mpsc::UnboundedSender;
use manga_core::image::Rect;

#[derive(Debug, Clone, PartialEq)]
pub enum ServerStatus {
    Stopped,
    Starting(String),
    Ready { model: String },
    Failed(String),
}

impl ServerStatus {
    pub fn label(&self) -> String {
        match self {
            ServerStatus::Stopped => "остановлен".into(),
            ServerStatus::Starting(s) => format!("запуск: {s}"),
            ServerStatus::Ready { model } => format!("готов · {model}"),
            ServerStatus::Failed(e) => format!("ошибка: {e}"),
        }
    }
    pub fn is_ready(&self) -> bool {
        matches!(self, ServerStatus::Ready { .. })
    }
}

/// A still picture of the screen with the texts found on it.
#[derive(Debug, Clone)]
pub struct FreezeView {
    pub shot: std::sync::Arc<manga_core::image::Frame>,
    pub lines: Vec<FreezeLine>,
    pub selected: Option<usize>,
    /// Text recognition still running.
    pub busy: bool,
    /// Changes when the picture changes (texture reload in the UI).
    pub id: u64,
}

#[derive(Debug, Clone)]
pub struct FreezeLine {
    /// Screen pixels.
    pub rect: Rect,
    pub src: String,
    pub dst: Option<String>,
    pub pending: bool,
}

#[derive(Debug, Clone)]
pub enum Command {
    ScreenTranslate,
    ToggleOriginal,
    TogglePanel,
    ShowPanel,
    SwitchModel(String),
    /// Use a .gguf file the user chose instead of downloading the model.
    UseModelFile(std::path::PathBuf),
    RestartServer,
    SaveCorrection { src: String, dst: String },
    TestTranslate(String),
    ReloadGlossary,
    /// Translate a chapter: a folder (any picture in it), a ZIP or a CBZ.
    TranslateFiles(std::path::PathBuf),
    /// Stop the chapter translation after the current page.
    StopFiles,
    /// Add a character name to the title's dictionary (or the user's glossary).
    AddName { src: String, dst: String },
    /// «Не имя»: never offer this original again.
    IgnoreName(String),
    /// Download the manga-ocr model (vertical Japanese).
    DownloadJaOcr,
    /// Open the dictionary of the current title (created on first use).
    OpenTitleDict,
    /// Game profile or text language changed: reload dictionaries, check OCR.
    GameChanged,
    /// Exit the program (tray menu or the panel's «Выход»).
    Quit,
    /// Show a test label over the game for 10 s (is the overlay visible at all?).
    TestOverlay,
    /// Save the screen as a new screenshot test case (Ctrl+F9).
    SaveCase,
    /// Dictionaries for sharing: write the bundle to this file / merge one in.
    ExportDicts(std::path::PathBuf),
    ImportDicts(std::path::PathBuf),
    /// The last dialogue translation is bad: forget it and translate again (Ctrl+F11).
    BadTranslation,
    /// Freeze frame (Ctrl+F8): a still picture of the screen, click a text to translate it.
    Freeze,
    FreezeTranslate(usize),
    FreezeClose,
    /// Add a pair to the user's glossary (from the freeze frame).
    AddTerm { src: String, dst: String },
    /// Self-check: OCR, model, game window, overlay, capture, hotkeys → report file.
    Diagnose,
    /// Open a web page in the default browser.
    OpenUrl(String),
    /// The panel was closed with ✕: explain once where the program went.
    PanelClosedToTray,
    RunBench,
    SaveConfig,
    Open(std::path::PathBuf),
}

#[derive(Debug, Clone)]
pub struct InterfaceItem {
    /// Physical screen pixels.
    pub rect: Rect,
    pub src: String,
    pub dst: Option<String>,
    pub bg: [u8; 3],
    pub fg: [u8; 3],
    /// Busy background: draw a plate instead of a flat mask.
    pub busy: bool,
    /// Height of one OCR line of the original, px: the starting font size.
    pub line_h: f32,
    /// How many OCR lines the bubble had (1 = a single label).
    pub lines: usize,
    /// Picture laid under the translation (screen px): the letters covered
    /// with paper in a bubble, or the background filled in over artwork.
    pub patch: Option<Arc<manga_core::clean::Patch>>,
    /// A sound effect on the artwork («쾅!», «ドキドキ»): translated briefly on
    /// its own (or left alone), never mixed with the dialogue.
    pub sfx: bool,
}

#[derive(Debug, Clone, Default)]
pub struct InterfaceView {
    pub items: Vec<InterfaceItem>,
    pub pending: usize,
    pub generation: u64,
}

/// What the dialogue OCR saw last time (shown in the panel for troubleshooting).
#[derive(Debug, Clone)]
pub struct OcrDebug {
    pub frame: std::sync::Arc<manga_core::image::Frame>,
    pub text: String,
    pub ms: u128,
    pub id: u64,
}

#[derive(Debug, Clone)]
pub struct RecentLine {
    pub src: String,
    pub dst: String,
    pub model: String,
    pub ms: u128,
    pub cached: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub ocr_ms_avg: f32,
    pub llm_ms_avg: f32,
    pub translated: u64,
    pub cache_hits: u64,
}

impl Stats {
    pub fn add_ocr(&mut self, ms: u128) {
        self.ocr_ms_avg = ema(self.ocr_ms_avg, ms as f32);
    }
    pub fn add_llm(&mut self, ms: u128, cached: bool) {
        if cached {
            self.cache_hits += 1;
        } else {
            self.translated += 1;
            self.llm_ms_avg = ema(self.llm_ms_avg, ms as f32);
        }
    }
}

fn ema(prev: f32, x: f32) -> f32 {
    if prev == 0.0 {
        x
    } else {
        prev * 0.8 + x * 0.2
    }
}

#[derive(Debug, Default)]
pub struct UiState {
    pub interface: Option<InterfaceView>,
    pub show_original: bool,
    /// Freeze frame being shown (Ctrl+F8).
    pub freeze: Option<FreezeView>,
    /// Hotkeys that could not be registered (for the self-check).
    pub hotkey_problems: Vec<String>,
    /// The panel was hidden only so it is not in a screenshot (F8, region,
    /// freeze frame): it comes back when that action is over. Only ✕ or
    /// minimising sends the panel to the tray.
    pub panel_return: bool,
    pub panel_open: bool,
    /// Bring the panel to the front on the next frames (it can open behind the game).
    pub focus_panel: bool,
    pub recent: VecDeque<RecentLine>,
    pub stats: Stats,
    pub notices: VecDeque<String>,
    pub server: Option<ServerStatus>,
    pub bench_progress: Option<String>,
    /// Chapter files being translated: what is happening now.
    pub files_progress: Option<String>,
    /// Vertical Japanese (manga-ocr): download progress or a problem.
    pub ja_ocr_status: Option<String>,
    /// Character names noticed in translations, offered for the title's dictionary.
    pub name_offers: Vec<(String, String)>,
    /// The page looks like another language than the chosen one: (chosen, seen),
    /// offered in the panel. `lang_declined`: pairs the reader said no to.
    pub lang_offer: Option<(manga_core::lang::Lang, manga_core::lang::Lang)>,
    pub lang_declined: Vec<(manga_core::lang::Lang, manga_core::lang::Lang)>,
    pub ocr_debug: Option<OcrDebug>,
    /// Windows OCR availability (None = not checked yet).
    pub ocr_status: Option<Result<(), String>>,
    /// (glossary terms, fix rules)
    pub dict_counts: (usize, usize),
    /// Result of the panel's "check translation" field.
    pub test_result: Option<String>,
    /// Until when the test label is shown over the screen.
    pub test_overlay_until: Option<std::time::Instant>,
}

impl UiState {
    /// Offer to switch the language of the text (once for each pair). True
    /// when this is a new offer.
    pub fn offer_language(&mut self, chosen: manga_core::lang::Lang, seen: manga_core::lang::Lang) -> bool {
        if self.lang_offer == Some((chosen, seen)) || self.lang_declined.contains(&(chosen, seen)) {
            return false;
        }
        self.lang_offer = Some((chosen, seen));
        self.notice(format!("На странице, похоже, {} текст, а выбран {}. Переключить можно на главной странице панели", seen.name_ru(), chosen.name_ru()));
        true
    }

    /// Offer character names noticed in translations (once each).
    pub fn offer_names(&mut self, pairs: Vec<(String, String)>) {
        for (src, dst) in pairs {
            if self.name_offers.iter().any(|(s, _)| *s == src) {
                continue;
            }
            self.notice(format!("Похоже на имя героя: {src} → {dst}. Добавить в словарь тайтла — раздел «Словари»"));
            self.name_offers.push((src, dst));
        }
    }

    /// Hide the panel for a screenshot; returns whether it was open.
    pub fn hide_panel_for_shot(&mut self) -> bool {
        let was = self.panel_open;
        if was {
            self.panel_open = false;
            self.panel_return = true;
        }
        was
    }

    /// The screenshot action is over: show the panel again if it hid for it.
    pub fn restore_panel(&mut self) {
        if self.panel_return {
            self.panel_return = false;
            self.panel_open = true;
        }
    }
}

impl UiState {
    pub fn notice(&mut self, s: impl Into<String>) {
        let s = s.into();
        tracing::warn!("{s}");
        self.notices.push_back(s);
        while self.notices.len() > 8 {
            self.notices.pop_front();
        }
    }
    pub fn push_recent(&mut self, r: RecentLine) {
        if self.recent.front().map_or(false, |x| x.src == r.src) {
            self.recent.pop_front();
        }
        self.recent.push_front(r);
        self.recent.truncate(60);
    }
}

pub struct Shared {
    pub paths: Paths,
    pub cfg: RwLock<Config>,
    pub ui: Mutex<UiState>,
    pub cmd: UnboundedSender<Command>,
    pub egui: OnceLock<eframe::egui::Context>,
    pub interface_gen: AtomicU64,
    pub reset_watcher: AtomicBool,
    /// Screen rectangles (physical px, x/y/w/h) the overlay painted in its last
    /// frame. Captures blank them out (see `capture::capture_clean`).
    pub overlay_px: Mutex<Vec<[i32; 4]>>,
}

impl Shared {
    pub fn repaint(&self) {
        if let Some(c) = self.egui.get() {
            c.request_repaint();
        }
    }
    pub fn send(&self, c: Command) {
        let _ = self.cmd.send(c);
    }
    pub fn cfg(&self) -> Config {
        self.cfg.read().unwrap().clone()
    }
    pub fn ui<R>(&self, f: impl FnOnce(&mut UiState) -> R) -> R {
        let r = f(&mut self.ui.lock().unwrap());
        self.repaint();
        r
    }
    pub fn save_config(&self) {
        let cfg = self.cfg();
        if let Err(e) = cfg.save(&self.paths) {
            self.ui(|u| u.notice(format!("Не удалось сохранить настройки: {e}")));
        }
    }
}
