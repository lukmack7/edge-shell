#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod config;
mod engine;
mod horolog;
mod hotkeys;
mod layout;
mod notifications;
mod platform;
mod icons;
mod insights;
mod previews;
mod radio;
mod settings;
mod spotify;
mod state;
mod stats;
mod sys;
mod tray;
mod watchdog;
mod palette;
mod player;
mod notch;
mod popup;
mod focus;
mod presence;
mod shot;
mod windows_mgr;
mod sysx;
mod automations;
mod backup;
mod stacks;

use tauri::{Emitter, Manager, RunEvent, WindowEvent};

fn main() {
    // Ten sam plik .exe działa też jako bezpiecznik (osobny proces).
    if watchdog::run_if_requested() {
        return;
    }
    let cfg = config::Config::load();
    let shared = state::Shared::new(cfg);

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Drugie uruchomienie = „pokaż mi Teraz”.
            layout::set_panel(app, "now", true);
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(hotkeys::plugin())
        .manage(shared)
        .setup(|app| {
            let handle = app.handle().clone();
            // Zapamiętaj pierwotny stan paska zadań (do przywrócenia przy wyjściu).
            if !state::read(&handle, |rt| rt.store.taskbar_saved) {
                let original = sys::taskbar_state();
                state::update(&handle, true, |rt| {
                    rt.store.taskbar_state = original;
                    rt.store.taskbar_saved = true;
                });
            }
            let cfg = state::cfg(&handle);
            if cfg.shell.hide_taskbar {
                sys::hide_taskbar(true, 0);
            }
            settings::set_autostart(&handle, cfg.shell.autostart);
            // Bezpiecznik: gdyby Edge Shell padł, przywróci pasek zadań i zasobnik.
            let restore = state::read(&handle, |rt| rt.store.taskbar_state);
            watchdog::spawn(restore, cfg.shell.hide_taskbar);
            layout::arrange(&handle);
            // Po schowaniu paska zadań obszar roboczy zmienia się z opóźnieniem.
            let h2 = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                layout::arrange(&h2);
            });
            hotkeys::register(&handle);
            horolog::start(handle.clone());
            radio::start_server(handle.clone());
            spotify::start(handle.clone());
            engine::start(handle.clone());
            notifications::start(handle.clone());
            stats::start(handle.clone());
            if cfg.shell.tray_in_rail {
                let h = handle.clone();
                tray::start(Box::new(move || {
                    let _ = h.emit_to("rail", "tray", tray::items());
                }));
            }
            layout::start_edges(handle.clone());
            stacks::init(&handle);
            palette::init(&handle);
            player::init(&handle);
            popup::init(&handle);
            focus::start(handle.clone());
            presence::start(handle.clone());
            windows_mgr::start(handle.clone());
            automations::start(handle.clone());

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
            if let Some(w) = app.get_webview_window("settings") {
                let w2 = w.clone();
                w.on_window_event(move |e| {
                    if let WindowEvent::CloseRequested { api, .. } = e {
                        api.prevent_close();
                        let _ = w2.hide();
                    }
                });
            }
            if let Some(w) = app.get_webview_window("calendar") {
                let h = handle.clone();
                w.on_window_event(move |e| {
                    if let WindowEvent::Focused(false) = e {
                        layout::hide_window(&h, "calendar");
                    }
                });
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
            commands::radio_quick_reply,
            commands::radio_focus,
            commands::radio_later,
            commands::radio_open,
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
            settings::get_config,
            settings::save_config,
            settings::open_settings,
            settings::pick_path,
            settings::horolog_test,
            settings::get_stats,
            settings::set_volume,
            settings::toggle_mute,
            icons::dock_icons,
            icons::app_icon,
            commands::launch_new,
            commands::dock_pin,
            commands::dock_cycle,
            commands::dock_reorder,
            commands::dock_open_with,
            commands::dock_add_paths,
            previews::previews_show,
            previews::previews_layout,
            previews::previews_hide,
            previews::previews_tip,
            previews::window_activate,
            previews::window_close,
            insights::get_week,
            insights::toggle_calendar,
            insights::open_report,
            insights::get_report,
            insights::tray_items,
            insights::tray_click,
            stacks::stack_open,
            stacks::stack_close,
            backup::backup_list,
            backup::backup_restore,
            backup::backup_export,
            backup::backup_import,
            palette::palette_query,
            player::player_toggle,
            notch::notch_size,
            notch::notch_focus,
            notch::notch_missed,
            notch::notch_clear_missed,
            notch::notch_open_missed,
            notch::notch_drawer,
            popup::popup_open,
            focus::one_thing_toggle,
            shot::shot_start,
            shot::shot_cancel,
            shot::shot_save,
            shot::quicklook,
            windows_mgr::layout_save,
            windows_mgr::layout_restore,
            windows_mgr::layout_list,
            windows_mgr::layout_delete,
            windows_mgr::zone_snap,
            focus::set_first_step,
            focus::get_first_step,
            insights::open_evening,
            insights::get_evening,
            insights::evening_tomorrow,
            insights::evening_done,
            insights::evening_note,
            insights::horolog_move,
            popup::popup_resize,
            popup::popup_hide,
            popup::sys_processes,
            popup::sys_kill,
            popup::wifi_list,
            popup::wifi_connect,
            popup::wifi_disconnect,
            popup::bt_list,
            popup::bt_batteries,
            popup::audio_state,
            popup::audio_set_output,
            popup::audio_app,
            popup::power_action,
            popup::process_icon,
            player::player_hide,
            player::player_mode,
            player::player_pin,
            player::player_drag_end,
            player::player_volume,
            player::player_panel,
            player::player_queue,
            player::player_playlists,
            player::player_play,
            player::player_skip,
            player::player_like,
            player::player_shuffle,
            player::player_repeat,
            palette::palette_run,
            palette::palette_show,
            palette::palette_hide,
            palette::palette_keys,
            palette::palette_commands,
            automations::automation_answer,
            automations::automation_run,
            automations::automation_status,
        ])
        .build(tauri::generate_context!())
        .expect("nie udało się uruchomić Edge Shell");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            layout::release(handle);
        }
    });
}
