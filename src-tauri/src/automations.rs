//! Automatyzacje: reguły „Gdy → to” z ustawień.
//!
//! Wątek co 2 s sprawdza warunki (czas, monitor, Wi‑Fi, folder, Horolog,
//! aplikacja, bateria, bezczynność). Reguła odpala się na zboczu — w chwili,
//! gdy warunek zaczyna być spełniony. Z opcją „przywróć” akcje są cofane,
//! gdy warunek minie (koniec spotkania, zamknięcie gry…). Z opcją „najpierw
//! zapytaj” pojawia się karta radia z „Tak / Nie”.

use crate::config::{expand_env, Rule, RuleAction, Trigger};
use crate::state::{self, LogEntry, RadioMsg};
use crate::{commands, layout, platform, radio, spotify, sys};
use chrono::{DateTime, Datelike, Duration as CDuration, Local, Timelike};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::AppHandle;

/// Co cofnąć po zakończeniu warunku.
#[derive(Clone, Debug)]
enum Revert {
    Mode(String),
    Spotify(&'static str),
    Quiet(Option<DateTime<Local>>),
    Restore(Vec<isize>),
    Volume(u8),
    ShowShell,
    OneThingOff,
}

/// Kontekst odpalenia: co wywołało regułę (plik, monitor, spotkanie…).
#[derive(Clone, Debug, Default)]
pub struct Ctx {
    pub file: Option<String>,
    pub label: String,
}

static REVERTS: Mutex<Option<HashMap<String, Vec<Revert>>>> = Mutex::new(None);
static PENDING: Mutex<Option<HashMap<String, Ctx>>> = Mutex::new(None);

#[derive(Default)]
struct Watch {
    active: bool,
    key: String,
    seen: HashSet<String>,
    seeded: bool,
}

// ---------------------------------------------------------------------------
// Opisy (do dziennika i kart radia)
// ---------------------------------------------------------------------------

const DAYS: [&str; 7] = ["pon", "wt", "śr", "czw", "pt", "sob", "nd"];

fn days_label(days: &[u32]) -> String {
    let mut d: Vec<u32> = days.iter().copied().filter(|d| (1..=7).contains(d)).collect();
    d.sort();
    match d.as_slice() {
        [1, 2, 3, 4, 5, 6, 7] => "codziennie".into(),
        [1, 2, 3, 4, 5] => "w dni robocze".into(),
        [6, 7] => "w weekend".into(),
        [] => "nigdy".into(),
        _ => d.iter().map(|n| DAYS[(*n - 1) as usize]).collect::<Vec<_>>().join(", "),
    }
}

pub fn describe_trigger(t: &Trigger) -> String {
    match t.kind.as_str() {
        "time" => format!("jest {} {}", t.time, days_label(&t.days)),
        "monitor" if t.monitor.trim().is_empty() => "podłączę dodatkowy monitor".into(),
        "monitor" => format!("podłączę monitor „{}”", t.monitor.trim()),
        "wifi" => format!("połączę się z Wi‑Fi „{}”", t.ssid.trim()),
        "folder" => {
            let name = t.folder.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or("folder");
            let what = if t.pattern.trim().is_empty() || t.pattern.trim() == "*" { "nowy plik".to_string() } else { format!("plik {}", t.pattern.trim()) };
            format!("w folderze {name} pojawi się {what}")
        }
        "horolog" => {
            let kind = match t.horolog_kind.as_str() {
                "meeting" => "spotkanie",
                "focus" => "blok fokusu",
                "task" => "zadanie",
                "habit" => "nawyk",
                _ => "dowolny blok",
            };
            if t.minutes_before > 0 { format!("za {} min zaczyna się {kind}", t.minutes_before) } else { format!("zaczyna się {kind}") }
        }
        "app" if t.fullscreen && t.app.trim().is_empty() => "coś działa na pełnym ekranie".into(),
        "app" if t.fullscreen => format!("{} działa na pełnym ekranie", t.app.trim()),
        "app" => format!("uruchomię {}", t.app.trim()),
        "battery" => format!("bateria spadnie poniżej {}%", t.battery_below),
        "idle" => format!("nie ruszam komputera {} min", t.idle_minutes),
        other => other.to_string(),
    }
}

pub fn describe_action(cfg: &crate::config::Config, a: &RuleAction) -> String {
    let v = a.value.trim();
    match a.kind.as_str() {
        "mode" => format!("tryb {}", cfg.modes.iter().find(|m| m.id == v).map(|m| m.label.clone()).unwrap_or_else(|| v.to_string())),
        "spotify" => match v {
            "pause" => "Spotify: pauza".into(),
            "next" => "Spotify: następny".into(),
            p if p.starts_with("spotify:") => "Spotify: playlista".into(),
            _ => "Spotify: graj".into(),
        },
        "quiet" if v.is_empty() => "radio: tylko pilne".into(),
        "quiet" if v.contains(':') => format!("radio: cisza do {v}"),
        "quiet" => format!("radio: cisza na {v} min"),
        "message" => format!("radio: „{v}”"),
        "shelf" => "połóż na półce".into(),
        "minimize" => format!("odłóż okna: {v}"),
        "close" => format!("zamknij: {v}"),
        "launch" => format!("uruchom {}", cfg.dock.iter().find(|d| d.id == v).map(|d| d.name.clone()).unwrap_or_else(|| v.to_string())),
        "brightness" => format!("jasność {v}%"),
        "volume" => format!("głośność {v}%"),
        "focus" => format!("fokus {v} min"),
        "hide_shell" => "schowaj dock i paski".into(),
        "layout" if v.is_empty() => "układ okien trybu".into(),
        "layout" => format!("układ okien „{v}”"),
        "one_thing" => "jedna rzecz".into(),
        "command" => format!("polecenie: {v}"),
        other => other.to_string(),
    }
}

pub fn describe_rule(cfg: &crate::config::Config, r: &Rule) -> String {
    let then = r.actions.iter().map(|a| describe_action(cfg, a)).collect::<Vec<_>>().join(", ");
    format!("Gdy {} → {}", describe_trigger(&r.trigger), if then.is_empty() { "nic".into() } else { then })
}

// ---------------------------------------------------------------------------
// Dziennik
// ---------------------------------------------------------------------------

fn log(app: &AppHandle, text: String) {
    state::update(app, true, |rt| {
        rt.store.automation_log.insert(0, LogEntry { at: Local::now(), text });
        rt.store.automation_log.truncate(50);
    });
}

fn when_label(at: DateTime<Local>) -> String {
    let today = Local::now().date_naive();
    let d = at.date_naive();
    if d == today {
        at.format("dziś %H:%M").to_string()
    } else if d == today - CDuration::days(1) {
        at.format("wczoraj %H:%M").to_string()
    } else {
        format!("{} {}", DAYS[(at.weekday().number_from_monday() - 1) as usize], at.format("%H:%M"))
    }
}

// ---------------------------------------------------------------------------
// Wykonanie
// ---------------------------------------------------------------------------

fn parse_quiet_until(v: &str) -> DateTime<Local> {
    let now = Local::now();
    let v = v.trim();
    if let Some(m) = state::parse_hhmm(v) {
        let mut t = now.date_naive().and_hms_opt(m / 60, m % 60, 0).and_then(|d| d.and_local_timezone(Local).single()).unwrap_or(now);
        if t <= now {
            t += CDuration::days(1);
        }
        return t;
    }
    if let Ok(min) = v.parse::<i64>() {
        return now + CDuration::minutes(min.clamp(1, 24 * 60));
    }
    // Bez wartości: do odwołania (przywrócenia), najdłużej 12 h.
    now + CDuration::hours(12)
}

async fn run_action(app: &AppHandle, a: &RuleAction, ctx: &Ctx) -> Option<Revert> {
    let v = a.value.trim().to_string();
    match a.kind.as_str() {
        "mode" => {
            let prev = state::read(app, |rt| rt.store.mode.clone());
            if prev == v {
                return None;
            }
            commands::set_mode(app.clone(), v).await.ok()?;
            Some(Revert::Mode(prev))
        }
        "spotify" => {
            let playing = state::read(app, |rt| rt.music.playing);
            match v.as_str() {
                "pause" => {
                    if !playing {
                        return None;
                    }
                    spotify::control(app, "pause", None).await.ok()?;
                    Some(Revert::Spotify("play"))
                }
                "next" => {
                    let _ = spotify::control(app, "next", None).await;
                    None
                }
                p if p.starts_with("spotify:") => {
                    spotify::play_playlist(app, p, None).await;
                    (!playing).then_some(Revert::Spotify("pause"))
                }
                _ => {
                    if playing {
                        return None;
                    }
                    spotify::control(app, "play", None).await.ok()?;
                    Some(Revert::Spotify("pause"))
                }
            }
        }
        "quiet" => {
            let until = parse_quiet_until(&v);
            let prev = state::update(app, true, |rt| rt.store.quiet_until.replace(until));
            Some(Revert::Quiet(prev))
        }
        "message" => {
            let text = v.replace("{plik}", ctx.file.as_deref().and_then(|f| f.rsplit(['\\', '/']).next()).unwrap_or(""));
            radio::deliver(app, card(text, None));
            None
        }
        "shelf" => {
            let path = ctx.file.clone().or_else(|| (!v.is_empty()).then(|| expand_env(&v)))?;
            commands::shelf_add(app.clone(), vec![path]);
            None
        }
        "minimize" => {
            let exes: Vec<String> = v.split([',', ';']).map(|s| exe_norm(s)).filter(|s| !s.is_empty()).collect();
            let mut hidden = Vec::new();
            for w in platform::app_windows() {
                if exes.contains(&platform::exe_name(&w.exe)) {
                    platform::minimize(w.hwnd);
                    hidden.push(w.hwnd);
                }
            }
            (!hidden.is_empty()).then_some(Revert::Restore(hidden))
        }
        "close" => {
            for exe in v.split([',', ';']).map(exe_norm).filter(|s| !s.is_empty()) {
                sys::close_windows_of(&exe);
            }
            None
        }
        "launch" => {
            let cfg = state::cfg(app);
            if cfg.dock.iter().any(|d| d.id == v) {
                let _ = commands::launch(app.clone(), v);
            } else if !v.is_empty() {
                let _ = commands::open_path(app.clone(), v);
            }
            None
        }
        "brightness" => {
            if let Ok(n) = v.parse::<u8>() {
                std::thread::spawn(move || sys::set_brightness(n.min(100)));
            }
            None
        }
        "volume" => {
            let n = v.parse::<u8>().ok()?;
            let prev = sys::volume().map(|(p, _)| p);
            sys::set_volume(n.min(100));
            crate::stats::refresh_volume(app);
            prev.map(Revert::Volume)
        }
        "focus" => {
            let mins = v.parse::<i64>().ok();
            commands::focus_start_adhoc(app.clone(), mins, None);
            None
        }
        "hide_shell" => {
            layout::FORCE_HIDE.store(true, Ordering::Relaxed);
            Some(Revert::ShowShell)
        }
        "layout" => {
            // Chwila na pojawienie się okien (np. po podłączeniu monitora).
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let _ = crate::windows_mgr::restore_layout(app, &v);
            None
        }
        "one_thing" => {
            crate::focus::set_one_thing(app, true);
            Some(Revert::OneThingOff)
        }
        "command" => {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let line = v.replace("{plik}", ctx.file.as_deref().unwrap_or(""));
                let _ = std::process::Command::new("cmd").args(["/C", &line]).creation_flags(0x0800_0000).spawn();
            }
            None
        }
        _ => None,
    }
}

