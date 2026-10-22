//! Rozmieszczenie okien na ekranie i ich pokazywanie/chowanie: pasek u góry
//! (na każdym monitorze), stały pasek z lewej, dock na dole, szuflada obok
//! lewego paska, Teraz z prawej, radio obok Teraz.
//! Tu też żyje wątek, który wysuwa panele, gdy kursor dotknie krawędzi.

use crate::platform::{self, Rect};
use crate::state;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

/// Gdzie stoi dock (do autoukrywania).
static DOCK_RECT: Mutex<Rect> = Mutex::new(Rect { left: 0, top: 0, right: 0, bottom: 0 });
static DOCK_VISIBLE: AtomicBool = AtomicBool::new(true);
/// Podglądy okien albo stos folderu są otwarte — dock ma zostać wysunięty.
pub static DOCK_HOLD: AtomicBool = AtomicBool::new(false);
/// Monitor zajęty przez aplikację pełnoekranową.
static FULLSCREEN: Mutex<Option<Rect>> = Mutex::new(None);

/// Okna zarejestrowane jako paski systemowe (do zwolnienia przy wyjściu).
static APPBARS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub monitor: Rect,
    pub work: Rect,
    pub scale: f64,
}

pub const BAR_H: f64 = 36.0;
pub const DOCK_H: f64 = 92.0;
pub const RAIL_W: f64 = 56.0;
const MARGIN: f64 = 12.0;
const DRAWER_W: f64 = 380.0;
const NOW_W: f64 = 400.0;
const ABE_LEFT: u32 = 0;
const ABE_TOP: u32 = 1;

fn win(app: &AppHandle, label: &str) -> Option<WebviewWindow> {
    app.get_webview_window(label)
}

fn rect_of(m: &tauri::Monitor) -> Rect {
    let p = m.position();
    let s = m.size();
    Rect { left: p.x, top: p.y, right: p.x + s.width as i32, bottom: p.y + s.height as i32 }
}

pub fn screen(app: &AppHandle) -> Screen {
    let (monitor, scale) = match app.primary_monitor().ok().flatten() {
        Some(m) => (rect_of(&m), m.scale_factor()),
        None => (Rect { left: 0, top: 0, right: 1920, bottom: 1080 }, 1.0),
    };
    let mut work = platform::work_area().unwrap_or(monitor);
    // Gdy paski nie rezerwują miejsca, i tak trzymamy panele obok nich.
    let bar = (BAR_H * scale).round() as i32;
    if work.top < monitor.top + bar {
        work.top = monitor.top + bar;
    }
    let cfg = state::cfg(app);
    if cfg.shell.show_rail {
        let rail = monitor.left + (RAIL_W * scale).round() as i32;
        if work.left < rail {
            work.left = rail;
        }
    }
    Screen { monitor, work, scale }
}

/// Monitory poza głównym, w stałej kolejności (po pozycji).
fn secondary_monitors(app: &AppHandle) -> Vec<(Rect, f64)> {
    let primary = app.primary_monitor().ok().flatten().map(|m| rect_of(&m));
    let mut list: Vec<(Rect, f64)> = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| (rect_of(m), m.scale_factor()))
        .filter(|(r, _)| primary.map_or(true, |p| p.left != r.left || p.top != r.top))
        .collect();
    list.sort_by_key(|(r, _)| (r.left, r.top));
    list
}

