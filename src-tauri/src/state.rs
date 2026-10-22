//! Wspólny stan aplikacji i „migawka” wysyłana do wszystkich okien.
//!
//! Każde okno renderuje się wyłącznie z migawki (zdarzenie `snapshot`), więc
//! pasek, dock, szuflada i panel Teraz zawsze pokazują to samo.

use crate::config::{section_colors, Config};
use chrono::{DateTime, Local, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayItem {
    pub key: String,
    pub intent_id: Option<String>,
    pub title: String,
    /// task | habit | focus | buffer | meeting | busy
    pub kind: String,
    pub priority: Option<u8>,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub moved: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mark {
    /// early | ontime | late
    pub status: String,
    pub delta_min: i64,
    pub date: String,
}

/// Jeden rozegrany sektor (blok planu) — do raportu tygodnia.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistEntry {
    pub date: String,
    pub title: String,
    pub kind: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    /// early | ontime | late | skipped
    pub status: String,
    pub delta_min: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfItem {
    pub name: String,
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashWindow {
    pub hwnd: isize,
    pub exe: String,
    pub title: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stash {
    pub id: String,
    pub name: String,
    pub created: DateTime<Local>,
    pub windows: Vec<StashWindow>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RadioMsg {
    pub id: String,
    pub name: String,
    pub number: String,
    pub color: String,
    pub source: String,
    pub message: String,
    pub reply: Option<String>,
    pub reply_url: Option<String>,
    pub urgent: bool,
    pub at: DateTime<Local>,
    /// Aplikacja, z której przyszło powiadomienie Windows.
    #[serde(default)]
    pub app: Option<String>,
    /// Co otworzyć przyciskiem „Otwórz”, np. `shell:AppsFolder\<AUMID>`.
    #[serde(default)]
    pub open_target: Option<String>,
    /// Pytanie automatyzacji: id reguły, karta pokazuje „Tak / Nie”.
    #[serde(default)]
    pub ask: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub at: DateTime<Local>,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotifyTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    /// Uprawnienia nadane przy logowaniu (do sprawdzenia, czy trzeba połączyć ponownie).
    #[serde(default)]
    pub scope: String,
}

/// To, co przeżywa restart: zapisywane w `%APPDATA%\EdgeShell\store.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Store {
    pub mode: String,
    pub shelf: Vec<ShelfItem>,
    pub stashes: Vec<Stash>,
    pub marks: HashMap<String, Mark>,
    pub plan_cache: Vec<DayItem>,
    pub plan_cached_at: Option<DateTime<Local>>,
    pub offline_captures: Vec<String>,
    pub waiting: Vec<RadioMsg>,
    pub spotify: Option<SpotifyTokens>,
    pub week_focus_min: i64,
    /// Stan paska zadań Windows sprzed pierwszego uruchomienia (do przywrócenia).
    pub taskbar_state: usize,
    pub taskbar_saved: bool,
    pub history: Vec<HistEntry>,
    pub week_cache: Vec<DayItem>,
    /// Radio w ciszy do tej chwili (automatyzacje).
    pub quiet_until: Option<DateTime<Local>>,
    /// Ostatnie wykonanie reguły automatyzacji.
    pub rule_last: HashMap<String, DateTime<Local>>,
    pub automation_log: Vec<LogEntry>,
    pub player: PlayerState,
}

/// Pływający odtwarzacz: czy widoczny, pozycja (px fizyczne), karta/pasek, na wierzchu.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlayerState {
    pub visible: bool,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub compact: bool,
    pub pinned: bool,
    /// Rozwinięty panel pod kartą: "" | "queue" | "playlists".
    pub panel: String,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self { visible: false, x: None, y: None, compact: false, pinned: true, panel: String::new() }
    }
}

impl Store {
    fn path() -> std::path::PathBuf {
        crate::config::data_dir().join("store.json")
    }
    pub fn load() -> Store {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        if let Ok(text) = serde_json::to_string(self) {
            let _ = std::fs::write(Self::path(), text);
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Adhoc {
    pub title: String,
    pub start: Option<DateTime<Local>>,
    pub end: Option<DateTime<Local>>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Music {
    pub connected: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub art: String,
    pub progress_ms: i64,
    pub duration_ms: i64,
    pub fetched_at_ms: i64,
    pub error: String,
    /// Głośność urządzenia Spotify (0–100).
    pub volume: Option<i64>,
    pub device: String,
    pub album: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileItem {
    pub name: String,
    pub path: String,
    pub modified: i64,
}

#[derive(Default)]
pub struct Runtime {
    pub store: Store,
    pub plan: Vec<DayItem>,
    pub week: Vec<DayItem>,
    pub connected: bool,
    pub horolog_error: String,
    pub last_sync: Option<DateTime<Local>>,
    pub adhoc: Option<Adhoc>,
    pub paused_since: Option<DateTime<Local>>,
    pub showing: VecDeque<RadioMsg>,
    pub clipboard: Vec<String>,
    pub foreground: String,
    pub battery: Option<(u8, bool)>,
    pub music: Music,
    pub drawer_open: bool,
    pub now_open: bool,
    pub running: HashSet<String>,
    pub window_counts: HashMap<String, usize>,
    pub recent: Vec<FileItem>,
    pub fired: HashSet<String>,
    pub last_batch: String,
    pub friction_app: Option<String>,
    pub glow_until: Option<DateTime<Local>>,
    pub notif_status: String,
    /// Nieprzeczytane powiadomienia na ikonach docka (id aplikacji → liczba).
    pub badges: HashMap<String, u32>,
    /// Uruchomione aplikacje z oknami, których nie ma w docku (w kolejności otwarcia).
    pub open_apps: Vec<OpenApp>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenApp {
    /// Nazwa pliku małymi literami, np. `notepad.exe`.
    pub exe: String,
    pub path: String,
    pub name: String,
    pub windows: usize,
}

pub struct Shared {
    pub cfg: RwLock<Config>,
    pub rt: Mutex<Runtime>,
}

impl Shared {
    pub fn new(cfg: Config) -> Arc<Shared> {
        let store = Store::load();
        let mut rt = Runtime::default();
        rt.plan = store.plan_cache.clone();
        rt.week = store.week_cache.clone();
        rt.store = store;
        if rt.store.mode.is_empty() || rt.store.mode == "wieczor" {
            rt.store.mode = cfg.start_mode.clone();
        }
        Arc::new(Shared { cfg: RwLock::new(cfg), rt: Mutex::new(rt) })
    }
}

pub fn shared(app: &AppHandle) -> Arc<Shared> {
    app.state::<Arc<Shared>>().inner().clone()
}

pub fn cfg(app: &AppHandle) -> Config {
    shared(app).cfg.read().map(|c| c.clone()).unwrap_or_default()
}

/// Wykonuje zmianę na stanie, zapisuje go i wysyła nową migawkę do okien.
pub fn update<R>(app: &AppHandle, persist: bool, f: impl FnOnce(&mut Runtime) -> R) -> R {
    let sh = shared(app);
    let out = {
        let mut rt = sh.rt.lock().unwrap_or_else(|e| e.into_inner());
        let out = f(&mut rt);
        if persist {
            rt.store.save();
        }
        out
    };
    emit_snapshot(app);
    out
}

pub fn read<R>(app: &AppHandle, f: impl FnOnce(&Runtime) -> R) -> R {
    let sh = shared(app);
    let rt = sh.rt.lock().unwrap_or_else(|e| e.into_inner());
    f(&rt)
}

pub fn emit_snapshot(app: &AppHandle) {
    let snap = snapshot(app);
    let _ = app.emit("snapshot", snap);
}

pub fn today_key() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// Element planu, który trwa teraz (pomija przeszłe i przyszłe).
pub fn current_item(plan: &[DayItem], now: DateTime<Local>) -> Option<DayItem> {
    plan.iter()
        .filter(|i| i.start <= now && now < i.end)
        // Spotkanie ma pierwszeństwo przed blokiem, który na nie nachodzi.
        .max_by_key(|i| if i.kind == "meeting" || i.kind == "busy" { 2 } else { 1 })
        .cloned()
}

pub fn next_item(plan: &[DayItem], now: DateTime<Local>) -> Option<DayItem> {
    plan.iter().filter(|i| i.start > now).min_by_key(|i| i.start).cloned()
}

pub fn snapshot(app: &AppHandle) -> Value {
    let cfg = cfg(app);
    let sh = shared(app);
    let rt = sh.rt.lock().unwrap_or_else(|e| e.into_inner());
    let now = Local::now();
    let mode = cfg.mode(&rt.store.mode);
    let today = today_key();

    // Dock pogrupowany według kolejności sekcji; separator między sekcjami.
    let section_index = |id: &str| cfg.sections.iter().position(|s| s.id == id).unwrap_or(usize::MAX);
    let mut ordered: Vec<&crate::config::DockApp> = cfg.dock.iter().collect();
    ordered.sort_by_key(|a| section_index(&a.category));
    let mut dock: Vec<Value> = ordered
        .iter()
        .enumerate()
        .map(|(n, a)| {
            let (bg, fg, stripe) = section_colors(&cfg.sections, &a.category);
            let exes = a.exe_names();
            let running = exes.iter().any(|e| rt.running.contains(e));
            let windows = exes.iter().filter_map(|e| rt.window_counts.get(e)).sum::<usize>();
            let separator = ordered.get(n + 1).is_some_and(|next| next.category != a.category);
            let badge = rt.badges.get(&a.id).copied().unwrap_or(0);
            let stack = crate::stacks::is_folder(&a.path);
            json!({
                "id": a.id, "name": a.name, "icon": a.icon, "category": a.category,
                "bg": bg, "fg": fg, "stripe": stripe,
                "dim": mode.dim.iter().any(|d| d == &a.id),
                "running": running, "windows": windows, "separatorAfter": separator,
                "badge": badge, "stack": stack,
            })
        })
        .collect();

    // Otwarte aplikacje spoza docka — za separatorem, żeby szybko do nich przejść.
    if !rt.open_apps.is_empty() {
        if let Some(last) = dock.last_mut() {
            last["separatorAfter"] = json!(true);
        }
        for o in &rt.open_apps {
            dock.push(json!({
                "id": format!("run:{}", o.exe), "name": o.name, "icon": "auto", "category": "",
                "bg": "#27272a", "fg": "#e4e4e7", "stripe": "rgba(255,255,255,0.35)",
                "dim": false, "running": true, "windows": o.windows, "separatorAfter": false,
                "badge": 0, "stack": false, "unpinned": true, "path": o.path,
            }));
        }
    }

    let day: Vec<Value> = rt
        .plan
        .iter()
        .map(|i| {
            let mark = rt.store.marks.get(&i.key).filter(|m| m.date == today);
            let status = if let Some(m) = mark {
                m.status.clone()
            } else if i.end <= now {
                "past".into()
            } else if i.start <= now {
                "now".into()
            } else {
                "next".into()
            };
            json!({
                "key": i.key, "intentId": i.intent_id, "title": i.title, "kind": i.kind,
                "priority": i.priority, "start": i.start, "end": i.end, "moved": i.moved,
                "status": status, "deltaMin": mark.map(|m| m.delta_min),
            })
        })
        .collect();

    let current = current_item(&rt.plan, now);
    let focus = if let Some(a) = &rt.adhoc {
        json!({ "source": "adhoc", "title": a.title, "kind": "focus", "start": a.start, "end": a.end,
                "key": "adhoc", "completable": true, "intentId": Value::Null })
    } else if let Some(c) = &current {
        json!({ "source": "block", "title": c.title, "kind": c.kind, "start": c.start, "end": c.end,
                "key": c.key, "priority": c.priority, "intentId": c.intent_id,
                "completable": c.kind != "meeting" && c.kind != "busy" && c.kind != "buffer" })
    } else {
        Value::Null
    };

    let next = next_item(&rt.plan, now);
    let next_batch = next_batch_time(&cfg.radio.batch_times, now);
    let modes: Vec<Value> = cfg
        .modes
        .iter()
        .map(|m| json!({ "id": m.id, "label": m.label, "accent": m.accent }))
        .collect();

    json!({
        "now": now,
        "mode": { "id": mode.id, "label": mode.label, "accent": mode.accent, "quiet": mode.quiet },
        "modes": modes,
        "dock": dock,
        "day": day,
        "focus": focus,
        "paused": rt.paused_since.is_some(),
        "pausedSince": rt.paused_since,
        "next": next.map(|n| json!({ "title": n.title, "kind": n.kind, "start": n.start, "end": n.end })),
        "horolog": {
            "connected": rt.connected, "error": rt.horolog_error, "lastSync": rt.last_sync,
            "url": cfg.horolog.url, "pendingCaptures": rt.store.offline_captures.len(),
        },
        "weekFocus": { "minutes": rt.store.week_focus_min, "goalHours": cfg.focus.weekly_goal_hours },
        "radio": {
            "current": rt.showing.front(),
            "showingCount": rt.showing.len(),
            "waiting": rt.store.waiting.len(),
            "nextBatch": next_batch,
            "notifications": rt.notif_status,
        },
        "shelf": rt.store.shelf,
        "stashes": rt.store.stashes,
        "pinned": cfg.pinned.iter().map(|p| {
            let (_, _, stripe) = section_colors(&cfg.sections, &p.category);
            json!({ "name": p.name, "path": p.path, "stripe": stripe })
        }).collect::<Vec<_>>(),
        "recent": rt.recent,
        "clipboard": rt.clipboard,
        "app": rt.foreground,
        "battery": rt.battery.map(|(p, c)| json!({ "percent": p, "charging": c })),
        "music": rt.music,
        "spotifyEnabled": !cfg.spotify.client_id.is_empty(),
        "player": rt.store.player,
        "panels": { "drawer": rt.drawer_open, "now": rt.now_open },
        "friction": rt.friction_app.as_ref().and_then(|id| cfg.dock.iter().find(|a| &a.id == id))
            .map(|a| json!({ "id": a.id, "name": a.name })),
        "hotkeys": cfg.hotkeys,
        "showBubbles": cfg.shell.show_bubbles,
        "barStats": cfg.bar.stats,
        "showRail": cfg.shell.show_rail,
        "clock": format!("{:02}:{:02}", now.hour(), now.minute()),
    })
}

pub fn next_batch_time(times: &[String], now: DateTime<Local>) -> Option<String> {
    let cur = now.hour() * 60 + now.minute();
    let mut mins: Vec<(u32, &String)> = times
        .iter()
        .filter_map(|t| parse_hhmm(t).map(|m| (m, t)))
        .collect();
    mins.sort();
    mins.iter().find(|(m, _)| *m > cur).or(mins.first()).map(|(_, t)| (*t).clone())
}

pub fn parse_hhmm(t: &str) -> Option<u32> {
    let (h, m) = t.split_once(':')?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    (h < 24 && m < 60).then_some(h * 60 + m)
}
