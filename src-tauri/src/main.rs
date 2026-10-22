#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod config;
mod engine;
mod horolog;
mod hotkeys;
mod layout;
mod platform;
mod radio;
mod spotify;
mod state;

use tauri::{Manager, RunEvent, WindowEvent};

fn main() {
    let cfg = config::Config::load();
    let shared = state::Shared::new(cfg);

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Drugie uruchomienie = „pokaż mi Teraz”.
            layout::set_panel(app, "now", true);
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(hotkeys::plugin())
        .manage(shared)
        .setup(|app| {
            let handle = app.handle().clone();
            if state::cfg(&handle).shell.hide_taskbar {
                platform::set_taskbar_visible(false);
            }
            layout::arrange(&handle);
            hotkeys::register(&handle);
            horolog::start(handle.clone());
            radio::start_server(handle.clone());
            spotify::start(handle.clone());
            engine::start(handle.clone());
            layout::start_edges(handle.clone());

            // Szuflada i Teraz chowają się, gdy klikniesz gdzie indziej.
            for label in ["drawer", "now"] {
                if let Some(w) = app.get_webview_window(label) {
                    let h = handle.clone();
                    w.on_window_event(move |e| {
                        if let WindowEvent::Focused(false) = e {
                            layout::blur_close(&h, label);
                        }
                    });
                }
            }
            if let Some(w) = app.get_webview_window("capture") {
                let h = handle.clone();
                w.on_window_event(move |e| {
                    if let WindowEvent::Focused(false) = e {
                        layout::hide_window(&h, "capture");
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::set_mode,
            commands::toggle_panel,
            commands::set_panel,
            commands::hide_window,
            commands::show_capture,
            commands::dock_resize,
            commands::launch,
            commands::launch_force,
            commands::friction_close,
            commands::friction_to_buffer,
            commands::open_path,
            commands::open_horolog,
            commands::capture,
            commands::replan,
            commands::focus_toggle_pause,
            commands::focus_start_adhoc,
            commands::focus_finish,
            commands::radio_reply,
            commands::radio_next,
            commands::radio_later,
            commands::radio_show_waiting,
            commands::shelf_add,
            commands::shelf_remove,
            commands::reveal,
            commands::copy_text,
            commands::stash_create,
            commands::stash_restore,
            commands::stash_delete,
            commands::search_index,
            commands::spotify_login,
            commands::spotify_control,
            commands::open_config,
            commands::reload_config,
            commands::quit,
        ])
        .build(tauri::generate_context!())
        .expect("nie udało się uruchomić Edge Shell");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            layout::release(handle);
        }
    });
}
