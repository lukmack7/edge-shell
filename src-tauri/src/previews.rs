//! Podglądy okien nad dockiem: żywe miniatury DWM wszystkich okien aplikacji.

use crate::platform::{self, AppWindow};
use crate::{state, sys};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

static THUMBS: Mutex<Vec<isize>> = Mutex::new(Vec::new());
/// Dolna krawędź karty metryki (px fizyczne) — wysokość okna dopasowuje się do treści w górę.
static METRIC_BOTTOM: Mutex<Option<i32>> = Mutex::new(None);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PreviewWin {
    pub hwnd: isize,
    pub title: String,
}

#[derive(Deserialize)]
pub struct Slot {
    pub hwnd: isize,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Jedna pozycja menu kontekstowego docka.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextItem {
    pub action: String,
    pub label: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub danger: bool,
    #[serde(default)]
    pub separator: bool,
    #[serde(default)]
    pub section: bool,
}

/// Okna danej aplikacji z docka (po nazwie procesu albo pliku .exe).
pub fn windows_of(app: &AppHandle, id: &str) -> Vec<AppWindow> {
    if let Some(exe) = id.strip_prefix("run:") {
        return platform::app_windows().into_iter().filter(|w| platform::exe_name(&w.exe) == exe).collect();
    }
    let cfg = state::cfg(app);
    let Some(a) = cfg.dock.iter().find(|a| a.id == id) else { return Vec::new() };
    let names = a.exe_names();
    if names.is_empty() {
        return Vec::new();
    }
    platform::app_windows()
        .into_iter()
        .filter(|w| names.contains(&platform::exe_name(&w.exe)))
        .collect()
}

fn clear_thumbs() {
    if let Ok(mut b) = METRIC_BOTTOM.lock() {
        *b = None;
    }
    if let Ok(mut t) = THUMBS.lock() {
        for th in t.drain(..) {
            sys::thumb_unregister(th);
        }
    }
}

/// Pokazuje okno podglądów nad ikoną (anchor = środek ikony w px CSS docka).
#[tauri::command]
pub fn previews_show(app: AppHandle, id: String, anchor: f64) -> usize {
    let wins = windows_of(&app, &id);
    let (Some(pv), Some(dock)) = (app.get_webview_window("preview"), app.get_webview_window("dock")) else { return 0 };
    if wins.is_empty() {
        // Brak okien: zostaje sama podpowiedź z nazwą.
        return 0;
    }
    crate::layout::DOCK_HOLD.store(true, std::sync::atomic::Ordering::Relaxed);
    let scale = dock.scale_factor().unwrap_or(1.0);
    let n = wins.len().min(6) as f64;
    let (w, h) = (((n * 236.0) + 24.0) * scale, 196.0 * scale);
    let dpos = dock.outer_position().unwrap_or(PhysicalPosition::new(0, 0));
    let x = dpos.x as f64 + anchor * scale - w / 2.0;
    let y = dpos.y as f64 - h + 6.0 * scale;
    clear_thumbs();
    let _ = pv.set_size(PhysicalSize::new(w as u32, h as u32));
    let _ = pv.set_position(PhysicalPosition::new(x.max(0.0) as i32, y as i32));
    let list: Vec<PreviewWin> = wins.into_iter().take(6).map(|w| PreviewWin { hwnd: w.hwnd, title: w.title }).collect();
    let count = list.len();
    let _ = app.emit_to("preview", "previews", &list);
    let _ = pv.show();
    count
}

/// Okno podglądów zgłasza, gdzie narysowało karty — tam kładziemy miniatury.
#[tauri::command]
pub fn previews_layout(app: AppHandle, slots: Vec<Slot>) {
    let Some(pv) = app.get_webview_window("preview") else { return };
    let Some(dest) = hwnd_of(&pv) else { return };
    let scale = pv.scale_factor().unwrap_or(1.0);
    clear_thumbs();
    let mut made = Vec::new();
    for s in slots {
        if let Some(th) = sys::thumb_register(dest, s.hwnd) {
            sys::thumb_place(
                th,
                (s.x * scale) as i32,
                (s.y * scale) as i32,
                ((s.x + s.w) * scale) as i32,
                ((s.y + s.h) * scale) as i32,
            );
            made.push(th);
        }
    }
    if let Ok(mut t) = THUMBS.lock() {
        *t = made;
    }
}

/// Podpowiedź z nazwą aplikacji nad ikoną docka (w oknie podglądów, bo okno
/// docka ma wysokość ikon i dymek by się w nim nie zmieścił).
#[tauri::command]
pub fn previews_tip(app: AppHandle, text: String, anchor: f64, drop_error: Option<bool>) {
    let (Some(pv), Some(dock)) = (app.get_webview_window("preview"), app.get_webview_window("dock")) else { return };
    clear_thumbs();
    let scale = dock.scale_factor().unwrap_or(1.0);
    let lines = text.lines().map(|l| l.chars().count() / 44 + 1).sum::<usize>().max(1) as f64;
    let (width, height) = if drop_error.is_some() { (380.0, 112.0) } else { (320.0, 24.0 + 16.0 * lines) };
    let dpos = dock.outer_position().unwrap_or(PhysicalPosition::new(0, 0));
    let monitor = crate::layout::screen(&app).monitor;
    let w = (width * scale).min((monitor.right - monitor.left).max(1) as f64);
    let h = height * scale;
    let x = dpos.x as f64 + anchor * scale - w / 2.0;
    let y = dpos.y as f64 - h + if drop_error.is_some() { -4.0 * scale } else { 8.0 * scale };
    let _ = pv.set_size(PhysicalSize::new(w as u32, h as u32));
    let x = x.clamp(monitor.left as f64, monitor.right as f64 - w);
    let _ = pv.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    let _ = app.emit_to("preview", "preview-tip", serde_json::json!({ "text": text, "dropError": drop_error }));
    let _ = pv.show();
}

/// Karta metryki (CPU/RAM/bateria) nad dockiem. To osobne okno, więc dock nie zmienia
/// rozmiaru. Pozycjonujemy je tu, a pokazujemy dopiero po dopasowaniu do treści.
#[tauri::command]
pub fn previews_metric(app: AppHandle, anchor: f64) {
    let (Some(pv), Some(dock)) = (app.get_webview_window("preview"), app.get_webview_window("dock")) else { return };
    clear_thumbs();
    crate::layout::DOCK_HOLD.store(true, std::sync::atomic::Ordering::Relaxed);
    let scale = dock.scale_factor().unwrap_or(1.0);
    let s = crate::layout::screen(&app);
    let w = 268.0 * scale;
    let dpos = dock.outer_position().unwrap_or(PhysicalPosition::new(0, 0));
    let x = (dpos.x as f64 + anchor * scale - w / 2.0).clamp(0.0, (s.monitor.right as f64 - w).max(0.0));
    let bottom = dpos.y + (14.0 * scale) as i32;
    if let Ok(mut b) = METRIC_BOTTOM.lock() {
        *b = Some(bottom);
    }
    let h = (260.0 * scale) as i32;
    let _ = pv.set_size(PhysicalSize::new(w as u32, h as u32));
    let _ = pv.set_position(PhysicalPosition::new(x as i32, bottom - h));
}

/// Okno karty dopasowuje wysokość do treści (px CSS); dolna krawędź stoi w miejscu.
#[tauri::command]
pub fn previews_metric_fit(app: AppHandle, height: f64) {
    let Some(pv) = app.get_webview_window("preview") else { return };
    let Some(bottom) = METRIC_BOTTOM.lock().ok().and_then(|b| *b) else { return };
    let scale = pv.scale_factor().unwrap_or(1.0);
    let (Ok(pos), Ok(size)) = (pv.outer_position(), pv.outer_size()) else { return };
    let h = (height.clamp(60.0, 640.0) * scale) as i32;
    let y = bottom - h;
    let done = hwnd_of(&pv).is_some_and(|raw| platform::set_rect(raw, pos.x, y, size.width as i32, h));
    if !done {
        let _ = pv.set_size(PhysicalSize::new(size.width, h as u32));
        let _ = pv.set_position(PhysicalPosition::new(pos.x, y));
    }
    let _ = pv.show();
}

/// Menu kontekstowe aplikacji nad dockiem. Rysuje je `preview.html`, dzięki
/// czemu menu nie jest ograniczone wysokością okna docka.
#[tauri::command]
pub fn previews_context(app: AppHandle, anchor: f64, items: Vec<ContextItem>) {
    let (Some(pv), Some(dock)) = (app.get_webview_window("preview"), app.get_webview_window("dock")) else { return };
    clear_thumbs();
    crate::layout::DOCK_HOLD.store(true, std::sync::atomic::Ordering::Relaxed);
    let scale = dock.scale_factor().unwrap_or(1.0);
    let s = crate::layout::screen(&app);
    let w = 340.0 * scale;
    let logical_h: f64 = items
        .iter()
        .map(|item| if item.section { 24.0 } else { 34.0 } + if item.separator { 6.0 } else { 0.0 })
        .sum::<f64>()
        + 12.0;
    let h = logical_h * scale;
    let dpos = dock.outer_position().unwrap_or(PhysicalPosition::new(0, 0));
    let x = (dpos.x as f64 + anchor * scale - w / 2.0).clamp(0.0, (s.monitor.right as f64 - w).max(0.0));
    let y = (dpos.y as f64 - h + 8.0 * scale).max(s.monitor.top as f64 + 6.0 * scale);
    let _ = pv.set_size(PhysicalSize::new(w as u32, h as u32));
    let _ = pv.set_position(PhysicalPosition::new(x as i32, y as i32));
    let _ = app.emit_to("preview", "preview-context", &items);
    let _ = pv.show();
}

#[tauri::command]
pub fn previews_hide(app: AppHandle) {
    clear_thumbs();
    crate::layout::DOCK_HOLD.store(false, std::sync::atomic::Ordering::Relaxed);
    if let Some(pv) = app.get_webview_window("preview") {
        let _ = pv.hide();
    }
    let _ = app.emit_to("dock", "preview-hidden", ());
}

#[tauri::command]
pub fn window_activate(app: AppHandle, hwnd: isize) {
    previews_hide(app);
    sys::activate(hwnd);
}

#[tauri::command]
pub fn window_close(hwnd: isize) {
    sys::close_window(hwnd);
}

#[cfg(windows)]
fn hwnd_of(w: &tauri::WebviewWindow) -> Option<isize> {
    w.hwnd().ok().map(|h| h.0 as isize)
}

#[cfg(not(windows))]
fn hwnd_of(_: &tauri::WebviewWindow) -> Option<isize> {
    None
}
