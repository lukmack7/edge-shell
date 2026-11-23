//! Licznik z domyślnej skrzynki klasycznego Outlooka. Nie uruchamia Outlooka.

pub const EXE: &str = "outlook.exe";

#[cfg(windows)]
pub fn start(app: tauri::AppHandle) {
    use crate::state;
    use std::time::{Duration, Instant};

    std::thread::spawn(move || {
        let interval = Duration::from_secs(30);
        let mut last = None;
        loop {
            let enabled = state::cfg(&app).mail.outlook_badge;
            if !enabled {
                last = None;
            }
            if !enabled || last.is_none_or(|t: Instant| t.elapsed() >= interval) {
                let count = if enabled {
                    last = Some(Instant::now());
                    unread()
                } else {
                    None
                };
                if state::read(&app, |rt| rt.outlook_unread != count) {
                    state::update(&app, false, |rt| rt.outlook_unread = count);
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

#[cfg(windows)]
fn unread() -> Option<u32> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    // GetActiveObject korzysta wyłącznie z istniejącej sesji Outlooka. 6 = olFolderInbox.
    // Odczytujemy tylko liczbę, nie listę wiadomości ani ich treść.
    let script = "$ErrorActionPreference='Stop'; try { \
        $app = [Runtime.InteropServices.Marshal]::GetActiveObject('Outlook.Application'); \
        [Console]::WriteLine([uint32]($app.Session.GetDefaultFolder(6).UnReadItemCount)); \
        } catch { exit 1 }";
    let exe = std::path::PathBuf::from(std::env::var_os("SystemRoot")?)
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let mut child = Command::new(exe)
        .args(["-NoProfile", "-NonInteractive", "-STA", "-Command", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) if started.elapsed() < Duration::from_secs(8) => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    std::str::from_utf8(&output.stdout).ok()?.trim().parse().ok()
}

#[cfg(not(windows))]
pub fn start(_: tauri::AppHandle) {}
