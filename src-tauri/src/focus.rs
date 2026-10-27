//! Pomoc przy skupieniu:
//! - tryb „jedna rzecz” — wszystko poza aktywnym oknem przygasa (półprzezroczysta
//!   warstwa tuż pod aktywnym oknem), dock się chowa, pasek pokazuje tylko zadanie,
//! - wykrywanie rozproszenia — kilka minut w YouTube/Discordzie w trakcie fokusu
//!   daje delikatną kartę „wracamy?”,
//! - rozpędzenie — na starcie bloku karta z pierwszym, 2‑minutowym krokiem.

use crate::state::{self, RadioMsg};
use crate::{engine, layout, platform, radio};
use chrono::{Duration as CDuration, Local};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

pub static ONE_THING: AtomicBool = AtomicBool::new(false);

/// Czy trwa teraz fokus (sesja ad hoc albo blok zadania/fokusu z Horologa).
pub fn focusing(app: &AppHandle) -> Option<String> {
    let (adhoc, paused) = state::read(app, |rt| (rt.adhoc.clone(), rt.paused_since.is_some()));
    if paused {
        return None;
    }
    if let Some(a) = adhoc {
        return Some(a.title);
    }
    engine::current(app).filter(|c| matches!(c.kind.as_str(), "task" | "focus" | "habit")).map(|c| c.title)
}

// ---------------------------------------------------------------------------
// Jedna rzecz
// ---------------------------------------------------------------------------

pub fn set_one_thing(app: &AppHandle, on: bool) {
    ONE_THING.store(on, Ordering::Relaxed);
    state::update(app, false, |rt| rt.one_thing = on);
    let Some(w) = app.get_webview_window("dimmer") else { return };
    if on {
        let s = layout::screen(app);
        let _ = w.set_position(tauri::PhysicalPosition::new(s.monitor.left, s.monitor.top));
        let _ = w.set_size(tauri::PhysicalSize::new((s.monitor.right - s.monitor.left) as u32, (s.monitor.bottom - s.monitor.top) as u32));
        let _ = w.set_ignore_cursor_events(true);
        let _ = w.show();
        place_dimmer(app);
        layout::set_dock_visible(app, false);
    } else {
        let _ = w.hide();
        layout::arrange(app);
    }
}

/// Warstwa idzie tuż pod aktywne okno — ono zostaje jasne, reszta przygasa.
fn place_dimmer(app: &AppHandle) {
    let Some(w) = app.get_webview_window("dimmer") else { return };
    #[cfg(windows)]
    if let Ok(h) = w.hwnd() {
        let fg = platform::foreground_hwnd();
        let ours = platform::is_own_window(fg);
        if fg != 0 && !ours {
            platform::place_below(h.0 as isize, fg);
        }
    }
    #[cfg(not(windows))]
    let _ = w;
}

#[tauri::command]
pub fn one_thing_toggle(app: AppHandle) {
    let on = !ONE_THING.load(Ordering::Relaxed);
    set_one_thing(&app, on);
}

// ---------------------------------------------------------------------------
// Rozproszenie i warstwa — wątek co 300 ms / 2 s
// ---------------------------------------------------------------------------

fn is_distraction(cfg: &crate::config::Config, mode_dim: &[String], exe: &str, title: &str) -> Option<String> {
    let exe_l = exe.to_lowercase();
    let title_l = title.to_lowercase();
    for d in &cfg.focus.distractions {
        let k = d.trim().to_lowercase();
        if k.len() >= 3 && (exe_l.contains(&k) || title_l.contains(&k)) {
            return Some(d.trim().to_string());
        }
    }
    // Aplikacje wyciszone w bieżącym trybie też rozpraszają.
    for id in mode_dim {
        if let Some(a) = cfg.dock.iter().find(|a| &a.id == id) {
            if a.exe_names().iter().any(|e| e == &exe_l) {
                return Some(a.name.clone());
            }
        }
    }
    None
}

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_fg: isize = 0;
        let mut away_since: Option<(Instant, String)> = None;
        let mut snooze_until: Option<Instant> = None;
        let mut last_work_hwnd: isize = 0;
        let mut tick = 0u64;
        loop {
            std::thread::sleep(Duration::from_millis(300));
            tick += 1;
            let fg = platform::foreground_hwnd();

            if ONE_THING.load(Ordering::Relaxed) && fg != last_fg {
                place_dimmer(&app);
            }
            last_fg = fg;

            if tick % 7 != 0 {
                continue;
            }
            // Koniec fokusu wyłącza „jedną rzecz”.
            let focus = focusing(&app);
            if ONE_THING.load(Ordering::Relaxed) && focus.is_none() && state::read(&app, |rt| rt.adhoc.is_none()) && engine::current(&app).is_none() {
                set_one_thing(&app, false);
            }
            let cfg = state::cfg(&app);
            if !cfg.focus.distraction_alert {
                continue;
            }
            let Some(task) = focus else {
                away_since = None;
                continue;
            };
            if platform::is_own_window(fg) {
                continue;
            }
            let (exe, title) = platform::window_info(fg);
            let dim = cfg.mode(&state::read(&app, |rt| rt.store.mode.clone())).dim;
            match is_distraction(&cfg, &dim, &exe, &title) {
                Some(what) => {
                    let since = away_since.get_or_insert_with(|| (Instant::now(), what.clone()));
                    let limit = Duration::from_secs(cfg.focus.distraction_minutes.max(1) * 60);
                    let snoozed = snooze_until.is_some_and(|t| Instant::now() < t);
                    if since.0.elapsed() >= limit && !snoozed {
                        snooze_until = Some(Instant::now() + Duration::from_secs(10 * 60));
                        state::update(&app, false, |rt| rt.return_hwnd = last_work_hwnd);
                        let mins = since.0.elapsed().as_secs() / 60;
                        radio::deliver(
                            &app,
                            card(
                                format!("{} od {mins} min. Wracamy do: {task}?", since.1),
                                "focus:return",
                                "Wracam",
                                "Jeszcze 10 min",
                            ),
                        );
                    }
                }
                None => {
                    away_since = None;
                    if fg != 0 {
                        last_work_hwnd = fg;
                    }
                }
            }
        }
    });
}

