//! Paleta poleceń: jedno pole na wszystko. Szybkie klawisze (fk 45, tp, m …),
//! kalkulator, Tailscale oraz wyszukiwanie: otwarte okna, aplikacje, pliki i
//! bloki z Horologa. Enter wykonuje, Tab pokazuje więcej akcji.

use crate::config::{expand_env, Config};
use crate::state::{self, DayItem};
use crate::{commands, engine, horolog, insights, layout, platform, radio, settings, spotify};
use chrono::{DateTime, Local, TimeZone};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Manager, WindowEvent};

// ---------------------------------------------------------------------------
// Wyniki
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Default)]
pub struct Preview {
    title: String,
    body: String,
    effect: String,
}

#[derive(Serialize, Clone)]
pub struct Act {
    id: String,
    label: String,
}

#[derive(Serialize, Clone)]
pub struct Row {
    id: String,
    mono: String,
    tile: String,
    title: String,
    sub: String,
    key: String,
    preview: Preview,
    actions: Vec<Act>,
    /// Ikona pliku/aplikacji (data URL), gdy jest.
    icon: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct Group {
    name: String,
    items: Vec<Row>,
}

#[derive(Serialize, Clone)]
pub struct Answer {
    chip: String,
    groups: Vec<Group>,
}

#[derive(Clone, Debug)]
enum Cmd {
    Focus(i64),
    FocusBlock,
    Pause,
    Mode(String),
    Capture(String),
    Stash,
    Spotify,
    SpotifyNext,
    Player,
    OneThing,
    Clip(String),
    Screenshot,
    LayoutSave(String),
    LayoutRestore(String),
    Evening,
    Tailscale(String),
    TailscaleOff,
    Calc(String),
    Replan,
    Report,
    Settings,
    Window(isize),
    File(String),
    DockApp(String),
    StartApp(String),
    Plan { title: String, minutes: i64, intent: Option<String>, kind: String },
}

static LAST: Mutex<Option<HashMap<String, Cmd>>> = Mutex::new(None);

const RED: &str = "#D42E25";
const BLUE: &str = "#2563EB";
const PURPLE: &str = "#7C3AED";
const GREEN: &str = "#16a34a";
const AMBER: &str = "#b45309";
const GREY: &str = "#3f3f46";

struct Builder {
    rows: Vec<(String, Row, Cmd)>,
}

impl Builder {
    fn add(&mut self, group: &str, mono: &str, tile: &str, title: String, sub: String, key: &str, preview: Preview, actions: &[(&str, &str)], cmd: Cmd) {
        let id = format!("r{}", self.rows.len());
        self.rows.push((
            group.to_string(),
            Row {
                id,
                mono: mono.to_string(),
                tile: tile.to_string(),
                title,
                sub,
                key: key.to_string(),
                preview,
                actions: actions.iter().map(|(i, l)| Act { id: i.to_string(), label: l.to_string() }).collect(),
                icon: None,
            },
            cmd,
        ));
    }

    fn finish(self, chip: String) -> Answer {
        let mut groups: Vec<Group> = Vec::new();
        let mut map = HashMap::new();
        for (g, row, cmd) in self.rows {
            map.insert(row.id.clone(), cmd);
            match groups.iter_mut().find(|x| x.name == g) {
                Some(x) => x.items.push(row),
                None => groups.push(Group { name: g, items: vec![row] }),
            }
        }
        if let Ok(mut l) = LAST.lock() {
            *l = Some(map);
        }
        Answer { chip, groups }
    }
}

fn pv(title: impl Into<String>, body: impl Into<String>, effect: impl Into<String>) -> Preview {
    Preview { title: title.into(), body: body.into(), effect: effect.into() }
}

// ---------------------------------------------------------------------------
// Indeks plików i aplikacji (odświeżany w tle co 5 min)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FileEntry {
    name: String,
    lower: String,
    path: String,
    place: String,
    modified: i64,
    dir: bool,
}

#[derive(Clone)]
struct AppEntry {
    name: String,
    lower: String,
    path: String,
}

struct Index {
    built: Instant,
    files: Vec<FileEntry>,
    apps: Vec<AppEntry>,
}

static INDEX: Mutex<Option<Index>> = Mutex::new(None);
static BUILDING: Mutex<bool> = Mutex::new(false);

fn roots(cfg: &Config) -> Vec<(String, std::path::PathBuf)> {
    let mut out = Vec::new();
    if let Some(d) = dirs::desktop_dir() {
        out.push(("Pulpit".to_string(), d));
    }
    if let Some(d) = dirs::document_dir() {
        out.push(("Dokumenty".to_string(), d));
    }
    if let Some(d) = dirs::download_dir() {
        out.push(("Pobrane".to_string(), d));
    }
    for p in &cfg.pinned {
        out.push((p.name.clone(), std::path::PathBuf::from(expand_env(&p.path))));
    }
    for f in &cfg.palette.folders {
        let p = std::path::PathBuf::from(expand_env(f));
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.clone());
        out.push((name, p));
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out.dedup_by(|a, b| a.1 == b.1);
    out
}

fn build_index(cfg: Config) {
    let mut files = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (place, root) in roots(&cfg) {
        let mut stack = vec![(root, 0u32)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                if files.len() > 30_000 {
                    break;
                }
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || name.starts_with("~$") || name.eq_ignore_ascii_case("desktop.ini") || name == "node_modules" {
                    continue;
                }
                let path = e.path();
                if !seen.insert(path.clone()) {
                    continue;
                }
                let Ok(md) = e.metadata() else { continue };
                let modified = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let dir_flag = md.is_dir();
                if dir_flag && depth < 3 {
                    stack.push((path.clone(), depth + 1));
                }
                files.push(FileEntry {
                    lower: name.to_lowercase(),
                    name,
                    path: path.to_string_lossy().to_string(),
                    place: place.clone(),
                    modified,
                    dir: dir_flag,
                });
            }
        }
    }
    let apps = commands::search_index()
        .into_iter()
        .map(|e| {
            let v = serde_json::to_value(&e).unwrap_or_default();
            let name = v["name"].as_str().unwrap_or_default().to_string();
            AppEntry { lower: name.to_lowercase(), name, path: v["path"].as_str().unwrap_or_default().to_string() }
        })
        .collect();
    if let Ok(mut g) = INDEX.lock() {
        *g = Some(Index { built: Instant::now(), files, apps });
    }
}

