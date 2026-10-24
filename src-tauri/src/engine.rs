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
            evening_tick(&app_tick);
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
        } else if matches!(item.kind.as_str(), "task" | "focus" | "habit") {
            // Start bloku z pierwszym 2‑minutowym krokiem („Jadę” / „Za 5 min”).
            crate::focus::ramp_card(app, &item.key, &item.title, item.end);
            if spotify_on && cfg.spotify.play_on_focus && (item.kind == "focus" || item.kind == "task") {
                spotify::play_playlist(app, &mode.playlist, Some(cfg.spotify.focus_volume)).await;
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

/// Wieczorne zamknięcie dnia (pon–pt o ustawionej porze).
fn evening_tick(app: &AppHandle) {
    use chrono::{Datelike, Timelike};
    let cfg = state::cfg(app);
    let now = Local::now();
    if !cfg.focus.evening_review || now.weekday().number_from_monday() > 5 {
        return;
    }
    let Some(at) = state::parse_hhmm(&cfg.focus.evening_time) else { return };
    if now.hour() * 60 + now.minute() == at && fire_once(app, format!("evening:{}", state::today_key())) {
        crate::insights::open_evening(app.clone());
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
    // Faktyczna długość — tylko gdy odhaczasz blisko końca bloku (nie wieczorem).
    let planned = (item.end - item.start).num_minutes();
    if matches!(item.kind.as_str(), "habit" | "focus") && (now - item.end).num_minutes() < planned.max(30) {
        if let Some(id) = &item.intent_id {
            let actual = (now - item.start).num_minutes().max(1);
            crate::insights::record_actual(app, id, &item.title, (item.end - item.start).num_minutes(), actual);
        }
    }
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
    if text.is_empty() || text.len() > 4000 || platform::clipboard_private() {
        return;
    }
    let is_new = state::read(app, |rt| rt.clipboard.first() != Some(&text));
    if is_new {
        state::update(app, true, |rt| {
            rt.clipboard.retain(|t| t != &text);
            rt.clipboard.insert(0, text.clone());
            rt.clipboard.truncate(12);
            // Trwała historia do palety (cb): przypięte zostają, reszta — ostatnie 150.
            let pinned = rt.store.clips.iter().any(|c| c.text == text && c.pinned);
            rt.store.clips.retain(|c| c.text != text);
            rt.store.clips.insert(0, crate::state::Clip { text, at: Local::now(), pinned });
            let mut unpinned = 0;
            rt.store.clips.retain(|c| {
                if c.pinned {
                    return true;
                }
                unpinned += 1;
                unpinned <= 150
            });
        });
    }
}

fn foreground(app: &AppHandle) {
    let Some(exe) = platform::foreground_exe() else { return };
    let file = platform::exe_name(&exe);
    let cfg = state::cfg(app);
    // Przejście do aplikacji zdejmuje jej plakietkę.
    if let Some(a) = cfg.dock.iter().find(|a| a.exe_names().contains(&file)) {
        if state::read(app, |rt| rt.badges.contains_key(&a.id)) {
            let id = a.id.clone();
            state::update(app, false, |rt| rt.badges.remove(&id));
        }
    }
    let name = cfg
        .dock
        .iter()
        .find(|a| a.exe_names().contains(&file))
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

static DESCRIPTIONS: std::sync::Mutex<Option<std::collections::HashMap<String, String>>> = std::sync::Mutex::new(None);

/// Ładna nazwa aplikacji: opis z pliku .exe (zapamiętany), inaczej nazwa pliku.
fn app_display_name(path: &str, exe: &str, title: &str) -> String {
    if exe == "applicationframehost.exe" {
        return title.rsplit(" - ").next().unwrap_or(title).trim().to_string();
    }
    if let Ok(g) = DESCRIPTIONS.lock() {
        if let Some(n) = g.as_ref().and_then(|m| m.get(path)) {
            return n.clone();
        }
    }
    let stem = exe.trim_end_matches(".exe");
    let fallback = {
        let mut c = stem.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    };
    let name = crate::sys::file_description(path)
        .filter(|d| d.chars().count() <= 32 && !d.to_lowercase().contains("host"))
        .unwrap_or(fallback);
    if let Ok(mut g) = DESCRIPTIONS.lock() {
        g.get_or_insert_with(std::collections::HashMap::new).insert(path.to_string(), name.clone());
    }
    name
}

fn running(app: &AppHandle) {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let windows = platform::app_windows();
    for w in &windows {
        *counts.entry(platform::exe_name(&w.exe)).or_insert(0) += 1;
    }
    let set: std::collections::HashSet<String> = counts.keys().cloned().collect();

    // Aplikacje spoza docka: kolejność otwarcia zostaje stała.
    let cfg = state::cfg(app);
    let pinned: std::collections::HashSet<String> = cfg
        .dock
        .iter()
        .flat_map(|a| a.exe_names())
        .collect();
    let mut open: Vec<state::OpenApp> = state::read(app, |rt| rt.open_apps.clone());
    open.retain(|o| counts.contains_key(&o.exe) && !pinned.contains(&o.exe));
    for o in open.iter_mut() {
        o.windows = counts[&o.exe];
    }
    for w in &windows {
        let exe = platform::exe_name(&w.exe);
        if exe.is_empty() || pinned.contains(&exe) || open.iter().any(|o| o.exe == exe) {
            continue;
        }
        open.push(state::OpenApp {
            name: app_display_name(&w.exe, &exe, &w.title),
            path: w.exe.clone(),
            windows: counts[&exe],
            exe,
        });
    }

    if state::read(app, |rt| rt.window_counts != counts || rt.open_apps != open) {
        state::update(app, false, |rt| {
            rt.running = set;
            rt.window_counts = counts;
            rt.open_apps = open;
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
