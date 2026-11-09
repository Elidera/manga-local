//! User configuration (`config.toml` in the data dir).
//! Every field has a default, so a partial or old config file still loads.

use crate::paths::Paths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub llm: Llm,
    pub models: Vec<ModelPreset>,
    pub dialogue: Dialogue,
    pub interface: Interface,
    pub ocr: Ocr,
    pub hotkeys: Hotkeys,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// Where the manga is read (see [`PROFILES`]): "browser", "screen" or "custom".
    /// For a known profile `window_match` is filled from it.
    pub profile: String,
    /// Language of the text: "ko", "ja", "zh" (or "en").
    pub source_lang: String,
    /// Title of the manga, shown to instruct models in the prompt (may be empty).
    pub game_title: String,
    /// Substrings of the reader window title separated by `|`. Capture pauses while
    /// another window is in the foreground. Empty (and no window class in the
    /// profile) = the whole screen, always active.
    pub window_match: String,
    /// Kept for old config files; translation now always waits for the game window.
    pub pause_when_unfocused: bool,
    /// Show the overlay and the panel on screenshots (Win+Shift+S, OBS).
    /// Off by default: then they can never get into our own OCR.
    pub visible_in_screenshots: bool,
    /// The "I'm still here, in the tray" notice was shown after the first ✕.
    pub tray_hint_shown: bool,
    /// The comic font replaced the old default «Segoe UI» once (older configs).
    pub comic_font_set: bool,
    /// Accent colour of the panel (R, G, B); бирюза by default.
    pub accent: [u8; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Llm {
    /// "managed": we start and watch llama-server ourselves.
    /// "external": connect to an already running OpenAI-compatible server
    /// (another llama-server, LM Studio, Ollama, a second PC on the LAN).
    pub mode: String,
    /// Relative to the install dir or absolute.
    pub server_exe: String,
    pub port: u16,
    pub external_url: String,
    pub active_model: String,
    /// Layers offloaded to GPU (-ngl). 0 = CPU only. 99 = everything.
    pub gpu_layers: i32,
    /// CPU threads for inference; 0 = auto (half of logical cores, min 2).
    pub threads: u32,
    pub ctx_size: u32,
    /// Run llama-server with below-normal priority so the game keeps its FPS.
    pub low_priority: bool,
    pub extra_args: Vec<String>,
    pub request_timeout_s: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelPreset {
    pub id: String,
    pub name: String,
    /// "hymt" or "instruct"
    pub prompt: String,
    /// HuggingFace reference for llama-server `-hf` (downloaded on first start).
    pub hf: String,
    /// Local .gguf path; filled automatically after the first download.
    pub path: String,
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub repeat_penalty: f32,
    pub max_tokens: u32,
    /// Ask the chat template to disable "thinking" (Qwen3.x).
    pub no_think: bool,
    /// Optional custom prompt template (see manga_core::prompt::render_custom).
    pub template: String,
    /// Approximate size for the UI.
    pub size_gb: f32,
}

impl Default for ModelPreset {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            prompt: "instruct".into(),
            hf: String::new(),
            path: String::new(),
            temperature: 0.3,
            top_p: 0.8,
            top_k: 20,
            repeat_penalty: 1.05,
            max_tokens: 256,
            no_think: false,
            template: String::new(),
            size_gb: 0.0,
        }
    }
}

/// How translations look and read. Kept under `[dialogue]` so older configs
/// load (the subtitle mode it once belonged to is gone).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Dialogue {
    pub context_lines: usize,
    pub show_original: bool,
    /// Font of the translation (one of the bundled comic fonts or any Windows font).
    pub overlay_font: String,
    /// Russian free-text style wish passed to instruct models.
    pub style_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Interface {
    /// "replace", "backdrop" or "list"
    pub mode: String,
    /// 0 = auto (2× below 4K).
    pub upscale: u32,
    pub font_scale: f32,
    pub max_lines: usize,
    /// Reading mode: hide while the page scrolls, translate again when it stops.
    pub auto_hide: bool,
    /// The mouse over a bubble takes our translation away from it, so the
    /// original under it can be read.
    pub hover_original: bool,
    /// Show the model the last bubbles of the previous page (who speaks, names, tone).
    pub page_context: bool,
    /// Japanese: bubbles found by the picture are read by manga-ocr (vertical
    /// columns, handwriting). Needs its model (~441 MB, downloaded on demand).
    pub vertical_ja: bool,
    /// Sound effects on the artwork: "translate" (briefly, apart from the
    /// dialogue) or "keep" (left as drawn).
    pub sfx: String,
    /// Text recognition: "windows" (built in) or "neural" (PaddleOCR models,
    /// downloaded on demand; reads titles, outlined and slanted text).
    pub ocr_engine: String,
    /// Keep every translated page for the developer (see `journal.rs`).
    pub journal: bool,
    /// The neural OCR on the video card (DirectML); off = the processor.
    pub ocr_gpu: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Ocr {
    /// BCP-47 tag for Windows OCR. Requires the Windows language pack's OCR
    /// component (Settings → Time & Language → Language → Add "中文(简体)").
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub screen_translate: String,
    pub show_original: String,
    pub panel: String,
    /// Save the screen as a test case for the developer.
    pub save_case: String,
    /// Freeze frame: still picture, click a text to translate it.
    pub freeze: String,
    /// The last translation is bad: forget it and translate again.
    pub bad_translation: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            llm: Llm::default(),
            models: default_models(),
            dialogue: Dialogue::default(),
            interface: Interface::default(),
            ocr: Ocr::default(),
            hotkeys: Hotkeys::default(),
        }
    }
}