fn exe_norm(s: &str) -> String {
    let s = s.trim().to_lowercase();
    if s.is_empty() || s.ends_with(".exe") { s } else { format!("{s}.exe") }
}

async fn undo(app: &AppHandle, r: Revert) {
    match r {
        Revert::Mode(m) => {
            let _ = commands::set_mode(app.clone(), m).await;
        }
        Revert::Spotify(a) => {
            let _ = spotify::control(app, a, None).await;
        }
        Revert::Quiet(prev) => {
            state::update(app, true, |rt| rt.store.quiet_until = prev);
            release_if_loud(app);
        }
        Revert::Restore(hwnds) => {
            for h in hwnds {
                platform::restore(h);
            }
        }
        Revert::Volume(v) => {
            sys::set_volume(v);
            crate::stats::refresh_volume(app);
        }
        Revert::ShowShell => {
            layout::FORCE_HIDE.store(false, Ordering::Relaxed);
        }
        Revert::OneThingOff => crate::focus::set_one_thing(app, false),
    }
}

/// Po końcu ciszy: wiadomości czekające wychodzą, jeśli tryb nie jest cichy.
fn release_if_loud(app: &AppHandle) {
    let cfg = state::cfg(app);
    let (mode, quiet) = state::read(app, |rt| (rt.store.mode.clone(), rt.store.quiet_until.is_some_and(|t| t > Local::now())));
    if !quiet && !cfg.mode(&mode).quiet {
        radio::release_batch(app);
    }
}