fn refresh_index(app: &AppHandle, force: bool) {
    let stale = INDEX.lock().ok().map_or(true, |g| g.as_ref().map_or(true, |i| i.built.elapsed() > Duration::from_secs(300)));
    if !(stale || force) {
        return;
    }
    if let Ok(mut b) = BUILDING.lock() {
        if *b {
            return;
        }
        *b = true;
    }
    let cfg = state::cfg(app);
    std::thread::spawn(move || {
        build_index(cfg);
        if let Ok(mut b) = BUILDING.lock() {
            *b = false;
        }
    });
}

/// 0 = nie pasuje; im więcej, tym lepiej.
fn score(lower: &str, q: &str, words: &[&str]) -> u32 {
    if q.is_empty() {
        return 0;
    }
    if lower == q {
        return 100;
    }
    if lower.starts_with(q) {
        return 60;
    }
    if lower.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(q)) {
        return 40;
    }
    if lower.contains(q) {
        return 25;
    }
    if words.len() > 1 && words.iter().all(|w| lower.contains(w)) {
        return 15;
    }
    0
}

fn ago(ts: i64) -> String {
    let now = Local::now().timestamp();
    let d = (now - ts).max(0);
    match d {
        0..=59 => "przed chwilą".into(),
        60..=3599 => format!("{} min temu", d / 60),
        3600..=86_399 => format!("{} h temu", d / 3600),
        86_400..=172_799 => "wczoraj".into(),
        _ => Local.timestamp_opt(ts, 0).single().map(|t| t.format("%d.%m.%Y").to_string()).unwrap_or_default(),
    }
}

fn file_mono(name: &str, dir: bool) -> (&'static str, &'static str) {
    if dir {
        return ("D", AMBER);
    }
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "xlsx" | "xls" | "csv" | "xlsm" => ("X", GREEN),
        "docx" | "doc" | "rtf" | "odt" => ("W", BLUE),
        "pptx" | "ppt" => ("P", "#c2410c"),
        "pdf" => ("P", "#b91c1c"),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" => ("I", PURPLE),
        "zip" | "7z" | "rar" => ("Z", GREY),
        "exe" | "msi" | "lnk" => ("A", GREY),
        _ => ("F", GREY),
    }
}

// ---------------------------------------------------------------------------
// Kalkulator
// ---------------------------------------------------------------------------

struct Calc<'a> {
    s: &'a [u8],
    i: usize,
}

impl Calc<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i] == b' ' {
            self.i += 1;
        }
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            self.ws();
            match self.s.get(self.i) {
                Some(b'+') => {
                    self.i += 1;
                    v += self.term()?;
                }
                Some(b'-') => {
                    self.i += 1;
                    v -= self.term()?;
                }
                _ => return Some(v),
            }
        }
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.power()?;
        loop {
            self.ws();
            match self.s.get(self.i) {
                Some(b'*') | Some(b'x') => {
                    self.i += 1;
                    v *= self.power()?;
                }
                Some(b'/') | Some(b':') => {
                    self.i += 1;
                    v /= self.power()?;
                }
                _ => return Some(v),
            }
        }
    }
    fn power(&mut self) -> Option<f64> {
        let b = self.unary()?;
        self.ws();
        if self.s.get(self.i) == Some(&b'^') {
            self.i += 1;
            let e = self.power()?;
            return Some(b.powf(e));
        }
        Some(b)
    }
    fn unary(&mut self) -> Option<f64> {
        self.ws();
        if self.s.get(self.i) == Some(&b'-') {
            self.i += 1;
            return Some(-self.unary()?);
        }
        let mut v = self.atom()?;
        self.ws();
        if self.s.get(self.i) == Some(&b'%') {
            self.i += 1;
            v /= 100.0;
        }
        Some(v)
    }
    fn atom(&mut self) -> Option<f64> {
        self.ws();
        if self.s.get(self.i) == Some(&b'(') {
            self.i += 1;
            let v = self.expr()?;
            self.ws();
            if self.s.get(self.i) != Some(&b')') {
                return None;
            }
            self.i += 1;
            return Some(v);
        }
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.') {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()
    }
}

fn calc(expr: &str) -> Option<String> {
    let e = expr.trim().trim_start_matches('=').replace(',', ".").to_lowercase();
    if e.trim().is_empty() {
        return None;
    }
    let mut c = Calc { s: e.as_bytes(), i: 0 };
    let v = c.expr()?;
    c.ws();
    if c.i != c.s.len() || !v.is_finite() {
        return None;
    }
    let r = (v * 1e10).round() / 1e10;
    let text = if r.fract() == 0.0 && r.abs() < 1e15 { format!("{}", r as i64) } else { format!("{r}") };
    Some(text.replace('.', ","))
}

fn looks_like_math(q: &str) -> bool {
    q.starts_with('=')
        || (q.chars().any(|c| c.is_ascii_digit())
            && q.chars().any(|c| "+-*/^%x:".contains(c))
            && q.chars().all(|c| c.is_ascii_digit() || " .,+-*/^()%x:".contains(c)))
}

// ---------------------------------------------------------------------------
// Polecenia (szybkie klawisze)
// ---------------------------------------------------------------------------

