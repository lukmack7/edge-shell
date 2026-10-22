//! Wszystko, co zależy od Windows: pozycja kursora, obszar roboczy, rezerwacja
//! miejsca na pasek (AppBar), aktywne okno, lista okien do odkładania
//! kontekstów, bateria i pasek zadań. Poza Windows funkcje są pustymi
//! zaślepkami, żeby reszta kodu kompilowała się wszędzie.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(Clone, Debug)]
pub struct AppWindow {
    pub hwnd: isize,
    pub title: String,
    pub exe: String,
}

#[cfg(windows)]
mod imp {
    use super::{AppWindow, Rect};
    use std::ffi::c_void;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Shell::{
        SHAppBarMessage, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, FindWindowW, GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindow,
        GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId, IsWindow, IsWindowVisible,
        SetForegroundWindow, ShowWindow, SystemParametersInfoW, GWL_EXSTYLE, GW_OWNER,
        SPI_GETWORKAREA, SW_HIDE, SW_MINIMIZE, SW_RESTORE, SW_SHOW,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WM_USER, WS_EX_TOOLWINDOW,
    };

    fn hwnd(raw: isize) -> HWND {
        HWND(raw as *mut c_void)
    }

    pub fn cursor() -> Option<(i32, i32)> {
        let mut p = POINT::default();
        unsafe { GetCursorPos(&mut p).ok()? };
        Some((p.x, p.y))
    }