fn card(message: String, ask: Option<String>) -> RadioMsg {
    RadioMsg {
        id: format!("auto{:x}{:04x}", Local::now().timestamp_millis(), rand::random::<u16>()),
        name: "AUTOMAT".into(),
        number: "A".into(),
        color: "#fbbf24".into(),
        source: "automatyzacja".into(),
        message,
        reply: None,
        reply_url: None,
        urgent: true,
        at: Local::now(),
        app: None,
        open_target: None,
        mail: None,
        ask,
        yes_label: None,
        no_label: None,
    }
}

/// Wykonuje akcje reguły (w tle) i zapisuje w dzienniku.
fn execute(app: &AppHandle, rule: Rule, ctx: Ctx, note: Option<String>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut reverts = Vec::new();
        for a in &rule.actions {
            if let Some(r) = run_action(&app, a, &ctx).await {
                reverts.push(r);
            }
        }
        if rule.revert && !reverts.is_empty() {
            if let Ok(mut g) = REVERTS.lock() {
                g.get_or_insert_with(HashMap::new).insert(rule.id.clone(), reverts);
            }
        }
        let cfg = state::cfg(&app);
        let what = rule.actions.iter().map(|a| describe_action(&cfg, a)).collect::<Vec<_>>().join(", ");
        let who = if ctx.label.is_empty() { describe_trigger(&rule.trigger) } else { ctx.label.clone() };
        let text = match note {
            Some(n) => format!("{who} → {n}, {what}"),
            None => format!("{who} → {what}"),
        };
        state::update(&app, true, |rt| {
            rt.store.rule_last.insert(rule.id.clone(), Local::now());
        });
        log(&app, text);
    });
}

