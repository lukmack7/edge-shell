//! Konfiguracja użytkownika: `%APPDATA%\EdgeShell\config.json`.
//!
//! Plik powstaje przy pierwszym uruchomieniu z wartościami domyślnymi. Każde
//! pole ma wartość domyślną, więc stary plik po aktualizacji aplikacji dalej się
//! wczytuje, a brakujące klucze dostają ustawienia fabryczne.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("EdgeShell");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub horolog: HorologCfg,
    pub start_mode: String,
    pub modes: Vec<Mode>,
    pub dock: Vec<DockApp>,
    pub pinned: Vec<Pinned>,
    pub contacts: Vec<Contact>,
    pub hotkeys: Hotkeys,
    pub radio: RadioCfg,
    pub spotify: SpotifyCfg,
    pub shell: ShellCfg,
    pub focus: FocusCfg,
    pub notifications: NotificationsCfg,
    /// Sekcje docka w kolejności; separator wstawiany między sekcjami.
    pub sections: Vec<Section>,
    pub bar: BarCfg,
    pub backup: BackupCfg,
    pub palette: PaletteCfg,
    pub automations: Vec<Rule>,
    pub presence: PresenceCfg,
    pub windows: WindowsCfg,
}

/// Obecność („body doubling”) przez serwer na home serwerze.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PresenceCfg {
    pub enabled: bool,
    /// np. http://homeserver.local:7320
    pub url: String,
    /// Jak widzą Cię inni (pusty = nazwa użytkownika Windows).
    pub name: String,
    pub token: String,
    /// Pokazuj innym tytuł bieżącego zadania (inaczej tylko „fokus”).
    pub share_task: bool,
}

impl Default for PresenceCfg {
    fn default() -> Self {
        Self { enabled: false, url: String::new(), name: String::new(), token: String::new(), share_task: false }
    }
}

/// Okna: strefy przyciągania i układy.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WindowsCfg {
    /// Układ stref: thirds | wide-center | halves | wide-left | quarters
    pub zones: String,
    /// Shift podczas przeciągania okna pokazuje strefy.
    pub shift_drag: bool,
    /// Ctrl+Alt+1…6 wkłada aktywne okno do strefy.
    pub zone_hotkeys: bool,
    /// Odstęp między oknami w strefach (px).
    pub gap: i32,
}

impl Default for WindowsCfg {
    fn default() -> Self {
        Self { zones: "thirds".into(), shift_drag: true, zone_hotkeys: true, gap: 8 }
    }
}

// ---------------------------------------------------------------------------
// Paleta poleceń
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PaletteCfg {
    /// Szybkie klawisze: polecenie → skrót (2–3 litery).
    pub keys: Vec<QuickKey>,
    /// Exit node Tailscale dla polecenia `ts` (nazwa maszyny w tailnecie).
    pub tailscale_node: String,
    /// Dodatkowe foldery przeszukiwane przez paletę (oprócz Pulpitu,
    /// Dokumentów, Pobranych i przypiętych folderów).
    pub folders: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QuickKey {
    /// focus | focus_block | pause | mode:<id> | capture | stash | spotify |
    /// spotify_next | tailscale | tailscale_off | calc | replan | report | settings
    pub cmd: String,
    pub key: String,
}

impl Default for PaletteCfg {
    fn default() -> Self {
        let k = |cmd: &str, key: &str| QuickKey { cmd: cmd.into(), key: key.into() };
        Self {
            keys: vec![
                k("focus", "fk"),
                k("focus_block", "fb"),
                k("pause", "fp"),
                k("mode:praca", "tp"),
                k("mode:nauka", "tn"),
                k("mode:rozrywka", "tr"),
                k("capture", "m"),
                k("stash", "og"),
                k("spotify", "sp"),
                k("spotify_next", "sn"),
                k("tailscale", "ts"),
                k("tailscale_off", "tx"),
                k("calc", "="),
                k("replan", "pl"),
                k("report", "rap"),
                k("settings", "ust"),
                k("one_thing", "jr"),
                k("clipboard", "cb"),
                k("screenshot", "zr"),
                k("layout_save", "uz"),
                k("layout_restore", "ul"),
                k("evening", "kd"),
            ],
            tailscale_node: "homeserver".into(),
            folders: vec![],
        }
    }
}

// ---------------------------------------------------------------------------
// Automatyzacje: „Gdy → to”
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub enabled: bool,
    pub trigger: Trigger,
    pub actions: Vec<RuleAction>,
    /// Najpierw zapytaj kartą radia („Tak / Nie”).
    pub ask: bool,
    /// Nie częściej niż co tyle minut (0 = bez limitu).
    pub cooldown_min: u32,
    /// Gdy warunek minie (spotkanie się skończy, gra zamknie…), cofnij akcje.
    pub revert: bool,
}