    pub fn work_area() -> Option<Rect> {
        let mut r = RECT::default();
        unsafe {
            SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                Some(&mut r as *mut RECT as *mut c_void),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .ok()?;
        }
        Some(Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom })
    }

    fn appbar_data(raw: isize, rect: Rect, edge: u32) -> APPBARDATA {
        APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: hwnd(raw),
            uCallbackMessage: WM_USER + 0x1E5,
            uEdge: edge,
            rc: RECT { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom },
            lParam: LPARAM(0),
        }
    }

    /// Rejestruje okno jako pasek systemowy u góry ekranu. Zwraca prostokąt,
    /// który Windows faktycznie przydzielił.
    pub fn appbar_register(raw: isize, rect: Rect, edge: u32) -> Rect {
        let height = rect.bottom - rect.top;
        let width = rect.right - rect.left;
        let mut abd = appbar_data(raw, rect, edge);
        unsafe {
            SHAppBarMessage(ABM_NEW, &mut abd);
            SHAppBarMessage(ABM_QUERYPOS, &mut abd);
            if edge == ABE_TOP {
                abd.rc.bottom = abd.rc.top + height;
            } else {
                abd.rc.right = abd.rc.left + width;
            }
            SHAppBarMessage(ABM_SETPOS, &mut abd);
        }
        Rect { left: abd.rc.left, top: abd.rc.top, right: abd.rc.right, bottom: abd.rc.bottom }
    }

    pub fn appbar_remove(raw: isize) {
        let mut abd = appbar_data(raw, Rect::default(), ABE_TOP);
        unsafe {
            SHAppBarMessage(ABM_REMOVE, &mut abd);
        }
    }

    fn exe_of(h: HWND) -> String {
        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid as *mut u32));
            if pid == 0 {
                return String::new();
            }
            let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return String::new();
            };
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
            let _ = CloseHandle(proc);
            if ok {
                String::from_utf16_lossy(&buf[..len as usize])
            } else {
                String::new()
            }
        }
    }

    fn pid_of(h: HWND) -> u32 {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(h, Some(&mut pid as *mut u32)) };
        pid
    }

    fn title_of(h: HWND) -> String {
        let mut buf = [0u16; 512];
        let n = unsafe { GetWindowTextW(h, &mut buf) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    fn class_of(h: HWND) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetClassNameW(h, &mut buf) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    pub fn foreground_hwnd() -> isize {
        unsafe { GetForegroundWindow().0 as isize }
    }

    pub fn foreground_exe() -> Option<String> {
        let h = unsafe { GetForegroundWindow() };
        if h.0.is_null() || pid_of(h) == unsafe { GetCurrentProcessId() } {
            return None;
        }
        let exe = exe_of(h);
        (!exe.is_empty()).then_some(exe)
    }

    fn is_app_window(h: HWND) -> bool {
        unsafe {
            if !IsWindowVisible(h).as_bool() {
                return false;
            }
            if let Ok(owner) = GetWindow(h, GW_OWNER) {
                if !owner.0.is_null() {
                    return false;
                }
            }
            let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
            if ex & WS_EX_TOOLWINDOW.0 != 0 {
                return false;
            }
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(
                h,
                DWMWA_CLOAKED,
                &mut cloaked as *mut u32 as *mut c_void,
                std::mem::size_of::<u32>() as u32,
            );
            if cloaked != 0 {
                return false;
            }
        }
        let class = class_of(h);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
            return false;
        }
        !title_of(h).trim().is_empty()
    }

    unsafe extern "system" fn collect(h: HWND, lparam: LPARAM) -> BOOL {
        let out = &mut *(lparam.0 as *mut Vec<isize>);
        out.push(h.0 as isize);
        BOOL(1)
    }

    pub fn app_windows() -> Vec<AppWindow> {
        let mut raws: Vec<isize> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(collect), LPARAM(&mut raws as *mut Vec<isize> as isize));
        }
        let me = unsafe { GetCurrentProcessId() };
        raws.into_iter()
            .map(hwnd)
            .filter(|h| is_app_window(*h) && pid_of(*h) != me)
            .map(|h| AppWindow { hwnd: h.0 as isize, title: title_of(h), exe: exe_of(h) })
            .collect()
    }

    pub fn minimize(raw: isize) {
        unsafe {
            let _ = ShowWindow(hwnd(raw), SW_MINIMIZE);
        }
    }

    /// Przywraca okno; zwraca false, jeśli już nie istnieje.
    pub fn restore(raw: isize) -> bool {
        unsafe {
            let h = hwnd(raw);
            if !IsWindow(h).as_bool() {
                return false;
            }
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
            true
        }
    }

    pub fn battery() -> Option<(u8, bool)> {
        let mut s = SYSTEM_POWER_STATUS::default();
        unsafe { GetSystemPowerStatus(&mut s).ok()? };
        if s.BatteryFlag == 128 || s.BatteryLifePercent == 255 {
            return None; // komputer stacjonarny — brak baterii
        }
        Some((s.BatteryLifePercent, s.ACLineStatus == 1))
    }

    pub fn set_taskbar_visible(visible: bool) {
        unsafe {
            for class in [windows::core::w!("Shell_TrayWnd"), windows::core::w!("Shell_SecondaryTrayWnd")] {
                if let Ok(h) = FindWindowW(class, PCWSTR::null()) {
                    if !h.0.is_null() {
                        let _ = ShowWindow(h, if visible { SW_SHOW } else { SW_HIDE });
                    }
                }
            }
        }
    }

    /// Monitor, który w całości zajmuje aktywne okno (gra, film, prezentacja).
    pub fn fullscreen_monitor() -> Option<Rect> {
        use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsZoomed, GWL_STYLE, WS_CAPTION};
        unsafe {
            let h = GetForegroundWindow();
            if h.0.is_null() || pid_of(h) == GetCurrentProcessId() {
                return None;
            }
            let class = class_of(h);
            if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
                return None;
            }
            // Zwykłe zmaksymalizowane okno z paskiem tytułu to nie pełny ekran.
            let style = GetWindowLongW(h, GWL_STYLE) as u32;
            if IsZoomed(h).as_bool() && style & WS_CAPTION.0 == WS_CAPTION.0 {
                return None;
            }
            let mut r = RECT::default();
            GetWindowRect(h, &mut r).ok()?;
            let mon = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(mon, &mut mi).as_bool() {
                return None;
            }
            let m = mi.rcMonitor;
            (r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom)
                .then_some(Rect { left: m.left, top: m.top, right: m.right, bottom: m.bottom })
        }
    }

    /// Ile sekund od ostatniego ruchu myszy / klawisza.
    pub fn idle_secs() -> u64 {
        use windows::Win32::System::SystemInformation::GetTickCount;
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
        let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        unsafe {
            if !GetLastInputInfo(&mut li).as_bool() {
                return 0;
            }
            (GetTickCount().wrapping_sub(li.dwTime) / 1000) as u64
        }
    }

    #[allow(dead_code)]
    pub fn unused(_: WPARAM) {}
}

#[cfg(not(windows))]
mod imp {
    use super::{AppWindow, Rect};
    pub fn cursor() -> Option<(i32, i32)> { None }
    pub fn work_area() -> Option<Rect> { None }
    pub fn appbar_register(_: isize, rect: Rect, _: u32) -> Rect { rect }
    pub fn appbar_remove(_: isize) {}
    pub fn foreground_hwnd() -> isize {
        unsafe { GetForegroundWindow().0 as isize }
    }

    pub fn foreground_exe() -> Option<String> { None }
    pub fn foreground_hwnd() -> isize { 0 }
    pub fn app_windows() -> Vec<AppWindow> { Vec::new() }
    pub fn minimize(_: isize) {}
    pub fn restore(_: isize) -> bool { false }
    pub fn battery() -> Option<(u8, bool)> { None }
    pub fn set_taskbar_visible(_: bool) {}
    pub fn fullscreen_monitor() -> Option<Rect> { None }
    pub fn idle_secs() -> u64 { 0 }
}

pub use imp::*;

/// Nazwa pliku wykonywalnego bez ścieżki, małymi literami: `chrome.exe`.
pub fn exe_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_lowercase()
}