fn cooled(app: &AppHandle, rule: &Rule) -> bool {
    if rule.cooldown_min == 0 {
        return true;
    }
    state::read(app, |rt| rt.store.rule_last.get(&rule.id).copied())
        .map_or(true, |t| Local::now() - t >= CDuration::minutes(rule.cooldown_min as i64))
}

fn fire(app: &AppHandle, rule: &Rule, ctx: Ctx) {
    if !cooled(app, rule) {
        return;
    }
    if rule.ask {
        let cfg = state::cfg(app);
        let then = rule.actions.iter().map(|a| describe_action(&cfg, a)).collect::<Vec<_>>().join(", ");
        let why = if ctx.label.is_empty() { describe_trigger(&rule.trigger) } else { ctx.label.clone() };
        if let Ok(mut g) = PENDING.lock() {
            g.get_or_insert_with(HashMap::new).insert(rule.id.clone(), ctx);
        }
        // Zapisujemy moment pytania, żeby limit częstotliwości obejmował też pytania.
        state::update(app, true, |rt| {
            rt.store.rule_last.insert(rule.id.clone(), Local::now());
        });
        radio::deliver(app, card(format!("{why}. Zrobić: {then}?"), Some(rule.id.clone())));
        return;
    }
    execute(app, rule.clone(), ctx, None);
}