const CMD_NAMES: &[(&str, &str)] = &[
    ("focus", "start fokusu"),
    ("focus_block", "fokus na bieżący blok"),
    ("pause", "pauza / wznów"),
    ("capture", "wrzuć myśl"),
    ("stash", "odłóż okna do garażu"),
    ("spotify", "Spotify: play / pauza"),
    ("spotify_next", "Spotify: następny"),
    ("tailscale", "Tailscale: exit node"),
    ("tailscale_off", "Tailscale: wyłącz exit node"),
    ("calc", "kalkulator"),
    ("replan", "przeplanuj dzień"),
    ("report", "raport tygodnia"),
    ("settings", "ustawienia"),
    ("one_thing", "jedna rzecz (przyciemnij resztę)"),
    ("clipboard", "historia schowka"),
    ("screenshot", "zrzut ekranu z adnotacją"),
    ("layout_save", "zapisz układ okien"),
    ("layout_restore", "przywróć układ okien"),
    ("evening", "zamknięcie dnia"),
];

fn cmd_label(cfg: &Config, cmd: &str) -> String {
    if let Some(id) = cmd.strip_prefix("mode:") {
        return format!("tryb {}", cfg.modes.iter().find(|m| m.id == id).map(|m| m.label.clone()).unwrap_or_else(|| id.into()));
    }
    CMD_NAMES.iter().find(|(c, _)| *c == cmd).map(|(_, l)| l.to_string()).unwrap_or_else(|| cmd.into())
}

/// Szybkie klawisze z konfiguracji + brakujące tryby (t + pierwsza litera).
fn keys(cfg: &Config) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> =
        cfg.palette.keys.iter().filter(|k| !k.key.trim().is_empty()).map(|k| (k.cmd.clone(), k.key.trim().to_lowercase())).collect();
    for m in &cfg.modes {
        let cmd = format!("mode:{}", m.id);
        if !out.iter().any(|(c, _)| *c == cmd) {
            let first = m.label.chars().next().map(|c| c.to_lowercase().to_string()).unwrap_or_default();
            out.push((cmd, format!("t{first}")));
        }
    }
    out.retain(|(c, _)| !c.starts_with("mode:") || cfg.modes.iter().any(|m| format!("mode:{}", m.id) == *c));
    out
}

