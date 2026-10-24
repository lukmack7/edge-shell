//! Wysuwane karty przy pasku: procesy (CPU/RAM), Wi‑Fi, Bluetooth, dźwięk,
//! zasilanie. Jedno okno `popup`, które pokazuje wskazany widok obok elementu,
//! z którego je otwarto.

use crate::{layout, state, sysx};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WindowEvent};

/// Karta schowana przez utratę fokusu — klik w ten sam przycisk ma ją zamknąć,
/// a nie otworzyć ponownie.
static LAST_HIDE: Mutex<Option<(String, Instant)>> = Mutex::new(None);

fn size(view: &str) -> (f64, f64) {
    match view {
        "power" => (260.0, 352.0),
        "cpu" | "mem" => (380.0, 400.0),
        "wifi" => (340.0, 440.0),
        "bt" => (340.0, 380.0),
        "sound" => (380.0, 460.0),
        _ => (320.0, 320.0),
    }
}

/// Otwiera kartę `view` przy elemencie (prostokąt w px CSS okna `from`).
/// Drugi klik w ten sam element zamyka kartę.
#[tauri::command]
pub fn popup_open(app: AppHandle, view: String, from: String, x: f64, y: f64, w: f64, h: f64) {
    let (Some(pop), Some(src)) = (app.get_webview_window("popup"), app.get_webview_window(&from)) else { return };
    let current = state::read(&app, |rt| rt.popup_view.clone());
    if pop.is_visible().unwrap_or(false) && current == view {
        let _ = pop.hide();
        return;
    }
    let just_hidden = LAST_HIDE.lock().ok().and_then(|g| g.clone()).is_some_and(|(v, t)| v == view && t.elapsed() < Duration::from_millis(350));
    if just_hidden {
        return;
    }
    let scale = src.scale_factor().unwrap_or(1.0);
    let origin = src.outer_position().unwrap_or(PhysicalPosition::new(0, 0));
    let (lw, lh) = size(&view);
    let (pw, ph) = ((lw * scale) as i32, (lh * scale) as i32);
    let s = layout::screen(&app);
    let gap = (6.0 * scale) as i32;
    let (mut px, mut py) = if from == "rail" {
        // Obok lewego paska, dolną krawędzią do przycisku.
        (origin.x + ((x + w) * scale) as i32 + gap, origin.y + ((y + h) * scale) as i32 - ph)
    } else {
        // Pod elementem paska, wyrównane do jego prawej krawędzi.
        (origin.x + ((x + w) * scale) as i32 - pw, origin.y + ((y + h) * scale) as i32 + gap)
    };
    let mon = app
        .available_monitors()
        .unwrap_or_default()
        .into_iter()
        .find(|m| {
            let p = m.position();
            let z = m.size();
            origin.x >= p.x && origin.x < p.x + z.width as i32 && origin.y >= p.y && origin.y < p.y + z.height as i32
        });
    let (left, top, right, bottom) = match &mon {
        Some(m) => (m.position().x, m.position().y, m.position().x + m.size().width as i32, m.position().y + m.size().height as i32),
        None => (s.monitor.left, s.monitor.top, s.monitor.right, s.monitor.bottom),
    };
    px = px.clamp(left + gap, (right - pw - gap).max(left));
    py = py.clamp(top + gap, (bottom - ph - gap).max(top));
    state::update(&app, false, |rt| rt.popup_view = view.clone());
    let _ = pop.set_size(PhysicalSize::new(pw as u32, ph as u32));
    let _ = pop.set_position(PhysicalPosition::new(px, py));
    let _ = app.emit_to("popup", "popup-view", &view);
    let _ = pop.show();
    let _ = pop.set_focus();
}

/// Karta dopasowuje wysokość do treści (px CSS).
#[tauri::command]
pub fn popup_resize(app: AppHandle, height: f64) {
    let Some(pop) = app.get_webview_window("popup") else { return };
    let scale = pop.scale_factor().unwrap_or(1.0);
    let Ok(sz) = pop.outer_size() else { return };
    let Ok(pos) = pop.outer_position() else { return };
    let new_h = (height.clamp(120.0, 640.0) * scale) as i32;
    let old_h = sz.height as i32;
    // Karta lewego paska rośnie w górę (jest przyklejona dołem do przycisku).
    let from_rail = state::read(&app, |rt| rt.popup_view == "power");
    let y = if from_rail { pos.y + old_h - new_h } else { pos.y };
    let _ = pop.set_size(PhysicalSize::new(sz.width, new_h as u32));
    let _ = pop.set_position(PhysicalPosition::new(pos.x, y));
}

