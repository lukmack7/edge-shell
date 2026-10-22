//! Zasobnik systemowy na lewym pasku.
//!
//! Aplikacje dodają ikony przez `Shell_NotifyIcon`, które wysyła `WM_COPYDATA`
//! do pierwszego okna klasy `Shell_TrayWnd`. Tworzymy własne (niewidoczne)
//! okno tej klasy nad paskiem Eksploratora, więc ikony trafiają do nas.
//! Każdą wiadomość przekazujemy też Eksploratorowi, żeby po zamknięciu Edge
//! Shell wszystko działało jak wcześniej. Tak samo robią alternatywne powłoki
//! (Cairo, RetroBar).

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayItem {
    pub key: String,
    pub tip: String,
    pub icon: Option<String>,
}

#[cfg(windows)]
mod imp {
    use super::TrayItem;
    use base64::Engine;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
    use std::sync::{Mutex, OnceLock};
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
        GetMessageW, GetWindowThreadProcessId, IsWindow, PostMessageW, PostQuitMessage, RegisterClassW,
        RegisterWindowMessageW, SendMessageTimeoutW, SetForegroundWindow, SetTimer, SetWindowPos, TranslateMessage,
        HWND_BROADCAST, HWND_TOPMOST, MSG, SMTO_ABORTIFHUNG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WM_CONTEXTMENU,
        WM_COPYDATA, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_TIMER,
        WM_USER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    };

    const NIM_ADD: u32 = 0;
    const NIM_MODIFY: u32 = 1;
    const NIM_DELETE: u32 = 2;
    const NIM_SETVERSION: u32 = 4;
    const NIF_MESSAGE: u32 = 0x1;
    const NIF_ICON: u32 = 0x2;
    const NIF_TIP: u32 = 0x4;
    const NIF_STATE: u32 = 0x8;
    const NIF_GUID: u32 = 0x20;
    const NIS_HIDDEN: u32 = 0x1;
    const NIN_SELECT: u32 = WM_USER;
    const WM_STOP: u32 = WM_USER + 0x4E5;

    #[derive(Clone, Debug, Default)]
    struct Icon {
        key: String,
        hwnd: isize,
        uid: u32,
        callback: u32,
        version: u32,
        tip: String,
        icon: isize,
        hidden: bool,
    }

    static ICONS: Mutex<Vec<Icon>> = Mutex::new(Vec::new());
    static PNG: Mutex<Option<HashMap<isize, Option<String>>>> = Mutex::new(None);
    static OUR: AtomicIsize = AtomicIsize::new(0);
    static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
    static NOTIFY: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

    fn changed() {
        if let Some(f) = NOTIFY.get() {
            f();
        }
    }

    fn u32_at(d: &[u8], o: usize) -> u32 {
        d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0)
    }

    fn wstr_at(d: &[u8], o: usize, max_chars: usize) -> String {
        let Some(bytes) = d.get(o..(o + max_chars * 2).min(d.len())) else { return String::new() };
        let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
        String::from_utf16_lossy(&units)
    }

    /// Układ SHELLTRAYDATA: magic, komunikat NIM_*, NOTIFYICONDATA w wersji
    /// 32-bitowej (uchwyty zawsze jako 32 bity).
    fn handle_notify(d: &[u8]) -> bool {
        if d.len() < 32 {
            return false;
        }
        let msg = u32_at(d, 4);
        let hwnd = u32_at(d, 12) as isize;
        let uid = u32_at(d, 16);
        let flags = u32_at(d, 20);
        let key = if flags & NIF_GUID != 0 && d.len() >= 960 {
            d[944..960].iter().map(|b| format!("{b:02x}")).collect::<String>()
        } else {
            format!("{hwnd:x}:{uid}")
        };
        let mut icons = match ICONS.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let pos = icons.iter().position(|i| i.key == key);
        match msg {
            NIM_DELETE => {
                if let Some(p) = pos {
                    icons.remove(p);
                }
            }
            NIM_SETVERSION => {
                if let Some(p) = pos {
                    icons[p].version = u32_at(d, 808);
                }
            }
            NIM_ADD | NIM_MODIFY => {
                if pos.is_none() && msg == NIM_MODIFY {
                    return false;
                }
                let idx = match pos {
                    Some(p) => p,
                    None => {
                        icons.push(Icon { key: key.clone(), ..Default::default() });
                        icons.len() - 1
                    }
                };
                let ic = &mut icons[idx];
                ic.hwnd = hwnd;
                ic.uid = uid;
                if flags & NIF_MESSAGE != 0 {
                    ic.callback = u32_at(d, 24);
                }
                if flags & NIF_ICON != 0 {
                    ic.icon = u32_at(d, 28) as isize;
                }
                if flags & NIF_TIP != 0 {
                    ic.tip = wstr_at(d, 32, 128);
                }
                if flags & NIF_STATE != 0 {
                    let (state, mask) = (u32_at(d, 288), u32_at(d, 292));
                    if mask & NIS_HIDDEN != 0 {
                        ic.hidden = state & NIS_HIDDEN != 0;
                    }
                }
            }
            _ => return true,
        }
        drop(icons);
        changed();
        true
    }

    /// Przekazuje wiadomość do zasobnika Eksploratora i zwraca jego odpowiedź.
    unsafe fn forward(wp: WPARAM, lp: LPARAM) -> LRESULT {
        let Some(explorer) = crate::sys::explorer_tray() else { return LRESULT(0) };
        let mut result: usize = 0;
        let _ = SendMessageTimeoutW(
            HWND(explorer as *mut c_void),
            WM_COPYDATA,
            wp,
            lp,
            SMTO_ABORTIFHUNG,
            1500,
            Some(&mut result as *mut usize),
        );
        LRESULT(result as isize)
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if msg == WM_COPYDATA && lp.0 != 0 {
            let cds = &*(lp.0 as *const COPYDATASTRUCT);
            if cds.dwData == 1 && !cds.lpData.is_null() {
                let data = std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
                let ours = handle_notify(data);
                let theirs = forward(wp, lp);
                return if ours { LRESULT(1) } else { theirs };
            }
            // Komunikaty AppBar (dwData 0) i inne: obsługuje je Eksplorator.
            return forward(wp, lp);
        }
        if msg == WM_TIMER {
            // Trzymamy się nad paskiem Eksploratora i sprzątamy ikony zamkniętych aplikacji.
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            let removed = {
                let mut icons = ICONS.lock().unwrap_or_else(|e| e.into_inner());
                let before = icons.len();
                icons.retain(|i| IsWindow(HWND(i.hwnd as *mut c_void)).as_bool());
                before != icons.len()
            };
            if removed {
                changed();
            }
            return LRESULT(0);
        }
        if msg == WM_STOP {
            let _ = DestroyWindow(hwnd);
            PostQuitMessage(0);
            return LRESULT(0);
        }
        let tc = TASKBAR_CREATED.load(Ordering::SeqCst);
        if tc != 0 && msg == tc {
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }

    pub fn start(notify: Box<dyn Fn() + Send + Sync>) {
        let _ = NOTIFY.set(notify);
        std::thread::spawn(|| unsafe {
            let Ok(module) = GetModuleHandleW(PCWSTR::null()) else { return };
            let class = w!("Shell_TrayWnd");
            let hinst: windows::Win32::Foundation::HINSTANCE = module.into();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinst,
                lpszClassName: class,
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                log::error!("zasobnik: nie mogę zarejestrować klasy okna");
                return;
            }
            let Ok(hwnd) = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class,
                w!(""),
                WS_POPUP,
                0,
                0,
                0,
                0,
                HWND::default(),
                windows::Win32::UI::WindowsAndMessaging::HMENU::default(),
                hinst,
                None,
            ) else {
                return;
            };
            OUR.store(hwnd.0 as isize, Ordering::SeqCst);
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            SetTimer(hwnd, 1, 1500, None);

            // Poproś wszystkie aplikacje, żeby dodały ikony od nowa — już do nas.
            let tc = RegisterWindowMessageW(w!("TaskbarCreated"));
            TASKBAR_CREATED.store(tc, Ordering::SeqCst);
            let _ = PostMessageW(HWND_BROADCAST, tc, WPARAM(0), LPARAM(0));

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }

    /// Zamyka nasz zasobnik i oddaje ikony Eksploratorowi.
    pub fn stop() {
        let raw = OUR.swap(0, Ordering::SeqCst);
        if raw == 0 {
            return;
        }
        unsafe {
            let _ = PostMessageW(HWND(raw as *mut c_void), WM_STOP, WPARAM(0), LPARAM(0));
            std::thread::sleep(std::time::Duration::from_millis(150));
            let tc = TASKBAR_CREATED.load(Ordering::SeqCst);
            if tc != 0 {
                let _ = PostMessageW(HWND_BROADCAST, tc, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn png_url(icon: isize) -> Option<String> {
        if let Ok(g) = PNG.lock() {
            if let Some(hit) = g.as_ref().and_then(|m| m.get(&icon)) {
                return hit.clone();
            }
        }
        let url = crate::sys::hicon_raw_png(icon)
            .map(|b| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b)));
        if let Ok(mut g) = PNG.lock() {
            let m = g.get_or_insert_with(HashMap::new);
            if m.len() > 400 {
                m.clear();
            }
            m.insert(icon, url.clone());
        }
        url
    }

    pub fn items() -> Vec<TrayItem> {
        let icons = ICONS.lock().map(|g| g.clone()).unwrap_or_default();
        icons
            .into_iter()
            .filter(|i| !i.hidden && i.icon != 0)
            .map(|i| TrayItem { key: i.key.clone(), tip: i.tip.clone(), icon: png_url(i.icon) })
            .collect()
    }

    /// Klik w ikonę: wysyłamy aplikacji to samo, co wysłałby pasek zadań.
    pub fn click(key: &str, action: &str) {
        let Some(ic) = ICONS.lock().ok().and_then(|g| g.iter().find(|i| i.key == key).cloned()) else { return };
        if ic.callback == 0 {
            return;
        }
        unsafe {
            let h = HWND(ic.hwnd as *mut c_void);
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid as *mut u32));
            let _ = AllowSetForegroundWindow(pid);
            let _ = SetForegroundWindow(h);
            let mut pt = windows::Win32::Foundation::POINT::default();
            let _ = GetCursorPos(&mut pt);
            let seq: &[u32] = match action {
                "right" => &[WM_RBUTTONDOWN, WM_RBUTTONUP, WM_CONTEXTMENU],
                "double" => &[WM_LBUTTONDBLCLK],
                _ => &[WM_LBUTTONDOWN, WM_LBUTTONUP, NIN_SELECT],
            };
            for &m in seq {
                // Stare API nie zna WM_CONTEXTMENU ani NIN_SELECT.
                if ic.version < 4 && (m == WM_CONTEXTMENU || m == NIN_SELECT) {
                    continue;
                }
                let (wp, lp) = if ic.version >= 4 {
                    let wp = ((pt.y as u16 as usize) << 16) | (pt.x as u16 as usize);
                    let lp = (((ic.uid & 0xffff) as isize) << 16) | (m as isize & 0xffff);
                    (WPARAM(wp), LPARAM(lp))
                } else {
                    (WPARAM(ic.uid as usize), LPARAM(m as isize))
                };
                let _ = PostMessageW(h, ic.callback, wp, lp);
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::TrayItem;
    pub fn start(_: Box<dyn Fn() + Send + Sync>) {}
    pub fn stop() {}
    pub fn items() -> Vec<TrayItem> { Vec::new() }
    pub fn click(_: &str, _: &str) {}
}

pub use imp::*;
