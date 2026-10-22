//! Pływający odtwarzacz Spotify „na wierzchu”: karta albo pasek, kolory z
//! okładki (liczone w oknie), pozycja zapamiętywana i przyciągana do rogów.

use crate::{layout, spotify, state};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

/// Rozmiary okna (px logiczne, z marginesem na cień).
const CARD: (f64, f64) = (424.0, 468.0);
const BAR: (f64, f64) = (404.0, 72.0);
const SNAP: i32 = 40;

fn size(compact: bool) -> (f64, f64) {
    if compact { BAR } else { CARD }
}

/// Ustawia okno według zapamiętanego stanu (rozmiar, pozycja, widoczność).
pub fn sync(app: &AppHandle) {
    let Some(w) = app.get_webview_window("player") else { return };
    let st = state::read(app, |rt| rt.store.player.clone());
    if !st.visible {
        let _ = w.hide();
        return;
    }
    let s = layout::screen(app);
    let (lw, lh) = size(st.compact);
    let (pw, ph) = ((lw * s.scale) as i32, (lh * s.scale) as i32);
    // Domyślnie: prawy dolny róg nad dockiem.
    let mut x = st.x.unwrap_or(s.work.right - pw - (16.0 * s.scale) as i32);
    let mut y = st.y.unwrap_or(s.work.bottom - ph - (120.0 * s.scale) as i32);
    // Pilnujemy, żeby nie wyjechał poza ekran (np. po odłączeniu monitora).
    let on_screen = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .any(|m| {
            let p = m.position();
            let z = m.size();
            x + pw / 2 >= p.x && x + pw / 2 < p.x + z.width as i32 && y + ph / 2 >= p.y && y + ph / 2 < p.y + z.height as i32
        });
    if !on_screen {
        x = s.work.right - pw - (16.0 * s.scale) as i32;
        y = s.work.bottom - ph - (120.0 * s.scale) as i32;
    }
    let _ = w.set_size(PhysicalSize::new(pw as u32, ph as u32));
    let _ = w.set_position(PhysicalPosition::new(x, y));
    let _ = w.set_always_on_top(st.pinned);
    let _ = w.show();
}

fn set(app: &AppHandle, f: impl FnOnce(&mut state::PlayerState)) {
    state::update(app, true, |rt| f(&mut rt.store.player));
    sync(app);
}

/// Pokaż / schowaj (pasek, dymek, paleta, Ctrl+Alt+M).
pub fn toggle(app: &AppHandle) {
    set(app, |p| p.visible = !p.visible);
    if state::read(app, |rt| rt.store.player.visible) {
        let _ = app.emit_to("player", "player-open", ());
    }
}

pub fn show(app: &AppHandle) {
    set(app, |p| p.visible = true);
    let _ = app.emit_to("player", "player-open", ());
}

#[tauri::command]
pub fn player_toggle(app: AppHandle) {
    toggle(&app);
}

#[tauri::command]
pub fn player_hide(app: AppHandle) {
    set(&app, |p| p.visible = false);
}

/// Karta ↔ pasek. Okno rośnie/kurczy się tak, żeby dolna krawędź została na miejscu.
#[tauri::command]
pub fn player_mode(app: AppHandle, compact: bool) {
    let Some(w) = app.get_webview_window("player") else { return };
    let s = layout::screen(&app);
    let (old_w, old_h) = size(state::read(&app, |rt| rt.store.player.compact));
    let (new_w, new_h) = size(compact);
    if let Ok(pos) = w.outer_position() {
        let mut x = pos.x + ((old_w - new_w) * s.scale / 2.0) as i32;
        let mut y = pos.y + ((old_h - new_h) * s.scale) as i32;
        // Przy górnej krawędzi rośnie w dół, nie poza ekran.
        if y < s.work.top {
            y = s.work.top;
        }
        x = x.max(s.monitor.left);
        set(&app, |p| {
            p.compact = compact;
            p.x = Some(x);
            p.y = Some(y);
        });
    } else {
        set(&app, |p| p.compact = compact);
    }
}

#[tauri::command]
pub fn player_pin(app: AppHandle, pinned: bool) {
    set(&app, |p| p.pinned = pinned);
}

/// Po przeciągnięciu: przyciągnij do krawędzi obszaru roboczego i zapamiętaj.
#[tauri::command]
pub fn player_drag_end(app: AppHandle) {
    let Some(w) = app.get_webview_window("player") else { return };
    let (Ok(pos), Ok(sz)) = (w.outer_position(), w.outer_size()) else { return };
    let (pw, ph) = (sz.width as i32, sz.height as i32);
    // Obszar roboczy monitora, na którym stoi środek okna.
    let center = (pos.x + pw / 2, pos.y + ph / 2);
    let s = layout::screen(&app);
    let mut area = s.work;
    for m in app.available_monitors().unwrap_or_default() {
        let p = m.position();
        let z = m.size();
        if center.0 >= p.x && center.0 < p.x + z.width as i32 && center.1 >= p.y && center.1 < p.y + z.height as i32 {
            if p.x != s.monitor.left || p.y != s.monitor.top {
                area = crate::platform::Rect { left: p.x, top: p.y + (layout::BAR_H * m.scale_factor()) as i32, right: p.x + z.width as i32, bottom: p.y + z.height as i32 };
            }
        }
    }
    let gap = (12.0 * s.scale) as i32;
    let (mut x, mut y) = (pos.x, pos.y);
    if (x - area.left).abs() < SNAP {
        x = area.left + gap;
    }
    if (area.right - (x + pw)).abs() < SNAP {
        x = area.right - pw - gap;
    }
    if (y - area.top).abs() < SNAP {
        y = area.top + gap;
    }
    if (area.bottom - (y + ph)).abs() < SNAP {
        y = area.bottom - ph - gap;
    }
    set(&app, |p| {
        p.x = Some(x);
        p.y = Some(y);
    });
}

/// Głośność Spotify z kółka myszy.
#[tauri::command]
pub async fn player_volume(app: AppHandle, volume: i64) -> Result<(), String> {
    let v = volume.clamp(0, 100);
    state::update(&app, false, |rt| rt.music.volume = Some(v));
    spotify::control(&app, &format!("volume:{v}"), None).await
}

pub fn init(app: &AppHandle) {
    sync(app);
}
