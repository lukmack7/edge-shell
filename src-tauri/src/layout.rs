//! Rozmieszczenie okien na ekranie i ich pokazywanie/chowanie: pasek u góry,
//! dock na dole, szuflada z lewej, Teraz z prawej, radio obok Teraz.
//! Tu też żyje wątek, który wysuwa panele, gdy kursor dotknie krawędzi.

use crate::platform::{self, Rect};
use crate::state;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

static TOPBAR_HWND: AtomicIsize = AtomicIsize::new(0);

#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub monitor: Rect,
    pub work: Rect,
    pub scale: f64,
}

pub const BAR_H: f64 = 36.0;
pub const DOCK_H: f64 = 92.0;
const MARGIN: f64 = 12.0;
const DRAWER_W: f64 = 380.0;
const NOW_W: f64 = 400.0;

fn win(app: &AppHandle, label: &str) -> Option<WebviewWindow> {
    app.get_webview_window(label)
}

pub fn screen(app: &AppHandle) -> Screen {
    let (monitor, scale) = match app.primary_monitor().ok().flatten() {
        Some(m) => {
            let p = m.position();
            let s = m.size();
            (
                Rect { left: p.x, top: p.y, right: p.x + s.width as i32, bottom: p.y + s.height as i32 },
                m.scale_factor(),
            )
        }
        None => (Rect { left: 0, top: 0, right: 1920, bottom: 1080 }, 1.0),
    };
    let mut work = platform::work_area().unwrap_or(monitor);
    // Gdy pasek nie rezerwuje miejsca, i tak trzymamy panele pod nim.
    let bar = (BAR_H * scale).round() as i32;
    if work.top < monitor.top + bar {
        work.top = monitor.top + bar;
    }
    Screen { monitor, work, scale }
}

fn px(v: f64, s: &Screen) -> i32 {
    (v * s.scale).round() as i32
}

fn place(w: &WebviewWindow, x: i32, y: i32, width: i32, height: i32) {
    let _ = w.set_size(PhysicalSize::new(width.max(1) as u32, height.max(1) as u32));
    let _ = w.set_position(PhysicalPosition::new(x, y));
}

/// Ustawia wszystkie okna według aktualnego ekranu.
pub fn arrange(app: &AppHandle) {
    let s = screen(app);
    let cfg = state::cfg(app);

    if let Some(bar) = win(app, "topbar") {
        let h = px(BAR_H, &s);
        let mut rect = Rect { left: s.monitor.left, top: s.monitor.top, right: s.monitor.right, bottom: s.monitor.top + h };
        if cfg.shell.reserve_top_bar {
            if let Some(raw) = hwnd_of(&bar) {
                rect = platform::appbar_register(raw, rect);
                TOPBAR_HWND.store(raw, Ordering::SeqCst);
            }
        }
        place(&bar, rect.left, rect.top, rect.right - rect.left, h);
        let _ = bar.show();
    }
    // Obszar roboczy mógł się zmienić po rezerwacji paska.
    let s = screen(app);

    if let Some(dock) = win(app, "dock") {
        if cfg.shell.show_dock {
            let _ = dock.show();
        }
    }
    place_dock(app, None);

    let top = s.work.top + px(MARGIN, &s);
    let height = s.work.bottom - top - px(MARGIN + DOCK_H, &s);
    if let Some(w) = win(app, "drawer") {
        place(&w, s.work.left + px(MARGIN, &s), top, px(DRAWER_W, &s), height);
    }
    if let Some(w) = win(app, "now") {
        place(&w, s.work.right - px(MARGIN + NOW_W, &s), top, px(NOW_W, &s), height);
    }
    if let Some(w) = win(app, "friction") {
        let (fw, fh) = (px(480.0, &s), px(300.0, &s));
        place(&w, (s.work.left + s.work.right - fw) / 2, (s.work.top + s.work.bottom - fh) / 2, fw, fh);
    }
    if let Some(w) = win(app, "capture") {
        let (cw, ch) = (px(640.0, &s), px(120.0, &s));
        place(&w, (s.work.left + s.work.right - cw) / 2, s.work.top + px(120.0, &s), cw, ch);
    }
    if let Some(w) = win(app, "bubbles") {
        let (bw, bh) = (px(84.0, &s), px(160.0, &s));
        place(&w, s.work.right - bw, (s.work.top + s.work.bottom - bh) / 2, bw, bh);
    }
    if let Some(w) = win(app, "glow") {
        place(&w, s.monitor.right - px(12.0, &s), s.work.top, px(12.0, &s), s.work.bottom - s.work.top);
        let _ = w.set_ignore_cursor_events(true);
    }
    place_radio(app);
    sync_bubbles(app);
}

