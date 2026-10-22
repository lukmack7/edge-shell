//! Silnik: co sekundę patrzy na plan dnia i decyduje, co ma się wydarzyć —
//! komunikaty pit wall, świecąca krawędź przed spotkaniem, reguły muzyki,
//! paczki powiadomień. Rzadziej odświeża schowek, aktywne okno, uruchomione
//! aplikacje, baterię i ostatnie pliki.

use crate::platform;
use crate::state::{self, current_item, DayItem, FileItem, Mark};
use crate::{layout, radio, spotify};
use chrono::{Duration as CDuration, Local};
use std::time::Duration;
use tauri::AppHandle;

pub fn start(app: AppHandle) {
    let app_tick = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut n: u64 = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            n += 1;
            plan_rules(&app_tick).await;
            radio::tick(&app_tick);
            report_tick(&app_tick);
            if n % 30 == 0 {
                // Raz na pół minuty: odśwież zegar w pasku i zakończ sesję ad hoc.
                finish_expired_adhoc(&app_tick);
                state::emit_snapshot(&app_tick);
            }
        }
    });

    // Rzeczy systemowe w osobnym wątku — wywołania Win32 są synchroniczne.
    std::thread::spawn(move || {
        let mut n: u64 = 0;
        loop {
            std::thread::sleep(Duration::from_millis(1000));
            n += 1;
            clipboard(&app);
            foreground(&app);
            if n % 3 == 1 {
                running(&app);
            }
            if n % 30 == 1 {
                let b = platform::battery();
                let changed = state::read(&app, |rt| rt.battery != b);
                if changed {
                    state::update(&app, false, |rt| rt.battery = b);
                }
                recent_files(&app);
            }
        }
    });
}

fn fire_once(app: &AppHandle, key: String) -> bool {
    let sh = state::shared(app);
    let mut rt = sh.rt.lock().unwrap_or_else(|e| e.into_inner());
    rt.fired.insert(key)
}

async fn plan_rules(app: &AppHandle) {
    let now = Local::now();
    let cfg = state::cfg(app);
    let (plan, mode_id, glow_on) = state::read(app, |rt| (rt.plan.clone(), rt.store.mode.clone(), rt.glow_until.is_some()));
    let mode = cfg.mode(&mode_id);
    let spotify_on = !cfg.spotify.client_id.is_empty() && state::read(app, |rt| rt.store.spotify.is_some());

    // Świecąca krawędź: spotkanie w ciągu N minut.
    let glow_window = CDuration::minutes(cfg.shell.edge_glow_minutes);
    let meeting_soon = plan
        .iter()
        .filter(|i| i.kind == "meeting" || i.kind == "busy")
        .find(|i| i.start > now && i.start - now <= glow_window);
    if meeting_soon.is_some() != glow_on {
        let until = meeting_soon.map(|m| m.start);
        state::update(app, false, |rt| rt.glow_until = until);
        layout::set_glow(app, until.is_some());
    }

    for item in &plan {
        let to_start = item.start - now;
        let started = item.start <= now && now - item.start < CDuration::seconds(30);

        if item.kind == "meeting" || item.kind == "busy" {
            if to_start <= CDuration::minutes(10) && to_start > CDuration::minutes(9) && fire_once(app, format!("m10:{}", item.key)) {
                radio::pit_wall(app, format!("{} za 10 minut. Domknij myśl i zapisz, gdzie jesteś.", item.title), Some("Przyjąłem".into()));
            }
            if spotify_on && cfg.spotify.pause_before_meeting && to_start <= CDuration::minutes(2) && to_start > CDuration::zero()
                && fire_once(app, format!("mpause:{}", item.key))
            {
                let _ = spotify::control(app, "pause", None).await;
            }
            continue;
        }

        if !started || !fire_once(app, format!("start:{}", item.key)) {
            continue;
        }
        if item.kind == "buffer" {
            let mins = (item.end - item.start).num_minutes();
            radio::pit_wall(app, format!("Box, box. {mins} minut bufora — wstań od biurka."), Some("Zjeżdżam".into()));
            if spotify_on && cfg.spotify.pause_on_buffer {
                let _ = spotify::control(app, "pause", None).await;
            }
        } else {
            radio::pit_wall(app, format!("Zielone światło: {}. Do {}.", item.title, item.end.format("%H:%M")), Some("Jadę".into()));
            if spotify_on && cfg.spotify.play_on_focus && (item.kind == "focus" || item.kind == "task") {
                spotify::play_playlist(app, &mode.playlist, Some(cfg.spotify.focus_volume)).await;
            }
        }
    }
}

