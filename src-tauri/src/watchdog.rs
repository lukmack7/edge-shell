//! Bezpiecznik: osobny, mały proces (ten sam plik .exe z `--watchdog`), który
//! pilnuje Edge Shell. Gdy Edge Shell padnie albo zawiesi się na dłużej,
//! przywraca pasek zadań Windows i każe aplikacjom ponownie zgłosić ikony
//! zasobnika — żeby nigdy nie zostać bez paska i bez zasobnika.

/// Wywoływane na samym początku `main`. Zwraca `true`, jeśli ten proces jest
/// strażnikiem (wtedy `main` kończy się zaraz potem).
pub fn run_if_requested() -> bool {
    let args: Vec<String> = std::env::args().collect();
    let Some(pos) = args.iter().position(|a| a == "--watchdog") else { return false };
    let pid = args.get(pos + 1).and_then(|s| s.parse::<u32>().ok());
    let restore = args.get(pos + 2).and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
    let hide = args.get(pos + 3).is_some_and(|s| s == "1");
    if let Some(pid) = pid {
        imp::guard(pid, restore, hide);
    }
    true
}

/// Uruchamia strażnika dla bieżącego procesu.
pub fn spawn(restore_state: usize, hides_taskbar: bool) {
    imp::spawn(restore_state, hides_taskbar);
}

#[cfg(windows)]
mod imp {
    use std::os::windows::process::CommandExt;
    use std::time::Duration;
    use windows::core::w;
    use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, WAIT_OBJECT_0, WPARAM};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsHungAppWindow, IsWindowVisible, RegisterWindowMessageW,
        SendNotifyMessageW, HWND_BROADCAST,
    };

    pub fn spawn(restore_state: usize, hides_taskbar: bool) {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        let Ok(exe) = std::env::current_exe() else { return };
        let pid = std::process::id().to_string();
        let _ = std::process::Command::new(exe)
            .args(["--watchdog", &pid, &restore_state.to_string(), if hides_taskbar { "1" } else { "0" }])
            .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
            .spawn();
    }

    struct Find {
        pid: u32,
        hwnd: HWND,
    }

    unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
        let f = &mut *(l.0 as *mut Find);
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid as *mut u32));
        if pid == f.pid && IsWindowVisible(h).as_bool() {
            f.hwnd = h;
            return BOOL(0);
        }
        BOOL(1)
    }

    /// Czy okna procesu przestały odpowiadać (Windows: >5 s bez obsługi wiadomości).
    fn hung(pid: u32) -> bool {
        let mut f = Find { pid, hwnd: HWND::default() };
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut f as *mut Find as isize));
            !f.hwnd.0.is_null() && IsHungAppWindow(f.hwnd).as_bool()
        }
    }

    fn restore(state: usize) {
        crate::sys::hide_taskbar(false, state);
        // Aplikacje dodają ikony zasobnika jeszcze raz (do zasobnika Eksploratora).
        unsafe {
            let msg = RegisterWindowMessageW(w!("TaskbarCreated"));
            if msg != 0 {
                let _ = SendNotifyMessageW(HWND_BROADCAST, msg, WPARAM(0), LPARAM(0));
            }
        }
    }

    pub fn guard(pid: u32, state: usize, hides_taskbar: bool) {
        let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
            return;
        };
        let mut hung_ticks = 0u32;
        let mut rescued = false;
        loop {
            let r = unsafe { WaitForSingleObject(process, 3000) };
            if r == WAIT_OBJECT_0 {
                let mut code = 0u32;
                let _ = unsafe { GetExitCodeProcess(process, &mut code) };
                // Kod 0 = zwykłe wyjście: Edge Shell sam wszystko przywrócił.
                if code != 0 || rescued {
                    restore(state);
                }
                break;
            }
            if hung(pid) {
                hung_ticks += 1;
                // ~15 s zawieszenia: oddajemy pasek zadań, nie zabijamy procesu.
                if hung_ticks >= 5 && !rescued {
                    restore(state);
                    rescued = true;
                }
            } else {
                hung_ticks = 0;
                if rescued {
                    // Odwiesił się — wracamy do układu Edge Shell.
                    if hides_taskbar {
                        crate::sys::hide_taskbar(true, 0);
                    }
                    rescued = false;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        unsafe {
            let _ = CloseHandle(process);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn spawn(_: usize, _: bool) {}
    pub fn guard(_: u32, _: usize, _: bool) {}
}