impl Default for General {
    fn default() -> Self {
        Self {
            profile: "browser".into(),
            source_lang: "ko".into(),
            game_title: String::new(),
            window_match: String::new(),
            pause_when_unfocused: true,
            visible_in_screenshots: false,
            tray_hint_shown: false,
            comic_font_set: false,
            accent: [229, 0, 232],
        }
    }
}

/// Where the manga is read: how to recognise that window and which
/// dictionaries to load (glossary files named `<id>_*.tsv` are loaded only for
/// that profile).
pub struct GameProfile {
    pub id: &'static str,
    pub name: &'static str,
    pub title: &'static str,
    pub window_match: &'static str,
    /// Languages offered for this profile, the first is the default.
    pub langs: &'static [&'static str],
    /// Window classes (prefixes, separated by `|`) of the reader window.
    /// Chromium browsers (Chrome, Edge, Yandex, Opera, Brave) are
    /// `Chrome_WidgetWin_1`, Firefox is `MozillaWindowClass`.
    pub window_class: &'static str,
}

/// Languages in the order the user reads them most: Korean and Japanese first.
pub const LANGS: &[&str] = &["ko", "ja", "zh", "en"];

pub const PROFILES: &[GameProfile] = &[
    GameProfile { id: "browser", name: "Браузер (манга на сайтах)", title: "", window_match: "", langs: LANGS, window_class: "Chrome_WidgetWin|MozillaWindowClass" },
    GameProfile { id: "screen", name: "Весь экран (любая программа)", title: "", window_match: "", langs: LANGS, window_class: "" },
    GameProfile { id: "custom", name: "Своя программа (по заголовку окна)", title: "", window_match: "", langs: LANGS, window_class: "" },
];

pub fn profile(id: &str) -> &'static GameProfile {
    PROFILES.iter().find(|p| p.id == id).unwrap_or(&PROFILES[0])
}

impl Default for Llm {
    fn default() -> Self {
        Self {
            mode: "managed".into(),
            server_exe: "llama\\llama-server.exe".into(),
            port: 8848,
            external_url: "http://127.0.0.1:8080".into(),
            active_model: "hymt2-1.8b".into(),
            gpu_layers: 99,
            threads: 0,
            ctx_size: 4096,
            low_priority: true,
            extra_args: vec![],
            request_timeout_s: 60,
        }
    }
}

/// The model to suggest for a video card with `vram_gb` of memory: Gemma 4
/// E4B (≈5 GB plus room for the page) from ~7 GB, Hy-MT2 1.8B below.
pub fn recommended_model(vram_gb: f32) -> &'static str {
    if vram_gb >= 6.5 {
        "gemma4-e4b"
    } else {
        "hymt2-1.8b"
    }
}

/// Will a model of `size_gb` fit into the video card (with ~1.2 GB for the
/// context and the picture on screen)?
pub fn fits_gpu(size_gb: f32, vram_gb: f32) -> bool {
    vram_gb <= 0.0 || size_gb + 1.2 <= vram_gb
}

pub fn default_models() -> Vec<ModelPreset> {
    vec![
        ModelPreset {
            id: "hymt2-1.8b".into(),
            name: "Hy-MT2 1.8B Q4_K_M — быстрый, для слабых ПК (по умолчанию)".into(),
            prompt: "hymt".into(),
            hf: "tencent/Hy-MT2-1.8B-GGUF:Q4_K_M".into(),
            temperature: 0.7,
            top_p: 0.6,
            top_k: 20,
            repeat_penalty: 1.05,
            size_gb: 1.13,
            ..Default::default()
        },
        ModelPreset {
            id: "hymt2-7b".into(),
            name: "Hy-MT2 7B Q4_K_M — точный, но медленнее Gemma (видеокарта от 6 ГБ)".into(),
            prompt: "hymt".into(),
            hf: "tencent/Hy-MT2-7B-GGUF:Q4_K_M".into(),
            temperature: 0.7,
            top_p: 0.6,
            top_k: 20,
            repeat_penalty: 1.05,
            size_gb: 4.62,
            ..Default::default()
        },
        ModelPreset {
            id: "gemma4-e4b".into(),
            name: "Gemma 4 E4B Q4_K_M — самый живой перевод (видеокарта от 6 ГБ)".into(),
            prompt: "instruct".into(),
            hf: "unsloth/gemma-4-E4B-it-GGUF:Q4_K_M".into(),
            temperature: 0.3,
            top_p: 0.8,
            top_k: 20,
            repeat_penalty: 1.05,
            no_think: true,
            size_gb: 5.0,
            ..Default::default()
        },
    ]
}

