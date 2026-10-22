//! Stosy: folder przypięty do docka. Kliknięcie pokazuje nad ikoną siatkę
//! jego zawartości (najnowsze na początku) — bez otwierania Eksploratora.

use crate::config::expand_env;
use crate::{layout, state, sys};
use base64::Engine;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WindowEvent};

const MAX_ITEMS: usize = 40;
const COLS: usize = 5;

static IS_DIR: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);
static ICONS: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

/// Czy wpis docka to folder (wynik zapamiętany — ścieżki sieciowe bywają wolne).
pub fn is_folder(path: &str) -> bool {
    let p = expand_env(path.trim());
    if p.is_empty() || p.contains("://") || !(p.contains('\\') || p.contains('/')) {
        return false;
    }
    if let Ok(g) = IS_DIR.lock() {
        if let Some(hit) = g.as_ref().and_then(|m| m.get(&p)) {
            return *hit;
        }
    }
    let dir = std::path::Path::new(&p).is_dir();
    if let Ok(mut g) = IS_DIR.lock() {
        g.get_or_insert_with(HashMap::new).insert(p, dir);
    }
    dir
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub name: String,
    pub path: String,
    pub dir: bool,
    pub icon: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Stack {
    pub id: String,
    pub name: String,
    pub path: String,
    pub items: Vec<Item>,
    pub total: usize,
}

/// Ikona pliku: programy i skróty mają własne, reszta — wspólną dla rozszerzenia.
fn icon(path: &std::path::Path, dir: bool) -> Option<String> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let own = dir || matches!(ext.as_str(), "exe" | "lnk" | "url" | "ico" | "appref-ms");
    let key = if dir { "<folder>".to_string() } else if own { path.to_string_lossy().to_string() } else { format!(".{ext}") };
    if let Ok(g) = ICONS.lock() {
        if let Some(hit) = g.as_ref().and_then(|m| m.get(&key)) {
            return hit.clone();
        }
    }
    let url = sys::icon_png(&path.to_string_lossy(), 0, 48)
        .map(|b| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b)));
    if let Ok(mut g) = ICONS.lock() {
        g.get_or_insert_with(HashMap::new).insert(key, url.clone());
    }
    url
}

fn read(dir: &str) -> (Vec<Item>, usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return (Vec::new(), 0) };
    let mut all: Vec<(std::time::SystemTime, std::path::PathBuf, bool)> = rd
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            !n.starts_with('.') && !n.eq_ignore_ascii_case("desktop.ini") && !n.starts_with("~$")
        })
        .filter_map(|e| {
            let md = e.metadata().ok()?;
            let t = md.modified().unwrap_or(std::time::UNIX_EPOCH);
            Some((t, e.path(), md.is_dir()))
        })
        .collect();
    let total = all.len();
    all.sort_by(|a, b| b.0.cmp(&a.0));
    let items = all
        .into_iter()
        .take(MAX_ITEMS)
        .map(|(_, p, d)| Item {
            name: p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            path: p.to_string_lossy().to_string(),
            dir: d,
            icon: icon(&p, d),
        })
        .collect();
    (items, total)
}

/// Otwiera stos nad ikoną (anchor = środek ikony w px CSS docka).
#[tauri::command]
pub async fn stack_open(app: AppHandle, id: String, anchor: f64) -> Result<(), String> {
    let cfg = state::cfg(&app);
    let entry = cfg.dock.iter().find(|a| a.id == id).cloned().ok_or("nie ma takiej ikony")?;
    let dir = expand_env(entry.path.trim());
    let d2 = dir.clone();
    let (items, total) = tauri::async_runtime::spawn_blocking(move || read(&d2)).await.map_err(|e| e.to_string())?;

    let (Some(st), Some(dock)) = (app.get_webview_window("stack"), app.get_webview_window("dock")) else {
        return Err("brak okna".into());
    };
    let scale = dock.scale_factor().unwrap_or(1.0);
    let rows = items.len().div_ceil(COLS).clamp(1, 4) as f64;
    let (w, h) = ((COLS as f64 * 100.0 + 40.0) * scale, (rows * 104.0 + 92.0) * scale);
    let dpos = dock.outer_position().unwrap_or(PhysicalPosition::new(0, 0));
    let x = dpos.x as f64 + anchor * scale - w / 2.0;
    let y = dpos.y as f64 - h + 6.0 * scale;
    layout::DOCK_HOLD.store(true, Ordering::Relaxed);
    if let Some(pv) = app.get_webview_window("preview") {
        let _ = pv.hide();
    }
    let _ = st.set_size(PhysicalSize::new(w as u32, h as u32));
    let _ = st.set_position(PhysicalPosition::new(x.max(0.0) as i32, y as i32));
    let _ = app.emit_to("stack", "stack", Stack { id, name: entry.name, path: dir, items, total });
    let _ = st.show();
    let _ = st.set_focus();
    Ok(())
}

#[tauri::command]
pub fn stack_close(app: AppHandle) {
    layout::DOCK_HOLD.store(false, Ordering::Relaxed);
    if let Some(st) = app.get_webview_window("stack") {
        let _ = st.hide();
    }
}

/// Stos chowa się po kliknięciu gdzie indziej.
pub fn init(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("stack") {
        let h = app.clone();
        w.on_window_event(move |e| {
            if let WindowEvent::Focused(false) = e {
                stack_close(h.clone());
            }
        });
    }
}
