//! Control panel: «Ночной вебтун» — dark navy, rounded tiles, one accent colour
//! the reader picks (бирюза by default), sidebar navigation.
//!
//! Pages: Главная (status, checklist, big actions, model) · Настройки ·
//! Словари · Проверка · Справка.

use crate::state::{Command, Shared};
use eframe::egui::{
    self, vec2, Align, Color32, FontFamily, FontId, Layout, Margin, RichText, Rounding, Sense, Stroke, TextStyle, Ui, Vec2,
};
use std::collections::HashMap;
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(17, 18, 27);
pub const NAV: Color32 = Color32::from_rgb(23, 25, 38);
pub const CARD: Color32 = Color32::from_rgb(28, 30, 45);
pub const CARD_HI: Color32 = Color32::from_rgb(38, 41, 64);
pub const RED: Color32 = Color32::from_rgb(255, 112, 112);
pub const GREEN: Color32 = Color32::from_rgb(95, 215, 150);
pub const YELLOW: Color32 = Color32::from_rgb(255, 201, 121);
pub const TEXT: Color32 = Color32::from_rgb(236, 234, 244);
pub const WEAK: Color32 = Color32::from_rgb(184, 180, 204);

/// Ready-made accent colours of the panel (the reader may also pick any).
pub const ACCENTS: &[(&str, [u8; 3])] = &[
    ("Бирюза", [60, 201, 180]),
    ("Янтарь", [255, 181, 71]),
    ("Лаванда", [157, 140, 255]),
    ("Голубой", [77, 163, 255]),
    ("Коралл", [255, 122, 144]),
];

static ACCENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3C_C9_B4);

/// The accent colour the reader chose (buttons, selection, icons).
pub fn accent() -> Color32 {
    let v = ACCENT.load(std::sync::atomic::Ordering::Relaxed);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Text on an accent fill: dark on light accents, white on dark ones.
pub fn on_accent() -> Color32 {
    let a = accent();
    let l = 0.299 * a.r() as f32 + 0.587 * a.g() as f32 + 0.114 * a.b() as f32;
    if l > 140.0 {
        Color32::from_rgb(14, 20, 26)
    } else {
        Color32::WHITE
    }
}

/// A dark tint of the accent (selected menu item, active tiles).
pub fn accent_tint() -> Color32 {
    let (a, n) = (accent(), NAV);
    let mix = |x: u8, y: u8| (x as f32 * 0.22 + y as f32 * 0.78) as u8;
    Color32::from_rgb(mix(a.r(), n.r()), mix(a.g(), n.g()), mix(a.b(), n.b()))
}

/// Apply the reader's colour if it changed (called every frame, cheap).
pub fn sync_accent(ctx: &egui::Context, rgb: [u8; 3]) {
    let v = (rgb[0] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[2] as u32;
    if ACCENT.swap(v, std::sync::atomic::Ordering::Relaxed) != v {
        apply_theme(ctx);
    }
}

/// Donation page. Empty = the «Поддержать» button is hidden.
pub const DONATE_URL: &str = "";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Home,
    Settings,
    Dictionaries,
    Check,
    Help,
}

#[derive(Default)]
pub struct PanelState {
    pub tab: Tab,
    pub edits: HashMap<String, String>,
    pub test_input: String,
    pub settings_query: String,
}

pub struct OcrView {
    pub tex: egui::TextureId,
    pub size: Vec2,
    pub text: String,
    pub ms: u128,
}

/// Global look for all egui widgets (the overlay paints with explicit colours,
/// so it is not affected).
pub fn apply_theme(ctx: &egui::Context) {
    // Always dark, regardless of the Windows light/dark setting.
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut style = (*ctx.style()).clone();
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = Color32::from_rgb(13, 14, 22);
    v.faint_bg_color = CARD;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = accent();
    v.selection.bg_fill = accent_tint();
    v.selection.stroke = Stroke::new(1.0, accent());
    v.window_rounding = Rounding::same(14.0);
    let r = Rounding::same(10.0);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.rounding = r;
    }
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.weak_bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(46, 49, 72));
    v.widgets.inactive.bg_fill = CARD_HI;
    v.widgets.inactive.weak_bg_fill = CARD_HI;
    v.widgets.hovered.bg_fill = Color32::from_rgb(50, 54, 82);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(50, 54, 82);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, accent());
    v.widgets.active.bg_fill = accent_tint();
    v.widgets.active.weak_bg_fill = accent_tint();
    style.visuals = v;
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.button_padding = vec2(12.0, 6.0);
    style.spacing.slider_width = 220.0;
    style.spacing.interact_size.y = 26.0;
    style.text_styles = [
        (TextStyle::Heading, FontId::new(22.0, FontFamily::Name("bold".into()))),
        (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(15.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(12.5, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.5, FontFamily::Monospace)),
    ]
    .into();
    ctx.set_style(style);
}

// ---------- small widgets ----------

fn card<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::none()
        .fill(CARD)
        .rounding(Rounding::same(18.0))
        .inner_margin(Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).size(16.5).family(FontFamily::Name("bold".into())).color(TEXT));
                ui.add_space(2.0);
            }
            add(ui)
        })
        .inner
}

fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).small().color(WEAK));
}

fn key_chip(ui: &mut Ui, key: &str) {
    if key.is_empty() {
        return;
    }
    egui::Frame::none()
        .fill(Color32::from_rgb(42, 45, 66))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(8.0, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(key).size(13.0).family(FontFamily::Name("bold".into())).color(TEXT));
        });
}

