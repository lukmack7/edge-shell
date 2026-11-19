//! Kosz w docku: stan (liczba elementów i rozmiar), otwieranie, opróżnianie
//! i przenoszenie plików upuszczonych na ikonę. Poza Windows wszystko jest puste.

use serde::Serialize;

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrashInfo {
    pub items: i64,
    pub bytes: i64,
}

#[cfg(windows)]
mod imp {
    use super::TrashInfo;
    use std::os::windows::process::CommandExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::{SHEmptyRecycleBinW, SHQueryRecycleBinW, SHQUERYRBINFO};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn info() -> TrashInfo {
        let mut q = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
        if unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut q) }.is_err() {
            return TrashInfo::default();
        }
        let (items, bytes) = (q.i64NumItems, q.i64Size);
        TrashInfo { items, bytes }
    }

    pub fn open() {
        let _ = std::process::Command::new("explorer.exe").arg("shell:RecycleBinFolder").spawn();
    }

    /// Windows sam pyta o potwierdzenie i pokazuje postęp.
    pub fn empty() -> Result<(), String> {
        unsafe { SHEmptyRecycleBinW(HWND::default(), PCWSTR::null(), 0) }.map_err(|e| e.to_string())
    }

    /// Pliki i foldery trafiają do kosza (z możliwością przywrócenia), nie są kasowane na stałe.
    pub fn send(paths: &[String]) -> Result<(), String> {
        const SCRIPT: &str = "Add-Type -AssemblyName Microsoft.VisualBasic; \
            foreach ($p in ($env:EDGE_TRASH -split \"`n\")) { if (-not $p) { continue }; \
            if (Test-Path -LiteralPath $p -PathType Container) { \
            [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($p, 'OnlyErrorDialogs', 'SendToRecycleBin') } \
            elseif (Test-Path -LiteralPath $p) { \
            [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($p, 'OnlyErrorDialogs', 'SendToRecycleBin') } }";
        let out = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
            .env("EDGE_TRASH", paths.join("\n"))
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::TrashInfo;
    pub fn info() -> TrashInfo { TrashInfo::default() }
    pub fn open() {}
    pub fn empty() -> Result<(), String> { Ok(()) }
    pub fn send(_: &[String]) -> Result<(), String> { Ok(()) }
}

#[tauri::command]
pub async fn trash_info() -> TrashInfo {
    tauri::async_runtime::spawn_blocking(imp::info).await.unwrap_or_default()
}

#[tauri::command]
pub fn trash_open() {
    imp::open();
}

#[tauri::command]
pub async fn trash_empty() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(imp::empty).await.map_err(|e| e.to_string())?
}

/// Upuszczone na kosz ścieżki; pomijamy puste i korzenie dysków.
#[tauri::command]
pub async fn trash_drop(paths: Vec<String>) -> Result<usize, String> {
    let paths: Vec<String> = paths
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| p.len() > 3 && std::path::Path::new(p).exists())
        .collect();
    let n = paths.len();
    if n == 0 {
        return Ok(0);
    }
    tauri::async_runtime::spawn_blocking(move || imp::send(&paths)).await.map_err(|e| e.to_string())??;
    Ok(n)
}
