//! Globalne skróty klawiszowe. Domyślnie Ctrl+Alt+…, bo Win+strzałki,
//! Win+K i Win+Space są zajęte przez Windows.

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
    let pairs: [(&str, &'static str); 6] = [
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
    if let Ok(mut a) = ACTIONS.lock() {
        *a = actions;
    }
}