/// iOS-style switch (from the egui demo).
fn toggle(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let size = vec2(2.2, 1.15) * ui.spacing().interact_size.y;
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool(resp.id, *on);
        let r = rect.height() / 2.0;
        let bg = if *on { accent().gamma_multiply(0.55) } else { Color32::from_rgb(58, 61, 86) };
        ui.painter().rect_filled(rect, Rounding::same(r), bg);
        let cx = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
        ui.painter().circle_filled(egui::pos2(cx, rect.center().y), r * 0.78, if *on { accent() } else { Color32::from_gray(190) });
    }
    resp
}

// ---------- settings search ----------

#[derive(Default)]
struct Search {
    words: Vec<String>,
    /// Title of the card being drawn: a query matching the title shows the whole card.
    title: String,
}

thread_local! {
    static SEARCH: std::cell::RefCell<Search> = std::cell::RefCell::new(Search::default());
}

fn fold(s: &str) -> String {
    s.to_lowercase().replace('ё', "е")
}

fn search_set(query: &str) {
    SEARCH.with(|s| {
        let mut s = s.borrow_mut();
        s.words = fold(query).split_whitespace().map(str::to_string).collect();
        s.title.clear();
    });
}

/// Does `text` (plus the current card title) contain every word of the query?
fn search_hit(text: &str) -> bool {
    SEARCH.with(|s| {
        let s = s.borrow();
        if s.words.is_empty() {
            return true;
        }
        let hay = fold(&format!("{} {text}", s.title));
        s.words.iter().all(|w| hay.contains(w.as_str()))
    })
}

/// A settings card that hides itself when nothing in it matches the search.
/// `keys` lists the words of all its rows (labels and hints).
fn search_card(ui: &mut Ui, title: &str, keys: &str, add: impl FnOnce(&mut Ui)) -> bool {
    SEARCH.with(|s| s.borrow_mut().title = title.to_string());
    let shown = search_hit(keys);
    if shown {
        card(ui, title, add);
        ui.add_space(8.0);
    }
    SEARCH.with(|s| s.borrow_mut().title.clear());
    shown
}

fn setting_row(ui: &mut Ui, label: &str, help: &str, add: impl FnOnce(&mut Ui) -> bool) -> bool {
    if !search_hit(&format!("{label} {help}")) {
        return false;
    }
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() * 0.5).max(180.0));
            ui.label(label);
            if !help.is_empty() {
                hint(ui, help);
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            changed = add(ui);
        });
    });
    changed
}

fn segmented(ui: &mut Ui, value: &mut String, options: &[(&str, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (v, label) in options {
            let selected = value == v;
            let text = RichText::new(*label).color(if selected { on_accent() } else { TEXT });
            let b = egui::Button::new(text).fill(if selected { accent() } else { CARD_HI }).rounding(Rounding::same(10.0));
            if ui.add(b).clicked() && !selected {
                *value = v.to_string();
                changed = true;
            }
        }
    });
    changed
}

fn big_action(ui: &mut Ui, title: &str, subtitle: &str, key: &str, active: bool, width: f32) -> bool {
    let fill = if active { accent_tint() } else { CARD };
    let stroke = if active { Stroke::new(1.5, accent()) } else { Stroke::NONE };
    let resp = egui::Frame::none()
        .fill(fill)
        .stroke(stroke)
        .rounding(Rounding::same(18.0))
        .inner_margin(Margin::same(16.0))
        .show(ui, |ui| {
            // The caller lays buttons out in a row: without an explicit vertical layout the
            // subtitle would be placed to the right of the title and widen the button.
            let w = (width - 32.0).max(120.0);
            ui.set_width(w);
            ui.set_max_width(w);
            ui.vertical(|ui| {
                ui.set_min_height(58.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).size(17.5).family(FontFamily::Name("bold".into())).color(if active { accent() } else { TEXT }));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| key_chip(ui, key));
                });
                hint(ui, subtitle);
            });
        })
        .response
        .interact(Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

/// The main action: a wide tile in the accent colour.
fn hero_action(ui: &mut Ui, title: &str, subtitle: &str, key: &str, active: bool) -> bool {
    let (fill, fg) = if active { (accent_tint(), accent()) } else { (accent(), on_accent()) };
    let resp = egui::Frame::none()
        .fill(fill)
        .stroke(if active { Stroke::new(1.5, accent()) } else { Stroke::NONE })
        .rounding(Rounding::same(22.0))
        .inner_margin(Margin::symmetric(22.0, 18.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).size(23.0).family(FontFamily::Name("bold".into())).color(fg));
                    ui.label(RichText::new(subtitle).size(14.5).color(fg));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !key.is_empty() {
                        egui::Frame::none().fill(if active { CARD_HI } else { Color32::from_rgb(14, 20, 26) }).rounding(Rounding::same(10.0)).inner_margin(Margin::symmetric(12.0, 5.0)).show(ui, |ui| {
                            ui.label(RichText::new(key).size(16.0).family(FontFamily::Name("bold".into())).color(Color32::WHITE));
                        });
                    }
                });
            });
        })
        .response
        .interact(Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

fn status_pill(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(color.gamma_multiply(0.16))
        .rounding(Rounding::same(999.0))
        .inner_margin(Margin::symmetric(12.0, 5.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).strong());
        });
}

fn check_item(ui: &mut Ui, ok: Option<bool>, title: &str, detail: &str) {
    ui.horizontal(|ui| {
        let (sym, col) = match ok {
            Some(true) => ("✔", GREEN),
            Some(false) => ("✖", RED),
            None => ("…", YELLOW),
        };
        ui.label(RichText::new(sym).size(17.0).color(col));
        ui.vertical(|ui| {
            ui.label(title);
            if !detail.is_empty() {
                hint(ui, detail);
            }
        });
    });
}

