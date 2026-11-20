//! Postęp pobierania do paska na ikonie docka.
//!
//! Przeglądarki Chromium (Vivaldi, Chrome, Edge, Brave) zapisują trwające
//! pobieranie jako `*.crdownload` w folderze Pobrane, więc jego rozmiar rośnie na
//! żywo. Całkowity rozmiar bierzemy z bazy historii przeglądarki (kopia pliku,
//! żeby nie blokować przeglądarki). Firefox (`*.part`) nie podaje rozmiaru w
//! pliku — wtedy pasek jest nieokreślony.

use crate::state;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use tauri::AppHandle;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadInfo {
    /// 0.0–1.0, `None` gdy całkowity rozmiar jest nieznany.
    pub progress: Option<f32>,
    pub count: usize,
}

pub const BROWSERS: [&str; 6] = ["vivaldi.exe", "chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "opera.exe"];

fn normalize(path: &str) -> String {
    path.trim().trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase()
}

/// Czy ścieżka (z dockiem, ze zmiennymi środowiskowymi) to systemowy folder Pobrane.
pub fn is_downloads_dir(path: &str) -> bool {
    dirs::download_dir().is_some_and(|d| normalize(&d.to_string_lossy()) == normalize(&crate::config::expand_env(path)))
}

/// Rozmiary trwających pobrań (pliki zmieniane w ostatnich 45 s).
fn partial_sizes(dir: &Path) -> Vec<u64> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension().is_some_and(|x| {
                let x = x.to_string_lossy().to_lowercase();
                matches!(x.as_str(), "crdownload" | "part" | "download" | "opdownload")
            })
        })
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.modified().ok().and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|d| d < Duration::from_secs(45)))
        .map(|m| m.len())
        .collect()
}

fn history_files() -> Vec<PathBuf> {
    let Some(base) = dirs::data_local_dir() else { return Vec::new() };
    ["Vivaldi\\User Data", "Google\\Chrome\\User Data", "Microsoft\\Edge\\User Data", "BraveSoftware\\Brave-Browser\\User Data"]
        .iter()
        .map(|p| base.join(p).join("Default").join("History"))
        .filter(|p| p.exists())
        .collect()
}

/// Suma `total_bytes` trwających pobrań (stan 0) w jednej bazie historii.
fn total_in(db: &Path) -> Option<u64> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let total: i64 = conn
        .query_row("SELECT IFNULL(SUM(CASE WHEN total_bytes > 0 THEN total_bytes ELSE 0 END), 0) FROM downloads WHERE state = 0", [], |r| r.get(0))
        .ok()?;
    Some(total.max(0) as u64)
}

/// Suma `total_bytes` trwających pobrań ze wszystkich przeglądarek (z kopii pliku historii).
fn in_progress_total(files: &[PathBuf]) -> u64 {
    let tmp = std::env::temp_dir().join("edge-shell-history-copy.db");
    let mut total = 0u64;
    for f in files {
        if std::fs::copy(f, &tmp).is_ok() {
            total += total_in(&tmp).unwrap_or(0);
        }
    }
    let _ = std::fs::remove_file(&tmp);
    total
}

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_db: Option<Instant> = None;
        let mut total = 0u64;
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let Some(dir) = dirs::download_dir() else { continue };
            let sizes = partial_sizes(&dir);
            let next = if sizes.is_empty() {
                last_db = None;
                total = 0;
                None
            } else {
                if last_db.map_or(true, |t| t.elapsed() > Duration::from_secs(6)) {
                    total = in_progress_total(&history_files());
                    last_db = Some(Instant::now());
                }
                let received: u64 = sizes.iter().sum();
                let progress = (total > 0).then(|| (received as f32 / total as f32).clamp(0.0, 0.99));
                Some(DownloadInfo { progress, count: sizes.len() })
            };
            if state::read(&app, |rt| rt.download != next) {
                state::update(&app, false, |rt| rt.download = next.clone());
            }
        }
    });
}