fn revert_rule(app: &AppHandle, id: &str) {
    let list = REVERTS.lock().ok().and_then(|mut g| g.as_mut().and_then(|m| m.remove(id)));
    if let Some(list) = list {
        let app = app.clone();
        let id = id.to_string();
        tauri::async_runtime::spawn(async move {
            for r in list.into_iter().rev() {
                undo(&app, r).await;
            }
            let cfg = state::cfg(&app);
            let label = cfg.automations.iter().find(|r| r.id == id).map(|r| describe_trigger(&r.trigger)).unwrap_or_default();
            log(&app, format!("koniec: {label} → przywrócono"));
        });
    }
}

// ---------------------------------------------------------------------------
// Warunki
// ---------------------------------------------------------------------------

fn wildcard(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn folder_files(t: &Trigger) -> Vec<String> {
    let dir = expand_env(t.folder.trim());
    let pattern = if t.pattern.trim().is_empty() { "*".to_string() } else { t.pattern.trim().to_string() };
    std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|f| f.is_file()).unwrap_or(false))
                .map(|e| e.path())
                .filter(|p| {
                    let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    // Pliki w trakcie pobierania pomijamy.
                    !n.ends_with(".crdownload") && !n.ends_with(".part") && !n.ends_with(".tmp") && !n.starts_with("~$") && wildcard(&pattern, &n)
                })
                .map(|p| p.to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Default)]
struct Inputs {
    monitor_sig: String,
    monitor_count: usize,
    monitor_names: Vec<String>,
    ssid: Option<String>,
    ssid_at: Option<Instant>,
}