// ---------- the panel ----------

/// Returns true if the config was edited (debounced save).
pub fn panel_ui(ctx: &egui::Context, shared: &Arc<Shared>, st: &mut PanelState, ocr_view: Option<&OcrView>) -> bool {
    let mut dirty = false;
    sync_accent(ctx, shared.cfg().general.accent);

    egui::SidePanel::left("wx_nav")
        .exact_width(190.0)
        .resizable(false)
        .frame(egui::Frame::none().fill(NAV).inner_margin(Margin::symmetric(14.0, 18.0)))
        .show(ctx, |ui| {
            // Logo.
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                ui.painter().rect_filled(rect, Rounding::same(12.0), accent());
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "ML", FontId::new(16.0, FontFamily::Name("bold".into())), on_accent());
                ui.add_space(4.0);
                ui.label(RichText::new("Manga Local").size(19.0).family(FontFamily::Name("bold".into())).color(TEXT));
            });
            ui.add_space(14.0);
            for (tab, icon, label) in [
                (Tab::Home, "⌂", "Главная"),
                (Tab::Settings, "⚙", "Настройки"),
                (Tab::Dictionaries, "📖", "Словари"),
                (Tab::Check, "🔍", "Проверка"),
                (Tab::Help, "❓", "Справка"),
            ] {
                let selected = st.tab == tab;
                let mut job = egui::text::LayoutJob::default();
                let fmt = |c: Color32| egui::TextFormat { font_id: FontId::new(16.0, FontFamily::Name("bold".into())), color: c, ..Default::default() };
                job.append(icon, 0.0, fmt(if selected { accent() } else { WEAK }));
                job.append(label, 10.0, fmt(if selected { Color32::WHITE } else { WEAK }));
                let b = egui::Button::new(job)
                    .fill(if selected { accent_tint() } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE)
                    .rounding(Rounding::same(12.0))
                    .min_size(vec2(162.0, 40.0));
                if ui.add(b).clicked() {
                    st.tab = tab;
                }
            }
            ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                ui.label(RichText::new(crate::build_info()).small().color(WEAK));
                ui.add_space(2.0);
                let quit = egui::Button::new(RichText::new("⏻  Выход").color(WEAK)).fill(Color32::TRANSPARENT).min_size(vec2(162.0, 28.0));
                if ui.add(quit).on_hover_text("Полностью закрыть программу. Крестик окна только прячет панель в трей.").clicked() {
                    shared.send(Command::Quit);
                }
                if !DONATE_URL.is_empty() {
                    let b = egui::Button::new(RichText::new("♥  Поддержать").color(on_accent()))
                        .fill(accent())
                        .rounding(Rounding::same(12.0))
                        .min_size(vec2(162.0, 32.0));
                    if ui.add(b).on_hover_text("Программа бесплатная. Если она вам помогает — можно угостить енотика печенькой.").clicked() {
                        shared.send(Command::OpenUrl(DONATE_URL.to_string()));
                    }
                }
                ui.add_space(4.0);
                let space = ui.available_height();
                if space > 90.0 {
                    let h = space.min(170.0) - 6.0;
                    let r = crate::mascot::widget(ui, vec2(162.0, h), mascot_mood(shared));
                    if r.clicked() && !DONATE_URL.is_empty() {
                        shared.send(Command::OpenUrl(DONATE_URL.to_string()));
                    }
                }
            });
        });

    egui::CentralPanel::default().frame(egui::Frame::none().fill(BG).inner_margin(Margin::symmetric(24.0, 20.0))).show(ctx, |ui| {
        header(ui, shared, st.tab);
        ui.add_space(12.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(ui.available_width());
            dirty |= match st.tab {
                Tab::Home => page_home(ui, shared),
                Tab::Settings => page_settings(ui, shared, &mut st.settings_query),
                Tab::Dictionaries => page_dictionaries(ui, shared, &mut st.edits),
                Tab::Check => page_check(ui, shared, &mut st.test_input, ocr_view),
                Tab::Help => page_help(ui, shared),
            };
        });
    });
    dirty
}

fn mascot_mood(shared: &Arc<Shared>) -> crate::mascot::Mood {
    use crate::mascot::Mood;
    use crate::state::ServerStatus;
    let u = shared.ui.lock().unwrap();
    let error = matches!(u.server, Some(ServerStatus::Failed(_))) || matches!(u.ocr_status, Some(Err(_)));
    let working = u.interface.as_ref().map_or(false, |i| i.pending > 0)
        || !matches!(u.server, Some(ServerStatus::Ready { .. }) | Some(ServerStatus::Failed(_)));
    if error {
        Mood::Error
    } else if working {
        Mood::Working
    } else {
        Mood::Idle
    }
}

/// Top line of the content: the page name and what the program is doing.
fn header(ui: &mut Ui, shared: &Arc<Shared>, tab: Tab) {
    let lang = shared.cfg().source_lang();
    let server = shared.ui.lock().unwrap().server.clone();
    let title = match tab {
        Tab::Home => "Главная",
        Tab::Settings => "Настройки",
        Tab::Dictionaries => "Словари",
        Tab::Check => "Проверка",
        Tab::Help => "Справка",
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(24.0).family(FontFamily::Name("bold".into())).color(TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            status_pill(ui, &format!("{} → русский", lang.name_ru()), WEAK);
            match server {
                Some(s) if s.is_ready() => status_pill(ui, "● Нейросеть готова", GREEN),
                Some(crate::state::ServerStatus::Failed(_)) => status_pill(ui, "● Ошибка модели", RED),
                Some(crate::state::ServerStatus::Starting(_)) => status_pill(ui, "● Загрузка модели…", YELLOW),
                _ => status_pill(ui, "● Запуск…", YELLOW),
            }
        });
    });
}