fn add_command(b: &mut Builder, app: &AppHandle, cfg: &Config, cmd: &str, key: &str, arg: &str) {
    let mode_id = state::read(app, |rt| rt.store.mode.clone());
    let mode = cfg.mode(&mode_id);
    let spotify_on = !cfg.spotify.client_id.is_empty() && state::read(app, |rt| rt.store.spotify.is_some());
    let quiet_note = if mode.quiet { "• zwykłe powiadomienia czekają na paczkę" } else { "• powiadomienia od razu (tryb bez ciszy)" };
    let g = "Akcje";
    match cmd {
        "focus" => {
            let mins = arg.trim().parse::<i64>().ok().filter(|m| (5..=180).contains(m)).unwrap_or(cfg.focus.adhoc_minutes);
            let title = format!("Start fokusu · {mins} min");
            let mut eff = format!("• timer {mins}:00 w pasku i dymku\n{quiet_note}");
            if spotify_on && cfg.spotify.play_on_focus {
                eff.push_str(&format!("\n• Spotify: playlista trybu {}, {}%", mode.label, cfg.spotify.focus_volume));
            }
            b.add(g, "F", RED, title.clone(), "Własna sesja — wpisz liczbę minut, np. fk 45".into(), key,
                pv(title, "Zaczyna się własna sesja. Pasek pokaże pierścień czasu, a dymek przy prawej krawędzi odliczanie.", eff), &[], Cmd::Focus(mins));
        }
        "focus_block" => {
            let cur = engine::current(app);
            let (sub, body, eff) = match &cur {
                Some(c) => (
                    format!("{} · do {}", c.title, c.end.format("%H:%M")),
                    format!("Bieżący blok z Horologa: „{}”. Fokus potrwa do końca bloku.", c.title),
                    format!("• timer do {}\n{quiet_note}", c.end.format("%H:%M")),
                ),
                None => ("Teraz nic nie trwa".into(), "W planie nie ma teraz bloku — użyj fk, żeby zacząć własną sesję.".into(), "• nic się nie stanie".into()),
            };
            b.add(g, "F", RED, "Fokus na bieżący blok z Horologa".into(), sub, key, pv("Fokus na bieżący blok", body, eff), &[], Cmd::FocusBlock);
        }
        "pause" => {
            let paused = state::read(app, |rt| rt.paused_since.is_some());
            let t = if paused { "Wznów fokus" } else { "Pauza fokusu" };
            b.add(g, "P", GREY, t.into(), "Zatrzymuje licznik i Spotify".into(), key,
                pv(t, "Licznik sesji staje, koniec przesunie się o czas pauzy.", if spotify_on { "• timer: pauza / wznowienie\n• Spotify: pauza / wznowienie" } else { "• timer: pauza / wznowienie" }), &[], Cmd::Pause);
        }
        c if c.starts_with("mode:") => {
            let id = &c[5..];
            let Some(m) = cfg.modes.iter().find(|m| m.id == id) else { return };
            let active = mode_id == m.id;
            let dim: Vec<String> = m.dim.iter().filter_map(|d| cfg.dock.iter().find(|a| &a.id == d).map(|a| a.name.clone())).collect();
            let mut eff = Vec::new();
            if !m.wallpaper.trim().is_empty() {
                eff.push("• tapeta trybu".to_string());
            }
            if let Some(br) = m.brightness {
                eff.push(format!("• jasność {br}%"));
            }
            eff.push(if m.quiet { "• radio w ciszy: paczki o pełnych godzinach".into() } else { "• powiadomienia od razu (bez ciszy)".into() });
            if !dim.is_empty() {
                eff.push(format!("• wyciszone w docku: {}", dim.join(", ")));
            }
            if spotify_on && cfg.spotify.play_on_mode_change && !m.playlist.is_empty() {
                eff.push("• Spotify: playlista trybu".into());
            }
            let tile = match m.id.as_str() {
                "praca" => RED,
                "nauka" => BLUE,
                _ => PURPLE,
            };
            let mono = m.label.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            b.add("Tryby", &mono, tile, format!("Tryb {}", m.label), if active { "Aktywny teraz".into() } else { "Przełącz".into() }, key,
                pv(format!("Tryb {}", m.label), if active { "Ten tryb jest już włączony." } else { "Okna zostają na miejscu — zmienia się otoczenie: dock, radio, tapeta i muzyka." }, eff.join("\n")),
                &[], Cmd::Mode(m.id.clone()));
        }
        "capture" => {
            let text = arg.trim();
            let shown = if text.is_empty() { "wpisz treść po spacji, np. m oddzwonić do serwisu jutro rano".to_string() } else { text.to_string() };
            b.add("Akcja", "M", AMBER, "Wrzuć do Horologa".into(), shown, key,
                pv("Wrzuć myśl", if text.is_empty() { "Napisz myśl po „m ”. Horolog sam rozpozna termin i czas trwania.".to_string() } else { format!("Do inboxu Horologa trafi: „{text}”. Horolog sam rozpozna termin i czas trwania.") },
                    "• trafia do Horologa (bez sieci — wyśle później)\n• okno się zamyka, fokus nieprzerwany"),
                &[], Cmd::Capture(text.to_string()));
        }
        "stash" => {
            let n = platform::app_windows().len();
            b.add(g, "G", GREY, "Odłóż okna do garażu".into(), format!("{n} otwartych okien"), key,
                pv("Odłóż okna", "Wszystkie okna minimalizują się jako jeden kontekst. Wrócisz do nich jednym kliknięciem z lewego paska → Odłożone.", format!("• {n} okien → garaż\n• pulpit czysty")), &[], Cmd::Stash);
        }
        "spotify" | "spotify_next" => {
            let (playing, title, artist) = state::read(app, |rt| (rt.music.playing, rt.music.title.clone(), rt.music.artist.clone()));
            let now = if title.is_empty() { "nic nie gra".to_string() } else { format!("{title} — {artist}") };
            if cmd == "spotify" {
                b.add("Spotify", "S", GREEN, "Odtwarzacz na wierzchu".into(), now.clone(), key,
                    pv("Odtwarzacz na wierzchu", "Karta z okładką i kolorami albumu, zawsze nad oknami. Przeciągasz ją gdziekolwiek, podwójny klik zwija do paska.", "• pokazuje / chowa odtwarzacz"), &[], Cmd::Player);
                let t = if playing { "Spotify: pauza" } else { "Spotify: graj" };
                b.add("Spotify", "S", GREEN, t.into(), now.clone(), key, pv(t, if spotify_on { now } else { "Spotify nie jest połączone — połącz w ustawieniach.".into() }, ""), &[], Cmd::Spotify);
            } else {
                b.add("Spotify", "S", GREEN, "Spotify: następny utwór".into(), now.clone(), key, pv("Następny utwór", now, ""), &[], Cmd::SpotifyNext);
            }
        }
        "tailscale" => {
            let node = if arg.trim().is_empty() { cfg.palette.tailscale_node.clone() } else { arg.trim().to_string() };
            b.add("Tailscale", "T", BLUE, format!("Exit node: {node}"), "Cały ruch przez ten komputer".into(), key,
                pv(format!("Exit node: {node}"), "Przydatne w publicznym Wi‑Fi. Horolog i radio działają niezależnie od tej opcji.", format!("• tailscale set --exit-node={node}")), &[], Cmd::Tailscale(node));
        }
        "tailscale_off" => {
            b.add("Tailscale", "T", GREY, "Wyłącz exit node".into(), "Tylko tailnet (domyślnie)".into(), key,
                pv("Wyłącz exit node", "Ruch do internetu wraca na zwykłe łącze, tailnet działa dalej.", "• tailscale set --exit-node="), &[], Cmd::TailscaleOff);
        }
        "calc" => {
            let res = calc(arg);
            let title = res.clone().map(|r| format!("= {r}")).unwrap_or_else(|| "Kalkulator".into());
            b.add("Kalkulator", "=", GREY, title.clone(), if arg.trim().is_empty() { "wpisz działanie, np. = 1250 * 23%".into() } else { arg.trim().into() }, key,
                pv(title, "Działa: + − × ÷ ^ %, nawiasy, przecinek dziesiętny.", if res.is_some() { "• wynik trafia do schowka" } else { "" }), &[], Cmd::Calc(res.unwrap_or_default()));
        }
        "replan" => b.add(g, "R", AMBER, "Przeplanuj dzień".into(), "Horolog ułoży resztę dnia od nowa".into(), key,
            pv("Przeplanuj", "Horolog przesuwa niezrobione bloki na wolne miejsca.", "• POST /api/plan/solve\n• klocki w pasku się odświeżą"), &[], Cmd::Replan),
        "report" => b.add(g, "R", GREY, "Raport tygodnia".into(), "Sektory, fokus i najlepsze godziny".into(), key,
            pv("Raport tygodnia", "Otwiera kartę raportu tygodnia.", ""), &[], Cmd::Report),
        "settings" => b.add(g, "U", GREY, "Ustawienia Edge Shell".into(), "Dock, tryby, automatyzacje, skróty".into(), key,
            pv("Ustawienia", "Otwiera okno ustawień.", ""), &[], Cmd::Settings),
        "one_thing" => {
            let on = state::read(app, |rt| rt.one_thing);
            let t = if on { "Wyłącz „jedną rzecz”" } else { "Jedna rzecz" };
            b.add(g, "1", RED, t.into(), "Wszystko poza aktywnym oknem przygasa".into(), key,
                pv(t, "Reszta ekranu przygasa, dock się chowa, pasek pokazuje tylko zadanie i czas. Wyłączasz Ctrl+Alt+J albo przyciskiem w pasku.", "• warstwa pod aktywnym oknem
• kończy się sama z końcem fokusu"), &[], Cmd::OneThing);
        }
        "clipboard" => {
            let clips = state::read(app, |rt| rt.store.clips.clone());
            let needle = arg.trim().to_lowercase();
            let mut shown = 0;
            for c in clips.iter().filter(|c| needle.is_empty() || c.text.to_lowercase().contains(&needle)).take(12) {
                shown += 1;
                let one_line: String = c.text.split_whitespace().collect::<Vec<_>>().join(" ");
                let title: String = one_line.chars().take(70).collect();
                let sub = format!("{}{}", if c.pinned { "📌 przypięte · " } else { "" }, c.at.format("%d.%m %H:%M"));
                let preview_text: String = c.text.chars().take(1200).collect();
                b.add("Schowek", if c.pinned { "📌" } else { "T" }, GREY, title, sub, key,
                    pv("Ze schowka", preview_text, "• Enter: wkleja w okno, w którym byłeś
• Tab: tylko kopiuj, przypnij, usuń"),
                    &[("copy", "Tylko kopiuj"), ("pin", if c.pinned { "Odepnij" } else { "Przypnij" }), ("del", "Usuń z historii")], Cmd::Clip(c.text.clone()));
            }
            if shown == 0 {
                b.add("Schowek", "T", GREY, "Historia schowka jest pusta".into(), "Skopiuj coś (Ctrl+C), a pojawi się tutaj".into(), key, pv("Historia schowka", "Edge Shell pamięta ostatnie 150 wpisów i wszystkie przypięte. Hasła z menedżerów haseł są pomijane.", ""), &[], Cmd::Evening);
            }
        }
        "screenshot" => b.add(g, "Z", PURPLE, "Zrzut ekranu z adnotacją".into(), "Zaznacz obszar, dorysuj strzałkę".into(), key,
            pv("Zrzut ekranu", "Zaznaczasz obszar ekranu pod kursorem, dorysowujesz strzałkę, ramkę albo pisak. Enter zapisuje.", "• plik w Obrazy\\Zrzuty ekranu
• obraz w schowku
• plik na półce"), &[], Cmd::Screenshot),
        "layout_save" => {
            let name = arg.trim().to_string();
            let n = platform::app_windows().len();
            b.add("Okna", "U", BLUE, if name.is_empty() { "Zapisz układ okien".into() } else { format!("Zapisz układ „{name}”") },
                format!("{n} okien · {}", if name.is_empty() { "nazwa: tryb + monitory" } else { "własna nazwa" }), key,
                pv("Zapisz układ", "Zapamiętuje, gdzie stoi każde okno. Przywrócisz to „ul” albo automatyzacją (np. po podłączeniu monitora).", "• pozycje i rozmiary okien
• tryb i zestaw monitorów"), &[], Cmd::LayoutSave(name));
        }
        "layout_restore" => {
            let layouts = state::read(app, |rt| rt.store.layouts.clone());
            let needle = arg.trim().to_lowercase();
            for l in layouts.iter().filter(|l| needle.is_empty() || l.name.to_lowercase().contains(&needle)).take(8) {
                b.add("Okna", "U", BLUE, format!("Układ: {}", l.name), format!("{} okien", l.windows.len()), key,
                    pv(format!("Układ: {}", l.name), l.windows.iter().map(|w| w.exe.trim_end_matches(".exe").to_string()).collect::<Vec<_>>().join(", "), "• przesuwa otwarte okna
• uruchamia brakujące aplikacje"),
                    &[], Cmd::LayoutRestore(l.name.clone()));
            }
            if layouts.is_empty() {
                b.add("Okna", "U", BLUE, "Nie masz zapisanych układów".into(), "Ułóż okna i wpisz „uz”".into(), key, pv("Układy okien", "Najpierw zapisz układ poleceniem „uz”.", ""), &[], Cmd::LayoutSave(String::new()));
            }
        }
        "evening" => b.add(g, "K", AMBER, "Zamknięcie dnia".into(), "Co zrobione, co na jutro, jedno zdanie".into(), key,
            pv("Zamknięcie dnia", "Karta z dzisiejszymi sektorami i niedokończonymi zadaniami — każde jednym kliknięciem na jutro.", ""), &[], Cmd::Evening),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Zapytanie
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn palette_query(app: AppHandle, query: String) -> Answer {
    let cfg = state::cfg(&app);
    let q = query.trim().to_lowercase();
    let mut b = Builder { rows: Vec::new() };
    let keys = keys(&cfg);

    // Puste pole: podpowiedź — wszystkie szybkie klawisze.
    if q.is_empty() {
        for (cmd, key) in &keys {
            if !matches!(cmd.as_str(), "calc" | "capture" | "clipboard" | "layout_restore") {
                add_command(&mut b, &app, &cfg, cmd, key, "");
            }
        }
        return b.finish("Polecenia".into());
    }

    let (head, rest) = match query.trim().split_once(char::is_whitespace) {
        Some((h, r)) => (h.to_lowercase(), r.trim().to_string()),
        None => (q.clone(), String::new()),
    };

    // 1) Dokładny szybki klawisz (z argumentem albo bez).
    let mut chip = String::new();
    if let Some((cmd, key)) = keys.iter().find(|(_, k)| *k == head) {
        add_command(&mut b, &app, &cfg, cmd, key, &rest);
        chip = match cmd.as_str() {
            "focus" => format!("Fokus · {} min", rest.parse::<i64>().unwrap_or(cfg.focus.adhoc_minutes)),
            "capture" => "Myśl → Horolog".into(),
            "tailscale" | "tailscale_off" => "Tailscale".into(),
            "calc" => "Kalkulator".into(),
            c if c.starts_with("mode:") => "Tryb".into(),
            _ => "Polecenie".into(),
        };
        // Klawisz trybu: pokaż obok pozostałe tryby.
        if cmd.starts_with("mode:") {
            for (c, k) in keys.iter().filter(|(c, _)| c.starts_with("mode:") && c != cmd) {
                add_command(&mut b, &app, &cfg, c, k, "");
            }
        }
        if cmd == "capture" || cmd == "calc" || cmd == "clipboard" || cmd == "layout_restore" || cmd == "layout_save" || (cmd == "focus" && !rest.is_empty()) || cmd == "tailscale" {
            if cmd == "tailscale" {
                if let Some((c, k)) = keys.iter().find(|(c, _)| c == "tailscale_off") {
                    add_command(&mut b, &app, &cfg, c, k, "");
                }
            }
            return b.finish(chip);
        }
    } else if looks_like_math(&q) {
        let key = keys.iter().find(|(c, _)| c == "calc").map(|(_, k)| k.clone()).unwrap_or_else(|| "=".into());
        add_command(&mut b, &app, &cfg, "calc", &key, &q);
        return b.finish("Kalkulator".into());
    } else {
        // 2) Polecenia pasujące po klawiszu albo nazwie.
        for (cmd, key) in &keys {
            let label = cmd_label(&cfg, cmd).to_lowercase();
            if key.starts_with(&head) || score(&label, &q, &[]) >= 25 {
                if !matches!(cmd.as_str(), "calc" | "capture" | "clipboard" | "layout_restore") {
                    add_command(&mut b, &app, &cfg, cmd, key, "");
                }
            }
        }
    }

    // 3) Wyszukiwanie.
    let words: Vec<&str> = q.split_whitespace().collect();
    if q.chars().count() >= 2 {
        // Otwarte okna.
        let mut wins: Vec<(u32, platform::AppWindow)> = platform::app_windows()
            .into_iter()
            .filter_map(|w| {
                let s = score(&w.title.to_lowercase(), &q, &words).max(score(&platform::exe_name(&w.exe).trim_end_matches(".exe").to_string(), &q, &words));
                (s > 0).then_some((s, w))
            })
            .collect();
        wins.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, w) in wins.into_iter().take(4) {
            let exe = platform::exe_name(&w.exe);
            let app_name = exe.trim_end_matches(".exe").to_string();
            let mono = app_name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            b.add("Otwarte okna", &mono, GREY, w.title.clone(), "Przełącz do okna".into(), "",
                pv(w.title.clone(), "Okno jest już otwarte, więc Enter przełącza do niego zamiast otwierać drugi raz.", "• aktywuje okno\n• Tab: zminimalizuj, zamknij"),
                &[("min", "Zminimalizuj"), ("close", "Zamknij okno")], Cmd::Window(w.hwnd));
        }

        // Aplikacje: dock + menu Start.
        refresh_index(&app, false);
        let idx = INDEX.lock().ok().and_then(|g| g.as_ref().map(|i| (i.files.clone(), i.apps.clone())));
        let mut app_rows = 0;
        for a in &cfg.dock {
            if score(&a.name.to_lowercase(), &q, &words) > 0 && app_rows < 4 {
                app_rows += 1;
                let mono = a.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
                b.add("Aplikacje", &mono, BLUE, a.name.clone(), "Z docka".into(), "",
                    pv(a.name.clone(), "Uruchamia aplikację — a jeśli już działa, przełącza do jej okna.", "• Tab: nowe okno"),
                    &[("new", "Nowe okno")], Cmd::DockApp(a.id.clone()));
            }
        }
        if let Some((files, apps)) = &idx {
            let mut found: Vec<(u32, &AppEntry)> = apps.iter().filter_map(|a| {
                let s = score(&a.lower, &q, &words);
                (s > 0 && !cfg.dock.iter().any(|d| d.name.eq_ignore_ascii_case(&a.name))).then_some((s, a))
            }).collect();
            found.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.name.len().cmp(&y.1.name.len())));
            for (_, a) in found.into_iter().take(4 - app_rows.min(4)) {
                let mono = a.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
                b.add("Aplikacje", &mono, GREY, a.name.clone(), "Menu Start".into(), "",
                    pv(a.name.clone(), "Aplikacja z menu Start.", "• uruchamia aplikację\n• Tab: pokaż skrót w folderze"),
                    &[("folder", "Pokaż w folderze")], Cmd::StartApp(a.path.clone()));
            }

            // Pliki: najlepsze dopasowanie, potem najnowsze.
            let mut hits: Vec<(u32, &FileEntry)> = files.iter().filter_map(|f| {
                let s = score(&f.lower, &q, &words);
                (s > 0).then_some((s, f))
            }).collect();
            hits.sort_by(|x, y| y.0.cmp(&x.0).then(y.1.modified.cmp(&x.1.modified)));
            for (_, f) in hits.into_iter().take(6) {
                let (mono, tile) = file_mono(&f.name, f.dir);
                let sub = format!("{} · {}", f.place, ago(f.modified));
                let (body, eff) = if f.dir {
                    (format!("Folder w: {}", f.path), "• otwiera folder\n• Tab: kopiuj ścieżkę, na półkę")
                } else {
                    (format!("{}\nZmieniony: {}", f.path, ago(f.modified)), "• otwiera plik w domyślnej aplikacji\n• Tab: otwórz folder, kopiuj ścieżkę, na półkę")
                };
                b.add("Pliki", mono, tile, f.name.clone(), sub, "", pv(f.name.clone(), body, eff),
                    &[("look", "Podgląd (Shift+Spacja)"), ("folder", "Pokaż w folderze"), ("copy", "Kopiuj ścieżkę"), ("shelf", "Połóż na półce")], Cmd::File(f.path.clone()));
            }
        }

        // Horolog: plan dziś i tydzień.
        let (plan, week) = state::read(&app, |rt| (rt.plan.clone(), rt.week.clone()));
        let mut items: Vec<DayItem> = plan;
        for w in week {
            if !items.iter().any(|i| i.key == w.key) {
                items.push(w);
            }
        }
        let now = Local::now();
        let mut hits: Vec<(u32, DayItem)> = items.into_iter().filter(|i| i.kind != "buffer").filter_map(|i| {
            let s = score(&i.title.to_lowercase(), &q, &words);
            (s > 0 && i.end > now - chrono::Duration::hours(12)).then_some((s, i))
        }).collect();
        hits.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.start.cmp(&y.1.start)));
        for (_, i) in hits.into_iter().take(3) {
            let when = when(&i.start, &i.end, now);
            let kind = kind_label(&i.kind);
            let minutes = (i.end - now.max(i.start)).num_minutes().max(5);
            let mut acts = vec![("focus", "Fokus na ten blok"), ("web", "Otwórz w Horologu")];
            if i.kind == "task" && i.intent_id.is_some() {
                acts.insert(1, ("done", "Oznacz jako zrobione"));
            }
            b.add("Horolog", "H", AMBER, i.title.clone(), format!("{kind} · {when}"), "",
                pv(i.title.clone(), format!("{kind} · {when}"), "• Enter: fokus na ten blok\n• Tab: zrobione, otwórz w Horologu"),
                &acts, Cmd::Plan { title: i.title.clone(), minutes, intent: i.intent_id.clone(), kind: i.kind.clone() });
        }
    }

    // 4) Zawsze na końcu: wrzuć wpisany tekst do Horologa.
    if q.chars().count() >= 3 {
        let key = keys.iter().find(|(c, _)| c == "capture").map(|(_, k)| k.clone()).unwrap_or_default();
        let text = query.trim().to_string();
        b.add("Inne", "M", AMBER, "Wrzuć do Horologa".into(), text.clone(), &key,
            pv("Wrzuć myśl", format!("Do inboxu Horologa trafi: „{text}”."), "• trafia do Horologa\n• okno się zamyka"), &[], Cmd::Capture(text));
    }

    if chip.is_empty() {
        chip = match b.rows.first().map(|r| r.0.as_str()) {
            Some("Tryby") => "Tryb".into(),
            Some("Akcje") => "Polecenie".into(),
            Some("Inne") => "Myśl → Horolog".into(),
            Some(_) => "Szukaj".into(),
            None => String::new(),
        };
    }
    b.finish(chip)
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "task" => "Zadanie",
        "habit" => "Nawyk",
        "focus" => "Fokus",
        "meeting" => "Spotkanie",
        "busy" => "Zajęte",
        _ => "Blok",
    }
}

fn when(start: &DateTime<Local>, end: &DateTime<Local>, now: DateTime<Local>) -> String {
    let day = if start.date_naive() == now.date_naive() {
        if *start <= now && now < *end {
            return format!("teraz · {}–{}", start.format("%H:%M"), end.format("%H:%M"));
        }
        "dziś".to_string()
    } else if start.date_naive() == now.date_naive() + chrono::Duration::days(1) {
        "jutro".to_string()
    } else {
        start.format("%d.%m").to_string()
    };
    format!("{day} {}–{}", start.format("%H:%M"), end.format("%H:%M"))
}

// ---------------------------------------------------------------------------
// Wykonanie
// ---------------------------------------------------------------------------

fn tailscale(arg: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let candidates = ["tailscale", "C:\\Program Files\\Tailscale\\tailscale.exe"];
        let mut last = String::new();
        for exe in candidates {
            match std::process::Command::new(exe).args(["set", &format!("--exit-node={arg}")]).creation_flags(0x0800_0000).output() {
                Ok(o) if o.status.success() => return Ok(()),
                Ok(o) => {
                    last = String::from_utf8_lossy(&o.stderr).trim().to_string();
                    break;
                }
                Err(e) => last = e.to_string(),
            }
        }
        Err(last)
    }
    #[cfg(not(windows))]
    {
        let _ = arg;
        Err("tylko Windows".into())
    }
}

#[tauri::command]
pub async fn palette_run(app: AppHandle, id: String, action: Option<String>) -> Result<(), String> {
    let cmd = LAST.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&id).cloned())).ok_or("Wynik nieaktualny")?;
    layout::hide_window(&app, "palette");
    let action = action.unwrap_or_default();
    use tauri_plugin_opener::OpenerExt;
    match cmd {
        Cmd::Focus(m) => commands::focus_start_adhoc(app.clone(), Some(m), None),
        Cmd::FocusBlock => {
            if let Some(c) = engine::current(&app) {
                let mins = (c.end - Local::now()).num_minutes().max(5);
                commands::focus_start_adhoc(app.clone(), Some(mins), Some(c.title));
            }
        }
        Cmd::Pause => {
            commands::focus_toggle_pause(app.clone()).await?;
        }
        Cmd::Mode(m) => commands::set_mode(app.clone(), m).await?,
        Cmd::Capture(t) => {
            if t.trim().is_empty() {
                layout::show_window(&app, "capture", true);
            } else {
                let msg = horolog::capture(&app, t).await?;
                if msg.starts_with("Brak") {
                    radio::pit_wall(&app, msg, None);
                }
            }
        }
        Cmd::Stash => {
            commands::stash_create(app.clone(), String::new())?;
        }
        Cmd::Spotify => {
            let playing = state::read(&app, |rt| rt.music.playing);
            spotify::control(&app, if playing { "pause" } else { "play" }, None).await?;
        }
        Cmd::SpotifyNext => spotify::control(&app, "next", None).await?,
        Cmd::Player => crate::player::show(&app),
        Cmd::Tailscale(node) => {
            let r = tauri::async_runtime::spawn_blocking(move || tailscale(&node)).await.map_err(|e| e.to_string())?;
            if let Err(e) = r {
                radio::pit_wall(&app, format!("Tailscale: {e}"), None);
            }
        }
        Cmd::TailscaleOff => {
            let r = tauri::async_runtime::spawn_blocking(|| tailscale("")).await.map_err(|e| e.to_string())?;
            if let Err(e) = r {
                radio::pit_wall(&app, format!("Tailscale: {e}"), None);
            }
        }
        Cmd::Calc(r) => {
            if !r.is_empty() {
                commands::copy_text(app.clone(), r)?;
            }
        }
        Cmd::Replan => commands::replan(app.clone()).await?,
        Cmd::Report => insights::open_report(app.clone()),
        Cmd::Settings => settings::open_settings(app.clone()),
        Cmd::OneThing => crate::focus::one_thing_toggle(app.clone()),
        Cmd::Screenshot => {
            // Chwila na zniknięcie palety, żeby nie trafiła na zrzut.
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            crate::shot::start(&app);
        }
        Cmd::Evening => crate::insights::open_evening(app.clone()),
        Cmd::LayoutSave(name) => {
            let n = crate::windows_mgr::save_layout(&app, &name);
            radio::pit_wall(&app, format!("Zapisano układ: {n} okien."), None);
        }
        Cmd::LayoutRestore(name) => {
            crate::windows_mgr::restore_layout(&app, &name)?;
        }
        Cmd::Clip(text) => match action.as_str() {
            "pin" => state::update(&app, true, |rt| {
                if let Some(c) = rt.store.clips.iter_mut().find(|c| c.text == text) {
                    c.pinned = !c.pinned;
                }
            }),
            "del" => state::update(&app, true, |rt| rt.store.clips.retain(|c| c.text != text)),
            "copy" => commands::copy_text(app.clone(), text)?,
            _ => {
                commands::copy_text(app.clone(), text)?;
                // Paleta zniknęła — fokus wraca do poprzedniego okna; wklejamy.
                tokio::time::sleep(std::time::Duration::from_millis(180)).await;
                platform::send_paste();
            }
        },
        Cmd::Window(h) => match action.as_str() {
            "close" => crate::sys::close_window(h),
            "min" => platform::minimize(h),
            _ => crate::sys::activate(h),
        },
        Cmd::File(p) => match action.as_str() {
            "look" => crate::shot::quicklook(app.clone(), p).await?,
            "folder" => commands::reveal(app.clone(), p)?,
            "copy" => commands::copy_text(app.clone(), p)?,
            "shelf" => commands::shelf_add(app.clone(), vec![p]),
            _ => commands::open_path(app.clone(), p)?,
        },
        Cmd::DockApp(id) => {
            if action == "new" {
                commands::launch_new(app.clone(), id)?;
            } else {
                commands::launch(app.clone(), id)?;
            }
        }
        Cmd::StartApp(p) => {
            if action == "folder" {
                commands::reveal(app.clone(), p)?;
            } else {
                app.opener().open_path(p, None::<&str>).map_err(|e| e.to_string())?;
            }
        }
        Cmd::Plan { title, minutes, intent, kind } => match action.as_str() {
            "done" => {
                if let Some(id) = intent {
                    horolog::complete(&app, &id).await?;
                    radio::pit_wall(&app, format!("Odhaczone: {title}"), None);
                }
            }
            "web" => commands::open_horolog(app.clone())?,
            _ => {
                let _ = kind;
                commands::focus_start_adhoc(app.clone(), Some(minutes.min(180)), Some(title));
            }
        },
    }
    Ok(())
}