/// Dock ustala szerokość sam (po wyrenderowaniu ikon) i prosi o nią komendą.
pub fn place_dock(app: &AppHandle, logical_width: Option<f64>) {
    let Some(dock) = win(app, "dock") else { return };
    let s = screen(app);
    let width = match logical_width {
        Some(w) => px(w, &s),
        None => dock.outer_size().map(|z| z.width as i32).unwrap_or(px(820.0, &s)),
    };
    let h = px(DOCK_H, &s);
    let x = (s.monitor.left + s.monitor.right - width) / 2;
    let y = s.work.bottom - h - px(6.0, &s);
    place(&dock, x, y, width, h);
}

pub fn place_radio(app: &AppHandle) {
    let Some(w) = win(app, "radio") else { return };
    let s = screen(app);
    let now_open = state::read(app, |rt| rt.now_open);
    let (rw, rh) = (px(380.0, &s), px(330.0, &s));
    let right = if now_open { s.work.right - px(MARGIN * 2.0 + NOW_W, &s) } else { s.work.right - px(MARGIN * 2.0, &s) };
    place(&w, right - rw, s.work.top + px(MARGIN, &s), rw, rh);
}

#[cfg(windows)]
fn hwnd_of(w: &WebviewWindow) -> Option<isize> {
    w.hwnd().ok().map(|h| h.0 as isize)
}

#[cfg(not(windows))]
fn hwnd_of(_: &WebviewWindow) -> Option<isize> {
    None
}

pub fn release(app: &AppHandle) {
    let raw = TOPBAR_HWND.swap(0, Ordering::SeqCst);
    if raw != 0 {
        platform::appbar_remove(raw);
    }
    if state::cfg(app).shell.hide_taskbar {
        let restore = state::read(app, |rt| rt.store.taskbar_state);
        crate::sys::hide_taskbar(false, restore);
    }
}

// ---------------------------------------------------------------------------
// Panele
// ---------------------------------------------------------------------------

pub fn set_panel(app: &AppHandle, label: &str, open: bool) {
    let Some(w) = win(app, label) else { return };
    if open {
        let _ = w.show();
        let _ = w.set_focus();
    } else {
        let _ = w.hide();
    }
    state::update(app, false, |rt| match label {
        "drawer" => rt.drawer_open = open,
        "now" => rt.now_open = open,
        _ => {}
    });
    if label == "now" {
        place_radio(app);
        sync_bubbles(app);
    }
}