fn page_home(ui: &mut Ui, shared: &Arc<Shared>) -> bool {
    let cfg = shared.cfg();
    let hk = cfg.hotkeys.clone();
    let (server, show_original, stats, notices, ocr_status, has_interface) = {
        let u = shared.ui.lock().unwrap();
        (
            u.server.clone(),
            u.show_original,
            u.stats.clone(),
            u.notices.iter().cloned().collect::<Vec<_>>(),
            u.ocr_status.clone(),
            u.interface.is_some(),
        )
    };

    // Checklist until everything is ready.
    let model_ok = server.as_ref().map(|s| s.is_ready());
    let ocr_ok = ocr_status.as_ref().map(|r| r.is_ok());
    if model_ok != Some(true) || ocr_ok != Some(true) {
        card(ui, "Подготовка", |ui| {
            let model_detail = match &server {
                Some(crate::state::ServerStatus::Starting(s)) => s.clone(),
                Some(crate::state::ServerStatus::Failed(e)) => e.clone(),
                Some(crate::state::ServerStatus::Ready { model }) => model.clone(),
                _ => "запускается…".into(),
            };
            check_item(
                ui,
                match &server {
                    Some(s) if s.is_ready() => Some(true),
                    Some(crate::state::ServerStatus::Failed(_)) => Some(false),
                    _ => None,
                },
                "Нейросеть загружена",
                &model_detail,
            );
            let ocr_detail = match &ocr_status {
                Some(Err(e)) => e.clone(),
                _ => String::new(),
            };
            let ocr_title = match cfg.source_lang() {
                manga_core::lang::Lang::En => "Распознавание английского текста (Windows OCR)",
                manga_core::lang::Lang::Zh => "Распознавание китайского текста (Windows OCR)",
                manga_core::lang::Lang::Ko => "Распознавание корейского текста (Windows OCR)",
                manga_core::lang::Lang::Ja => "Распознавание японского текста (Windows OCR)",
            };
            check_item(ui, ocr_ok, ocr_title, &ocr_detail);
            check_item(ui, None, "Откройте мангу в браузере", "Страница должна быть видна на экране. F8 — и дальше просто листайте, перевод появляется сам");
        });
        ui.add_space(8.0);
    }

    // Main action, then the others as tiles.
    let sub_f8 = if has_interface { "Перевод включён — нажмите ещё раз, чтобы убрать" } else { "Все пузыри на экране. Листайте — перевод появится сам" };
    if hero_action(ui, "Перевести страницу", sub_f8, &hk.screen_translate, has_interface) {
        // ScreenTranslate hides the panel itself and waits until it is off the screen.
        shared.send(Command::ScreenTranslate);
    }
    ui.add_space(4.0);
    let gap = ui.spacing().item_spacing.x;
    let w = ((ui.available_width() - 2.0 * gap) / 3.0).floor();
    ui.horizontal(|ui| {
        if big_action(ui, "Стоп-кадр", "Нажмите на текст — перевод", &hk.freeze, false, w) {
            shared.send(Command::Freeze);
        }
        let sub = if show_original { "Сейчас показан оригинал" } else { "Сейчас показан перевод" };
        if big_action(ui, "Оригинал / перевод", sub, &hk.show_original, show_original, w) {
            shared.send(Command::ToggleOriginal);
        }
        if big_action(ui, "Плохой перевод", "Перевести страницу заново", &hk.bad_translation, false, w) {
            shared.send(Command::BadTranslation);
        }
    });
    ui.add_space(4.0);
    // Chapter files: a folder, ZIP or CBZ — the translation is lettered into the pictures.
    let files_progress = shared.ui.lock().unwrap().files_progress.clone();
    let full = ui.available_width();
    match &files_progress {
        Some(p) => {
            if big_action(ui, "Перевод главы идёт… (нажмите, чтобы остановить)", p, "", true, full) {
                shared.send(Command::StopFiles);
            }
        }
        None => {
            if big_action(ui, "Перевести файлы главы", "Папка с картинками, ZIP или CBZ: текст сотрётся, перевод впишется в картинки, рядом появится переведённая глава", "", false, full) {
                let sh = shared.clone();
                std::thread::spawn(move || {
                    if let Some(p) = crate::filepick::open_chapter() {
                        sh.send(Command::TranslateFiles(p));
                    }
                });
            }
        }
    }
    ui.add_space(8.0);

    // Model.
    card(ui, "Нейросеть-переводчик", |ui| {
        let mut active = cfg.llm.active_model.clone();
        let current = cfg.active_model();
        egui::ComboBox::from_label("").selected_text(&current.name).width(ui.available_width() - 4.0).show_ui(ui, |ui| {
            let vram = crate::win::vram_gb();
            for m in &cfg.models {
                let downloaded = !m.path.is_empty() && std::path::Path::new(&m.path).exists();
                let mut label = if downloaded { m.name.clone() } else { format!("{}  · скачается {:.1} ГБ", m.name, m.size_gb) };
                if m.size_gb > 0.0 && !crate::config::fits_gpu(m.size_gb, vram) {
                    label.push_str("  · ⚠ может не поместиться в видеокарту");
                }
                ui.selectable_value(&mut active, m.id.clone(), label);
            }
        });
        if active != cfg.llm.active_model {
            shared.send(Command::SwitchModel(active));
        }
        // What this PC can run.
        match crate::win::gpu() {
            Some((name, mem)) => {
                let gb = mem as f32 / 1_073_741_824.0;
                let rec = crate::config::recommended_model(gb);
                let rec_name = cfg.models.iter().find(|m| m.id == rec).map(|m| m.name.split(" — ").next().unwrap_or(&m.name).to_string()).unwrap_or_default();
                let text = if current.id == rec {
                    format!("Ваша видеокарта: {name}, {gb:.0} ГБ — выбрана подходящая модель.")
                } else if !crate::config::fits_gpu(current.size_gb, gb) {
                    format!("Ваша видеокарта: {name}, {gb:.0} ГБ — этой модели может не хватить памяти, перевод будет медленным. Советуем {rec_name}.")
                } else {
                    format!("Ваша видеокарта: {name}, {gb:.0} ГБ — советуем {rec_name}.")
                };
                hint(ui, &text);
            }
            None => hint(ui, "Видеокарта не найдена — перевод пойдёт на процессоре; советуем Hy-MT2 1.8B."),
        }
        hint(
            ui,
            match current.id.as_str() {
                "hymt2-1.8b" => "Быстрая модель перевода, подходит слабым ПК. Иногда ошибается в смысле.",
                "hymt2-7b" => "Точный перевод, но медленнее Gemma 4. Нужна видеокарта с 6+ ГБ памяти.",
                "gemma4-e4b" => "Самый живой и естественный перевод в наших проверках. Нужна видеокарта с 6+ ГБ памяти.",
                id if id.starts_with("qwen") => "Старая модель: в проверках переводит хуже Gemma 4 и Hy-MT2.",
                _ => "",
            },
        );
        ui.horizontal(|ui| {
            if let Some(s) = &server {
                ui.label(RichText::new(s.label()).small().color(WEAK));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("Перезапустить").clicked() {
                    shared.send(Command::RestartServer);
                }
                if ui.small_button("Свой файл модели…").on_hover_text("Если модель не скачивается: скачайте файл .gguf сами и укажите его здесь").clicked() {
                    let sh = shared.clone();
                    std::thread::spawn(move || {
                        if let Some(p) = crate::filepick::pick_gguf() {
                            sh.send(Command::UseModelFile(p));
                        }
                    });
                }
            });
        });
        // The first download is slow or fails (HuggingFace unreachable): offer the manual way.
        let downloading = !std::path::Path::new(&current.path).exists();
        let stuck = match &server {
            Some(crate::state::ServerStatus::Failed(_)) => true,
            Some(crate::state::ServerStatus::Starting(m)) => m.contains("первая загрузка"),
            _ => false,
        };
        if downloading && stuck {
            hint(ui, "Не скачивается? Скачайте файл модели в браузере (файл .gguf с «Q4_K_M» в названии) и нажмите «Свой файл модели…».");
            let repo = current.hf.split(':').next().unwrap_or("");
            if !repo.is_empty() {
                ui.hyperlink_to(format!("huggingface.co/{repo}"), format!("https://huggingface.co/{repo}/tree/main"));
            }
        }
    });
    ui.add_space(8.0);

    // Stats.
    card(ui, "", |ui| {
        ui.columns(4, |c| {
            stat(&mut c[0], &format!("{:.0} мс", stats.ocr_ms_avg), "распознавание");
            stat(&mut c[1], &format!("{:.1} с", stats.llm_ms_avg / 1000.0), "перевод");
            stat(&mut c[2], &stats.translated.to_string(), "переведено");
            stat(&mut c[3], &stats.cache_hits.to_string(), "из памяти");
        });
    });

    if !notices.is_empty() {
        ui.add_space(8.0);
        card(ui, "Сообщения", |ui| {
            for n in notices.iter().rev().take(6) {
                ui.label(RichText::new(format!("• {n}")).color(Color32::from_rgb(230, 200, 170)));
            }
            if ui.small_button("Очистить").clicked() {
                shared.ui(|u| u.notices.clear());
            }
        });
    }
    false
}

