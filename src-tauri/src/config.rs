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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FocusCfg {
    pub adhoc_minutes: i64,
    pub weekly_goal_hours: f64,
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
        }
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
            icon: String::new(),
            process: String::new(),
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
            hide_taskbar: false,
            show_dock: true,
            show_bubbles: true,
            edge_delay_ms: 150,
            edge_glow_minutes: 15,
        }
    }
}

impl Default for FocusCfg {
    fn default() -> Self {
        Self { adhoc_minutes: 25, weekly_goal_hours: 15.0 }
    }
}

fn app(id: &str, name: &str, path: &str, category: &str, icon: &str, process: &str) -> DockApp {
    DockApp {
        id: id.into(),
        name: name.into(),
        path: path.into(),
        category: category.into(),
        icon: icon.into(),
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
                    quiet: true,
                    playlist: String::new(),
                },
                Mode {
                    id: "nauka".into(),
                    label: "Nauka".into(),
                    accent: "#2563EB".into(),
                    dim: vec!["discord".into(), "youtube".into(), "steam".into(), "outlook".into()],
                    quiet: true,
                    playlist: String::new(),
                },
                Mode {
                    id: "wieczor".into(),
                    label: "Wieczór".into(),
                    accent: "#7C3AED".into(),
                    dim: vec!["excel".into(), "outlook".into(), "terminal".into()],
                    quiet: false,
                    playlist: String::new(),
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
        }
    }
}

impl Config {
    pub fn load() -> Config {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Config>(&text) {
                Ok(cfg) => cfg,
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

pub fn category_colors(category: &str) -> (&'static str, &'static str, &'static str) {
    // (tło kafelka, kolor ikony, pasek kategorii)
    match category {
        "praca" => ("#1e3a5f", "#bfdbfe", "#60a5fa"),
        "projekty" => ("#14422e", "#bbf7d0", "#22c55e"),
        "komunikacja" => ("#4a3410", "#fde68a", "#eab308"),
        "tworcze" => ("#3b1f5c", "#e9d5ff", "#c084fc"),
        _ => ("#27272a", "#e4e4e7", "#a1a1aa"),
    }
}
