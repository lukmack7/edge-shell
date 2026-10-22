//! Radio: powiadomienia w stylu rozmów z pit wall.
//!
//! Źródła: Horolog (komunikaty planu generuje `engine`) oraz lokalny endpoint
//! `POST http://127.0.0.1:7311/radio`, do którego może pisać cokolwiek —
//! skrypt, n8n, Home Assistant, Tasker na telefonie.
//!
//! W trybie „cichym” zwykłe wiadomości czekają na paczkę o pełnej godzinie z
//! konfiguracji. Pilne (flaga `urgent`) i od kontaktów z numerem 1–9
//! przechodzą od razu.

use crate::layout;
use crate::state::{self, RadioMsg};
use chrono::{Local, Timelike};
use serde::Deserialize;
use std::io::Read;
use tauri::AppHandle;

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Incoming {
    /// Nadawca — dopasowywany do kontaktów z konfiguracji.
    pub from: String,
    pub message: String,
    /// Źródło widoczne pod kartą, np. `komunikator`, `poczta`, `sms`.
    pub source: String,
    /// Propozycja odpowiedzi (np. wygenerowana przez model).
    pub reply: Option<String>,
    /// Gdzie wysłać odpowiedź (POST JSON `{ "reply": "..." }`). Bez tego
    /// odpowiedź trafia do schowka.
    pub reply_url: Option<String>,
    pub urgent: bool,
    /// Opcjonalnie: nadpisanie numeru i koloru.
    pub number: Option<String>,
    pub color: Option<String>,
}

fn new_id() -> String {
    format!("{:x}{:04x}", Local::now().timestamp_millis(), rand::random::<u16>())
}