static BLUR_CLOSED: std::sync::Mutex<Option<(&'static str, Instant)>> = std::sync::Mutex::new(None);

/// Panel traci fokus → chowa się. Zapamiętujemy moment, bo kliknięcie
/// przycisku, który miał go zamknąć, przychodzi zaraz po utracie fokusu.
pub fn blur_close(app: &AppHandle, label: &'static str) {
    if let Ok(mut g) = BLUR_CLOSED.lock() {
        *g = Some((label, Instant::now()));
    }
    set_panel(app, label, false);
}

pub fn toggle_panel(app: &AppHandle, label: &str) {
    let just_closed = BLUR_CLOSED
        .lock()
        .ok()
        .and_then(|g| *g)
        .is_some_and(|(l, t)| l == label && t.elapsed() < Duration::from_millis(350));
    if just_closed {
        return;
    }
    let open = state::read(app, |rt| match label {
        "drawer" => rt.drawer_open,
        "now" => rt.now_open,
        _ => false,
    });
    set_panel(app, label, !open);
}

/// Dymki są widoczne tylko wtedy, gdy panel Teraz jest schowany.
pub fn sync_bubbles(app: &AppHandle) {
    let Some(w) = win(app, "bubbles") else { return };
    let show = state::cfg(app).shell.show_bubbles && !state::read(app, |rt| rt.now_open);
    let _ = if show { w.show() } else { w.hide() };
}

pub fn show_window(app: &AppHandle, label: &str, focus: bool) {
    if let Some(w) = win(app, label) {
        let _ = w.show();
        if focus {
            let _ = w.set_focus();
        }
    }
}

pub fn hide_window(app: &AppHandle, label: &str) {
    if let Some(w) = win(app, label) {
        let _ = w.hide();
    }
}

pub fn set_glow(app: &AppHandle, on: bool) {
    if on {
        show_window(app, "glow", false);
    } else {
        hide_window(app, "glow");
    }
}

// ---------------------------------------------------------------------------
// Krawędzie
// ---------------------------------------------------------------------------

/// Wątek sprawdzający kursor: dotknięcie lewej krawędzi wysuwa szufladę,
/// prawej — panel Teraz. Panel wysunięty najazdem chowa się, gdy kursor
/// odjedzie od niego.
pub fn start_edges(app: AppHandle) {
    std::thread::spawn(move || {
        let mut left_since: Option<Instant> = None;
        let mut right_since: Option<Instant> = None;
        let mut hover_opened: Option<&'static str> = None;
        let mut away_since: Option<Instant> = None;
        let mut s = screen(&app);
        let mut delay = Duration::from_millis(state::cfg(&app).shell.edge_delay_ms);
        let mut refreshed = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(40));
            // Ekran i opóźnienie odświeżamy co 2 s, nie przy każdym ruchu.
            if refreshed.elapsed() > Duration::from_secs(2) {
                s = screen(&app);
                delay = Duration::from_millis(state::cfg(&app).shell.edge_delay_ms);
                refreshed = Instant::now();
            }
            let Some((x, y)) = platform::cursor() else { continue };
            let in_band = y > s.work.top + px(40.0, &s) && y < s.work.bottom - px(DOCK_H + 20.0, &s);

            let at_left = in_band && x <= s.monitor.left;
            let at_right = in_band && x >= s.monitor.right - 1;
            left_since = if at_left { left_since.or(Some(Instant::now())) } else { None };
            right_since = if at_right { right_since.or(Some(Instant::now())) } else { None };

            let (drawer_open, now_open) = state::read(&app, |rt| (rt.drawer_open, rt.now_open));
            if left_since.is_some_and(|t| t.elapsed() >= delay) && !drawer_open {
                set_panel(&app, "drawer", true);
                hover_opened = Some("drawer");
                left_since = None;
            }
            if right_since.is_some_and(|t| t.elapsed() >= delay) && !now_open {
                set_panel(&app, "now", true);
                hover_opened = Some("now");
                right_since = None;
            }

            // Chowanie po odjechaniu kursora (tylko dla otwartych najazdem).
            if let Some(label) = hover_opened {
                let still_open = state::read(&app, |rt| if label == "drawer" { rt.drawer_open } else { rt.now_open });
                if !still_open {
                    hover_opened = None;
                    away_since = None;
                    continue;
                }
                let far = if label == "drawer" {
                    x > s.work.left + px(MARGIN + DRAWER_W + 60.0, &s)
                } else {
                    x < s.work.right - px(MARGIN + NOW_W + 60.0, &s)
                };
                away_since = if far { away_since.or(Some(Instant::now())) } else { None };
                if away_since.is_some_and(|t| t.elapsed() >= Duration::from_millis(600)) {
                    set_panel(&app, label, false);
                    hover_opened = None;
                    away_since = None;
                }
            }
        }
    });
}