fn monitor_signature(app: &AppHandle) -> (String, usize) {
    let list = app.available_monitors().unwrap_or_default();
    let sig = list.iter().map(|m| format!("{:?}{:?}", m.position(), m.size())).collect::<Vec<_>>().join(";");
    (sig, list.len())
}

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut watches: HashMap<String, Watch> = HashMap::new();
        let mut inputs = Inputs::default();
        let mut folder_at = Instant::now() - Duration::from_secs(60);
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let cfg = state::cfg(&app);

            // Cisza z automatyzacji minęła → paczka wychodzi.
            let expired = state::read(&app, |rt| rt.store.quiet_until.is_some_and(|t| t <= Local::now()));
            if expired {
                state::update(&app, true, |rt| rt.store.quiet_until = None);
                release_if_loud(&app);
            }

            let rules: Vec<&Rule> = cfg.automations.iter().filter(|r| r.enabled && !r.actions.is_empty()).collect();
            watches.retain(|id, _| rules.iter().any(|r| &r.id == id));
            if rules.is_empty() {
                continue;
            }
            let has = |k: &str| rules.iter().any(|r| r.trigger.kind == k);

            // Monitory: nazwy z WMI tylko po zmianie układu (to wolne wywołanie).
            if has("monitor") {
                let (sig, count) = monitor_signature(&app);
                if sig != inputs.monitor_sig {
                    inputs.monitor_sig = sig;
                    inputs.monitor_count = count;
                    if rules.iter().any(|r| r.trigger.kind == "monitor" && !r.trigger.monitor.trim().is_empty()) {
                        inputs.monitor_names = sys::monitor_names();
                    }
                }
            }
            if has("wifi") && inputs.ssid_at.map_or(true, |t| t.elapsed() > Duration::from_secs(10)) {
                inputs.ssid = crate::stats::latest().wifi_ssid.or_else(|| sys::wifi().0);
                inputs.ssid_at = Some(Instant::now());
            }
            let scan_folders = folder_at.elapsed() > Duration::from_secs(4);
            if scan_folders {
                folder_at = Instant::now();
            }
            let now = Local::now();
            let (running, plan) = state::read(&app, |rt| (rt.running.clone(), rt.plan.clone()));
            let fullscreen = has("app") && platform::fullscreen_monitor().is_some();
            let battery = if has("battery") { platform::battery() } else { None };
            let idle = if has("idle") { platform::idle_secs() } else { 0 };

            for rule in rules {
                let t = &rule.trigger;
                let w = watches.entry(rule.id.clone()).or_default();
                // (warunek, kontekst, klucz wystąpienia)
                let (cond, ctx, key): (bool, Ctx, String) = match t.kind.as_str() {
                    "time" => {
                        let ok = state::parse_hhmm(&t.time).is_some_and(|m| m == now.hour() * 60 + now.minute())
                            && t.days.contains(&now.weekday().number_from_monday());
                        (ok, Ctx { label: format!("Jest {}", t.time.trim()), ..Ctx::default() }, format!("{} {}", now.date_naive(), t.time))
                    }
                    "folder" => {
                        if !scan_folders {
                            continue;
                        }
                        let files = folder_files(t);
                        if !w.seeded {
                            w.seen = files.into_iter().collect();
                            w.seeded = true;
                            continue;
                        }
                        let fresh: Vec<String> = files.iter().filter(|f| !w.seen.contains(*f)).cloned().collect();
                        w.seen = files.into_iter().collect();
                        for f in fresh {
                            let name = f.rsplit(['\\', '/']).next().unwrap_or(&f).to_string();
                            // Plik z folderu nie podlega limitowi częstotliwości na regułę.
                            let mut r = rule.clone();
                            r.cooldown_min = 0;
                            fire(&app, &r, Ctx { file: Some(f.clone()), label: name });
                        }
                        continue;
                    }
                    "monitor" => {
                        let want = t.monitor.trim().to_lowercase();
                        let hit = if want.is_empty() {
                            (inputs.monitor_count > 1).then(|| "Dodatkowy monitor".to_string())
                        } else {
                            inputs.monitor_names.iter().find(|n| n.to_lowercase().contains(&want)).cloned()
                        };
                        (hit.is_some(), Ctx { label: hit.unwrap_or_default(), ..Ctx::default() }, String::new())
                    }
                    "wifi" => {
                        let ok = inputs.ssid.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(t.ssid.trim()));
                        (ok, Ctx { label: format!("Wi‑Fi {}", t.ssid.trim()), ..Ctx::default() }, String::new())
                    }
                    "horolog" => {
                        let item = plan.iter().find(|i| {
                            let kind_ok = match t.horolog_kind.as_str() {
                                "any" | "" => i.kind != "buffer",
                                "meeting" => i.kind == "meeting" || i.kind == "busy",
                                k => i.kind == k,
                            };
                            kind_ok && i.start - CDuration::minutes(t.minutes_before.max(0)) <= now && now < i.end
                        });
                        match item {
                            Some(i) => (true, Ctx { label: i.title.clone(), ..Ctx::default() }, i.key.clone()),
                            None => (false, Ctx::default(), String::new()),
                        }
                    }
                    "app" => {
                        let exe = exe_norm(&t.app);
                        let run = !exe.is_empty() && running.contains(&exe);
                        let ok = if t.fullscreen { fullscreen && (exe.is_empty() || run) } else { run };
                        let label = if t.app.trim().is_empty() { "Pełny ekran".to_string() } else { t.app.trim().to_string() };
                        (ok, Ctx { label, ..Ctx::default() }, String::new())
                    }
                    "battery" => {
                        let ok = battery.is_some_and(|(p, ch)| p < t.battery_below && !ch);
                        (ok, Ctx { label: format!("Bateria {}%", battery.map(|b| b.0).unwrap_or(0)), ..Ctx::default() }, String::new())
                    }
                    "idle" => {
                        let ok = idle >= t.idle_minutes.max(1) as u64 * 60;
                        (ok, Ctx { label: format!("Bezczynność {} min", t.idle_minutes), ..Ctx::default() }, String::new())
                    }
                    _ => continue,
                };

                // Klucz (czas, blok Horologa): ten sam blok/minuta odpala raz.
                let new_occurrence = cond && (!w.active || (!key.is_empty() && key != w.key));
                if new_occurrence {
                    if !key.is_empty() && w.active && rule.revert {
                        revert_rule(&app, &rule.id);
                    }
                    w.key = key;
                    fire(&app, rule, ctx);
                } else if !cond && w.active && rule.revert {
                    revert_rule(&app, &rule.id);
                }
                if !cond {
                    w.key.clear();
                }
                w.active = cond;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Komendy