pub fn from_incoming(app: &AppHandle, inc: Incoming) -> RadioMsg {
    let cfg = state::cfg(app);
    let contact = cfg.contacts.iter().find(|c| {
        c.matches.iter().any(|m| m.eq_ignore_ascii_case(inc.from.trim()))
            || c.name.eq_ignore_ascii_case(inc.from.trim())
    });
    let name = contact
        .map(|c| c.name.clone())
        .unwrap_or_else(|| inc.from.trim().to_uppercase());
    let number = inc
        .number
        .or_else(|| contact.map(|c| c.number.clone()))
        .unwrap_or_else(|| initials(&name));
    let color = inc
        .color
        .or_else(|| contact.map(|c| c.color.clone()))
        .unwrap_or_else(|| "#a1a1aa".into());
    RadioMsg {
        id: new_id(),
        name: if name.is_empty() { "RADIO".into() } else { name },
        number,
        color,
        source: if inc.source.is_empty() { "radio".into() } else { inc.source },
        message: inc.message,
        reply: inc.reply.filter(|r| !r.trim().is_empty()),
        reply_url: inc.reply_url.filter(|u| u.starts_with("http://") || u.starts_with("https://")),
        urgent: inc.urgent,
        at: Local::now(),
        app: None,
        open_target: None,
        ask: None,
    }
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

/// Komunikat z pit wall (Horolog) — zawsze przechodzi od razu.
pub fn pit_wall(app: &AppHandle, message: String, reply: Option<String>) {
    let accent = {
        let cfg = state::cfg(app);
        let mode = state::read(app, |rt| rt.store.mode.clone());
        cfg.mode(&mode).accent
    };
    deliver(
        app,
        RadioMsg {
            id: new_id(),
            name: "PIT WALL".into(),
            number: "PW".into(),
            color: lighten(&accent),
            source: "horolog".into(),
            message,
            reply,
            reply_url: None,
            urgent: true,
            at: Local::now(),
            app: None,
            open_target: None,
            ask: None,
        },
    );
}

/// Jaśniejsza wersja akcentu, żeby tekst na ciemnej karcie był czytelny.
fn lighten(hex: &str) -> String {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 {
        return "#f87171".into();
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(128) as u16;
    let mix = |c: u16| ((c + (255 - c) * 2 / 5).min(255)) as u8;
    format!("#{:02x}{:02x}{:02x}", mix(p(0)), mix(p(2)), mix(p(4)))
}

pub fn deliver(app: &AppHandle, msg: RadioMsg) {
    let cfg = state::cfg(app);
    let mode = state::read(app, |rt| rt.store.mode.clone());
    let quiet_override = state::read(app, |rt| rt.store.quiet_until.is_some_and(|t| t > Local::now()));
    let quiet = cfg.mode(&mode).quiet || quiet_override;
    let vip = msg.number.parse::<u8>().is_ok_and(|n| (1..=9).contains(&n));
    if msg.urgent || vip || !quiet {
        state::update(app, false, |rt| rt.showing.push_back(msg));
        layout::place_radio(app);
        layout::show_window(app, "radio", false);
    } else {
        state::update(app, true, |rt| rt.store.waiting.push(msg));
    }
}

/// Pokazuje wszystko, co czeka w paczce.
pub fn release_batch(app: &AppHandle) {
    let any = state::update(app, true, |rt| {
        let waiting = std::mem::take(&mut rt.store.waiting);
        let any = !waiting.is_empty();
        rt.showing.extend(waiting);
        any || !rt.showing.is_empty()
    });
    if any {
        layout::place_radio(app);
        layout::show_window(app, "radio", false);
    }
}

/// Zdejmuje bieżącą kartę; chowa okno, gdy nic nie zostało.
pub fn dismiss(app: &AppHandle, to_batch: bool) {
    let empty = state::update(app, true, |rt| {
        if let Some(msg) = rt.showing.pop_front() {
            if to_batch {
                rt.store.waiting.push(msg);
            }
        }
        rt.showing.is_empty()
    });
    if empty {
        layout::hide_window(app, "radio");
    }
}

pub async fn send_reply(app: &AppHandle, text: String) -> Result<String, String> {
    let msg = state::read(app, |rt| rt.showing.front().cloned());
    let Some(msg) = msg else { return Err("Brak wiadomości".into()) };
    if let Some(url) = &msg.reply_url {
        reqwest::Client::new()
            .post(url)
            .timeout(std::time::Duration::from_secs(10))
            .json(&serde_json::json!({ "reply": text, "id": msg.id, "to": msg.name }))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        Ok("Wysłano".into())
    } else {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        app.clipboard().write_text(text).map_err(|e| e.to_string())?;
        Ok("Skopiowano do schowka".into())
    }
}

/// Wywoływane co sekundę z silnika: dostarcza paczkę o umówionej godzinie.
pub fn tick(app: &AppHandle) {
    let now = Local::now();
    let cfg = state::cfg(app);
    let hhmm = format!("{:02}:{:02}", now.hour(), now.minute());
    let due = cfg.radio.batch_times.iter().any(|t| {
        state::parse_hhmm(t).is_some_and(|m| m == now.hour() * 60 + now.minute())
    });
    if !due {
        return;
    }
    let first = state::update(app, false, |rt| {
        if rt.last_batch == hhmm {
            false
        } else {
            rt.last_batch = hhmm.clone();
            true
        }
    });
    if first && state::read(app, |rt| !rt.store.waiting.is_empty()) {
        release_batch(app);
    }
}

/// Lokalny serwer HTTP na 127.0.0.1 — tylko z tego komputera.
pub fn start_server(app: AppHandle) {
    let cfg = state::cfg(&app);
    let port = cfg.radio.port;
    let token = cfg.radio.token.clone();
    std::thread::spawn(move || {
        let server = match tiny_http::Server::http(("127.0.0.1", port)) {
            Ok(s) => s,
            Err(err) => {
                log::error!("radio: nie mogę otworzyć portu {port}: {err}");
                return;
            }
        };
        for mut req in server.incoming_requests() {
            let path = req.url().split('?').next().unwrap_or("").to_string();
            let method = req.method().as_str().to_uppercase();
            let authorized = token.is_empty()
                || req.headers().iter().any(|h| {
                    h.field.equiv("Authorization") && h.value.as_str() == format!("Bearer {token}")
                });
            let (code, body) = if !authorized {
                (401, r#"{"error":"unauthorized"}"#.to_string())
            } else if method == "GET" && path == "/health" {
                (200, r#"{"status":"ok"}"#.to_string())
            } else if method == "POST" && path == "/radio" {
                let mut text = String::new();
                let _ = req.as_reader().take(64 * 1024).read_to_string(&mut text);
                match serde_json::from_str::<Incoming>(&text) {
                    Ok(inc) if !inc.message.trim().is_empty() => {
                        let msg = from_incoming(&app, inc);
                        let id = msg.id.clone();
                        deliver(&app, msg);
                        (200, format!(r#"{{"id":"{id}"}}"#))
                    }
                    _ => (400, r#"{"error":"expected JSON with from and message"}"#.to_string()),
                }
            } else {
                (404, r#"{"error":"not found"}"#.to_string())
            };
            let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).ok();
            let mut resp = tiny_http::Response::from_string(body).with_status_code(code);
            if let Some(h) = header {
                resp = resp.with_header(h);
            }
            let _ = req.respond(resp);
        }
    });
}