#[tauri::command]
pub fn popup_hide(app: AppHandle) {
    if let Some(p) = app.get_webview_window("popup") {
        let _ = p.hide();
    }
}

#[tauri::command]
pub async fn sys_processes() -> Vec<sysx::ProcRow> {
    tauri::async_runtime::spawn_blocking(sysx::processes).await.unwrap_or_default()
}

#[tauri::command]
pub fn sys_kill(pids: Vec<u32>) -> usize {
    sysx::kill(&pids)
}

#[tauri::command]
pub async fn wifi_list() -> Vec<sysx::WifiNet> {
    tauri::async_runtime::spawn_blocking(sysx::wifi_networks).await.unwrap_or_default()
}

#[tauri::command]
pub async fn wifi_connect(ssid: String, password: Option<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || sysx::wifi_connect(&ssid, password.as_deref()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn wifi_disconnect() {
    let _ = tauri::async_runtime::spawn_blocking(sysx::wifi_disconnect).await;
}

#[tauri::command]
pub async fn bt_list() -> Vec<sysx::BtDevice> {
    tauri::async_runtime::spawn_blocking(sysx::bt_devices).await.unwrap_or_default()
}

#[tauri::command]
pub async fn bt_batteries() -> Vec<(String, u8)> {
    tauri::async_runtime::spawn_blocking(sysx::bt_batteries).await.unwrap_or_default()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioState {
    outputs: Vec<sysx::AudioOut>,
    apps: Vec<sysx::AudioApp>,
    volume: Option<u8>,
    muted: bool,
}

#[tauri::command]
pub async fn audio_state() -> AudioState {
    tauri::async_runtime::spawn_blocking(|| {
        let (volume, muted) = crate::sys::volume().map(|(v, m)| (Some(v), m)).unwrap_or((None, false));
        AudioState { outputs: sysx::audio_outputs(), apps: sysx::audio_apps(), volume, muted }
    })
    .await
    .unwrap_or(AudioState { outputs: vec![], apps: vec![], volume: None, muted: false })
}

#[tauri::command]
pub async fn audio_set_output(app: AppHandle, id: String) -> Result<(), String> {
    let r = tauri::async_runtime::spawn_blocking(move || sysx::set_default_output(&id)).await.map_err(|e| e.to_string())?;
    crate::stats::refresh_volume(&app);
    r
}

#[tauri::command]
pub async fn audio_app(pid: u32, volume: Option<u8>, mute: Option<bool>) {
    let _ = tauri::async_runtime::spawn_blocking(move || sysx::set_app_volume(pid, volume, mute)).await;
}

/// Zasilanie z lewego paska. Przed restartem/wyłączeniem przywracamy pasek zadań,
/// żeby Windows zapamiętał go w normalnym stanie.
#[tauri::command]
pub fn power_action(app: AppHandle, action: String) -> Result<(), String> {
    popup_hide(app.clone());
    if matches!(action.as_str(), "restart" | "shutdown" | "logout") {
        layout::release(&app);
    }
    sysx::power(&action)
}

/// Ikona aplikacji po numerze procesu (do miksera).
#[tauri::command]
pub async fn process_icon(pid: u32) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = sysx::exe_path(pid);
        if path.is_empty() { None } else { crate::icons::icon_for(&path) }
    })
    .await
    .ok()
    .flatten()
}

pub fn init(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("popup") {
        let h = app.clone();
        w.on_window_event(move |e| {
            if let WindowEvent::Focused(false) = e {
                let view = state::read(&h, |rt| rt.popup_view.clone());
                if let Ok(mut g) = LAST_HIDE.lock() {
                    *g = Some((view, Instant::now()));
                }
                popup_hide(h.clone());
            }
        });
    }
}
