//! Kalendarz pod zegarem, raport tygodnia i komendy lewego paska.

use crate::state::{self, DayItem};
use crate::{layout, tray};
use chrono::{Datelike, Duration, Local, Timelike};
use serde_json::{json, Value};
use std::collections::HashMap;
use tauri::{AppHandle, Manager};

#[tauri::command]
pub fn get_week(app: AppHandle) -> Vec<DayItem> {
    state::read(&app, |rt| rt.week.clone())
}

#[tauri::command]
pub fn toggle_calendar(app: AppHandle) {
    if let Some(w) = app.get_webview_window("calendar") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            layout::arrange(&app);
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

#[tauri::command]
pub fn open_report(app: AppHandle) {
    layout::show_window(&app, "report", true);
}

const DAYS: [&str; 7] = ["pon", "wt", "śr", "czw", "pt", "sob", "nd"];

/// Raport ostatnich 7 dni z historii sektorów.
#[tauri::command]
pub fn get_report(app: AppHandle) -> Value {
    let cfg = state::cfg(&app);
    let (history, planned) = state::read(&app, |rt| (rt.store.history.clone(), rt.store.week_focus_min));
    let today = Local::now().date_naive();
    let from = today - Duration::days(6);

    let mut days: Vec<Value> = Vec::new();
    let (mut early, mut ontime, mut late, mut skipped) = (0, 0, 0, 0);
    let mut focus_done_min = 0i64;
    let mut hours: HashMap<u32, i64> = HashMap::new();
    let mut best: Option<(i64, String)> = None;
    let mut worst: Option<(i64, String)> = None;

    for d in 0..7 {
        let date = from + Duration::days(d);
        let key = date.format("%Y-%m-%d").to_string();
        let mut entries: Vec<_> = history.iter().filter(|h| h.date == key).cloned().collect();
        entries.sort_by_key(|h| h.start);
        for h in &entries {
            match h.status.as_str() {
                "early" => early += 1,
                "ontime" => ontime += 1,
                "late" => late += 1,
                _ => skipped += 1,
            }
            if h.status != "skipped" {
                focus_done_min += (h.end - h.start).num_minutes().max(0);
                if h.status != "late" {
                    *hours.entry(h.start.hour()).or_insert(0) += 1;
                }
                if best.as_ref().map_or(true, |(b, _)| h.delta_min < *b) {
                    best = Some((h.delta_min, h.title.clone()));
                }
                if worst.as_ref().map_or(true, |(w, _)| h.delta_min > *w) {
                    worst = Some((h.delta_min, h.title.clone()));
                }
            }
        }
        days.push(json!({
            "date": key,
            "label": DAYS[date.weekday().num_days_from_monday() as usize],
            "day": date.day(),
            "today": date == today,
            "sectors": entries.iter().map(|h| json!({
                "title": h.title, "kind": h.kind, "status": h.status, "deltaMin": h.delta_min,
                "minutes": (h.end - h.start).num_minutes(),
            })).collect::<Vec<_>>(),
        }));
    }

    let mut best_hours: Vec<(u32, i64)> = hours.into_iter().collect();
    best_hours.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let best_hours: Vec<String> = best_hours.iter().take(2).map(|(h, _)| format!("{h:02}:00–{:02}:00", h + 1)).collect();

    json!({
        "from": from.format("%d.%m").to_string(),
        "to": today.format("%d.%m").to_string(),
        "days": days,
        "counts": { "early": early, "ontime": ontime, "late": late, "skipped": skipped },
        "focusDoneMin": focus_done_min,
        "focusPlannedMin": planned,
        "goalHours": cfg.focus.weekly_goal_hours,
        "bestSector": best.filter(|(d, _)| *d < 0).map(|(d, t)| json!({ "title": t, "deltaMin": d })),
        "worstSector": worst.filter(|(d, _)| *d > 2).map(|(d, t)| json!({ "title": t, "deltaMin": d })),
        "bestHours": best_hours,
    })
}

#[tauri::command]
pub fn tray_items() -> Vec<tray::TrayItem> {
    tray::items()
}

#[tauri::command]
pub fn tray_click(key: String, action: String) {
    tray::click(&key, &action);
}
