//! Global hotkeys via RegisterHotKey on a dedicated message-loop thread.
//! Note: a registered key is consumed by Windows and does not reach the game,
//! so pick keys the game does not use (configurable in config.toml).

use crate::config::Hotkeys;
use crate::state::Command;
use crate::win::parse_hotkey;
use tokio::sync::mpsc::UnboundedSender;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT, VK_DOWN, VK_END, VK_HOME, VK_NEXT, VK_PRIOR, VK_SPACE, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, HHOOK, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_HOTKEY,
    WM_KEYDOWN, WM_MOUSEHWHEEL, WM_MOUSEWHEEL,
};

static CLOCK: OnceLock<Instant> = OnceLock::new();
static LAST_SCROLL: AtomicU64 = AtomicU64::new(0);

/// Milliseconds on the app's own clock (for comparing with `last_scroll_ms`).
pub fn now_ms() -> u64 {
    CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// When the user last turned the mouse wheel or pressed a scrolling key
/// (PageUp/PageDown, arrows, Space, Home/End) anywhere; 0 = never.
pub fn last_scroll_ms() -> u64 {
    LAST_SCROLL.load(Ordering::Relaxed)
}

// Low-level hooks only note the time; they must return fast or Windows drops them.
unsafe extern "system" fn mouse_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 && (w.0 as u32 == WM_MOUSEWHEEL || w.0 as u32 == WM_MOUSEHWHEEL) {
        LAST_SCROLL.store(now_ms(), Ordering::Relaxed);
    }
    CallNextHookEx(HHOOK::default(), code, w, l)
}

unsafe extern "system" fn key_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 && w.0 as u32 == WM_KEYDOWN {
        let k = (*(l.0 as *const KBDLLHOOKSTRUCT)).vkCode as u16;
        if [VK_NEXT, VK_PRIOR, VK_SPACE, VK_UP, VK_DOWN, VK_HOME, VK_END].iter().any(|v| v.0 == k) {
            LAST_SCROLL.store(now_ms(), Ordering::Relaxed);
        }
    }
    CallNextHookEx(HHOOK::default(), code, w, l)
}

/// Returns human-readable problems (unparsable or already taken keys).
pub fn spawn(keys: &Hotkeys, tx: UnboundedSender<Command>) -> std::sync::mpsc::Receiver<Vec<String>> {
    let table: Vec<(String, Command)> = vec![
        (keys.screen_translate.clone(), Command::ScreenTranslate),
        (keys.show_original.clone(), Command::ToggleOriginal),
        (keys.panel.clone(), Command::TogglePanel),
        (keys.save_case.clone(), Command::SaveCase),
        (keys.freeze.clone(), Command::Freeze),
        (keys.bad_translation.clone(), Command::BadTranslation),
    ];
    let (rep_tx, rep_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("hotkeys".into())
        .spawn(move || unsafe {
            let mut problems = Vec::new();
            let mut ids: Vec<(i32, Command)> = Vec::new();
            for (i, (key, cmd)) in table.into_iter().enumerate() {
                if key.trim().is_empty() {
                    continue;
                }
                let Some((mods, vk)) = parse_hotkey(&key) else {
                    problems.push(format!("Не распознана горячая клавиша «{key}»"));
                    continue;
                };
                let id = 100 + i as i32;
                let m = HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT;
                if RegisterHotKey(HWND::default(), id, m, vk).is_err() {
                    problems.push(format!("Клавиша «{key}» уже занята другой программой"));
                    continue;
                }
                ids.push((id, cmd));
            }
            let _ = rep_tx.send(problems);
            // Scrolling is noticed at once (the page translation hides before the
            // page moves away from under it); this thread's message loop serves them.
            now_ms();
            let module = GetModuleHandleW(None).unwrap_or_default();
            if let Err(e) = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0) {
                tracing::warn!("перехват колёсика мыши: {e}");
            }
            if let Err(e) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_hook), module, 0) {
                tracing::warn!("перехват клавиш прокрутки: {e}");
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
                if msg.message == WM_HOTKEY {
                    let id = msg.wParam.0 as i32;
                    if let Some((_, c)) = ids.iter().find(|(i, _)| *i == id) {
                        if tx.send(c.clone()).is_err() {
                            break;
                        }
                    }
                }
            }
        })
        .expect("hotkey thread");
    rep_rx
}
