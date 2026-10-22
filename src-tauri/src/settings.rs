//! Komendy okna ustawień i sterowania systemem z paska.

use crate::config::Config;
use crate::{hotkeys, icons, layout, state, stats, sys};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

type Res<T> = Result<T, String>;

#[tauri::command]
pub fn get_config(app: AppHandle) -> Config {
    state::cfg(&app)
}

/// Zapisuje konfigurację z okna ustawień i od razu ją stosuje.
#[tauri::command]
pub fn save_config(app: AppHandle, config: Config) -> Res<()> {
    let before = state::cfg(&app);
    for a in &config.dock {
        let old = before.dock.iter().find(|b| b.id == a.id);
        if old.map(|o| o.path != a.path || o.icon_path != a.icon_path).unwrap_or(true) {
            icons::forget(if a.icon_path.trim().is_empty() { &a.path } else { &a.icon_path });
        }
    }
    config.save();
    crate::backup::snapshot(&config);
    if let Ok(mut c) = state::shared(&app).cfg.write() {
        *c = config.clone();
    }
    apply(&app, &before, &config);
    Ok(())
}

pub fn apply(app: &AppHandle, before: &Config, now: &Config) {
    hotkeys::register(app);
    if before.shell.hide_taskbar != now.shell.hide_taskbar {
        let restore = state::read(app, |rt| rt.store.taskbar_state);
        sys::hide_taskbar(now.shell.hide_taskbar, restore);
        let h = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1200));
            layout::arrange(&h);
        });
    }
    set_autostart(app, now.shell.autostart);
    layout::arrange(app);
    state::emit_snapshot(app);
    let _ = app.emit("config-changed", ());
}

pub fn set_autostart(app: &AppHandle, on: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    let enabled = al.is_enabled().unwrap_or(false);
    if on && !enabled {
        let _ = al.enable();
    } else if !on && enabled {
        let _ = al.disable();
    }
}

#[tauri::command]
pub fn open_settings(app: AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Okno wyboru pliku (programu) albo folderu.
#[tauri::command]
pub async fn pick_path(app: AppHandle, folder: bool, images: Option<bool>) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dialog = app.dialog().file();
        let picked = if folder {
            dialog.blocking_pick_folder()
        } else if images.unwrap_or(false) {
            dialog.add_filter("Obrazy", &["jpg", "jpeg", "png", "bmp", "webp"]).blocking_pick_file()
        } else {
            dialog.add_filter("Programy i skróty", &["exe", "lnk", "url", "bat", "cmd"]).blocking_pick_file()
        };
        picked.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().to_string())
    })
    .await
    .ok()
    .flatten()
}

/// Sprawdza, czy pod adresem odpowiada Horolog.
#[tauri::command]
pub async fn horolog_test(url: String) -> Res<String> {
    let url = format!("{}/api/health", url.trim().trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(6))
        .send()
        .await
        .map_err(|e| format!("Brak odpowiedzi: {e}"))?;
    if resp.status().is_success() {
        Ok("Horolog odpowiada".into())
    } else {
        Err(format!("Horolog zwrócił {}", resp.status()))
    }
}

#[tauri::command]
pub fn get_stats() -> sys::Stats {
    stats::latest()
}

#[tauri::command]
pub fn set_volume(app: AppHandle, level: u8) {
    sys::set_volume(level);
    stats::refresh_volume(&app);
}

#[tauri::command]
pub fn toggle_mute(app: AppHandle) {
    sys::toggle_mute();
    stats::refresh_volume(&app);
}
