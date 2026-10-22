//! Odczyty systemu do paska (CPU, pamięć, sieć, głośność, Wi‑Fi, Bluetooth),
//! sterowanie paskiem zadań Windows i wyciąganie ikon aplikacji.
//! Poza Windows wszystko zwraca puste wartości.

use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub cpu: Option<f32>,
    pub mem: Option<f32>,
    pub mem_used_gb: Option<f32>,
    pub mem_total_gb: Option<f32>,
    pub down_bps: Option<u64>,
    pub up_bps: Option<u64>,
    pub volume: Option<u8>,
    pub muted: bool,
    pub wifi_ssid: Option<String>,
    pub wifi_signal: Option<u8>,
    pub bt_present: bool,
    pub bt_connected: u32,
    pub battery: Option<u8>,
    pub charging: bool,
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::os::windows::process::CommandExt;
    use windows::core::{PCWSTR, HSTRING};
    use windows::Win32::Devices::Bluetooth::{
        BluetoothFindDeviceClose, BluetoothFindFirstDevice, BluetoothFindFirstRadio, BluetoothFindNextDevice,
        BluetoothFindRadioClose, BLUETOOTH_DEVICE_INFO, BLUETOOTH_DEVICE_SEARCH_PARAMS, BLUETOOTH_FIND_RADIO_PARAMS,
    };
    use windows::Win32::Foundation::{CloseHandle, BOOL, FILETIME, HANDLE, HWND, LPARAM};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, DIB_RGB_COLORS, HDC,
    };
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
    use windows::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows::Win32::System::Threading::GetSystemTimes;
    use windows::Win32::UI::Shell::{
        SHAppBarMessage, SHGetFileInfoW, ABM_GETSTATE, ABM_SETSTATE, APPBARDATA, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, FindWindowW, GetIconInfo, IsWindowVisible, PrivateExtractIconsW, ShowWindow, HICON, ICONINFO,
        SW_HIDE, SW_SHOW,
    };

    // ---------------- CPU / pamięć / sieć ----------------

    fn ft(f: FILETIME) -> u64 {
        ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
    }

    /// (idle, kernel, user) — do liczenia obciążenia z różnicy.
    pub fn cpu_times() -> Option<(u64, u64, u64)> {
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        unsafe { GetSystemTimes(Some(&mut idle as *mut _), Some(&mut kernel as *mut _), Some(&mut user as *mut _)).ok()? };
        Some((ft(idle), ft(kernel), ft(user)))
    }

    /// (procent użycia, użyte GB, całość GB)
    pub fn memory() -> Option<(f32, f32, f32)> {
        let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        unsafe { GlobalMemoryStatusEx(&mut m).ok()? };
        let gb = 1024.0 * 1024.0 * 1024.0;
        let total = m.ullTotalPhys as f64 / gb;
        let used = (m.ullTotalPhys - m.ullAvailPhys) as f64 / gb;
        Some((m.dwMemoryLoad as f32, used as f32, total as f32))
    }

    /// Suma bajtów (odebrane, wysłane) po fizycznych, aktywnych interfejsach.
    pub fn net_bytes() -> Option<(u64, u64)> {
        unsafe {
            let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
            if GetIfTable2(&mut table).is_err() || table.is_null() {
                return None;
            }
            let n = (*table).NumEntries as usize;
            let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), n);
            let (mut down, mut up) = (0u64, 0u64);
            for r in rows {
                let flags = r.InterfaceAndOperStatusFlags._bitfield;
                let hardware = flags & 0b1 != 0;
                let filter = flags & 0b10 != 0;
                if hardware && !filter && r.OperStatus == IfOperStatusUp {
                    down += r.InOctets;
                    up += r.OutOctets;
                }
            }
            FreeMibTable(table as *const c_void);
            Some((down, up))
        }
    }

    // ---------------- Dźwięk ----------------

    fn endpoint() -> Option<IAudioEndpointVolume> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
            dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
        }
    }

    pub fn volume() -> Option<(u8, bool)> {
        let ep = endpoint()?;
        unsafe {
            let level = ep.GetMasterVolumeLevelScalar().ok()?;
            let muted = ep.GetMute().ok()?.as_bool();
            Some(((level * 100.0).round() as u8, muted))
        }
    }

    pub fn set_volume(percent: u8) {
        if let Some(ep) = endpoint() {
            unsafe {
                let _ = ep.SetMasterVolumeLevelScalar(percent.min(100) as f32 / 100.0, std::ptr::null());
                let _ = ep.SetMute(false, std::ptr::null());
            }
        }
    }

    pub fn toggle_mute() {
        if let Some(ep) = endpoint() {
            unsafe {
                if let Ok(m) = ep.GetMute() {
                    let _ = ep.SetMute(!m.as_bool(), std::ptr::null());
                }
            }
        }
    }

    // ---------------- Wi‑Fi i Bluetooth ----------------

    /// SSID i siła sygnału z `netsh wlan show interfaces` (bez okna konsoli).
    pub fn wifi() -> (Option<String>, Option<u8>) {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let Ok(out) = std::process::Command::new("netsh")
            .args(["wlan", "show", "interfaces"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        else {
            return (None, None);
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let mut ssid = None;
        let mut signal = None;
        for line in text.lines() {
            let Some((k, v)) = line.split_once(':') else { continue };
            let (k, v) = (k.trim(), v.trim());
            if k == "SSID" && !v.is_empty() {
                ssid = Some(v.to_string());
            } else if let Some(p) = v.strip_suffix('%') {
                if let Ok(n) = p.trim().parse::<u8>() {
                    signal = Some(n);
                }
            }
        }
        if ssid.is_none() {
            signal = None;
        }
        (ssid, signal)
    }

    /// (jest moduł Bluetooth, liczba połączonych urządzeń)
    pub fn bluetooth() -> (bool, u32) {
        unsafe {
            let params = BLUETOOTH_FIND_RADIO_PARAMS { dwSize: std::mem::size_of::<BLUETOOTH_FIND_RADIO_PARAMS>() as u32 };
            let mut radio = HANDLE::default();
            let Ok(find) = BluetoothFindFirstRadio(&params, &mut radio) else { return (false, 0) };
            let _ = BluetoothFindRadioClose(find);
            let _ = CloseHandle(radio);

            let search = BLUETOOTH_DEVICE_SEARCH_PARAMS {
                dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
                fReturnAuthenticated: BOOL(0),
                fReturnRemembered: BOOL(0),
                fReturnUnknown: BOOL(0),
                fReturnConnected: BOOL(1),
                fIssueInquiry: BOOL(0),
                cTimeoutMultiplier: 0,
                hRadio: HANDLE::default(),
            };
            let mut info = BLUETOOTH_DEVICE_INFO { dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32, ..Default::default() };
            let mut count = 0;
            if let Ok(dev) = BluetoothFindFirstDevice(&search, &mut info) {
                loop {
                    if info.fConnected.as_bool() {
                        count += 1;
                    }
                    if BluetoothFindNextDevice(dev, &mut info).is_err() {
                        break;
                    }
                }
                let _ = BluetoothFindDeviceClose(dev);
            }
            (true, count)
        }
    }

    // ---------------- Pasek zadań Windows ----------------

    fn taskbars() -> Vec<HWND> {
        let mut out = Vec::new();
        unsafe {
            for class in [windows::core::w!("Shell_TrayWnd"), windows::core::w!("Shell_SecondaryTrayWnd")] {
                if let Ok(h) = FindWindowW(class, PCWSTR::null()) {
                    if !h.0.is_null() {
                        out.push(h);
                    }
                }
            }
        }
        out
    }

    fn appbar(h: HWND, state: isize) -> APPBARDATA {
        APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: h,
            lParam: LPARAM(state),
            ..Default::default()
        }
    }

    /// Bieżący stan paska (ABS_AUTOHIDE = 1, ABS_ALWAYSONTOP = 2).
    pub fn taskbar_state() -> usize {
        let Some(h) = taskbars().into_iter().next() else { return 0 };
        let mut abd = appbar(h, 0);
        unsafe { SHAppBarMessage(ABM_GETSTATE, &mut abd) }
    }

    /// Chowa pasek: autoukrywanie (żeby oddał miejsce oknom) + ukrycie okna
    /// (żeby nie wyskakiwał, gdy kursor zjedzie na dół do docka).
    pub fn hide_taskbar(hide: bool, restore_state: usize) {
        for h in taskbars() {
            let state = if hide { 1 } else { restore_state as isize };
            let mut abd = appbar(h, state);
            unsafe {
                SHAppBarMessage(ABM_SETSTATE, &mut abd);
                let _ = ShowWindow(h, if hide { SW_HIDE } else { SW_SHOW });
            }
        }
    }

    pub fn taskbar_visible() -> bool {
        taskbars().into_iter().any(|h| unsafe { IsWindowVisible(h).as_bool() })
    }

    // ---------------- Ikony aplikacji ----------------

    fn reg_string(root: HKEY, key: &str, value: Option<&str>) -> Option<String> {
        let key = HSTRING::from(key);
        let value_h = value.map(HSTRING::from);
        let value_p = value_h.as_ref().map(|v| PCWSTR(v.as_ptr())).unwrap_or(PCWSTR::null());
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let rc = unsafe {
            RegGetValueW(root, &key, value_p, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut c_void), Some(&mut size as *mut u32))
        };
        if rc.is_err() {
            return None;
        }
        let len = (size as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
    }

    /// Zamienia to, co jest w docku, na plik z ikoną: `excel.exe` → pełna
    /// ścieżka z App Paths, `spotify:` → DefaultIcon protokołu.
    pub fn resolve_icon_source(path: &str) -> Option<(String, i32)> {
        let p = path.trim();
        if p.is_empty() || p.starts_with("http://") || p.starts_with("https://") {
            return None;
        }
        // URI: `spotify:`, `steam://...`, `discord:`
        if let Some((scheme, _)) = p.split_once(':') {
            if scheme.len() > 1 && !scheme.contains('\\') {
                let icon = reg_string(HKEY_CLASSES_ROOT, &format!("{scheme}\\DefaultIcon"), None)
                    .or_else(|| reg_string(HKEY_CLASSES_ROOT, &format!("{scheme}\\shell\\open\\command"), None));
                return icon.map(|s| split_icon_spec(&s));
            }
        }
        if std::path::Path::new(p).is_absolute() {
            return Some((p.to_string(), 0));
        }
        let key = format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{p}");
        if let Some(full) = reg_string(HKEY_LOCAL_MACHINE, &key, None).or_else(|| reg_string(HKEY_CURRENT_USER, &key, None)) {
            return Some((full.trim_matches('"').to_string(), 0));
        }
        let path_var = std::env::var("PATH").unwrap_or_default();
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        for dir in path_var.split(';').chain(std::iter::once(windir.as_str())) {
            let cand = std::path::Path::new(dir.trim()).join(p);
            if cand.is_file() {
                return Some((cand.to_string_lossy().to_string(), 0));
            }
        }
        None
    }

    /// `"C:\x\app.exe",0` albo `"C:\x\app.exe" --arg "%1"` → (ścieżka, indeks)
    fn split_icon_spec(s: &str) -> (String, i32) {
        let s = s.trim();
        if let Some(rest) = s.strip_prefix('"') {
            let end = rest.find('"').unwrap_or(rest.len());
            let file = rest[..end].to_string();
            let idx = rest[end..].trim_start_matches('"').trim().trim_start_matches(',').trim().parse().unwrap_or(0);
            return (file, idx);
        }
        match s.rsplit_once(',') {
            Some((f, i)) if i.trim().parse::<i32>().is_ok() => (f.trim().to_string(), i.trim().parse().unwrap_or(0)),
            _ => (s.split(" -").next().unwrap_or(s).trim().to_string(), 0),
        }
    }

    fn extract_hicon(file: &str, index: i32, size: i32) -> Option<HICON> {
        let lower = file.to_lowercase();
        if lower.ends_with(".exe") || lower.ends_with(".dll") || lower.ends_with(".ico") {
            let mut name = [0u16; 260];
            for (i, c) in file.encode_utf16().take(259).enumerate() {
                name[i] = c;
            }
            let mut icons = [HICON::default(); 1];
            let got = unsafe { PrivateExtractIconsW(&name, index, size, size, Some(&mut icons), None, 0) };
            if got > 0 && got != u32::MAX && !icons[0].0.is_null() {
                return Some(icons[0]);
            }
        }
        // .lnk, foldery i reszta: ikona z powłoki (32 px).
        let wide = HSTRING::from(file);
        let mut info = SHFILEINFOW::default();
        let ok = unsafe {
            SHGetFileInfoW(
                &wide,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(&mut info as *mut SHFILEINFOW),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON,
            )
        };
        (ok != 0 && !info.hIcon.0.is_null()).then_some(info.hIcon)
    }

    /// PNG (RGBA) z ikony pliku/aplikacji.
    pub fn icon_png(file: &str, index: i32, size: i32) -> Option<Vec<u8>> {
        let hicon = extract_hicon(file, index, size)?;
        let result = (|| unsafe {
            let mut ii = ICONINFO::default();
            GetIconInfo(hicon, &mut ii).ok()?;
            let mut bm = BITMAP::default();
            GetObjectW(ii.hbmColor, std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut BITMAP as *mut c_void));
            let (w, h) = (bm.bmWidth, bm.bmHeight);
            if w <= 0 || h <= 0 {
                let _ = DeleteObject(ii.hbmColor);
                let _ = DeleteObject(ii.hbmMask);
                return None;
            }
            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            };
            let mut px = vec![0u8; (w * h * 4) as usize];
            let dc = CreateCompatibleDC(HDC::default());
            let lines = GetDIBits(dc, ii.hbmColor, 0, h as u32, Some(px.as_mut_ptr() as *mut c_void), &mut bmi, DIB_RGB_COLORS);
            let _ = DeleteDC(dc);
            let _ = DeleteObject(ii.hbmColor);
            let _ = DeleteObject(ii.hbmMask);
            if lines == 0 {
                return None;
            }
            let has_alpha = px.chunks(4).any(|p| p[3] != 0);
            for p in px.chunks_mut(4) {
                p.swap(0, 2); // BGRA → RGBA
                if !has_alpha {
                    p[3] = 255;
                }
            }
            let mut out = Vec::new();
            {
                let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                let mut wr = enc.write_header().ok()?;
                wr.write_image_data(&px).ok()?;
            }
            Some(out)
        })();
        unsafe {
            let _ = DestroyIcon(hicon);
        }
        result
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn cpu_times() -> Option<(u64, u64, u64)> { None }
    pub fn memory() -> Option<(f32, f32, f32)> { None }
    pub fn net_bytes() -> Option<(u64, u64)> { None }
    pub fn volume() -> Option<(u8, bool)> { None }
    pub fn set_volume(_: u8) {}
    pub fn toggle_mute() {}
    pub fn wifi() -> (Option<String>, Option<u8>) { (None, None) }
    pub fn bluetooth() -> (bool, u32) { (false, 0) }
    pub fn taskbar_state() -> usize { 0 }
    pub fn hide_taskbar(_: bool, _: usize) {}
    pub fn taskbar_visible() -> bool { false }
    pub fn resolve_icon_source(_: &str) -> Option<(String, i32)> { None }
    pub fn icon_png(_: &str, _: i32, _: i32) -> Option<Vec<u8>> { None }
}

pub use imp::*;