fn card(message: String, ask: &str, yes: &str, no: &str) -> RadioMsg {
    RadioMsg {
        id: format!("focus{:x}{:04x}", Local::now().timestamp_millis(), rand::random::<u16>()),
        name: "PIT WALL".into(),
        number: "PW".into(),
        color: "#fca5a5".into(),
        source: "fokus".into(),
        message,
        reply: None,
        reply_url: None,
        urgent: true,
        at: Local::now(),
        app: None,
        open_target: None,
        mail: None,
        ask: Some(ask.to_string()),
        yes_label: Some(yes.to_string()),
        no_label: Some(no.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Rozpędzenie: pierwszy krok na starcie bloku
// ---------------------------------------------------------------------------

const DEFAULT_STEP: &str = "otwórz to, czego potrzebujesz, i zrób najmniejszy ruch — 2 minuty";

pub fn first_step(app: &AppHandle, title: &str) -> String {
    state::read(app, |rt| rt.store.first_steps.get(&title.to_lowercase()).cloned()).unwrap_or_else(|| DEFAULT_STEP.to_string())
}

/// Karta startu bloku (zamiast samego „Zielone światło”).
pub fn ramp_card(app: &AppHandle, key: &str, title: &str, end: chrono::DateTime<Local>) {
    let step = first_step(app, title);
    radio::deliver(
        app,
        card(format!("Zielone światło: {title} (do {}). Pierwszy krok: {step}.", end.format("%H:%M")), &format!("ramp:{key}"), "Jadę", "Za 5 min"),
    );
}

/// Odpowiedź na karty fokusu (zwraca true, jeśli to była karta fokusu).
pub fn answer(app: &AppHandle, ask: &str, yes: bool) -> bool {
    if ask == "focus:return" {
        if yes {
            let h = state::read(app, |rt| rt.return_hwnd);
            if h != 0 {
                crate::sys::activate(h);
            }
        }
        return true;
    }
    if let Some(key) = ask.strip_prefix("ramp:") {
        if yes {
            if state::cfg(app).focus.one_thing_on_start {
                set_one_thing(app, true);
            }
        } else {
            // Za 5 minut ta sama karta jeszcze raz.
            let app2 = app.clone();
            let key = key.to_string();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(5 * 60)).await;
                let item = state::read(&app2, |rt| rt.plan.iter().find(|i| i.key == key).cloned());
                if let Some(i) = item.filter(|i| i.end > Local::now() + CDuration::minutes(5)) {
                    ramp_card(&app2, &i.key, &i.title, i.end);
                }
            });
        }
        return true;
    }
    false
}

/// Zapisuje pierwszy krok dla zadania (po tytule).
#[tauri::command]
pub fn set_first_step(app: AppHandle, title: String, step: String) {
    let key = title.trim().to_lowercase();
    state::update(&app, true, |rt| {
        if step.trim().is_empty() {
            rt.store.first_steps.remove(&key);
        } else {
            rt.store.first_steps.insert(key, step.trim().to_string());
        }
    });
}

#[tauri::command]
pub fn get_first_step(app: AppHandle, title: String) -> Option<String> {
    state::read(&app, |rt| rt.store.first_steps.get(&title.trim().to_lowercase()).cloned())
}