// ---------------------------------------------------------------------------

/// Odpowiedź na kartę „Tak / Nie”.
#[tauri::command]
pub fn automation_answer(app: AppHandle, yes: bool) {
    let rule_id = state::read(&app, |rt| rt.showing.front().and_then(|m| m.ask.clone()));
    radio::dismiss(&app, false);
    let Some(id) = rule_id else { return };
    if crate::focus::answer(&app, &id, yes) || crate::insights::answer(&app, &id, yes) {
        return;
    }
    let ctx = PENDING.lock().ok().and_then(|mut g| g.as_mut().and_then(|m| m.remove(&id))).unwrap_or_default();
    let cfg = state::cfg(&app);
    let Some(rule) = cfg.automations.iter().find(|r| r.id == id).cloned() else { return };
    if yes {
        execute(&app, rule, ctx, Some("zapytano: Tak".into()));
    } else {
        let who = if ctx.label.is_empty() { describe_trigger(&rule.trigger) } else { ctx.label };
        log(&app, format!("{who} → zapytano, odpowiedź: Nie"));
    }
}

/// „Uruchom teraz” z ustawień — bez pytania i limitu, na regule z edytora.
#[tauri::command]
pub fn automation_run(app: AppHandle, rule: Rule) {
    let mut ctx = Ctx { label: "Ręcznie".into(), ..Ctx::default() };
    if rule.trigger.kind == "folder" {
        // Na próbę: najnowszy pasujący plik.
        let newest = folder_files(&rule.trigger)
            .into_iter()
            .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        ctx.file = newest;
    }
    execute(&app, rule, ctx, None);
}

#[derive(Serialize)]
pub struct AutomationStatus {
    log: Vec<Value>,
    last: HashMap<String, String>,
    monitors: Vec<String>,
    ssid: Option<String>,
    layouts: Vec<String>,
}

#[tauri::command]
pub async fn automation_status(app: AppHandle) -> AutomationStatus {
    let (log, last) = state::read(&app, |rt| {
        let log = rt
            .store
            .automation_log
            .iter()
            .take(12)
            .map(|e| {
                let t = if e.at.date_naive() == Local::now().date_naive() { e.at.format("%H:%M").to_string() } else { when_label(e.at) };
                json!({ "t": t, "text": e.text })
            })
            .collect::<Vec<_>>();
        let last = rt.store.rule_last.iter().map(|(k, v)| (k.clone(), when_label(*v))).collect();
        (log, last)
    });
    let (monitors, ssid) = tauri::async_runtime::spawn_blocking(|| (sys::monitor_names(), crate::stats::latest().wifi_ssid.or_else(|| sys::wifi().0)))
        .await
        .unwrap_or_default();
    let layouts = state::read(&app, |rt| rt.store.layouts.iter().map(|l| l.name.clone()).collect());
    AutomationStatus { log, last, monitors, ssid, layouts }
}