fn stat(ui: &mut Ui, value: &str, label: &str) {
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(value).size(19.0).family(FontFamily::Name("bold".into())).color(accent()));
        ui.label(RichText::new(label).small().color(WEAK));
    });
}

fn page_settings(ui: &mut Ui, shared: &Arc<Shared>, query: &mut String) -> bool {
    let mut dirty = false;
    hint(ui, "Изменения применяются сразу и сохраняются автоматически.");
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(query).hint_text("🔍  Поиск по настройкам: «шрифт», «скрин», «F7»…").desired_width((ui.available_width() - 40.0).max(160.0)));
        if !query.is_empty() && ui.small_button("✕").on_hover_text("Очистить поиск").clicked() {
            query.clear();
        }
    });
    ui.add_space(8.0);
    search_set(query);
    let mut c = shared.cfg.write().unwrap();
    let mut game_changed = false;
    let mut any = false;

    any |= search_card(ui, "Вид панели", "вид панели цвет оформления тема бирюза янтарь лаванда голубой коралл свой цвет", |ui| {
        let g = &mut c.general;
        if search_hit("цвет оформления бирюза янтарь лаванда голубой коралл свой цвет") {
            ui.label("Цвет оформления");
            hint(ui, "Кнопки, выбранный раздел и переключатели. Можно взять готовый или свой любой.");
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (name, rgb) in ACCENTS {
                    let selected = g.accent == *rgb;
                    let col = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
                    ui.painter().circle_filled(rect.center(), 15.0, col);
                    if selected {
                        ui.painter().circle_stroke(rect.center(), 16.5, Stroke::new(2.5, Color32::WHITE));
                    }
                    if resp.on_hover_text(*name).clicked() && !selected {
                        g.accent = *rgb;
                        dirty = true;
                    }
                }
                ui.add_space(10.0);
                ui.label(RichText::new("Свой:").color(WEAK));
                let mut rgb = g.accent;
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    g.accent = rgb;
                    dirty = true;
                }
            });
        }
    });
    any |= search_card(ui, "Где читаете", "где читаете браузер сайт весь экран программа язык текста корейский японский китайский английский заголовок окна название тайтла манга манхва маньхуа", |ui| {
        let g = &mut c.general;
        let options: Vec<(&str, &str)> = crate::config::PROFILES.iter().map(|p| (p.id, p.name)).collect();
        let mut prof = g.profile.clone();
        if setting_row(ui, "Где читаете", "Браузер — перевод только пока браузер впереди; весь экран — любая программа для чтения", |ui| segmented(ui, &mut prof, &options)) {
            g.profile = prof;
            game_changed = true;
        }
        if setting_row(ui, "Язык текста", "Манхва — корейский, манга — японский, маньхуа — китайский", |ui| {
            segmented(ui, &mut g.source_lang, &[("ko", "Корейский"), ("ja", "Японский"), ("zh", "Китайский"), ("en", "Английский")])
        }) {
            game_changed = true;
        }
        if g.profile == "custom" {
            dirty |= setting_row(ui, "Заголовок окна", "Часть названия окна программы; несколько вариантов — через |", |ui| {
                ui.add(egui::TextEdit::singleline(&mut g.window_match).desired_width(220.0)).changed()
            });
        }
        dirty |= setting_row(ui, "Название тайтла", "Необязательно: включает словарь тайтла (Словари) и подсказывает нейросети", |ui| {
            ui.add(egui::TextEdit::singleline(&mut g.game_title).hint_text("например: Поднятие уровня в одиночку").desired_width(220.0)).changed()
        });
    });
    if game_changed {
        c.apply_profile();
    }

    any |= search_card(ui, "Перевод страницы", "как показывать замена в пузыре подложкой списком размер шрифта высоты исходного текста вертикальный японский манга столбцы рукописные помнить предыдущую страницу контекст оригинал под мышью наведение следить за прокруткой листать скрывать при прокрутке шрифт перевода комикс пожелание к стилю gemma", |ui| {
        let i = &mut c.interface;
        dirty |= setting_row(ui, "Как показывать", "Замена — перевод внутри пузыря поверх оригинала", |ui| segmented(ui, &mut i.mode, &[("replace", "Замена"), ("backdrop", "Подложка"), ("list", "Списком")]));
        dirty |= setting_row(ui, "Размер шрифта", "Относительно высоты исходного текста; если не влезает — уменьшается сам", |ui| {
            ui.add(egui::Slider::new(&mut i.font_scale, 0.5..=1.2).custom_formatter(|v, _| format!("{:.0} %", v * 100.0))).changed()
        });
        if crate::jaocr::available() {
            let was = i.vertical_ja;
            dirty |= setting_row(ui, "Вертикальный японский", "Манга: столбцы текста и рукописные надписи читает отдельная нейросеть (скачается ~440 МБ)", |ui| toggle(ui, &mut i.vertical_ja).changed());
            if i.vertical_ja && !was && !crate::jaocr::models_ready(&crate::jaocr::model_dir(shared)) {
                shared.send(Command::DownloadJaOcr);
            }
            if let Some(s) = shared.ui.lock().unwrap().ja_ocr_status.clone() {
                hint(ui, &s);
            }
        }
        dirty |= setting_row(ui, "Помнить предыдущую страницу", "Нейросеть видит последние реплики прошлой страницы: имена, обращения и тон не скачут", |ui| toggle(ui, &mut i.page_context).changed());
        dirty |= setting_row(ui, "Оригинал под мышью", "Наведите мышь на пузырь — перевод с него уйдёт и будет виден оригинал", |ui| toggle(ui, &mut i.hover_original).changed());
        dirty |= setting_row(ui, "Следить за прокруткой", "Пока листаете, перевод спрятан; страница остановилась — новые пузыри переводятся сами", |ui| toggle(ui, &mut i.auto_hide).changed());
        let d = &mut c.dialogue;
        dirty |= setting_row(ui, "Шрифт перевода", "Balsamiq Sans и Pangolin — шрифты в стиле комиксов, идут с программой", |ui| {
            let mut changed = false;
            egui::ComboBox::from_id_source("overlay_font").selected_text(d.overlay_font.clone()).show_ui(ui, |ui| {
                for f in ["Balsamiq Sans", "Pangolin", "Segoe UI", "Arial", "Tahoma", "Verdana", "Calibri", "Trebuchet MS", "Georgia", "Times New Roman", "Palatino Linotype", "Comic Sans MS"] {
                    changed |= ui.selectable_value(&mut d.overlay_font, f.to_string(), f).changed();
                }
            });
            changed
        });
        dirty |= setting_row(ui, "Пожелание к стилю", "Только для Gemma 4 и других универсальных моделей", |ui| {
            ui.add(egui::TextEdit::singleline(&mut d.style_hint).hint_text("например: проще, как в сёнэне").desired_width(220.0)).changed()
        });
    });

    any |= search_card(ui, "Прочее", "видно на скриншотах скриншот перевод использовать видеокарту тормозит модель процессор", |ui| {
        let g = &mut c.general;
        dirty |= setting_row(ui, "Видно на скриншотах", "Включайте, чтобы сделать скриншот с переводом", |ui| toggle(ui, &mut g.visible_in_screenshots).changed());
        let l = &mut c.llm;
        let mut gpu = l.gpu_layers > 0;
        if setting_row(ui, "Использовать видеокарту", "Выключите, если компьютер тормозит — модель уйдёт на процессор", |ui| toggle(ui, &mut gpu).changed()) {
            l.gpu_layers = if gpu { 99 } else { 0 };
            dirty = true;
            shared.send(Command::RestartServer);
        }
    });
    ui.add_space(8.0);

    let hk = c.hotkeys.clone();
    let keys = format!(
        "горячие клавиши {} {} {} {} {} {} перевод страницы оригинал панель стоп-кадр плохой перевод снимок config.toml",
        hk.screen_translate, hk.show_original, hk.panel, hk.freeze, hk.bad_translation, hk.save_case
    );
    any |= search_card(ui, "Горячие клавиши", &keys, |ui| {
        for (k, what) in [
            (&hk.screen_translate, "Перевести страницу / убрать перевод"),
            (&hk.show_original, "Оригинал / перевод"),
            (&hk.panel, "Открыть / закрыть эту панель"),
            (&hk.freeze, "Стоп-кадр: нажмите на текст — перевод"),
            (&hk.bad_translation, "Плохой перевод: перевести страницу заново"),
            (&hk.save_case, "Сохранить экран для проверки (разработчику)"),
        ] {
            if !search_hit(&format!("{k} {what}")) {
                continue;
            }
            ui.horizontal(|ui| {
                key_chip(ui, k);
                ui.label(what);
            });
        }
        hint(ui, "Изменить клавиши можно в config.toml (кнопка ниже), затем перезапустить программу.");
        if ui.button("Открыть config.toml").clicked() {
            shared.send(Command::Open(shared.paths.config()));
        }
    });
    drop(c);
    search_set("");
    if !any {
        hint(ui, "Ничего не найдено. Попробуйте другое слово.");
    }
    if game_changed {
        shared.send(Command::GameChanged);
    }
    dirty
}

