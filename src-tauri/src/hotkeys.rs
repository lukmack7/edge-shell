//! Globalne skróty klawiszowe. Domyślnie Ctrl+Alt+…, bo Win+strzałki,
//! Win+K i Win+Space są zajęte przez Windows. Ctrl+Alt+A/C/E/L/N/O/S/U/X/Z
//! odpadają: na polskiej klawiaturze to AltGr, czyli ą ć ę ł ń ó ś € ź ż.

use crate::{layout, state};
use std::sync::Mutex;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

static ACTIONS: Mutex<Vec<(u32, &'static str)>> = Mutex::new(Vec::new());

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let action = ACTIONS
                .lock()
                .ok()
                .and_then(|a| a.iter().find(|(id, _)| *id == shortcut.id()).map(|(_, act)| *act));
            if let Some(action) = action {
                run(app, action);
            }
        })
        .build()
}

fn run(app: &AppHandle, action: &str) {
    match action {
        "drawer" => layout::toggle_panel(app, "drawer"),
        "now" => layout::toggle_panel(app, "now"),
        "capture" => layout::show_window(app, "capture", true),
        "search" => crate::palette::show(app),
        "radio" => crate::radio::release_batch(app),
        "player" => crate::player::toggle(app),
        "one_thing" => crate::focus::one_thing_toggle(app.clone()),
        "screenshot" => crate::shot::start(app),
        z if z.starts_with("zone") => crate::windows_mgr::snap_foreground(app, z[4..].parse().unwrap_or(1)),
        "focus" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let has_focus = state::read(&app, |rt| rt.adhoc.is_some())
                    || crate::engine::current(&app).is_some_and(|c| c.kind != "meeting" && c.kind != "busy");
                if has_focus {
                    let _ = crate::commands::focus_toggle_pause(app.clone()).await;
                } else {
                    crate::commands::focus_start_adhoc(app.clone(), None, None);
                }
            });
        }
        _ => {}
    }
}

pub fn register(app: &AppHandle) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let keys = state::cfg(app).hotkeys;
    let pairs: [(&str, &'static str); 9] = [
        (&keys.one_thing, "one_thing"),
        (&keys.screenshot, "screenshot"),
        (&keys.player, "player"),
        (&keys.drawer, "drawer"),
        (&keys.now, "now"),
        (&keys.capture, "capture"),
        (&keys.focus, "focus"),
        (&keys.search, "search"),
        (&keys.radio, "radio"),
    ];
    let mut actions = Vec::new();
    for (combo, action) in pairs {
        if combo.trim().is_empty() {
            continue;
        }
        if crate::config::altgr_conflict(combo) {
            log::warn!("pomijam skrót {combo}: na polskiej klawiaturze to AltGr (polski znak)");
            continue;
        }
        match combo.parse::<Shortcut>() {
            Ok(sc) => {
                let id = sc.id();
                if let Err(err) = gs.register(sc) {
                    log::warn!("skrót {combo} zajęty: {err}");
                } else {
                    actions.push((id, action));
                }
            }
            Err(err) => log::warn!("nie rozumiem skrótu {combo}: {err}"),
        }
    }
    // Strefy okien: Ctrl+Alt+1…6 (układ strefy z ustawień).
    for (n, label) in [(1, "zone1"), (2, "zone2"), (3, "zone3"), (4, "zone4"), (5, "zone5"), (6, "zone6")] {
        if !state::cfg(app).windows.zone_hotkeys {
            break;
        }
        if let Ok(sc) = format!("Ctrl+Alt+Digit{n}").parse::<Shortcut>() {
            let id = sc.id();
            if gs.register(sc).is_ok() {
                actions.push((id, label));
            }
        }
    }
    if let Ok(mut a) = ACTIONS.lock() {
        *a = actions;
    }
}
