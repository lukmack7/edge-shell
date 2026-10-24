//! Okna: strefy przyciągania (pod ultrawide) i zapamiętane układy.
//!
//! Strefy: Ctrl+Alt+1…6 wkłada aktywne okno do strefy; przeciąganie okna z
//! wciśniętym Shift pokazuje strefy na ekranie i po puszczeniu dopasowuje okno.
//! Układy: zapis pozycji wszystkich okien (np. „Praca na ultrawide”) i ich
//! przywrócenie z palety albo automatyzacją — brakujące aplikacje uruchamiamy.

use crate::platform::{self, Rect};
use crate::state::{self, WinLayout, WinPlace};
use std::sync::atomic::{AtomicIsize, Ordering};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

/// Strefy jako ułamki obszaru roboczego: (x, y, w, h).
pub fn template(name: &str) -> Vec<(f64, f64, f64, f64)> {
    match name {
        "wide-center" => vec![(0.0, 0.0, 0.25, 1.0), (0.25, 0.0, 0.5, 1.0), (0.75, 0.0, 0.25, 1.0), (0.0, 0.0, 0.75, 1.0), (0.25, 0.0, 0.75, 1.0), (0.0, 0.0, 1.0, 1.0)],
        "halves" => vec![(0.0, 0.0, 0.5, 1.0), (0.5, 0.0, 0.5, 1.0), (0.0, 0.0, 1.0, 1.0)],
        "wide-left" => vec![(0.0, 0.0, 0.6, 1.0), (0.6, 0.0, 0.4, 1.0), (0.6, 0.0, 0.4, 0.5), (0.6, 0.5, 0.4, 0.5), (0.0, 0.0, 1.0, 1.0)],
        "quarters" => vec![(0.0, 0.0, 0.5, 0.5), (0.5, 0.0, 0.5, 0.5), (0.0, 0.5, 0.5, 0.5), (0.5, 0.5, 0.5, 0.5), (0.0, 0.0, 0.5, 1.0), (0.5, 0.0, 0.5, 1.0)],
        // thirds
        _ => vec![(0.0, 0.0, 1.0 / 3.0, 1.0), (1.0 / 3.0, 0.0, 1.0 / 3.0, 1.0), (2.0 / 3.0, 0.0, 1.0 / 3.0, 1.0), (0.0, 0.0, 2.0 / 3.0, 1.0), (1.0 / 3.0, 0.0, 2.0 / 3.0, 1.0), (0.0, 0.0, 1.0, 1.0)],
    }
}

/// Obszar roboczy monitora, na którym jest punkt (bez naszego paska i docka).
fn work_area_at(app: &AppHandle, x: i32, y: i32) -> Rect {
    let s = crate::layout::screen(app);
    for m in app.available_monitors().unwrap_or_default() {
        let p = m.position();
        let z = m.size();
        let r = Rect { left: p.x, top: p.y, right: p.x + z.width as i32, bottom: p.y + z.height as i32 };
        if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
            if r.left == s.monitor.left && r.top == s.monitor.top {
                // Główny: obszar roboczy minus miejsce na dock, gdy dock jest stały.
                let cfg = state::cfg(app);
                let mut w = s.work;
                if cfg.shell.show_dock && !cfg.shell.dock_autohide {
                    w.bottom -= (crate::layout::DOCK_H * s.scale) as i32;
                }
                return w;
            }
            let bar = if state::cfg(app).shell.bar_on_all_monitors { (crate::layout::BAR_H * m.scale_factor()) as i32 } else { 0 };
            return Rect { top: r.top + bar, ..r };
        }
    }
    s.work
}

pub fn zone_rects(app: &AppHandle, x: i32, y: i32) -> Vec<Rect> {
    let cfg = state::cfg(app);
    let area = work_area_at(app, x, y);
    let gap = cfg.windows.gap.max(0);
    let (w, h) = ((area.right - area.left) as f64, (area.bottom - area.top) as f64);
    template(&cfg.windows.zones)
        .into_iter()
        .map(|(fx, fy, fw, fh)| Rect {
            left: area.left + (fx * w) as i32 + gap,
            top: area.top + (fy * h) as i32 + gap,
            right: area.left + ((fx + fw) * w) as i32 - gap,
            bottom: area.top + ((fy + fh) * h) as i32 - gap,
        })
        .collect()
}