/// Podpis układu monitorów — do wykrywania podłączenia/odłączenia ekranu.
pub fn monitors_signature(app: &AppHandle) -> String {
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let r = rect_of(m);
            format!("{},{},{},{}", r.left, r.top, r.right, r.bottom)
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn px(v: f64, s: &Screen) -> i32 {
    (v * s.scale).round() as i32
}

fn place(w: &WebviewWindow, x: i32, y: i32, width: i32, height: i32) {
    let _ = w.set_size(PhysicalSize::new(width.max(1) as u32, height.max(1) as u32));
    let _ = w.set_position(PhysicalPosition::new(x, y));
}

fn register_appbar(w: &WebviewWindow, rect: Rect, edge: u32) -> Rect {
    let Some(raw) = hwnd_of(w) else { return rect };
    if let Ok(mut list) = APPBARS.lock() {
        if !list.contains(&raw) {
            list.push(raw);
        }
    }
    platform::appbar_register(raw, rect, edge)
}

fn unregister_appbar(w: &WebviewWindow) {
    let Some(raw) = hwnd_of(w) else { return };
    if let Ok(mut list) = APPBARS.lock() {
        if let Some(pos) = list.iter().position(|r| *r == raw) {
            list.remove(pos);
            platform::appbar_remove(raw);
        }
    }
}

fn place_topbar(w: &WebviewWindow, monitor: Rect, scale: f64, reserve: bool) {
    let h = (BAR_H * scale).round() as i32;
    let mut rect = Rect { left: monitor.left, top: monitor.top, right: monitor.right, bottom: monitor.top + h };
    if reserve {
        rect = register_appbar(w, rect, ABE_TOP);
    } else {
        unregister_appbar(w);
    }
    place(w, rect.left, rect.top, rect.right - rect.left, h);
    let _ = w.show();
}

/// Paski na pozostałych monitorach: tworzy brakujące okna, zamyka zbędne.
fn arrange_secondary_bars(app: &AppHandle, reserve: bool, enabled: bool) {
    let monitors = if enabled { secondary_monitors(app) } else { Vec::new() };
    let mut missing = false;
    for (i, (rect, scale)) in monitors.iter().enumerate() {
        let label = format!("topbar-{}", i + 1);
        match win(app, &label) {
            Some(w) => place_topbar(&w, *rect, *scale, reserve),
            None => missing = true,
        }
    }
    // Zbędne paski (odłączony monitor albo wyłączona opcja).
    for (label, w) in app.webview_windows() {
        if let Some(n) = label.strip_prefix("topbar-").and_then(|n| n.parse::<usize>().ok()) {
            if n > monitors.len() {
                unregister_appbar(&w);
                let _ = w.destroy();
            }
        }
    }
    if missing {
        // Okna tworzymy w wątku głównym (inaczej Windows potrafi się zawiesić).
        let app2 = app.clone();
        let count = monitors.len();
        let _ = app.run_on_main_thread(move || {
            for i in 1..=count {
                let label = format!("topbar-{i}");
                if app2.get_webview_window(&label).is_some() {
                    continue;
                }
                let _ = tauri::WebviewWindowBuilder::new(&app2, &label, tauri::WebviewUrl::App("topbar.html".into()))
                    .title("Edge Shell — pasek")
                    .decorations(false)
                    .transparent(true)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    .resizable(false)
                    .shadow(false)
                    .focused(false)
                    .visible(false)
                    .build();
            }
            let app3 = app2.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                arrange(&app3);
            });
        });
    }
}