/// Pokazuje paletę (skrót, lupa na lewym pasku).
pub fn show(app: &AppHandle) {
    refresh_index(app, false);
    if let Some(w) = app.get_webview_window("palette") {
        let s = layout::screen(app);
        let (pw, ph) = ((780.0 * s.scale) as i32, (600.0 * s.scale) as i32);
        let _ = w.set_size(tauri::PhysicalSize::new(pw as u32, ph as u32));
        let _ = w.set_position(tauri::PhysicalPosition::new((s.work.left + s.work.right - pw) / 2, s.work.top + (90.0 * s.scale) as i32));
        let _ = w.show();
        let _ = w.set_focus();
        use tauri::Emitter;
        let _ = app.emit_to("palette", "palette-open", ());
    }
}

#[tauri::command]
pub fn palette_show(app: AppHandle) {
    show(&app);
}

#[tauri::command]
pub fn palette_hide(app: AppHandle) {
    layout::hide_window(&app, "palette");
}

/// Lista szybkich klawiszy (do ustawień i podpowiedzi).
#[tauri::command]
pub fn palette_keys(app: AppHandle) -> Vec<serde_json::Value> {
    let cfg = state::cfg(&app);
    keys(&cfg).into_iter().map(|(c, k)| serde_json::json!({ "cmd": c, "key": k, "label": cmd_label(&cfg, &c) })).collect()
}

/// Dostępne polecenia (do edycji klawiszy w ustawieniach).
#[tauri::command]
pub fn palette_commands(app: AppHandle) -> Vec<serde_json::Value> {
    let cfg = state::cfg(&app);
    let mut out: Vec<serde_json::Value> = CMD_NAMES.iter().map(|(c, l)| serde_json::json!({ "cmd": c, "label": l })).collect();
    for m in &cfg.modes {
        out.push(serde_json::json!({ "cmd": format!("mode:{}", m.id), "label": format!("tryb {}", m.label) }));
    }
    out
}

pub fn init(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("palette") {
        let h = app.clone();
        w.on_window_event(move |e| {
            if let WindowEvent::Focused(false) = e {
                layout::hide_window(&h, "palette");
            }
        });
    }
    refresh_index(app, true);
}