/// Ctrl+Alt+N: aktywne okno do strefy N.
pub fn snap_foreground(app: &AppHandle, n: usize) {
    let fg = platform::foreground_hwnd();
    if fg == 0 || platform::is_own_window(fg) {
        return;
    }
    let Some(r) = imp::window_rect(fg) else { return };
    let zones = zone_rects(app, (r.left + r.right) / 2, (r.top + r.bottom) / 2);
    if let Some(z) = zones.get(n.saturating_sub(1)) {
        imp::place(fg, *z, false);
    }
}

// ---------------------------------------------------------------------------
// Shift + przeciąganie: strefy na ekranie
// ---------------------------------------------------------------------------

static MOVING: AtomicIsize = AtomicIsize::new(0);

pub fn start(app: AppHandle) {
    imp::watch_moves();
    std::thread::spawn(move || {
        let mut shown = false;
        let mut hover: Option<Rect> = None;
        let mut last_moving = 0isize;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(40));
            let moving = MOVING.load(Ordering::Relaxed);
            let shift = imp::shift_down();
            if moving != 0 {
                last_moving = moving;
            }
            if moving != 0 && shift && state::cfg(&app).windows.shift_drag {
                let Some((cx, cy)) = platform::cursor() else { continue };
                let zones = zone_rects(&app, cx, cy);
                let hit = zones
                    .iter()
                    .find(|z| cx >= z.left && cx < z.right && cy >= z.top && cy < z.bottom)
                    .copied()
                    // Kursor w szczelinie między strefami: najbliższa w tym pasie.
                    .or_else(|| zones.iter().filter(|z| z.top <= cy && cy < z.bottom).min_by_key(|z| ((z.left + z.right) / 2 - cx).abs()).copied());
                if !shown {
                    show_overlay(&app, cx, cy, &zones);
                    shown = true;
                }
                if hit != hover {
                    hover = hit;
                    let _ = app.emit_to("zones", "zones-hover", hit.and_then(|h| zones.iter().position(|z| *z == h)));
                }
                continue;
            }
            if shown {
                if let Some(w) = app.get_webview_window("zones") {
                    let _ = w.hide();
                }
                shown = false;
            }
            // Okno puszczone z wciśniętym Shift nad strefą → dopasuj.
            if moving == 0 && last_moving != 0 {
                if shift {
                    if let Some(z) = hover {
                        imp::place(last_moving, z, false);
                    }
                }
                last_moving = 0;
            }
            hover = None;
        }
    });
}