fn page_dictionaries(ui: &mut Ui, shared: &Arc<Shared>, edits: &mut HashMap<String, String>) -> bool {
    let (recent, counts) = {
        let u = shared.ui.lock().unwrap();
        (u.recent.iter().cloned().collect::<Vec<_>>(), u.dict_counts)
    };
    hint(ui, "Так переводчик узнаёт имена, термины и ваш стиль. Файлы открываются в Блокноте; после правки нажмите «Перечитать».");
    ui.add_space(6.0);

    let w = (ui.available_width() - 16.0) / 3.0;
    ui.horizontal(|ui| {
        for (title, sub, path) in [
            ("Глоссарий", format!("{} терминов", counts.0), shared.paths.user_glossary()),
            ("Исправления OCR", format!("{} правил", counts.1), shared.paths.fixes()),
            ("Мои переводы", "исправленные реплики".to_string(), shared.paths.corrections()),
        ] {
            ui.allocate_ui(vec2(w, 90.0), |ui| {
                card(ui, title, |ui| {
                    hint(ui, &sub);
                    if ui.button("Открыть").clicked() {
                        shared.send(Command::Open(path));
                    }
                });
            });
        }
    });
    let title = cfg_title(shared);
    card(ui, "Словарь тайтла", |ui| {
        if title.is_empty() {
            hint(ui, "Имена героев одного тайтла — чтобы во всех главах они звучали одинаково. Сначала впишите название тайтла: Настройки → «Где читаете».");
        } else {
            hint(ui, &format!("«{title}»: имена героев и особые слова этого тайтла. Работает, пока в настройках указан этот тайтл."));
            if ui.button("Открыть").clicked() {
                shared.send(Command::OpenTitleDict);
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.button("⟳  Перечитать словари").clicked() {
            shared.send(Command::ReloadGlossary);
        }
        if ui.button("Экспорт…").on_hover_text("Сохранить ваш глоссарий, исправления и переводы в один файл — чтобы поделиться").clicked() {
            let sh = shared.clone();
            let (date, _) = crate::win::local_date_time();
            std::thread::spawn(move || {
                if let Some(p) = crate::filepick::save_dicts(&format!("manga-словари-{date}.txt")) {
                    sh.send(Command::ExportDicts(p));
                }
            });
        }
        if ui.button("Импорт…").on_hover_text("Добавить словари из файла другого читателя; ваши записи не пропадут").clicked() {
            let sh = shared.clone();
            std::thread::spawn(move || {
                if let Some(p) = crate::filepick::open_dicts() {
                    sh.send(Command::ImportDicts(p));
                }
            });
        }
    });
    ui.add_space(8.0);

    card(ui, "Последние реплики", |ui| {
        hint(ui, "Исправьте перевод и сохраните — эта реплика всегда будет переводиться так.");
        if recent.is_empty() {
            ui.label(RichText::new("Пока пусто.").color(WEAK));
        }
        for r in recent.iter().take(30) {
            egui::Frame::none().fill(CARD_HI).rounding(Rounding::same(8.0)).inner_margin(Margin::same(10.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(&r.src).size(16.0).color(Color32::from_rgb(200, 190, 170)));
                let e = edits.entry(r.src.clone()).or_insert_with(|| r.dst.clone());
                ui.add(egui::TextEdit::multiline(e).desired_rows(1).desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} · {} мс{}", r.model, r.ms, if r.cached { " · из памяти" } else { "" })).small().color(WEAK));
                    if *e != r.dst {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(egui::Button::new(RichText::new("Сохранить").color(Color32::BLACK)).fill(accent())).clicked() {
                                shared.send(Command::SaveCorrection { src: r.src.clone(), dst: e.clone() });
                            }
                        });
                    }
                });
            });
        }
    });
    false
}

