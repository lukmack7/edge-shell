//! Notch: czarne okno na środku górnego paska. Okno ma zawsze rozmiar tego,
//! co widać (plus ramiona), bo przezroczysta część okna i tak łapałaby kliknięcia.
//! Stronie (notch.html) zostaje rysowanie stanów; tu tylko rozmiar, pozycja,
//! fokus do pisania odpowiedzi i kilka akcji.

use crate::{layout, radio, state};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

/// Strona prosi o rozmiar (px CSS, razem z ramionami). Okno stoi przy górnej
/// krawędzi głównego monitora, wyśrodkowane.
#[tauri::command]
pub fn notch_size(app: AppHandle, width: f64, height: f64) {
    let Some(w) = app.get_webview_window("notch") else { return };
    let s = layout::screen(&app);
    let (pw, ph) = ((width.max(80.0) * s.scale).round() as i32, (height.max(30.0) * s.scale).round() as i32);
    let x = (s.monitor.left + s.monitor.right - pw) / 2;
    let _ = w.set_size(PhysicalSize::new(pw as u32, ph as u32));
    let _ = w.set_position(PhysicalPosition::new(x, s.monitor.top));
}

/// Fokus klawiatury (pisanie odpowiedzi w notchu).
#[tauri::command]
pub fn notch_focus(app: AppHandle) {
    if let Some(w) = app.get_webview_window("notch") {
        let _ = w.set_focus();
    }
}

/// Wiadomość schowana bez reakcji → „Co przegapiłeś”.
#[tauri::command]
pub fn notch_missed(app: AppHandle) {
    radio::to_missed(&app);
}

#[tauri::command]
pub fn notch_clear_missed(app: AppHandle, index: Option<usize>) {
    state::update(&app, true, |rt| match index {
        Some(i) if i < rt.store.missed.len() => {
            rt.store.missed.remove(i);
        }
        _ => rt.store.missed.clear(),
    });
}

/// Otwiera aplikację, z której przyszła przegapiona wiadomość.
#[tauri::command]
pub fn notch_open_missed(app: AppHandle, index: usize) -> Result<(), String> {
    let target = state::read(&app, |rt| rt.store.missed.get(index).and_then(|m| m.open_target.clone().map(|t| (t, m.app.clone()))));
    notch_clear_missed(app.clone(), Some(index));
    match target {
        Some((t, name)) => crate::notifications::open_source(&app, &t, name.as_deref()),
        None => Ok(()),
    }
}

/// Półka i schowek z panelu notcha: szuflada w odpowiednim widoku.
#[tauri::command]
pub fn notch_drawer(app: AppHandle, view: String) {
    layout::set_panel(&app, "drawer", true);
    let _ = app.emit_to("drawer", "drawer-view", view);
}

/// Nowa wiadomość: notch ma się obudzić (strona sama wybiera stan).
pub fn wake(app: &AppHandle) {
    let _ = app.emit_to("notch", "notch-wake", ());
}

/// Pokazuje / chowa okno notcha zgodnie z ustawieniem (wywoływane z arrange).
pub fn sync(app: &AppHandle) {
    let Some(w) = app.get_webview_window("notch") else { return };
    if state::cfg(app).shell.notch {
        let _ = w.show();
        let _ = app.emit_to("notch", "notch-layout", ());
    } else {
        let _ = w.hide();
    }
}
