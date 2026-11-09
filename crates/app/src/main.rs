//! Manga Local — local neural manga / manhwa / manhua translator (Korean / Japanese / Chinese → Russian).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("Manga Local работает только в Windows 10/11 x64. Ядро (manga-core) собирается и тестируется на любой ОС.");
}

#[cfg(windows)]
mod bench;
#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod config;
#[cfg(windows)]
mod extras;
#[cfg(windows)]
mod filepick;
#[cfg(windows)]
mod files;
#[cfg(windows)]
mod hotkeys;
#[cfg(windows)]
mod interface;
#[cfg(windows)]
mod jaocr;
#[cfg(windows)]
mod journal;
#[cfg(windows)]
mod llm;
#[cfg(windows)]
mod neural;
#[cfg(windows)]
mod mascot;
#[cfg(windows)]
mod ocr;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod paths;
#[cfg(windows)]
mod selftest;
#[cfg(windows)]
mod state;
#[cfg(windows)]
mod translate;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod ui_panel;
#[cfg(windows)]
mod win;

/// "v0.2.0 · 08.11.2026 14:30 · abc1234" — the build date and commit are set by CI.
#[cfg(windows)]
pub fn build_info() -> String {
    match option_env!("MANGA_BUILD") {
        Some(b) if !b.is_empty() => format!("v{} · {b}", env!("CARGO_PKG_VERSION")),
        _ => format!("v{} · локальная сборка", env!("CARGO_PKG_VERSION")),
    }
}

#[cfg(windows)]
fn main() {
    if let Err(e) = app::run() {
        app::fatal(&format!("{e:#}"));
    }
    // Everything is saved by now; skip the library unloading that can crash.
    win::hard_exit(0);
}

#[cfg(windows)]
mod app {
    use crate::config::Config;
    use crate::llm::client::LlmClient;
    use crate::llm::server::{self, ServerCmd};
    use crate::ocr::Ocr;
    use crate::paths::Paths;
    use crate::state::{Command, Shared, UiState};
    use crate::translate::Translator;
    use crate::{bench, files, hotkeys, interface, ui, win};
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::{Arc, Mutex, OnceLock, RwLock};
    use tokio::sync::mpsc;

    pub fn fatal(msg: &str) {
        use windows::core::HSTRING;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        tracing::error!("{msg}");
        unsafe {
            MessageBoxW(HWND::default(), &HSTRING::from(msg), &HSTRING::from("Manga Local — ошибка"), MB_OK | MB_ICONERROR);
        }
    }

    fn init_logging(paths: &Paths) -> tracing_appender::non_blocking::WorkerGuard {
        use tracing_subscriber::{fmt, EnvFilter};
        let file = tracing_appender::rolling::daily(paths.logs(), "manga.log");
        let (nb, guard) = tracing_appender::non_blocking(file);
        let filter = EnvFilter::try_from_env("MANGA_LOG").unwrap_or_else(|_| EnvFilter::new("info,llama=debug,dialogue=debug"));
        fmt().with_env_filter(filter).with_writer(nb).with_ansi(false).init();
        guard
    }

    fn open_path(p: &std::path::Path) {
        let r = if p.is_dir() {
            std::process::Command::new("explorer").arg(p).spawn()
        } else {
            if !p.exists() {
                let _ = std::fs::write(p, "");
            }
            std::process::Command::new("notepad").arg(p).spawn()
        };
        if let Err(e) = r {
            tracing::warn!("открыть {}: {e}", p.display());
        }
    }

