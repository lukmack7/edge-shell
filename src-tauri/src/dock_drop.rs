//! Odbiór dropów z okna docka i diagnostyka bez zapisywania nazw plików.

use serde_json::json;
use std::{
    fs::OpenOptions,
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
};
use tauri::{DragDropEvent, Emitter, WebviewWindow, WindowEvent};

fn log_line(stage: &str) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(crate::config::data_dir().join("dock-drop.log"))
    {
        let _ = writeln!(
            f,
            "{} {stage}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f")
        );
    }
}

pub fn attach(dock: &WebviewWindow) {
    let _ = std::fs::write(crate::config::data_dir().join("dock-drop.log"), "");
    log_line("native:ready");
    let target = dock.clone();
    let over_logged = AtomicBool::new(false);
    dock.on_window_event(move |event| {
        let WindowEvent::DragDrop(event) = event else { return };
        let payload = match event {
            DragDropEvent::Enter { paths, position } => {
                over_logged.store(false, Ordering::Relaxed);
                log_line(&format!("native:enter files={}", paths.len()));
                json!({ "type": "enter", "position": position })
            }
            DragDropEvent::Over { position } => {
                if !over_logged.swap(true, Ordering::Relaxed) {
                    log_line("native:over");
                }
                json!({ "type": "over", "position": position })
            }
            DragDropEvent::Drop { paths, position } => {
                log_line(&format!("native:drop files={}", paths.len()));
                json!({ "type": "drop", "position": position, "paths": paths })
            }
            DragDropEvent::Leave => {
                log_line("native:leave");
                json!({ "type": "leave" })
            }
            _ => return,
        };
        if target.emit_to("dock", "dock-native-drop", payload).is_err() {
            log_line("native:emit-error");
        }
    });
}

#[tauri::command]
pub fn dock_drop_trace(webview: tauri::Webview, stage: &str) {
    if webview.label() == "dock" && matches!(stage,
        "ui:ready" | "ui:listen-error" | "ui:drop" | "ui:outside" | "ui:no-paths" |
        "action:pin" | "action:open" | "action:folder" | "action:trash" |
        "action:ok" | "action:error"
    ) {
        log_line(stage);
    }
}
