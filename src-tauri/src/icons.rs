//! Prawdziwe ikony aplikacji do docka i ustawień (PNG jako data URL).
//! Wynik trzymamy w pamięci i na dysku, bo wyciąganie ikon jest wolne.

use crate::config::expand_env;
use crate::{state, sys};
use base64::Engine;
use sha2::Digest;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::AppHandle;

static CACHE: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

fn cache_dir() -> std::path::PathBuf {
    let d = crate::config::data_dir().join("icons");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// v2: ikony skrótów .lnk z celu (bez strzałki skrótu).
fn cache_file(key: &str) -> std::path::PathBuf {
    cache_dir().join(format!("v2-{:x}.png", sha2::Sha256::digest(key.as_bytes())))
}

/// Ikona dla ścieżki/URI jako `data:image/png;base64,...` (albo None).
pub fn icon_for(path: &str) -> Option<String> {
    let key = expand_env(path.trim());
    if key.is_empty() {
        return None;
    }
    if let Ok(g) = CACHE.lock() {
        if let Some(hit) = g.as_ref().and_then(|m| m.get(&key)) {
            return hit.clone();
        }
    }
    let file = cache_file(&key);
    let png = std::fs::read(&file).ok().or_else(|| {
        let (src, idx) = sys::resolve_icon_source(&key)?;
        let bytes = sys::icon_png(&src, idx, 64)?;
        let _ = std::fs::write(&file, &bytes);
        Some(bytes)
    });
    let url = png.map(|b| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b)));
    if let Ok(mut g) = CACHE.lock() {
        g.get_or_insert_with(HashMap::new).insert(key, url.clone());
    }
    url
}

/// Ikony wszystkich aplikacji z docka, które mają `icon` = `auto`.
#[tauri::command]
pub async fn dock_icons(app: AppHandle) -> HashMap<String, String> {
    let cfg = state::cfg(&app);
    tauri::async_runtime::spawn_blocking(move || {
        cfg.dock
            .iter()
            .filter(|a| a.icon == "auto")
            .filter_map(|a| {
                let src = if a.icon_path.trim().is_empty() { &a.path } else { &a.icon_path };
                icon_for(src).map(|u| (a.id.clone(), u))
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// Ikona dowolnej ścieżki — do podglądu w ustawieniach.
#[tauri::command]
pub async fn app_icon(path: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || icon_for(&path)).await.ok().flatten()
}

/// Czyści pamięć podręczną (po zmianie ikony w ustawieniach).
pub fn forget(path: &str) {
    let key = expand_env(path.trim());
    if let Ok(mut g) = CACHE.lock() {
        if let Some(m) = g.as_mut() {
            m.remove(&key);
        }
    }
    let _ = std::fs::remove_file(cache_file(&key));
}