    pub fn run() -> anyhow::Result<()> {
        // Screenshot tests (CI and the developer): no window, no second-copy check.
        let args: Vec<String> = std::env::args().collect();
        if args.get(1).map(String::as_str) == Some("--selftest") {
            win::hard_exit(crate::selftest::run_cli(&args));
        }
        // Only one copy: a second launch just brings up the panel of the first one.
        if !win::claim_single_instance() {
            win::signal_show_panel();
            return Ok(());
        }
        win::set_dpi_aware();
        let paths = Paths::detect()?;
        let _log_guard = init_logging(&paths);
        tracing::info!("Manga Local {} · данные: {}", crate::build_info(), paths.data.display());
        std::panic::set_hook(Box::new(|info| {
            tracing::error!("panic: {info}");
        }));

        let cfg = Config::load(&paths)?;
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Command>();
        let shared = Arc::new(Shared {
            paths: paths.clone(),
            cfg: RwLock::new(cfg.clone()),
            ui: Mutex::new(UiState { show_original: cfg.dialogue.show_original, ..Default::default() }),
            cmd: cmd_tx.clone(),
            egui: OnceLock::new(),
            interface_gen: AtomicU64::new(0),
            reset_watcher: AtomicBool::new(false),
            overlay_px: Mutex::new(Vec::new()),
        });

        crate::neural::init(crate::neural::model_dir(&shared));
        crate::neural::set_on(cfg.interface.ocr_engine == "neural");
        crate::journal::set_on(cfg.interface.journal);

        // Hotkeys.
        let problems = hotkeys::spawn(&cfg.hotkeys, cmd_tx.clone());
        if let Ok(p) = problems.recv_timeout(std::time::Duration::from_secs(2)) {
            shared.ui(|u| u.hotkey_problems = p.clone());
            for msg in p {
                shared.ui(|u| u.notice(msg));
            }
        }

        // OCR.
        let ocr = match Ocr::spawn() {
            Ok(o) => {
                // A missing language pack is not fatal: the user may install it or switch the language.
                let status = o.check_blocking(cfg.source_lang()).map_err(|e| e.to_string());
                tracing::info!("Windows OCR ({}): {:?} · макс. размер {} px", cfg.source_lang().name_ru(), status, o.max_dim);
                let failed = status.is_err();
                shared.ui(|u| {
                    u.ocr_status = Some(status);
                    if failed {
                        u.panel_open = true;
                    }
                });
                Some(o)
            }
            Err(e) => {
                shared.ui(|u| {
                    u.ocr_status = Some(Err(e.to_string()));
                    u.panel_open = true;
                });
                None
            }
        };

        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().thread_name("manga-rt").build()?;
        let client = LlmClient::new(&server::base_url(&cfg), cfg.llm.request_timeout_s);
        let translator = {
            let _g = rt.enter();
            Arc::new(Translator::new(paths.clone(), client.clone(), cfg.dialogue.context_lines, &cfg.general.profile))
        };
        {
            let counts = translator.dict_counts();
            shared.ui(|u| u.dict_counts = counts);
        }
        let (srv_tx, srv_rx) = mpsc::unbounded_channel::<ServerCmd>();
        rt.spawn(server::supervise(shared.clone(), client, srv_rx));
        rt.spawn(dispatcher(shared.clone(), translator.clone(), ocr.clone(), cmd_rx, srv_tx));

        // The panel is the only visible sign that the program runs: always show it at start.
        shared.ui(|u| {
            u.panel_open = true;
            u.focus_panel = true;
        });
        crate::overlay::spawn(shared.clone());
        crate::tray::spawn(cmd_tx.clone());
        {
            let tx = cmd_tx.clone();
            win::listen_show_panel(move || {
                let _ = tx.send(Command::ShowPanel);
            });
        }

        let options = eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_title(ui::OVERLAY_TITLE)
                .with_transparent(true)
                .with_decorations(false)
                .with_always_on_top()
                .with_mouse_passthrough(true)
                .with_taskbar(false)
                .with_resizable(false)
                .with_position([0.0, 0.0])
                // The root egui window only hosts the panel and the region picker:
                // it stays a transparent 1×1 px window. Translations are drawn by
                // the native overlay window (overlay.rs).
                .with_inner_size([1.0, 1.0])
                .with_icon(crate::mascot::icon_data()),
            renderer: eframe::Renderer::Glow,
            vsync: true,
            ..Default::default()
        };
        let sh2 = shared.clone();
        let res = eframe::run_native("manga-local", options, Box::new(move |cc| Ok(Box::new(ui::MangaApp::new(cc, sh2)))));
        shared.save_config();
        crate::tray::remove();
        tracing::info!("выход");
        rt.shutdown_timeout(std::time::Duration::from_secs(2));
        res.map_err(|e| anyhow::anyhow!("окно: {e}"))
    }

    async fn dispatcher(
        shared: Arc<Shared>,
        tr: Arc<Translator>,
        ocr: Option<Ocr>,
        mut rx: mpsc::UnboundedReceiver<Command>,
        srv: mpsc::UnboundedSender<ServerCmd>,
    ) {
        while let Some(cmd) = rx.recv().await {
            tracing::info!("команда: {}", format!("{cmd:?}").chars().take(120).collect::<String>());
            match cmd {
                Command::ScreenTranslate => match &ocr {
                    Some(o) => {
                        let (sh, tr, o) = (shared.clone(), tr.clone(), o.clone());
                        tokio::spawn(async move {
                            // The panel would be photographed and "translated" too: hide it
                            // first and give Windows time to take it off the screen.
                            // Pressed in the panel: hide it only for the screenshot (it would be
                            // "translated" too) and bring it back afterwards.
                            let starting = sh.ui.lock().unwrap().interface.is_none();
                            if starting && sh.ui(|u| u.hide_panel_for_shot()) {
                                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                            }
                            interface::toggle(sh.clone(), tr, o).await;
                            sh.ui(|u| u.restore_panel());
                        });
                    }
                    None => shared.ui(|u| u.notice("OCR не готов — перевод экрана недоступен")),
                },
                Command::ToggleOriginal => shared.ui(|u| u.show_original = !u.show_original),
                Command::TogglePanel => shared.ui(|u| {
                    u.panel_open = !u.panel_open;
                    u.focus_panel = u.panel_open;
                }),
                Command::ShowPanel => shared.ui(|u| {
                    u.panel_open = true;
                    u.focus_panel = true;
                }),
                Command::SwitchModel(id) => {
                    shared.cfg.write().unwrap().llm.active_model = id;
                    shared.save_config();
                    tr.clear_history();
                    let _ = srv.send(ServerCmd::Restart);
                }
                Command::RestartServer => {
                    let _ = srv.send(ServerCmd::Restart);
                }
                Command::UseModelFile(path) => {
                    let id = {
                        let mut c = shared.cfg.write().unwrap();
                        let id = c.preset_for_file(&path);
                        if let Some(m) = c.models.iter_mut().find(|m| m.id == id) {
                            m.path = path.display().to_string();
                        }
                        c.llm.active_model = id.clone();
                        id
                    };
                    shared.save_config();
                    tr.clear_history();
                    tracing::info!("модель из файла: {} → {id}", path.display());
                    shared.ui(|u| u.notice(format!("Модель берётся из файла {} — скачивать ничего не нужно.", path.display())));
                    let _ = srv.send(ServerCmd::Restart);
                }
                Command::SaveCorrection { src, dst } => {
                    tr.save_correction(&src, &dst);
                    shared.ui(|u| {
                        for r in u.recent.iter_mut().filter(|r| r.src == src) {
                            r.dst = dst.clone();
                        }
                    });
                }
                Command::GameChanged => {
                    let cfg = shared.cfg();
                    let (n, f) = tr.reload_dictionaries(&cfg.general.profile);
                    tr.clear_history();
                    shared.reset_watcher.store(true, std::sync::atomic::Ordering::SeqCst);
                    let status = match &ocr {
                        Some(o) => o.check(cfg.source_lang()).await.map_err(|e| e.to_string()),
                        None => Err("OCR не запущен".into()),
                    };
                    shared.ui(|u| {
                        u.dict_counts = (n, f);
                        u.ocr_status = Some(status);
                    });
                    shared.save_config();
                }
                Command::SaveCase => match &ocr {
                    Some(o) => {
                        tokio::spawn(crate::selftest::save_case(shared.clone(), tr.clone(), o.clone()));
                    }
                    None => shared.ui(|u| u.notice("OCR не готов — снимок для теста не сохранить")),
                },
                Command::ExportDicts(path) => crate::extras::export_dicts(&shared, &tr, &path),
                Command::ImportDicts(path) => crate::extras::import_dicts(&shared, &tr, &path),
                Command::BadTranslation => {
                    tokio::spawn(crate::extras::bad_translation(shared.clone(), tr.clone()));
                }
                Command::Freeze => match &ocr {
                    Some(o) => {
                        tokio::spawn(crate::extras::freeze(shared.clone(), o.clone()));
                    }
                    None => shared.ui(|u| u.notice("Стоп-кадр: распознавание текста не готово")),
                },
                Command::FreezeTranslate(i) => {
                    tokio::spawn(crate::extras::freeze_translate(shared.clone(), tr.clone(), i));
                }
                Command::FreezeClose => shared.ui(|u| {
                    u.freeze = None;
                    u.restore_panel();
                }),
                Command::AddTerm { src, dst } => crate::extras::add_term(&shared, &tr, &src, &dst),
                Command::AddName { src, dst } => {
                    crate::extras::add_name(&shared, &tr, &src, &dst);
                    shared.ui(|u| u.name_offers.retain(|(s, _)| *s != src));
                }
                Command::IgnoreName(src) => {
                    tr.ignore_name(&src);
                    shared.ui(|u| u.name_offers.retain(|(s, _)| *s != src));
                }
                Command::Diagnose => {
                    tokio::spawn(crate::extras::diagnose(shared.clone(), tr.clone(), ocr.clone()));
                }
                Command::TestOverlay => {
                    shared.ui(|u| u.test_overlay_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(10)));
                }
                Command::Quit => {
                    tracing::info!("выход по команде пользователя");
                    // Settings changed in the last moment (the panel saves them
                    // with a short delay) must not be lost.
                    shared.save_config();
                    crate::tray::remove();
                    if let Some(ctx) = shared.egui.get() {
                        ctx.send_viewport_cmd_to(eframe::egui::ViewportId::ROOT, eframe::egui::ViewportCommand::Close);
                        ctx.request_repaint();
                    }
                    // Safety net: if the window loop does not end, leave anyway
                    // (llama-server dies with us through the job object).
                    std::thread::spawn(|| {
                        std::thread::sleep(std::time::Duration::from_secs(5));
                        win::hard_exit(0);
                    });
                }
                Command::PanelClosedToTray => {
                    let first = !shared.cfg().general.tray_hint_shown;
                    if first {
                        crate::tray::balloon(
                            "Manga Local работает в фоне",
                            "Перевод и горячие клавиши продолжают работать. Значок с енотиком — у часов: клик открывает панель, правый клик — «Выход».",
                        );
                        shared.cfg.write().unwrap().general.tray_hint_shown = true;
                        shared.save_config();
                    }
                }
                Command::ReloadGlossary => {
                    let (n, f) = tr.reload_dictionaries(&shared.cfg().general.profile);
                    shared.ui(|u| {
                        u.dict_counts = (n, f);
                        u.notice(format!("Словари перечитаны: терминов {n}, исправлений {f}"));
                    });
                }
                Command::TranslateFiles(p) => match &ocr {
                    Some(o) => {
                        let (sh, tr, o) = (shared.clone(), tr.clone(), o.clone());
                        tokio::spawn(async move { files::translate_chapter(sh, tr, o, p).await });
                    }
                    None => shared.ui(|u| u.notice("OCR не готов — перевод файлов недоступен")),
                },
                Command::StopFiles => files::stop(),
                Command::DownloadJaOcr => {
                    let sh = shared.clone();
                    tokio::spawn(async move {
                        sh.ui(|u| u.ja_ocr_status = Some("Скачиваю распознавание японского…".into()));
                        let r = crate::jaocr::download(sh.clone()).await;
                        sh.ui(|u| match &r {
                            Ok(()) => {
                                u.ja_ocr_status = None;
                                u.notice("Распознавание вертикального японского скачано и включено");
                            }
                            Err(e) => u.ja_ocr_status = Some(format!("Не скачалось: {e}. Проверьте интернет и включите ещё раз")),
                        });
                    });
                }
                Command::RunBench => {
                    let sh = shared.clone();
                    tokio::spawn(async move {
                        let r = bench::run(sh.clone()).await;
                        sh.ui(|u| {
                            u.bench_progress = None;
                            match &r {
                                Ok(p) => u.notice(format!("Бенчмарк готов: {}", p.display())),
                                Err(e) => u.notice(format!("Бенчмарк: {e}")),
                            }
                        });
                        if let Ok(p) = r {
                            open_path(&p);
                        }
                    });
                }
                Command::TestTranslate(text) => {
                    let (sh, tr) = (shared.clone(), tr.clone());
                    tokio::spawn(async move {
                        sh.ui(|u| u.test_result = Some("…".into()));
                        let cfg = sh.cfg();
                        let preset = cfg.active_model().clone();
                        let lang = manga_core::lang::detect(&text).unwrap_or(cfg.source_lang());
                        let src = tr.fix_ocr(&manga_core::text::normalize_for(&text, lang));
                        let short = match lang {
                            manga_core::lang::Lang::Zh | manga_core::lang::Lang::Ja => src.chars().count() <= 8,
                            manga_core::lang::Lang::En | manga_core::lang::Lang::Ko => src.split_whitespace().count() <= 2,
                        };
                        let kind = if short && !src.contains(['，', '。', '！', '？', '.', '!', '?']) {
                            manga_core::prompt::TextKind::Interface
                        } else {
                            manga_core::prompt::TextKind::Dialogue
                        };
                        let opts = crate::translate::Opts { use_cache: true, use_history: false };
                        let r = tr.translate(&preset, &cfg.general.game_title, &cfg.dialogue.style_hint, &src, kind, opts, |_| {}).await;
                        sh.ui(|u| match r {
                            Ok(o) => {
                                u.test_result = Some(format!("{}  ({} мс{})", o.text, o.ms, if o.cached { ", кэш" } else { "" }));
                                u.push_recent(crate::state::RecentLine { src: src.clone(), dst: o.text, model: preset.id.clone(), ms: o.ms, cached: o.cached });
                            }
                            Err(e) => u.test_result = Some(format!("Ошибка: {e}")),
                        });
                    });
                }
                Command::SaveConfig => {
                    crate::neural::set_on(shared.cfg().interface.ocr_engine == "neural");
                    crate::journal::set_on(shared.cfg().interface.journal);
                    shared.save_config();
                }
                Command::DownloadNeural => {
                    let sh = shared.clone();
                    let lang = shared.cfg().source_lang();
                    tokio::spawn(async move {
                        sh.ui(|u| u.neural_status = Some("Скачиваю распознавание…".into()));
                        let r = crate::neural::download(sh.clone(), lang).await;
                        sh.ui(|u| match &r {
                            Ok(()) => {
                                u.neural_status = None;
                                u.notice(format!("Распознавание-нейросеть для языка «{}» скачано и включено", lang.name_ru()));
                            }
                            Err(e) => u.neural_status = Some(format!("Не скачалось: {e}. Проверьте интернет и попробуйте ещё раз")),
                        });
                    });
                }
                Command::Open(p) => open_path(&p),
                Command::OpenTitleDict => {
                    let title = shared.cfg().general.game_title;
                    match tr.title_glossary_file(&title) {
                        Some(p) => open_path(&p),
                        None => shared.ui(|u| u.notice("Сначала впишите название тайтла: Настройки → «Где читаете»")),
                    }
                }
                Command::OpenUrl(u) => {
                    if u.starts_with("https://") {
                        let _ = std::process::Command::new("explorer").arg(&u).spawn();
                    }
                }
            }
        }
    }
}