/// Raport tygodnia o ustawionej porze (domyślnie piątek 16:00).
fn report_tick(app: &AppHandle) {
    use chrono::{Datelike, Timelike};
    let cfg = state::cfg(app);
    let now = Local::now();
    if cfg.focus.report_weekday == 0 || now.weekday().number_from_monday() != cfg.focus.report_weekday {
        return;
    }
    let Some(at) = state::parse_hhmm(&cfg.focus.report_time) else { return };
    if now.hour() * 60 + now.minute() != at {
        return;
    }
    if fire_once(app, format!("report:{}", state::today_key())) {
        layout::show_window(app, "report", true);
        radio::pit_wall(app, "Raport tygodnia gotowy. Zobacz, jak poszły sektory.".into(), None);
    }
}

fn finish_expired_adhoc(app: &AppHandle) {
    let expired = state::read(app, |rt| {
        rt.adhoc.as_ref().and_then(|a| a.end).is_some_and(|e| e <= Local::now())
    });
    if expired {
        state::update(app, false, |rt| rt.adhoc = None);
        radio::pit_wall(app, "Koniec sesji fokusu. Zapisz, gdzie skończyłeś.".into(), None);
    }
}

/// Zapis „Skończone”: porównuje z planem i nadaje kolor sektora.
pub fn mark_done(app: &AppHandle, item: &DayItem) -> Mark {
    let now = Local::now();
    let delta = (now - item.end).num_minutes();
    let status = if delta < -2 { "early" } else if delta <= 2 { "ontime" } else { "late" };
    let mark = Mark { status: status.into(), delta_min: delta, date: state::today_key() };
    let key = item.key.clone();
    let m = mark.clone();
    let entry = crate::state::HistEntry {
        date: state::today_key(),
        title: item.title.clone(),
        kind: item.kind.clone(),
        start: item.start,
        end: item.end,
        status: mark.status.clone(),
        delta_min: delta,
    };
    state::update(app, true, |rt| {
        rt.store.marks.insert(key, m);
        rt.store.history.retain(|h| !(h.date == entry.date && h.title == entry.title && h.start == entry.start));
        rt.store.history.push(entry);
        rt.paused_since = None;
    });
    mark
}

pub fn current(app: &AppHandle) -> Option<DayItem> {
    state::read(app, |rt| current_item(&rt.plan, Local::now()))
}

fn clipboard(app: &AppHandle) {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let Ok(text) = app.clipboard().read_text() else { return };
    let text = text.trim().to_string();
    if text.is_empty() || text.len() > 4000 {
        return;
    }
    let is_new = state::read(app, |rt| rt.clipboard.first() != Some(&text));
    if is_new {
        state::update(app, false, |rt| {
            rt.clipboard.retain(|t| t != &text);
            rt.clipboard.insert(0, text);
            rt.clipboard.truncate(12);
        });
    }
}

fn foreground(app: &AppHandle) {
    let Some(exe) = platform::foreground_exe() else { return };
    let file = platform::exe_name(&exe);
    let cfg = state::cfg(app);
    // Przejście do aplikacji zdejmuje jej plakietkę.
    if let Some(a) = cfg.dock.iter().find(|a| !a.process.is_empty() && a.process.to_lowercase() == file) {
        if state::read(app, |rt| rt.badges.contains_key(&a.id)) {
            let id = a.id.clone();
            state::update(app, false, |rt| rt.badges.remove(&id));
        }
    }
    let name = cfg
        .dock
        .iter()
        .find(|a| !a.process.is_empty() && a.process.to_lowercase() == file)
        .map(|a| a.name.clone())
        .unwrap_or_else(|| {
            let stem = file.trim_end_matches(".exe");
            let mut c = stem.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        });
    if state::read(app, |rt| rt.foreground != name) {
        state::update(app, false, |rt| rt.foreground = name);
    }
}

fn running(app: &AppHandle) {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for w in platform::app_windows() {
        *counts.entry(platform::exe_name(&w.exe)).or_insert(0) += 1;
    }
    let set: std::collections::HashSet<String> = counts.keys().cloned().collect();
    if state::read(app, |rt| rt.window_counts != counts) {
        state::update(app, false, |rt| {
            rt.running = set;
            rt.window_counts = counts;
        });
    }
}

fn recent_files(app: &AppHandle) {
    let Some(dir) = dirs::config_dir().map(|d| d.join("Microsoft").join("Windows").join("Recent")) else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let mut items: Vec<FileItem> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")))
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            let ms = modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
            let name = e.path().file_stem()?.to_string_lossy().to_string();
            // Skróty do folderów nie mają rozszerzenia pliku — pomijamy je.
            if !name.contains('.') {
                return None;
            }
            Some(FileItem { name, path: e.path().to_string_lossy().to_string(), modified: ms })
        })
        .collect();
    items.sort_by(|a, b| b.modified.cmp(&a.modified));
    items.truncate(6);
    let changed = state::read(app, |rt| {
        rt.recent.len() != items.len() || rt.recent.iter().zip(&items).any(|(a, b)| a.path != b.path)
    });
    if changed {
        state::update(app, false, |rt| rt.recent = items);
    }
}