#[derive(serde::Serialize, Clone)]
struct ZoneBox {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn show_overlay(app: &AppHandle, cx: i32, cy: i32, zones: &[Rect]) {
    let Some(w) = app.get_webview_window("zones") else { return };
    let mon = app.available_monitors().unwrap_or_default().into_iter().find(|m| {
        let p = m.position();
        let z = m.size();
        cx >= p.x && cx < p.x + z.width as i32 && cy >= p.y && cy < p.y + z.height as i32
    });
    let Some(m) = mon else { return };
    let (mx, my) = (m.position().x, m.position().y);
    let scale = m.scale_factor();
    let _ = w.set_position(PhysicalPosition::new(mx, my));
    let _ = w.set_size(PhysicalSize::new(m.size().width, m.size().height));
    let _ = w.set_ignore_cursor_events(true);
    // Strefy w px CSS okna nakładki.
    let boxes: Vec<ZoneBox> = zones
        .iter()
        .map(|z| ZoneBox {
            x: ((z.left - mx) as f64 / scale) as i32,
            y: ((z.top - my) as f64 / scale) as i32,
            w: ((z.right - z.left) as f64 / scale) as i32,
            h: ((z.bottom - z.top) as f64 / scale) as i32,
        })
        .collect();
    let _ = app.emit_to("zones", "zones", boxes);
    let _ = w.show();
}

// ---------------------------------------------------------------------------
// Układy okien
// ---------------------------------------------------------------------------

/// Zapisuje bieżące położenie okien jako układ (nazwa pusta = tryb + monitory).
pub fn save_layout(app: &AppHandle, name: &str) -> usize {
    let mode = state::read(app, |rt| rt.store.mode.clone());
    let monitors = crate::layout::monitors_signature(app);
    let cfg = state::cfg(app);
    let label = if name.trim().is_empty() {
        format!("{} · {} monitor(y)", cfg.mode(&mode).label, app.available_monitors().map(|m| m.len()).unwrap_or(1))
    } else {
        name.trim().to_string()
    };
    let windows: Vec<WinPlace> = platform::app_windows()
        .into_iter()
        .filter_map(|w| {
            let (r, maximized) = imp::placement(w.hwnd)?;
            Some(WinPlace {
                exe: platform::exe_name(&w.exe),
                path: w.exe,
                title: w.title,
                x: r.left,
                y: r.top,
                w: r.right - r.left,
                h: r.bottom - r.top,
                maximized,
            })
        })
        .collect();
    let n = windows.len();
    state::update(app, true, |rt| {
        rt.store.layouts.retain(|l| l.name != label);
        rt.store.layouts.insert(0, WinLayout { name: label, mode, monitors, windows });
        rt.store.layouts.truncate(20);
    });
    n
}

/// Przywraca układ: istniejące okna przesuwa, brakujące aplikacje uruchamia
/// i ustawia, gdy się pojawią (do 15 s).
pub fn restore_layout(app: &AppHandle, name: &str) -> Result<usize, String> {
    let layout = state::read(app, |rt| {
        rt.store.layouts.iter().find(|l| l.name == name).cloned().or_else(|| {
            // Bez nazwy: układ bieżącego trybu przy tych monitorach.
            let sig = crate::layout::monitors_signature(app);
            rt.store.layouts.iter().find(|l| l.mode == rt.store.mode && l.monitors == sig).cloned()
        })
    })
    .ok_or("Nie ma takiego układu")?;
    let mut used: Vec<isize> = Vec::new();
    let mut missing: Vec<WinPlace> = Vec::new();
    let mut placed = 0;
    let windows = platform::app_windows();
    for wp in &layout.windows {
        // Najpierw to samo okno po tytule, potem dowolne okno tej aplikacji.
        let hit = windows
            .iter()
            .find(|w| !used.contains(&w.hwnd) && platform::exe_name(&w.exe) == wp.exe && w.title == wp.title)
            .or_else(|| windows.iter().find(|w| !used.contains(&w.hwnd) && platform::exe_name(&w.exe) == wp.exe));
        match hit {
            Some(w) => {
                used.push(w.hwnd);
                imp::place(w.hwnd, Rect { left: wp.x, top: wp.y, right: wp.x + wp.w, bottom: wp.y + wp.h }, wp.maximized);
                placed += 1;
            }
            None => {
                if !missing.iter().any(|m| m.exe == wp.exe) {
                    missing.push(wp.clone());
                }
            }
        }
    }
    if !missing.is_empty() {
        for m in &missing {
            if std::path::Path::new(&m.path).exists() {
                let _ = std::process::Command::new(&m.path).spawn();
            }
        }
        std::thread::spawn(move || {
            for _ in 0..30 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let wins = platform::app_windows();
                missing.retain(|m| match wins.iter().find(|w| platform::exe_name(&w.exe) == m.exe && !used.contains(&w.hwnd)) {
                    Some(w) => {
                        imp::place(w.hwnd, Rect { left: m.x, top: m.y, right: m.x + m.w, bottom: m.y + m.h }, m.maximized);
                        false
                    }
                    None => true,
                });
                if missing.is_empty() {
                    break;
                }
            }
        });
    }
    Ok(placed)
}

#[tauri::command]
pub fn layout_save(app: AppHandle, name: String) -> usize {
    save_layout(&app, &name)
}

#[tauri::command]
pub fn layout_restore(app: AppHandle, name: String) -> Result<usize, String> {
    restore_layout(&app, &name)
}

#[tauri::command]
pub fn layout_list(app: AppHandle) -> Vec<WinLayout> {
    state::read(&app, |rt| rt.store.layouts.clone())
}