/// Ustawia wszystkie okna według aktualnych ekranów.
pub fn arrange(app: &AppHandle) {
    let cfg = state::cfg(app);
    let s = screen(app);

    if let Some(bar) = win(app, "topbar") {
        place_topbar(&bar, s.monitor, s.scale, cfg.shell.reserve_top_bar);
    }
    arrange_secondary_bars(app, cfg.shell.reserve_top_bar, cfg.shell.bar_on_all_monitors);

    if let Some(rail) = win(app, "rail") {
        if cfg.shell.show_rail {
            let w = px(RAIL_W, &s);
            let top = s.monitor.top + px(BAR_H, &s);
            let mut rect = Rect { left: s.monitor.left, top, right: s.monitor.left + w, bottom: s.monitor.bottom };
            if cfg.shell.reserve_top_bar {
                rect = register_appbar(&rail, rect, ABE_LEFT);
            }
            place(&rail, rect.left, top, w, s.monitor.bottom - top);
            let _ = rail.show();
        } else {
            unregister_appbar(&rail);
            let _ = rail.hide();
        }
    }

    // Obszar roboczy mógł się zmienić po rezerwacji pasków.
    let s = screen(app);

    place_dock(app, None);
    let dock_on = cfg.shell.show_dock && !cfg.shell.dock_autohide;
    DOCK_VISIBLE.store(!dock_on, Ordering::Relaxed);
    set_dock_visible(app, dock_on);

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
    if let Some(w) = win(app, "report") {
        let (fw, fh) = (px(960.0, &s), px(660.0, &s));
        place(&w, (s.work.left + s.work.right - fw) / 2, (s.work.top + s.work.bottom - fh) / 2, fw, fh);
    }
    if let Some(w) = win(app, "calendar") {
        let (cw, ch) = (px(400.0, &s), px(600.0, &s));
        place(&w, s.work.right - cw - px(MARGIN, &s), s.work.top + px(6.0, &s), cw, ch);
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
    let fs = FULLSCREEN.lock().ok().and_then(|g| *g);
    if let Some(fs) = fs {
        hide_covered(app, fs);
    }
}

fn inside(r: Rect, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Chowa nasze okna leżące na monitorze zajętym przez pełny ekran.
fn hide_covered(app: &AppHandle, fs: Rect) {
    for (label, w) in app.webview_windows() {
        let ours = label == "topbar" || label.starts_with("topbar-") || matches!(label.as_str(), "rail" | "dock" | "bubbles" | "preview" | "stack");
        if !ours {
            continue;
        }
        if let Ok(p) = w.outer_position() {
            if inside(fs, p.x + 2, p.y + 2) {
                let _ = w.hide();
            }
        }
    }
}

fn set_fullscreen(app: &AppHandle, fs: Option<Rect>) {
    if let Ok(mut g) = FULLSCREEN.lock() {
        *g = fs;
    }
    match fs {
        Some(r) => {
            hide_covered(app, r);
            DOCK_VISIBLE.store(false, Ordering::Relaxed);
            DOCK_HOLD.store(false, Ordering::Relaxed);
        }
        None => arrange(app),
    }
}

/// Pokazuje/chowa dock; przy wysunięciu dock dostaje krótką animację.
pub fn set_dock_visible(app: &AppHandle, visible: bool) {
    if DOCK_VISIBLE.swap(visible, Ordering::Relaxed) == visible {
        return;
    }
    let Some(dock) = win(app, "dock") else { return };
    if visible {
        let _ = dock.show();
        let _ = app.emit_to("dock", "dock-reveal", ());
    } else {
        let _ = dock.hide();
        DOCK_HOLD.store(false, Ordering::Relaxed);
        if let Some(pv) = win(app, "preview") {
            let _ = pv.hide();
        }
        if let Some(st) = win(app, "stack") {
            let _ = st.hide();
        }
    }
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
    let x = (s.work.left + s.work.right - width) / 2;
    let y = s.work.bottom - h - px(6.0, &s);
    place(&dock, x, y, width, h);
    if let Ok(mut r) = DOCK_RECT.lock() {
        *r = Rect { left: x, top: y, right: x + width, bottom: y + h };
    }
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
    if let Ok(mut list) = APPBARS.lock() {
        for raw in list.drain(..) {
            platform::appbar_remove(raw);
        }
    }
    crate::tray::stop();
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
        let mut monitors = monitors_signature(&app);
        let mut shell = state::cfg(&app).shell;
        let mut fs_checked = Instant::now();
        let mut last_fs: Option<Rect> = None;
        let mut dock_edge_since: Option<Instant> = None;
        let mut dock_away_since: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_millis(40));
            // Ekran i opóźnienie odświeżamy co 2 s, nie przy każdym ruchu.
            if refreshed.elapsed() > Duration::from_secs(2) {
                let sig = monitors_signature(&app);
                if sig != monitors {
                    monitors = sig;
                    arrange(&app);
                }
                s = screen(&app);
                shell = state::cfg(&app).shell;
                delay = Duration::from_millis(shell.edge_delay_ms);
                refreshed = Instant::now();
            }

            // Pełny ekran: sprawdzane co ćwierć sekundy.
            if fs_checked.elapsed() > Duration::from_millis(250) {
                fs_checked = Instant::now();
                let fs = if shell.hide_in_fullscreen { platform::fullscreen_monitor() } else { None };
                if fs != last_fs {
                    last_fs = fs;
                    set_fullscreen(&app, fs);
                }
            }
            let Some((x, y)) = platform::cursor() else { continue };

            // Autoukrywanie docka: wysuwa się od razu po dotknięciu dolnej
            // krawędzi pod dockiem, chowa pół sekundy po odjechaniu kursora.
            let dock_fs = last_fs.is_some_and(|f| inside(f, s.monitor.left + 2, s.monitor.top + 2));
            if shell.show_dock && shell.dock_autohide && !dock_fs {
                let r = DOCK_RECT.lock().map(|g| *g).unwrap_or_default();
                let visible = DOCK_VISIBLE.load(Ordering::Relaxed);
                if !visible {
                    let at_edge = y >= s.monitor.bottom - 2 && x >= r.left && x <= r.right;
                    dock_edge_since = if at_edge { dock_edge_since.or(Some(Instant::now())) } else { None };
                    if dock_edge_since.is_some_and(|t| t.elapsed() >= Duration::from_millis(60)) {
                        set_dock_visible(&app, true);
                        dock_edge_since = None;
                        dock_away_since = None;
                    }
                } else {
                    let pad = px(24.0, &s);
                    let near = x >= r.left - pad && x <= r.right + pad && y >= r.top - pad;
                    let hold = DOCK_HOLD.load(Ordering::Relaxed);
                    dock_away_since = if near || hold { None } else { dock_away_since.or(Some(Instant::now())) };
                    if dock_away_since.is_some_and(|t| t.elapsed() >= Duration::from_millis(500)) {
                        set_dock_visible(&app, false);
                        dock_away_since = None;
                    }
                }
            }
            // Na pełnym ekranie krawędzie nie wysuwają paneli (np. w grach).
            if dock_fs {
                left_since = None;
                right_since = None;
                continue;
            }
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