fn page_check(ui: &mut Ui, shared: &Arc<Shared>, test_input: &mut String, ocr_view: Option<&OcrView>) -> bool {
    let (test_result, bench) = {
        let u = shared.ui.lock().unwrap();
        (u.test_result.clone(), u.bench_progress.clone())
    };
    ui.add_space(6.0);
    card(ui, "Видно ли перевод поверх браузера", |ui| {
        hint(ui, "Нажмите кнопку и сразу переключитесь в браузер (Alt+Tab). 10 секунд в центре экрана должна гореть надпись. Если её не видно — перевод тоже не будет виден: напишите об этом.");
        if ui.add(egui::Button::new(RichText::new("Показать надпись поверх окон").color(Color32::BLACK)).fill(accent())).clicked() {
            shared.send(Command::TestOverlay);
        }
    });
    ui.add_space(8.0);
    card(ui, "Самодиагностика", |ui| {
        hint(ui, "Проверит распознавание текста, нейросеть, окно браузера, окно перевода, снимок экрана и горячие клавиши. Откройте мангу перед проверкой. Отчёт откроется в Блокноте — его можно прислать разработчику.");
        if ui.add(egui::Button::new(RichText::new("Проверить всё").color(Color32::BLACK)).fill(accent())).clicked() {
            shared.send(Command::Diagnose);
        }
    });
    ui.add_space(8.0);
    card(ui, "Перевести текст", |ui| {
        hint(ui, "Вставьте корейский, японский или китайский текст — например, с сайта — и нажмите Enter.");
        let resp = ui.add(egui::TextEdit::multiline(test_input).hint_text("뭐야, 벌써 왔어?").desired_rows(2).desired_width(f32::INFINITY));
        let enter = resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
        if (ui.add(egui::Button::new(RichText::new("Перевести").color(Color32::BLACK)).fill(accent())).clicked() || enter) && !test_input.trim().is_empty() {
            shared.send(Command::TestTranslate(test_input.trim().to_string()));
        }
        if let Some(r) = &test_result {
            egui::Frame::none().fill(CARD_HI).rounding(Rounding::same(8.0)).inner_margin(Margin::same(10.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(r).size(17.0));
            });
        }
    });
    ui.add_space(8.0);
    card(ui, "Что видит распознавание", |ui| match ocr_view {
        Some(v) => {
            let w = ui.available_width().min(v.size.x);
            let h = v.size.y * w / v.size.x.max(1.0);
            ui.image(egui::load::SizedTexture::new(v.tex, vec2(w, h)));
            ui.label(RichText::new(format!("Распознано за {} мс:", v.ms)).small().color(WEAK));
            ui.label(RichText::new(if v.text.is_empty() { "(ничего)" } else { v.text.as_str() }).monospace());
            hint(ui, "Каждая строка — один найденный пузырь. Если пузыри склеились или разорвались, нажмите Ctrl+F9 и пришлите снимок разработчику.");
        }
        None => {
            hint(ui, "Нажмите F8 на странице с мангой — здесь появится то, что распознано.");
        }
    });
    ui.add_space(8.0);
    card(ui, "Сравнить модели", |ui| {
        hint(ui, "Переводит тестовые реплики всеми моделями и открывает отчёт. Модели, которых нет, будут скачаны.");
        match bench {
            Some(b) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(b);
                });
            }
            None => {
                if ui.button("Запустить сравнение").clicked() {
                    shared.send(Command::RunBench);
                }
            }
        }
    });
    false
}

