//! Na Windows nowa instancja musi wystartować po zwolnieniu blokady starej.

#[cfg(windows)]
pub fn request(app: &tauri::AppHandle) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .arg("--edge-shell-relaunch")
        .arg(std::process::id().to_string())
        .args(std::env::args_os().skip(1))
        .spawn()
        .map_err(|e| e.to_string())?;
    app.exit(0);
    Ok(())
}

#[cfg(not(windows))]
pub fn request(app: &tauri::AppHandle) -> Result<(), String> {
    app.request_restart();
    Ok(())
}

#[cfg(windows)]
pub fn run_if_requested() -> bool {
    use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};

    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--edge-shell-relaunch")) {
        return false;
    }
    let Some(pid) = args.next().and_then(|arg| arg.to_str()?.parse::<u32>().ok()).filter(|&pid| pid != 0) else {
        return true;
    };
    let exited = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
        Ok(handle) => unsafe {
            let result = WaitForSingleObject(handle, 30_000);
            let _ = CloseHandle(handle);
            result == WAIT_OBJECT_0
        },
        // Proces mógł już zakończyć się, zanim pomocnik dostał czas procesora.
        Err(e) => e.code() == ERROR_INVALID_PARAMETER.to_hresult(),
    };
    if exited {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).args(args).spawn();
        }
    }
    true
}

#[cfg(not(windows))]
pub fn run_if_requested() -> bool {
    false
}
