//! Podglądy okien nad dockiem: żywe miniatury DWM wszystkich okien aplikacji.

use crate::platform::{self, AppWindow};
use crate::{state, sys};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

static THUMBS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

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
pub fn previews_tip(app: AppHandle, text: String, anchor: f64) {
    let (Some(pv), Some(dock)) = (app.get_webview_window("preview"), app.get_webview_window("dock")) else { return };
    clear_thumbs();
    let scale = dock.scale_factor().unwrap_or(1.0);
    let (w, h) = (320.0 * scale, 40.0 * scale);
    let dpos = dock.outer_position().unwrap_or(PhysicalPosition::new(0, 0));
    let x = dpos.x as f64 + anchor * scale - w / 2.0;
    let y = dpos.y as f64 - h + 8.0 * scale;
    let _ = pv.set_size(PhysicalSize::new(w as u32, h as u32));
    let _ = pv.set_position(PhysicalPosition::new(x.max(0.0) as i32, y as i32));
    let _ = app.emit_to("preview", "preview-tip", text);
    let _ = pv.show();
}

#[tauri::command]
pub fn previews_hide(app: AppHandle) {
    clear_thumbs();
    crate::layout::DOCK_HOLD.store(false, std::sync::atomic::Ordering::Relaxed);
    if let Some(pv) = app.get_webview_window("preview") {
        let _ = pv.hide();
    }
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