fn page_help(ui: &mut Ui, shared: &Arc<Shared>) -> bool {
    let hk = shared.cfg().hotkeys;
    ui.add_space(6.0);
    card(ui, "Быстрый старт", |ui| {
        for (i, s) in [
            "В «Настройках → Где читаете» выберите язык: корейский для манхвы, японский для манги.".to_string(),
            "Откройте главу в браузере так, чтобы пузыри были видны на экране.".to_string(),
            format!("Нажмите {} — перевод появится прямо в пузырях. Повторное нажатие убирает перевод.", hk.screen_translate),
            format!("Прокрутили страницу — перевод исчезнет сам; нажмите {} ещё раз.", hk.screen_translate),
            format!("Плохой перевод? Нажмите {} — страница переведётся заново. Своё исправление можно внести в «Словари».", hk.bad_translation),
        ]
        .iter()
        .enumerate()
        {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{}.", i + 1)).color(accent()).strong());
                ui.label(s);
            });
        }
    });
    ui.add_space(8.0);
    card(ui, "Если что-то не работает", |ui| {
        for (q, a) in [
            ("Перевод не появляется", "Проверьте язык текста в «Настройках». На странице «Проверка» видно, что распознаётся."),
            ("Текст не распознаётся", "Увеличьте страницу в браузере (Ctrl и +): мелкий текст Windows читает хуже."),
            ("Компьютер тормозит", "В «Настройках → Прочее» выключите «Использовать видеокарту» или выберите модель 1.8B."),
            ("Перевод поверх браузера не виден", "Проверьте на странице «Проверка» кнопкой «Показать надпись поверх окон»."),
            ("Клавиши не работают", "Их может занимать другая программа — смотрите «Сообщения» на главной."),
        ] {
            ui.label(RichText::new(q).strong());
            hint(ui, a);
            ui.add_space(2.0);
        }
    });
    ui.add_space(8.0);
    card(ui, "Файлы", |ui| {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Папка данных").clicked() {
                shared.send(Command::Open(shared.paths.data.clone()));
            }
            if ui.button("Логи").clicked() {
                shared.send(Command::Open(shared.paths.logs()));
            }
            if ui.button("config.toml").clicked() {
                shared.send(Command::Open(shared.paths.config()));
            }
        });
        hint(ui, &format!("Данные: {}", shared.paths.data.display()));
    });
    false
}

fn cfg_title(shared: &Shared) -> String {
    shared.cfg().general.game_title.trim().to_string()
}
