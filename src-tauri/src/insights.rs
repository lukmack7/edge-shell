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

// ---------------------------------------------------------------------------
// Prawdziwe czasy: korekta długości nawyków w Horologu
// ---------------------------------------------------------------------------

/// Zapisuje faktyczną długość bloku. Gdy trzy ostatnie razy różnią się od planu
/// w tę samą stronę (≥20% i ≥10 min), karta radia proponuje poprawkę.
pub fn record_actual(app: &AppHandle, intent_id: &str, title: &str, planned: i64, actual: i64) {
    let samples = state::update(app, true, |rt| {
        let v = rt.store.actuals.entry(intent_id.to_string()).or_default();
        v.push(actual);
        if v.len() > 5 {
            v.remove(0);
        }
        v.clone()
    });
    if samples.len() < 3 || planned <= 0 {
        return;
    }
    let last: Vec<i64> = samples[samples.len() - 3..].to_vec();
    let far = |a: i64| (a - planned).abs() >= 10 && (a - planned).abs() * 5 >= planned;
    let longer = last.iter().all(|a| *a > planned && far(*a));
    let shorter = last.iter().all(|a| *a < planned && far(*a));
    if !(longer || shorter) {
        return;
    }
    let avg = last.iter().sum::<i64>() / 3;
    let rounded = ((avg + 7) / 15 * 15).max(15);
    let week = Local::now().format("%G-%V").to_string();
    let asked = state::read(app, |rt| rt.store.dur_asked.get(intent_id).cloned());
    if asked.as_deref() == Some(week.as_str()) {
        return;
    }
    state::update(app, true, |rt| {
        rt.store.dur_asked.insert(intent_id.to_string(), week);
    });
    let mut msg = crate::state::RadioMsg {
        id: format!("dur{:x}", Local::now().timestamp_millis()),
        name: "PIT WALL".into(),
        number: "PW".into(),
        color: "#93c5fd".into(),
        source: "horolog".into(),
        message: format!("„{title}”: plan {planned} min, realnie ostatnio ~{avg} min. Ustawić {rounded} min w Horologu?"),
        reply: None,
        reply_url: None,
        urgent: false,
        at: Local::now(),
        app: None,
        open_target: None,
        ask: Some(format!("dur:{intent_id}:{rounded}")),
        yes_label: Some(format!("Tak, {rounded} min")),
        no_label: Some("Zostaw".into()),
    };
    msg.urgent = false;
    crate::radio::deliver(app, msg);
}

/// Odpowiedzi na karty z tego modułu (true = obsłużone).
pub fn answer(app: &AppHandle, ask: &str, yes: bool) -> bool {
    let Some(rest) = ask.strip_prefix("dur:") else { return false };
    if yes {
        let mut parts = rest.rsplitn(2, ':');
        let mins = parts.next().and_then(|m| m.parse::<i64>().ok()).unwrap_or(0);
        let id = parts.next().unwrap_or("").to_string();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            match crate::horolog::scale_minutes(&app, &id, mins).await {
                Ok(()) => {
                    state::update(&app, true, |rt| {
                        rt.store.actuals.remove(&id);
                    });
                    crate::radio::pit_wall(&app, format!("Gotowe — Horolog planuje teraz {mins} min."), None);
                }
                Err(e) => crate::radio::pit_wall(&app, format!("Horolog: {e}"), None),
            }
        });
    }
    true
}

// ---------------------------------------------------------------------------
// Wieczorne zamknięcie dnia
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn open_evening(app: AppHandle) {
    layout::show_window(&app, "evening", true);
    let _ = tauri::Emitter::emit_to(&app, "evening", "evening-open", ());
}

#[tauri::command]
pub fn get_evening(app: AppHandle) -> Value {
    let today = state::today_key();
    let now = Local::now();
    state::read(&app, |rt| {
        let done: Vec<Value> = rt
            .store
            .history
            .iter()
            .filter(|h| h.date == today && h.status != "skipped")
            .map(|h| json!({ "title": h.title, "status": h.status, "deltaMin": h.delta_min }))
            .collect();
        let open: Vec<Value> = rt
            .plan
            .iter()
            .filter(|i| matches!(i.kind.as_str(), "task" | "focus" | "habit") && i.intent_id.is_some())
            .filter(|i| !rt.store.marks.get(&i.key).is_some_and(|m| m.date == today))
            .map(|i| json!({
                "key": i.key, "intentId": i.intent_id, "title": i.title, "kind": i.kind,
                "start": i.start, "end": i.end, "past": i.end <= now,
            }))
            .collect();
        json!({ "done": done, "open": open, "note": rt.store.day_notes.get(&today).cloned().unwrap_or_default() })
    })
}

/// Zadanie z dzisiaj → jutro o tej samej porze.
#[tauri::command]
pub async fn evening_tomorrow(app: AppHandle, intent_id: String, start: chrono::DateTime<Local>) -> Result<(), String> {
    crate::horolog::move_block(&app, &intent_id, start + Duration::days(1)).await
}

#[tauri::command]
pub async fn evening_done(app: AppHandle, key: String) -> Result<(), String> {
    let item = state::read(&app, |rt| rt.plan.iter().find(|i| i.key == key).cloned()).ok_or("Nie ma takiego bloku")?;
    crate::engine::mark_done(&app, &item);
    if let (Some(id), "task") = (&item.intent_id, item.kind.as_str()) {
        crate::horolog::complete(&app, id).await?;
    }
    Ok(())
}

#[tauri::command]
pub fn evening_note(app: AppHandle, text: String) {
    state::update(&app, true, |rt| {
        if text.trim().is_empty() {
            rt.store.day_notes.remove(&state::today_key());
        } else {
            rt.store.day_notes.insert(state::today_key(), text.trim().to_string());
        }
    });
}

/// Przesunięcie bloku z kalendarza (przeciągnięcie).
#[tauri::command]
pub async fn horolog_move(app: AppHandle, intent_id: String, start: chrono::DateTime<Local>) -> Result<(), String> {
    crate::horolog::move_block(&app, &intent_id, start).await
}

#[tauri::command]
pub fn tray_items() -> Vec<tray::TrayItem> {
    tray::items()
}

#[tauri::command]
pub fn tray_click(key: String, action: String) {
    tray::click(&key, &action);
}
