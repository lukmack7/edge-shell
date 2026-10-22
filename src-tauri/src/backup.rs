//! Kopia ustawień: przy każdym zapisie z okna ustawień powstaje kopia lokalna
//! (ostatnie 20) i — jeśli wskazano folder, np. udział na home serwerze —
//! kopia `EdgeShell-<komputer>.json` w tym folderze. Do tego eksport/import.

use crate::config::{data_dir, expand_env, Config};
use crate::settings;
use crate::state;
use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

const KEEP: usize = 20;

fn local_dir() -> std::path::PathBuf {
    let d = data_dir().join("backups");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn computer() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "komputer".into())
}

/// Zapisuje kopię (lokalnie + do wskazanego folderu). Błędy nie przerywają zapisu.
pub fn snapshot(cfg: &Config) {
    let Ok(text) = serde_json::to_string_pretty(cfg) else { return };
    let name = format!("config-{}.json", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    let _ = std::fs::write(local_dir().join(name), &text);
    prune();
    let folder = expand_env(cfg.backup.folder.trim());
    if !folder.is_empty() {
        let text = text.clone();
        // Udział sieciowy bywa wolny albo niedostępny — nie blokujemy okna.
        std::thread::spawn(move || {
            let dir = std::path::Path::new(&folder);
            if let Err(err) = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(dir.join(format!("EdgeShell-{}.json", computer())), text)) {
                log::warn!("kopia ustawień do {folder}: {err}");
            }
        });
    }
}

fn prune() {
    let mut files = list_files();
    if files.len() > KEEP {
        files.sort();
        for f in &files[..files.len() - KEEP] {
            let _ = std::fs::remove_file(local_dir().join(f));
        }
    }
}

fn list_files() -> Vec<String> {
    std::fs::read_dir(local_dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.starts_with("config-") && n.ends_with(".json"))
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    pub file: String,
    /// `2026-10-22 21:40`
    pub label: String,
}

/// Lokalne kopie, najnowsze pierwsze.
#[tauri::command]
pub fn backup_list() -> Vec<BackupEntry> {
    let mut files = list_files();
    files.sort();
    files.reverse();
    files
        .into_iter()
        .map(|f| {
            let stamp = f.trim_start_matches("config-").trim_end_matches(".json");
            let label = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%d-%H%M%S")
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|_| stamp.to_string());
            BackupEntry { file: f, label }
        })
        .collect()
}

fn apply_text(app: &AppHandle, text: &str) -> Result<(), String> {
    let cfg: Config = serde_json::from_str(text).map_err(|e| format!("To nie jest plik ustawień Edge Shell ({e})"))?;
    // Bieżące ustawienia też trafiają do kopii — przywrócenie da się cofnąć.
    snapshot(&state::cfg(app));
    settings::save_config(app.clone(), cfg)
}

#[tauri::command]
pub fn backup_restore(app: AppHandle, file: String) -> Result<(), String> {
    if file.contains(['/', '\\']) || !file.ends_with(".json") {
        return Err("zła nazwa pliku".into());
    }
    let text = std::fs::read_to_string(local_dir().join(&file)).map_err(|e| e.to_string())?;
    apply_text(&app, &text)
}

/// Zapisuje ustawienia do wybranego pliku. Zwraca ścieżkę albo None (anulowano).
#[tauri::command]
pub async fn backup_export(app: AppHandle) -> Result<Option<String>, String> {
    let cfg = state::cfg(&app);
    let text = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    let name = format!("EdgeShell-{}-{}.json", computer(), chrono::Local::now().format("%Y-%m-%d"));
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_file_name(name).add_filter("Ustawienia Edge Shell", &["json"]).blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else { return Ok(None) };
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(Some(path.to_string_lossy().to_string()))
}

/// Wczytuje ustawienia z wybranego pliku i od razu je stosuje.
#[tauri::command]
pub async fn backup_import(app: AppHandle) -> Result<bool, String> {
    let a2 = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        a2.dialog().file().add_filter("Ustawienia Edge Shell", &["json"]).blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else { return Ok(false) };
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    apply_text(&app, &text)?;
    Ok(true)
}
