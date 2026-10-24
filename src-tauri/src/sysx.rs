//! Rozszerzone odczyty i sterowanie systemem dla kart z górnego paska:
//! procesy (CPU/RAM), sieci Wi‑Fi, urządzenia Bluetooth z baterią, wyjścia
//! audio i głośność aplikacji, oraz zasilanie (lewy pasek).

use serde::Serialize;

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProcRow {
    pub name: String,
    pub exe: String,
    pub pids: Vec<u32>,
    pub cpu: f32,
    pub mem_mb: f32,
    pub protected: bool,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct WifiNet {
    pub ssid: String,
    pub signal: u8,
    pub secure: bool,
    pub known: bool,
    pub connected: bool,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct BtDevice {
    pub name: String,
    pub connected: bool,
    pub battery: Option<u8>,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioOut {
    pub id: String,
    pub name: String,
    pub default: bool,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioApp {
    pub pid: u32,
    pub name: String,
    pub exe: String,
    pub volume: u8,
    pub muted: bool,
}

/// Procesy, których nie pozwalamy zakończyć z paska (system by się wysypał).
const PROTECTED: &[&str] = &[
    "system", "idle", "registry", "smss.exe", "csrss.exe", "wininit.exe", "winlogon.exe", "services.exe",
    "lsass.exe", "svchost.exe", "fontdrvhost.exe", "dwm.exe", "memory compression", "secure system",
];

#[cfg(windows)]
mod imp {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::os::windows::process::CommandExt;
    use windows::core::{Interface, GUID, HRESULT, IUnknown, PCWSTR, PWSTR};
    use windows::Win32::Devices::Bluetooth::{
        BluetoothFindDeviceClose, BluetoothFindFirstDevice, BluetoothFindNextDevice, BLUETOOTH_DEVICE_INFO,
        BLUETOOTH_DEVICE_SEARCH_PARAMS,
    };
    use windows::Win32::Foundation::{CloseHandle, BOOL, FILETIME, HANDLE, S_OK};
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume,
        MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

    const NO_WINDOW: u32 = 0x0800_0000;

    fn ft(f: FILETIME) -> u64 {
        ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
    }

    fn wide_str(buf: &[u16]) -> String {
        let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..n])
    }

    fn snapshot() -> Vec<(u32, String)> {
        let mut out = Vec::new();
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
            let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            if Process32FirstW(snap, &mut e).is_ok() {
                loop {
                    out.push((e.th32ProcessID, wide_str(&e.szExeFile)));
                    if Process32NextW(snap, &mut e).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
        }
        out
    }

    /// (czas CPU w 100 ns, pamięć w bajtach, pełna ścieżka)
    fn proc_stats(pid: u32) -> Option<(u64, u64, String)> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let (mut c, mut x, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
            let times = GetProcessTimes(h, &mut c, &mut x, &mut k, &mut u).ok().map(|_| ft(k) + ft(u)).unwrap_or(0);
            let mut mc = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
            let mem = if GetProcessMemoryInfo(h, &mut mc, mc.cb).is_ok() { mc.WorkingSetSize as u64 } else { 0 };
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let path = if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok() {
                String::from_utf16_lossy(&buf[..len as usize])
            } else {
                String::new()
            };
            let _ = CloseHandle(h);
            Some((times, mem, path))
        }
    }

    fn cpus() -> f32 {
        let mut si = SYSTEM_INFO::default();
        unsafe { GetSystemInfo(&mut si) };
        si.dwNumberOfProcessors.max(1) as f32
    }

    /// Procesy pogrupowane po nazwie pliku; CPU z dwóch próbek co 500 ms.
    pub fn processes() -> Vec<ProcRow> {
        let first: HashMap<u32, u64> = snapshot().into_iter().filter_map(|(pid, _)| proc_stats(pid).map(|s| (pid, s.0))).collect();
        let t0 = std::time::Instant::now();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let wall = t0.elapsed().as_nanos() as f32 / 100.0;
        let n = cpus();
        let mut groups: HashMap<String, ProcRow> = HashMap::new();
        for (pid, exe) in snapshot() {
            if pid == 0 {
                continue;
            }
            let Some((t, mem, path)) = proc_stats(pid) else { continue };
            let cpu = first.get(&pid).map(|t1| (t.saturating_sub(*t1)) as f32 / wall / n * 100.0).unwrap_or(0.0);
            let key = exe.to_lowercase();
            let g = groups.entry(key.clone()).or_insert_with(|| ProcRow {
                name: crate::sys::file_description(&path).filter(|d| d.chars().count() <= 40).unwrap_or_else(|| exe.trim_end_matches(".exe").to_string()),
                exe: key.clone(),
                protected: PROTECTED.contains(&key.as_str()),
                ..ProcRow::default()
            });
            g.pids.push(pid);
            g.cpu += cpu;
            g.mem_mb += mem as f32 / 1_048_576.0;
        }
        groups.into_values().collect()
    }

    pub fn kill(pids: &[u32]) -> usize {
        let mut done = 0;
        for pid in pids {
            unsafe {
                if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, *pid) {
                    if TerminateProcess(h, 1).is_ok() {
                        done += 1;
                    }
                    let _ = CloseHandle(h);
                }
            }
        }
        done
    }

    // ---------------- Wi‑Fi (netsh) ----------------

    fn netsh(args: &[&str]) -> String {
        std::process::Command::new("netsh")
            .args(args)
            .creation_flags(NO_WINDOW)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default()
    }

    fn known_profiles() -> Vec<String> {
        // Etykiety są tłumaczone („All User Profile” / „Profil wszystkich użytkowników”),
        // więc bierzemy wartości z wciętych linii „… : nazwa” w sekcji profili.
        let text = netsh(&["wlan", "show", "profiles"]);
        let mut out = Vec::new();
        let mut in_list = false;
        for line in text.lines() {
            if line.trim().starts_with("---") {
                in_list = true;
                continue;
            }
            if in_list && line.starts_with("    ") {
                if let Some((_, v)) = line.split_once(" : ") {
                    let v = v.trim();
                    if !v.is_empty() {
                        out.push(v.to_string());
                    }
                }
            }
        }
        out
    }

    pub fn wifi_networks() -> Vec<WifiNet> {
        let text = netsh(&["wlan", "show", "networks", "mode=bssid"]);
        let known = known_profiles();
        let current = crate::sys::wifi().0;
        let mut out: Vec<WifiNet> = Vec::new();
        for line in text.lines() {
            let t = line.trim();
            let Some((k, v)) = t.split_once(':') else { continue };
            let (k, v) = (k.trim(), v.trim());
            if k.starts_with("SSID") && !k.starts_with("BSSID") {
                out.push(WifiNet {
                    ssid: v.to_string(),
                    known: known.iter().any(|p| p == v),
                    connected: current.as_deref() == Some(v),
                    secure: true,
                    ..WifiNet::default()
                });
            } else if let Some(last) = out.last_mut() {
                if let Some(p) = v.strip_suffix('%').and_then(|p| p.trim().parse::<u8>().ok()) {
                    last.signal = last.signal.max(p);
                } else {
                    let lv = v.to_lowercase();
                    if lv == "open" || lv.starts_with("otwart") {
                        last.secure = false;
                    }
                }
            }
        }
        out.retain(|n| !n.ssid.is_empty());
        out.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.signal.cmp(&a.signal)));
        out
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
    }

    /// Łączy z siecią; nowa sieć z hasłem dostaje profil WPA2‑Personal.
    pub fn wifi_connect(ssid: &str, password: Option<&str>) -> Result<(), String> {
        if let Some(pw) = password.filter(|p| !p.is_empty()) {
            let hex: String = ssid.bytes().map(|b| format!("{b:02X}")).collect();
            let xml = format!(
                r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
<name>{n}</name><SSIDConfig><SSID><hex>{hex}</hex><name>{n}</name></SSID></SSIDConfig>
<connectionType>ESS</connectionType><connectionMode>auto</connectionMode>
<MSM><security><authEncryption><authentication>WPA2PSK</authentication><encryption>AES</encryption><useOneX>false</useOneX></authEncryption>
<sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{k}</keyMaterial></sharedKey></security></MSM>
</WLANProfile>"#,
                n = xml_escape(ssid),
                k = xml_escape(pw)
            );
            let path = std::env::temp_dir().join(format!("edge-wifi-{}.xml", std::process::id()));
            std::fs::write(&path, xml).map_err(|e| e.to_string())?;
            let out = netsh(&["wlan", "add", "profile", &format!("filename={}", path.display()), "user=current"]);
            let _ = std::fs::remove_file(&path);
            if out.trim().is_empty() {
                return Err("Nie udało się zapisać profilu sieci".into());
            }
        } else if !known_profiles().iter().any(|p| p == ssid) {
            return Err("needs-password".into());
        }
        let out = std::process::Command::new("netsh")
            .args(["wlan", "connect", &format!("name={ssid}"), &format!("ssid={ssid}")])
            .creation_flags(NO_WINDOW)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stdout).trim().to_string()) }
    }

    pub fn wifi_disconnect() {
        netsh(&["wlan", "disconnect"]);
    }

    // ---------------- Bluetooth ----------------

    pub fn bt_devices() -> Vec<BtDevice> {
        let mut out = Vec::new();
        unsafe {
            let search = BLUETOOTH_DEVICE_SEARCH_PARAMS {
                dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
                fReturnAuthenticated: BOOL(1),
                fReturnRemembered: BOOL(1),
                fReturnUnknown: BOOL(0),
                fReturnConnected: BOOL(1),
                fIssueInquiry: BOOL(0),
                cTimeoutMultiplier: 0,
                hRadio: HANDLE::default(),
            };
            let mut info = BLUETOOTH_DEVICE_INFO { dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32, ..Default::default() };
            if let Ok(h) = BluetoothFindFirstDevice(&search, &mut info) {
                loop {
                    let name = wide_str(&info.szName);
                    if !name.is_empty() && !out.iter().any(|d: &BtDevice| d.name == name) {
                        out.push(BtDevice { name, connected: info.fConnected.as_bool(), battery: None });
                    }
                    if BluetoothFindNextDevice(h, &mut info).is_err() {
                        break;
                    }
                }
                let _ = BluetoothFindDeviceClose(h);
            }
        }
        out.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.cmp(&b.name)));
        out
    }

    /// Poziom baterii urządzeń BT (właściwość PnP z Windows; wolne — w tle).
    pub fn bt_batteries() -> Vec<(String, u8)> {
        let script = "Get-PnpDevice -Class Bluetooth -PresentOnly -ErrorAction SilentlyContinue | ForEach-Object { $b = (Get-PnpDeviceProperty -InstanceId $_.InstanceId -KeyName '{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2' -ErrorAction SilentlyContinue).Data; if ($b -ne $null) { \"$($_.FriendlyName)`t$b\" } }";
        let Ok(out) = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(NO_WINDOW)
            .output()
        else {
            return Vec::new();
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| {
                let (n, b) = l.split_once('\t')?;
                Some((n.trim().to_string(), b.trim().parse::<u8>().ok()?))
            })
            .collect()
    }

    // ---------------- Dźwięk ----------------

    fn enumerator() -> Option<IMMDeviceEnumerator> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()
        }
    }

    fn device_id(dev: &windows::Win32::Media::Audio::IMMDevice) -> Option<String> {
        unsafe {
            let p = dev.GetId().ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const c_void));
            s
        }
    }

    pub fn audio_outputs() -> Vec<AudioOut> {
        use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
        let Some(en) = enumerator() else { return Vec::new() };
        let mut out = Vec::new();
        unsafe {
            let default = en.GetDefaultAudioEndpoint(eRender, eConsole).ok().and_then(|d| device_id(&d));
            let Ok(col) = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return out };
            let n = col.GetCount().unwrap_or(0);
            for i in 0..n {
                let Ok(dev) = col.Item(i) else { continue };
                let Some(id) = device_id(&dev) else { continue };
                let name = dev
                    .OpenPropertyStore(STGM_READ)
                    .ok()
                    .and_then(|ps: IPropertyStore| ps.GetValue(&PKEY_Device_FriendlyName).ok())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "Urządzenie audio".into());
                out.push(AudioOut { default: default.as_deref() == Some(id.as_str()), id, name });
            }
        }
        out
    }

    /// Ustawia domyślne wyjście (nieudokumentowany IPolicyConfig — tak robi
    /// każde narzędzie do przełączania audio w Windows).
    pub fn set_default_output(id: &str) -> Result<(), String> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let clsid = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
            let iid = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
            let unk: IUnknown = CoCreateInstance(&clsid, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
            let mut p: *mut c_void = std::ptr::null_mut();
            let hr = unk.query(&iid, &mut p);
            if hr != S_OK || p.is_null() {
                return Err(format!("IPolicyConfig: {hr:?}"));
            }
            let vtbl = *(p as *const *const usize);
            type SetDefault = unsafe extern "system" fn(*mut c_void, PCWSTR, i32) -> HRESULT;
            type Release = unsafe extern "system" fn(*mut c_void) -> u32;
            let set: SetDefault = std::mem::transmute(*vtbl.add(13));
            let release: Release = std::mem::transmute(*vtbl.add(2));
            let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
            let mut result = Ok(());
            for role in 0..3 {
                let hr = set(p, PCWSTR(wide.as_ptr()), role);
                if hr.is_err() {
                    result = Err(format!("SetDefaultEndpoint: {hr:?}"));
                }
            }
            release(p);
            result
        }
    }

    pub fn exe_path(pid: u32) -> String {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
            let _ = CloseHandle(h);
            if ok { String::from_utf16_lossy(&buf[..len as usize]) } else { String::new() }
        }
    }

    fn sessions() -> Vec<(IAudioSessionControl2, ISimpleAudioVolume)> {
        let mut out = Vec::new();
        let Some(en) = enumerator() else { return out };
        unsafe {
            let Ok(dev) = en.GetDefaultAudioEndpoint(eRender, eConsole) else { return out };
            let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { return out };
            let Ok(list) = mgr.GetSessionEnumerator() else { return out };
            let n = list.GetCount().unwrap_or(0);
            for i in 0..n {
                let Ok(s) = list.GetSession(i) else { continue };
                let (Ok(c2), Ok(vol)) = (s.cast::<IAudioSessionControl2>(), s.cast::<ISimpleAudioVolume>()) else { continue };
                if c2.IsSystemSoundsSession() == S_OK {
                    continue;
                }
                out.push((c2, vol));
            }
        }
        out
    }

    pub fn audio_apps() -> Vec<AudioApp> {
        let mut out: Vec<AudioApp> = Vec::new();
        for (c2, vol) in sessions() {
            unsafe {
                let pid = c2.GetProcessId().unwrap_or(0);
                if pid == 0 || out.iter().any(|a| a.pid == pid) {
                    continue;
                }
                let path = exe_path(pid);
                let exe = crate::platform::exe_name(&path);
                if exe.is_empty() {
                    continue;
                }
                let name = crate::sys::file_description(&path).filter(|d| d.chars().count() <= 32).unwrap_or_else(|| exe.trim_end_matches(".exe").to_string());
                let volume = (vol.GetMasterVolume().unwrap_or(1.0) * 100.0).round() as u8;
                let muted = vol.GetMute().map(|b| b.as_bool()).unwrap_or(false);
                out.push(AudioApp { pid, name, exe, volume, muted });
            }
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out
    }

    pub fn set_app_volume(pid: u32, volume: Option<u8>, mute: Option<bool>) {
        for (c2, vol) in sessions() {
            unsafe {
                if c2.GetProcessId().unwrap_or(0) != pid {
                    continue;
                }
                if let Some(v) = volume {
                    let _ = vol.SetMasterVolume(v.min(100) as f32 / 100.0, std::ptr::null());
                }
                if let Some(m) = mute {
                    let _ = vol.SetMute(m, std::ptr::null());
                }
            }
        }
    }

    // ---------------- Zasilanie ----------------

    pub fn power(action: &str) -> Result<(), String> {
        use windows::Win32::System::Power::SetSuspendState;
        let run = |exe: &str, args: &[&str]| {
            std::process::Command::new(exe).args(args).creation_flags(NO_WINDOW).spawn().map(|_| ()).map_err(|e| e.to_string())
        };
        match action {
            "lock" => run("rundll32.exe", &["user32.dll,LockWorkStation"]),
            "sleep" => {
                if unsafe { SetSuspendState(false, false, false) }.as_bool() { Ok(()) } else { Err("Windows nie pozwolił uśpić komputera".into()) }
            }
            "hibernate" => {
                if unsafe { SetSuspendState(true, false, false) }.as_bool() {
                    Ok(())
                } else {
                    Err("Hibernacja jest wyłączona w Windows (powercfg /hibernate on)".into())
                }
            }
            "restart" => run("shutdown.exe", &["/r", "/t", "0"]),
            "shutdown" => run("shutdown.exe", &["/s", "/hybrid", "/t", "0"]),
            "logout" => run("shutdown.exe", &["/l"]),
            _ => Err("nieznana akcja".into()),
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn processes() -> Vec<ProcRow> { Vec::new() }
    pub fn kill(_: &[u32]) -> usize { 0 }
    pub fn wifi_networks() -> Vec<WifiNet> { Vec::new() }
    pub fn wifi_connect(_: &str, _: Option<&str>) -> Result<(), String> { Err("tylko Windows".into()) }
    pub fn wifi_disconnect() {}
    pub fn bt_devices() -> Vec<BtDevice> { Vec::new() }
    pub fn bt_batteries() -> Vec<(String, u8)> { Vec::new() }
    pub fn audio_outputs() -> Vec<AudioOut> { Vec::new() }
    pub fn set_default_output(_: &str) -> Result<(), String> { Err("tylko Windows".into()) }
    pub fn audio_apps() -> Vec<AudioApp> { Vec::new() }
    pub fn set_app_volume(_: u32, _: Option<u8>, _: Option<bool>) {}
    pub fn power(_: &str) -> Result<(), String> { Err("tylko Windows".into()) }
    pub fn exe_path(_: u32) -> String { String::new() }
}

pub use imp::*;