impl Default for Rule {
    fn default() -> Self {
        Self { id: String::new(), enabled: true, trigger: Trigger::default(), actions: vec![], ask: false, cooldown_min: 30, revert: false }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Trigger {
    /// time | monitor | wifi | folder | horolog | app | battery | idle
    pub kind: String,
    /// Czas: `18:00` i dni tygodnia 1–7 (pon–nd).
    pub time: String,
    pub days: Vec<u32>,
    /// Monitor: fragment nazwy (pusty = dowolny dodatkowy monitor).
    pub monitor: String,
    /// Wi‑Fi: nazwa sieci.
    pub ssid: String,
    /// Folder: ścieżka i wzorzec pliku, np. `*.pdf`.
    pub folder: String,
    pub pattern: String,
    /// Horolog: meeting | focus | task | habit | any i ile minut wcześniej.
    pub horolog_kind: String,
    pub minutes_before: i64,
    /// Aplikacja: proces (np. `steam.exe`) albo pusty + `fullscreen`.
    pub app: String,
    pub fullscreen: bool,
    /// Bateria poniżej (%).
    pub battery_below: u8,
    /// Bezczynność dłuższa niż (min).
    pub idle_minutes: u32,
}

impl Default for Trigger {
    fn default() -> Self {
        Self {
            kind: "time".into(),
            time: "18:00".into(),
            days: vec![1, 2, 3, 4, 5],
            monitor: String::new(),
            ssid: String::new(),
            folder: "%USERPROFILE%\\Downloads".into(),
            pattern: "*".into(),
            horolog_kind: "meeting".into(),
            minutes_before: 1,
            app: String::new(),
            fullscreen: false,
            battery_below: 20,
            idle_minutes: 10,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RuleAction {
    /// mode | spotify | quiet | message | shelf | minimize | close | launch |
    /// brightness | volume | focus | hide_shell | command
    pub kind: String,
    pub value: String,
}

pub fn default_rules() -> Vec<Rule> {
    let a = |kind: &str, value: &str| RuleAction { kind: kind.into(), value: value.into() };
    let t = |kind: &str| Trigger { kind: kind.into(), ..Trigger::default() };
    vec![
        Rule { id: "monitor-praca".into(), enabled: false, trigger: t("monitor"), actions: vec![a("mode", "praca")], ..Rule::default() },
        Rule {
            id: "wieczor".into(),
            enabled: false,
            trigger: Trigger { time: "18:00".into(), ..t("time") },
            actions: vec![a("mode", "rozrywka"), a("spotify", "play")],
            ask: true,
            ..Rule::default()
        },
        Rule {
            id: "wifi-biuro".into(),
            enabled: false,
            trigger: Trigger { ssid: "Biuro".into(), ..t("wifi") },
            actions: vec![a("mode", "praca"), a("quiet", "12:00")],
            ..Rule::default()
        },
        Rule {
            id: "pdf-polka".into(),
            enabled: false,
            trigger: Trigger { pattern: "*.pdf".into(), ..t("folder") },
            actions: vec![a("shelf", ""), a("message", "Nowy PDF na półce")],
            cooldown_min: 0,
            ..Rule::default()
        },
        Rule {
            id: "spotkanie".into(),
            enabled: false,
            trigger: t("horolog"),
            actions: vec![a("spotify", "pause"), a("minimize", "Discord.exe")],
            revert: true,
            cooldown_min: 0,
            ..Rule::default()
        },
        Rule {
            id: "bateria".into(),
            enabled: false,
            trigger: t("battery"),
            actions: vec![a("brightness", "50")],
            ..Rule::default()
        },
        Rule {
            id: "gra".into(),
            enabled: false,
            trigger: Trigger { fullscreen: true, ..t("app") },
            actions: vec![a("quiet", "")],
            revert: true,
            cooldown_min: 0,
            ..Rule::default()
        },
    ]
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BackupCfg {
    /// Folder na automatyczną kopię ustawień (np. udział na home serwerze).
    /// Pusty = tylko kopie lokalne.
    pub folder: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Section {
    pub id: String,
    pub name: String,
    pub color: String,
}

impl Default for Section {
    fn default() -> Self {
        Self { id: String::new(), name: String::new(), color: "#a1a1aa".into() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BarCfg {
    /// Które statystyki pokazać w pasku: network, cpu, memory, volume,
    /// bluetooth, wifi, battery.
    pub stats: Vec<String>,
}

impl Default for BarCfg {
    fn default() -> Self {
        Self {
            stats: ["network", "cpu", "memory", "volume", "bluetooth", "wifi", "battery"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

pub fn default_sections() -> Vec<Section> {
    let s = |id: &str, name: &str, color: &str| Section { id: id.into(), name: name.into(), color: color.into() };
    vec![
        s("praca", "Praca", "#60a5fa"),
        s("projekty", "Projekty", "#22c55e"),
        s("komunikacja", "Komunikacja", "#eab308"),
        s("tworcze", "Twórcze i rozrywka", "#c084fc"),
        s("system", "System", "#a1a1aa"),
    ]
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotificationsCfg {
    /// Przechwytuj powiadomienia wszystkich aplikacji Windows do radia.
    pub enabled: bool,
    /// Reguły dla aplikacji: wyciszenie, pilność, kolor, nazwa.
    pub apps: Vec<NotifApp>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotifApp {
    /// Fragment identyfikatora aplikacji (AUMID) albo jej nazwa, np. `Teams`.
    pub matches: String,
    pub name: String,
    pub color: String,
    /// Zawsze od razu, nawet w trybie cichym.
    pub urgent: bool,
    /// Pomijaj całkowicie.
    pub ignore: bool,
}

impl Default for NotificationsCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            apps: vec![
                NotifApp { matches: "Spotify".into(), ignore: true, ..NotifApp::default() },
                NotifApp { matches: "Teams".into(), color: "#a5b4fc".into(), ..NotifApp::default() },
                NotifApp { matches: "WhatsApp".into(), color: "#4ade80".into(), ..NotifApp::default() },
            ],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HorologCfg {
    /// Adres API Horologa, np. `http://homeserver.local:8000`.
    pub url: String,
    /// Adres interfejsu WWW (otwierany z docka), np. `http://homeserver.local:3000`.
    pub web_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Mode {
    pub id: String,
    pub label: String,
    pub accent: String,
    /// Identyfikatory aplikacji z docka, które w tym trybie są wyciszone.
    pub dim: Vec<String>,
    /// Czy zwykłe powiadomienia czekają na paczkę.
    pub quiet: bool,
    /// Playlista Spotify (URI `spotify:playlist:...`) dla tego trybu.
    pub playlist: String,
    /// Tapeta dla trybu (pusta = bez zmiany).
    pub wallpaper: String,
    /// Jasność ekranów 0–100 (None = bez zmiany).
    pub brightness: Option<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DockApp {
    pub id: String,
    pub name: String,
    /// Ścieżka do .exe, nazwa z App Paths (`excel.exe`), URI (`spotify:`) albo URL.
    pub path: String,
    pub args: Vec<String>,
    /// praca | komunikacja | tworcze | projekty | system
    pub category: String,
    /// Nazwa wbudowanej ikony (folder, globe, sheet, terminal, mail, clock, chat,
    /// note, music, video, game, code, image, calc, settings) albo 1–2 litery.
    pub icon: String,
    /// Nazwa procesu do kropki „uruchomione”, np. `chrome.exe`. Puste = bez kropki.
    pub process: String,
    /// Plik, z którego wziąć ikonę, gdy `icon` = `auto` (puste = z `path`).
    pub icon_path: String,
    /// Wstaw separator za tą ikoną.
    pub separator_after: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Pinned {
    pub name: String,
    pub path: String,
    pub category: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Contact {
    /// Nazwa na karcie radia, np. `KASIA`.
    pub name: String,
    /// Numer startowy. Numery 1–9 przechodzą przez ciszę od razu.
    pub number: String,
    pub color: String,
    /// Nadawcy (z pola `from`), których dopasować do tego kontaktu.
    pub matches: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Hotkeys {
    pub one_thing: String,
    pub screenshot: String,
    pub player: String,
    pub drawer: String,
    pub now: String,
    pub capture: String,
    pub focus: String,
    pub search: String,
    pub radio: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RadioCfg {
    /// Port lokalnego endpointu `POST /radio` (tylko 127.0.0.1).
    pub port: u16,
    /// Opcjonalny token: nagłówek `Authorization: Bearer <token>`.
    pub token: String,
    /// Godziny dostarczania paczek, np. ["10:00", "12:00", "15:00", "17:00"].
    pub batch_times: Vec<String>,
    /// Cisza nocna: między tymi godzinami przechodzą tylko kontakty z numerem 1–9.
    pub night_quiet: bool,
    pub night_start: String,
    pub night_end: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SpotifyCfg {
    /// Client ID aplikacji z developer.spotify.com (pusty = Spotify wyłączone).
    pub client_id: String,
    pub redirect_port: u16,
    pub focus_volume: u8,
    pub play_on_focus: bool,
    pub pause_on_buffer: bool,
    pub pause_before_meeting: bool,
    pub play_on_mode_change: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShellCfg {
    /// Rezerwuj miejsce na górny pasek (zmaksymalizowane okna go nie zasłonią).
    pub reserve_top_bar: bool,
    /// Ukryj pasek zadań Windows (eksperymentalne; przywracany przy wyjściu).
    pub hide_taskbar: bool,
    pub show_dock: bool,
    pub show_bubbles: bool,
    /// Ile ms kursor musi stać przy krawędzi, żeby wysunąć panel.
    pub edge_delay_ms: u64,
    pub edge_glow_minutes: i64,
    /// Uruchamiaj Edge Shell razem z Windows.
    pub autostart: bool,
    /// Stały pasek przy lewej krawędzi (rezerwuje miejsce jak górny pasek).
    pub show_rail: bool,
    /// Ikony zasobnika systemowego na lewym pasku.
    pub tray_in_rail: bool,
    /// Pasek na każdym monitorze (dock i panele zostają na głównym).
    pub bar_on_all_monitors: bool,
    /// Dock chowa się i wysuwa po dotknięciu dolnej krawędzi.
    pub dock_autohide: bool,
    /// Gra, film albo prezentacja na pełnym ekranie chowa pasek, dock i dymki.
    pub hide_in_fullscreen: bool,
    /// Notch na środku górnego paska: plan dnia, muzyka, radio i panel w jednym miejscu.
    pub notch: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FocusCfg {
    /// Karta „wracamy?”, gdy w trakcie fokusu siedzisz w rozpraszaczu.
    pub distraction_alert: bool,
    pub distraction_minutes: u64,
    /// Fragmenty nazw procesów albo tytułów okien (np. karta YouTube w przeglądarce).
    pub distractions: Vec<String>,
    /// Kliknięcie „Jadę” na starcie bloku włącza tryb „jedna rzecz”.
    pub one_thing_on_start: bool,
    pub adhoc_minutes: i64,
    pub weekly_goal_hours: f64,
    /// Dzień raportu tygodnia: 1 = poniedziałek … 7 = niedziela (0 = wyłączony).
    pub report_weekday: u32,
    pub report_time: String,
    /// Wieczorne zamknięcie dnia: co zrobione, co na jutro, jedno zdanie.
    pub evening_review: bool,
    pub evening_time: String,
}

impl Default for HorologCfg {
    fn default() -> Self {
        Self {
            url: "http://homeserver.local:8000".into(),
            web_url: "http://homeserver.local:3000".into(),
        }
    }
}

impl Default for Mode {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            accent: "#D42E25".into(),
            dim: vec![],
            quiet: true,
            playlist: String::new(),
            wallpaper: String::new(),
            brightness: None,
        }
    }
}

impl DockApp {
    /// Nazwy plików .exe tej aplikacji (małymi literami): z pola „proces”,
    /// ze ścieżki .exe albo z celu skrótu .lnk — do kropek, podglądów i
    /// odróżnienia od otwartych aplikacji spoza docka.
    pub fn exe_names(&self) -> Vec<String> {
        let mut v = Vec::new();
        let name = |p: &str| p.rsplit(['\\', '/']).next().unwrap_or(p).to_lowercase();
        if !self.process.trim().is_empty() {
            v.push(self.process.trim().to_lowercase());
        }
        let path = expand_env(self.path.trim());
        let lower = path.to_lowercase();
        if lower.ends_with(".exe") {
            v.push(name(&path));
        } else if lower.ends_with(".lnk") {
            if let Some(t) = crate::sys::shortcut_target(&path) {
                if t.to_lowercase().ends_with(".exe") {
                    v.push(name(&t));
                }
            }
        }
        v.dedup();
        v
    }
}

impl Default for DockApp {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            path: String::new(),
            args: vec![],
            category: "system".into(),
            icon: "auto".into(),
            process: String::new(),
            icon_path: String::new(),
            separator_after: false,
        }
    }
}

impl Default for Pinned {
    fn default() -> Self {
        Self { name: String::new(), path: String::new(), category: "praca".into() }
    }
}

impl Default for Contact {
    fn default() -> Self {
        Self { name: String::new(), number: String::new(), color: "#fbbf24".into(), matches: vec![] }
    }
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            one_thing: "Ctrl+Alt+J".into(),
            screenshot: "Ctrl+Alt+S".into(),
            player: "Ctrl+Alt+M".into(),
            drawer: "Ctrl+Alt+ArrowLeft".into(),
            now: "Ctrl+Alt+ArrowRight".into(),
            capture: "Ctrl+Alt+K".into(),
            focus: "Ctrl+Alt+F".into(),
            search: "Ctrl+Alt+Space".into(),
            radio: "Ctrl+Alt+R".into(),
        }
    }
}

impl Default for RadioCfg {
    fn default() -> Self {
        Self {
            port: 7311,
            token: String::new(),
            batch_times: vec!["10:00".into(), "12:00".into(), "15:00".into(), "17:00".into()],
            night_quiet: true,
            night_start: "22:00".into(),
            night_end: "07:00".into(),
        }
    }
}

impl Default for SpotifyCfg {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            redirect_port: 7312,
            focus_volume: 30,
            play_on_focus: true,
            pause_on_buffer: true,
            pause_before_meeting: true,
            play_on_mode_change: true,
        }
    }
}

impl Default for ShellCfg {
    fn default() -> Self {
        Self {
            reserve_top_bar: true,
            hide_taskbar: true,
            show_dock: true,
            show_bubbles: true,
            edge_delay_ms: 150,
            edge_glow_minutes: 15,
            autostart: true,
            show_rail: true,
            tray_in_rail: true,
            bar_on_all_monitors: true,
            dock_autohide: true,
            hide_in_fullscreen: true,
            notch: true,
        }
    }
}

impl Default for FocusCfg {
    fn default() -> Self {
        Self {
            distraction_alert: true,
            distraction_minutes: 3,
            distractions: ["YouTube", "Netflix", "Reddit", "Facebook", "Instagram", "TikTok", "Twitch", "discord.exe", "steam.exe"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            one_thing_on_start: false,
            adhoc_minutes: 25,
            weekly_goal_hours: 15.0,
            report_weekday: 5,
            report_time: "16:00".into(),
            evening_review: true,
            evening_time: "17:30".into(),
        }
    }
}

fn app(id: &str, name: &str, path: &str, category: &str, icon: &str, process: &str) -> DockApp {
    let real = !path.starts_with("http");
    DockApp {
        id: id.into(),
        name: name.into(),
        path: path.into(),
        category: category.into(),
        icon: if real { "auto".into() } else { icon.into() },
        process: process.into(),
        ..DockApp::default()
    }
}

impl Default for Config {
    fn default() -> Self {
        let mut terminal = app("terminal", "Terminal", "wt.exe", "projekty", "terminal", "WindowsTerminal.exe");
        terminal.separator_after = true;
        let mut chat = app("discord", "Discord", "discord:", "komunikacja", "chat", "Discord.exe");
        chat.separator_after = true;
        Self {
            horolog: HorologCfg::default(),
            start_mode: "praca".into(),
            modes: vec![
                Mode {
                    id: "praca".into(),
                    label: "Praca".into(),
                    accent: "#D42E25".into(),
                    dim: vec!["discord".into(), "youtube".into(), "steam".into()],
                    ..Mode::default()
                },
                Mode {
                    id: "nauka".into(),
                    label: "Nauka".into(),
                    accent: "#2563EB".into(),
                    dim: vec!["discord".into(), "youtube".into(), "steam".into(), "outlook".into()],
                    ..Mode::default()
                },
                Mode {
                    id: "rozrywka".into(),
                    label: "Rozrywka".into(),
                    accent: "#7C3AED".into(),
                    dim: vec!["excel".into(), "outlook".into(), "terminal".into()],
                    quiet: false,
                    ..Mode::default()
                },
            ],
            dock: vec![
                app("pliki", "Pliki", "explorer.exe", "praca", "folder", ""),
                app("edge", "Edge", "msedge.exe", "praca", "globe", "msedge.exe"),
                app("chrome", "Chrome", "chrome.exe", "praca", "globe", "chrome.exe"),
                app("excel", "Excel", "excel.exe", "projekty", "sheet", "EXCEL.EXE"),
                terminal,
                app("outlook", "Outlook", "outlook.exe", "komunikacja", "mail", "OUTLOOK.EXE"),
                app("horolog", "Horolog", "http://homeserver.local:3000", "komunikacja", "clock", ""),
                chat,
                app("onenote", "OneNote", "onenote.exe", "tworcze", "note", "ONENOTE.EXE"),
                app("spotify", "Spotify", "spotify:", "tworcze", "music", "Spotify.exe"),
                app("youtube", "YouTube", "https://www.youtube.com", "tworcze", "video", ""),
                app("steam", "Steam", "steam://open/main", "tworcze", "game", "steam.exe"),
            ],
            pinned: vec![
                Pinned { name: "Pobrane".into(), path: "%USERPROFILE%\\Downloads".into(), category: "praca".into() },
                Pinned { name: "Dokumenty".into(), path: "%USERPROFILE%\\Documents".into(), category: "komunikacja".into() },
                Pinned { name: "Pulpit".into(), path: "%USERPROFILE%\\Desktop".into(), category: "projekty".into() },
                Pinned { name: "Zrzuty".into(), path: "%USERPROFILE%\\Pictures\\Screenshots".into(), category: "tworcze".into() },
            ],
            contacts: vec![Contact {
                name: "PRZYKŁAD".into(),
                number: "7".into(),
                color: "#fbbf24".into(),
                matches: vec!["Przyklad".into()],
            }],
            hotkeys: Hotkeys::default(),
            radio: RadioCfg::default(),
            spotify: SpotifyCfg::default(),
            shell: ShellCfg::default(),
            focus: FocusCfg::default(),
            notifications: NotificationsCfg::default(),
            sections: default_sections(),
            bar: BarCfg::default(),
            backup: BackupCfg::default(),
            palette: PaletteCfg::default(),
            automations: default_rules(),
            presence: PresenceCfg::default(),
            windows: WindowsCfg::default(),
        }
    }
}

impl Config {
    pub fn load() -> Config {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Config>(&text) {
                Ok(mut cfg) => {
                    // Migracja z wersji bez sekcji: prawdziwe ikony zamiast wbudowanych.
                    let old = serde_json::from_str::<serde_json::Value>(&text)
                        .map(|v| v.get("sections").is_none())
                        .unwrap_or(false);
                    if old {
                        for a in cfg.dock.iter_mut() {
                            if !a.path.starts_with("http") {
                                a.icon = "auto".into();
                            }
                        }
                        cfg.save();
                    }
                    // Tryb „Wieczór” zastąpiony przez „Rozrywka”.
                    let mut renamed = false;
                    for m in cfg.modes.iter_mut().filter(|m| m.id == "wieczor") {
                        m.id = "rozrywka".into();
                        m.label = "Rozrywka".into();
                        renamed = true;
                    }
                    if cfg.start_mode == "wieczor" {
                        cfg.start_mode = "rozrywka".into();
                        renamed = true;
                    }
                    if renamed {
                        cfg.save();
                    }
                    cfg
                }
                Err(err) => {
                    log::error!("config.json nieczytelny ({err}); używam domyślnych");
                    // Nie nadpisujemy pliku użytkownika — zostawiamy go do poprawy.
                    Config::default()
                }
            },
            Err(_) => {
                let cfg = Config::default();
                cfg.save();
                cfg
            }
        }
    }

    pub fn save(&self) {
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(config_path(), text);
        }
    }

    pub fn mode(&self, id: &str) -> Mode {
        self.modes
            .iter()
            .find(|m| m.id == id)
            .or_else(|| self.modes.first())
            .cloned()
            .unwrap_or_default()
    }
}

/// Rozwija `%ZMIENNE%` środowiskowe w ścieżce.
pub fn expand_env(path: &str) -> String {
    let mut out = String::new();
    let mut rest = path;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('%') {
            let name = &after[..end];
            match std::env::var(name) {
                Ok(val) => out.push_str(&val),
                Err(_) => {
                    out.push('%');
                    out.push_str(name);
                    out.push('%');
                }
            }
            rest = &after[end + 1..];
        } else {
            out.push('%');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// (tło kafelka, kolor ikony, pasek sekcji) wyliczone z koloru sekcji.
pub fn section_colors(sections: &[Section], id: &str) -> (String, String, String) {
    let color = sections
        .iter()
        .find(|s| s.id == id)
        .map(|s| s.color.clone())
        .unwrap_or_else(|| "#a1a1aa".into());
    let rgb = parse_hex(&color).unwrap_or((161, 161, 170));
    let mix = |c: u8, base: u8, t: f32| ((c as f32) * t + (base as f32) * (1.0 - t)).round() as u8;
    let bg = (mix(rgb.0, 20, 0.28), mix(rgb.1, 20, 0.28), mix(rgb.2, 23, 0.28));
    let fg = (mix(rgb.0, 255, 0.35), mix(rgb.1, 255, 0.35), mix(rgb.2, 255, 0.35));
    (hex(bg), hex(fg), color)
}

fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&h[0..2], 16).ok()?,
        u8::from_str_radix(&h[2..4], 16).ok()?,
        u8::from_str_radix(&h[4..6], 16).ok()?,
    ))
}

fn hex(c: (u8, u8, u8)) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}