#[tauri::command]
pub fn layout_delete(app: AppHandle, name: String) {
    state::update(&app, true, |rt| rt.store.layouts.retain(|l| l.name != name));
}

#[tauri::command]
pub fn zone_snap(app: AppHandle, zone: usize) {
    snap_foreground(&app, zone);
}

#[cfg(windows)]
mod imp {
    use super::{Rect, MOVING};
    use std::ffi::c_void;
    use std::sync::atomic::Ordering;
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_SHIFT};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, GetWindowPlacement, GetWindowRect, IsZoomed, SetWindowPos, ShowWindow, TranslateMessage,
        EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART, MSG, SWP_NOACTIVATE, SWP_NOZORDER, SW_MAXIMIZE, SW_RESTORE,
        WINDOWPLACEMENT, WINEVENT_OUTOFCONTEXT,
    };

    fn hwnd(raw: isize) -> HWND {
        HWND(raw as *mut c_void)
    }

    pub fn shift_down() -> bool {
        unsafe { (GetAsyncKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0 }
    }

    pub fn window_rect(raw: isize) -> Option<Rect> {
        let mut r = RECT::default();
        unsafe { GetWindowRect(hwnd(raw), &mut r).ok()? };
        Some(Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom })
    }

    /// (prostokąt „normalny” okna, czy zmaksymalizowane)
    pub fn placement(raw: isize) -> Option<(Rect, bool)> {
        unsafe {
            let h = hwnd(raw);
            let maximized = IsZoomed(h).as_bool();
            let mut wp = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
            let r = if maximized {
                GetWindowPlacement(h, &mut wp).ok()?;
                // rcNormalPosition jest we współrzędnych obszaru roboczego — wystarczy
                // jako miejsce, z którego okno się zmaksymalizuje na właściwym ekranie.
                wp.rcNormalPosition
            } else {
                let mut r = RECT::default();
                GetWindowRect(h, &mut r).ok()?;
                r
            };
            Some((Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }, maximized))
        }
    }

    /// Ustawia okno tak, żeby jego widoczna ramka (bez niewidzialnych marginesów
    /// Windows 10/11) wypełniła prostokąt.
    pub fn place(raw: isize, target: Rect, maximize: bool) {
        unsafe {
            let h = hwnd(raw);
            if IsZoomed(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            let mut win = RECT::default();
            let mut frame = RECT::default();
            let _ = GetWindowRect(h, &mut win);
            let has_frame = DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut frame as *mut RECT as *mut c_void, std::mem::size_of::<RECT>() as u32).is_ok();
            let (l, t, r, b) = if has_frame {
                (frame.left - win.left, frame.top - win.top, win.right - frame.right, win.bottom - frame.bottom)
            } else {
                (0, 0, 0, 0)
            };
            let _ = SetWindowPos(
                h,
                HWND::default(),
                target.left - l,
                target.top - t,
                (target.right - target.left) + l + r,
                (target.bottom - target.top) + t + b,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            if maximize {
                let _ = ShowWindow(h, SW_MAXIMIZE);
            }
        }
    }

    unsafe extern "system" fn on_event(_: HWINEVENTHOOK, event: u32, h: HWND, id_object: i32, _: i32, _: u32, _: u32) {
        if id_object != 0 {
            return;
        }
        if event == EVENT_SYSTEM_MOVESIZESTART {
            MOVING.store(h.0 as isize, Ordering::Relaxed);
        } else if event == EVENT_SYSTEM_MOVESIZEEND {
            MOVING.store(0, Ordering::Relaxed);
        }
    }

    /// Wątek z pętlą komunikatów — tam przychodzą zdarzenia przesuwania okien.
    pub fn watch_moves() {
        std::thread::spawn(|| unsafe {
            let _hook = SetWinEventHook(EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MOVESIZEEND, windows::Win32::Foundation::HMODULE::default(), Some(on_event), 0, 0, WINEVENT_OUTOFCONTEXT);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Rect;
    pub fn shift_down() -> bool { false }
    pub fn window_rect(_: isize) -> Option<Rect> { None }
    pub fn placement(_: isize) -> Option<(Rect, bool)> { None }
    pub fn place(_: isize, _: Rect, _: bool) {}
    pub fn watch_moves() {}
}