impl Default for Dialogue {
    fn default() -> Self {
        Self { context_lines: 3, show_original: false, overlay_font: "Balsamiq Sans".into(), style_hint: String::new() }
    }
}

impl Default for Interface {
    fn default() -> Self {
        Self { mode: "replace".into(), upscale: 0, font_scale: 0.82, max_lines: 120, auto_hide: true, hover_original: true, page_context: true, vertical_ja: false, sfx: "translate".into(), ocr_engine: "neural".into(), journal: false, ocr_gpu: true }
    }
}

impl Default for Ocr {
    fn default() -> Self {
        Self { language: "zh-Hans-CN".into() }
    }
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            screen_translate: "F8".into(),
            show_original: "F9".into(),
            panel: "F10".into(),
            save_case: "Ctrl+F9".into(),
            freeze: "Ctrl+F8".into(),
            bad_translation: "Ctrl+F11".into(),
        }
    }
}

impl Config {
    /// Load the user config; on first run copy the bundled default, falling back to built-ins.
    pub fn load(paths: &Paths) -> anyhow::Result<Config> {
        let p = paths.config();
        let first_run = !p.exists();
        if !p.exists() {
            if paths.default_config().exists() {
                std::fs::copy(paths.default_config(), &p)?;
            } else {
                Config::default().save(paths)?;
            }
        }
        let text = std::fs::read_to_string(&p)?;
        let mut cfg: Config = toml::from_str(&text).map_err(|e| anyhow::anyhow!("Ошибка в {}: {e}", p.display()))?;
        if cfg.models.is_empty() {
            cfg.models = default_models();
        } else {
            // Presets added in newer versions appear in older configs too; the
            // Qwen presets of the first versions translated worse and are gone
            // unless the reader had one of them selected.
            let active = cfg.llm.active_model.clone();
            cfg.models.retain(|m| !(m.id.starts_with("qwen3.5-") && m.id != active));
            for m in default_models() {
                if !cfg.models.iter().any(|x| x.id == m.id) {
                    cfg.models.push(m);
                }
            }
        }
        cfg.apply_profile();
        // First start: the best model the video card can hold.
        if first_run {
            let id = recommended_model(crate::win::vram_gb());
            if cfg.llm.active_model != id && cfg.models.iter().any(|m| m.id == id) {
                tracing::info!("первый запуск: видеопамять {:.1} ГБ → модель {id}", crate::win::vram_gb());
                cfg.llm.active_model = id.into();
                let _ = cfg.save(paths);
            }
        }
        // Configs from before the bundled comic font: switch the old default
        // once (a font the reader chose is kept).
        if !cfg.general.comic_font_set {
            if cfg.dialogue.overlay_font == "Segoe UI" {
                cfg.dialogue.overlay_font = "Balsamiq Sans".into();
            }
            cfg.general.comic_font_set = true;
            let _ = cfg.save(paths);
        }
        Ok(cfg)
    }

    pub fn save(&self, paths: &Paths) -> anyhow::Result<()> {
        let tmp = paths.config().with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(tmp, paths.config())?;
        Ok(())
    }

    /// Fill the window match from a known profile ("custom" keeps the user's values).
    pub fn apply_profile(&mut self) {
        let p = profile(&self.general.profile);
        self.general.profile = p.id.into();
        if p.id != "custom" {
            self.general.window_match = p.window_match.into();
        }
        if !p.langs.contains(&self.general.source_lang.as_str()) {
            self.general.source_lang = p.langs[0].into();
        }
    }

    /// Does the profile follow one window (browser, a named program)? Otherwise
    /// the whole screen is read all the time.
    pub fn watches_window(&self) -> bool {
        !self.general.window_match.trim().is_empty() || !profile(&self.general.profile).window_class.is_empty()
    }

    /// Is the reader window in front?
    pub fn window_in_front(&self) -> bool {
        crate::win::game_in_foreground(&self.general.window_match, profile(&self.general.profile).window_class)
    }

    pub fn source_lang(&self) -> manga_core::lang::Lang {
        manga_core::lang::Lang::from_code(&self.general.source_lang)
    }

    pub fn active_model(&self) -> &ModelPreset {
        self.models.iter().find(|m| m.id == self.llm.active_model).unwrap_or(&self.models[0])
    }

    /// Which preset a user-chosen .gguf belongs to, by its file name
    /// ("Hy-MT2-7B-Q4_K_M.gguf" → hymt2-7b); unknown files replace the active model.
    pub fn preset_for_file(&self, path: &std::path::Path) -> String {
        let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let name = name.replace(['-', '_', ' '], "");
        let guess = if name.contains("hymt") || name.contains("hunyuan") {
            if name.contains("7b") {
                "hymt2-7b"
            } else {
                "hymt2-1.8b"
            }
        } else if name.contains("qwen") {
            if name.contains("4b") {
                "qwen3.5-4b"
            } else {
                "qwen3.5-2b"
            }
        } else {
            ""
        };
        if self.models.iter().any(|m| m.id == guess) {
            guess.to_string()
        } else {
            self.active_model().id.clone()
        }
    }
}
