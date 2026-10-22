//! Wszystkie powiadomienia Windows → radio.
//!
//! Windows zapisuje każde powiadomienie (toast) każdej aplikacji w bazie
//! `%LOCALAPPDATA%\Microsoft\Windows\Notifications\wpndatabase.db`. Czytamy ją
//! tylko do odczytu i co sekundę zbieramy nowe wpisy. Działa bez pakowania
//! aplikacji jako MSIX i obejmuje też powiadomienia stron z przeglądarki.
//!
//! Żeby nie widzieć powiadomień podwójnie, włącz w Windows „Nie przeszkadzać”:
//! wtedy systemowe dymki znikają, a powiadomienia i tak trafiają do bazy.

use crate::config::NotifApp;
use crate::radio;
use crate::state::{self, RadioMsg};
use chrono::Local;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;
use std::time::Duration;
use tauri::AppHandle;

fn db_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("Microsoft").join("Windows").join("Notifications").join("wpndatabase.db"))
}

fn open() -> Result<Connection, String> {
    let path = db_path().ok_or("brak katalogu LOCALAPPDATA")?;
    if !path.exists() {
        return Err("nie ma bazy powiadomień Windows".into());
    }
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_millis(500)).map_err(|e| e.to_string())?;
    Ok(conn)
}

struct Raw {
    id: i64,
    payload: String,
    aumid: String,
}

fn fetch_new(conn: &Connection, after: i64) -> Result<Vec<Raw>, String> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT n.Id, n.Payload, IFNULL(h.PrimaryId, '') FROM Notification n \
             LEFT JOIN NotificationHandler h ON h.RecordId = n.HandlerId \
             WHERE n.Id > ?1 AND n.Type = 'toast' ORDER BY n.Id LIMIT 50",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([after], |row| {
            let payload = match row.get_ref(1)? {
                ValueRef::Blob(b) | ValueRef::Text(b) => String::from_utf8_lossy(b).into_owned(),
                _ => String::new(),
            };
            Ok(Raw { id: row.get(0)?, payload, aumid: row.get(2)? })
        })
        .map_err(|e| e.to_string())?;
    let out: Vec<Raw> = rows.filter_map(|r| r.ok()).collect();
    Ok(out)
}

fn max_id(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT IFNULL(MAX(Id), 0) FROM Notification", [], |r| r.get(0))
        .map_err(|e| e.to_string())
}

/// Teksty z XML toastu, w kolejności: zwykle [nadawca/tytuł, treść, ...].
pub fn toast_texts(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<text") {
        let after = &rest[start + 5..];
        // `<text>` albo `<text ...>` — ale nie `<textBox`.
        if !(after.starts_with('>') || after.starts_with(' ') || after.starts_with('/')) {
            rest = after;
            continue;
        }
        let Some(open_end) = after.find('>') else { break };
        let attrs = &after[..open_end];
        if attrs.ends_with('/') {
            rest = &after[open_end + 1..];
            continue;
        }
        let body = &after[open_end + 1..];
        let Some(close) = body.find("</text>") else { break };
        let text = decode_xml(body[..close].trim());
        if !text.is_empty() && !attrs.contains("placement=\"attribution\"") {
            out.push(text);
        }
        rest = &body[close + 7..];
    }
    out
}

fn decode_xml(s: &str) -> String {
    let s = s.strip_prefix("<![CDATA[").and_then(|x| x.strip_suffix("]]>")).unwrap_or(s);
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let Some(semi) = tail.find(';').filter(|&p| p <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=semi]),
        }
        rest = &tail[semi + 1..];
    }
    out.push_str(rest);
    out
}

/// Przyjazna nazwa aplikacji z identyfikatora AUMID.
pub fn app_name(aumid: &str) -> String {
    let lower = aumid.to_lowercase();
    let known = [
        ("teams", "Teams"),
        ("outlook", "Outlook"),
        ("olk", "Outlook"),
        ("whatsapp", "WhatsApp"),
        ("discord", "Discord"),
        ("slack", "Slack"),
        ("telegram", "Telegram"),
        ("messenger", "Messenger"),
        ("signal", "Signal"),
        ("spotify", "Spotify"),
        ("thunderbird", "Thunderbird"),
        ("windowscommunicationsapps", "Poczta"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("vivaldi", "Vivaldi"),
        ("firefox", "Firefox"),
        ("phonelink", "Telefon"),
        ("yourphone", "Telefon"),
        ("windows.defender", "Defender"),
        ("securityandmaintenance", "Windows"),
        ("windows.systemtoast", "Windows"),
    ];
    if let Some((_, name)) = known.iter().find(|(k, _)| lower.contains(k)) {
        return (*name).to_string();
    }
    // `Firma.Aplikacja_hash!App` → „Aplikacja”
    let base = aumid.split('!').next().unwrap_or(aumid);
    let base = base.split('_').next().unwrap_or(base);
    let last = base.rsplit(['.', '\\']).find(|s| !s.is_empty() && !s.eq_ignore_ascii_case("exe")).unwrap_or(base);
    if last.is_empty() { "Windows".into() } else { last.to_string() }
}

/// Ikona docka, do której należy powiadomienie (po procesie, nazwie albo id).
fn dock_app_for(app: &AppHandle, aumid: &str, source: &str) -> Option<String> {
    let cfg = state::cfg(app);
    let aumid = aumid.to_lowercase();
    let source = source.to_lowercase();
    cfg.dock
        .iter()
        .find(|a| {
            let exe = a.exe_names().into_iter().next().unwrap_or_default();
            let stem = exe.trim_end_matches(".exe").trim_start_matches("ms-").to_string();
            [stem, a.name.to_lowercase(), a.id.to_lowercase()]
                .iter()
                .any(|k| k.len() >= 3 && (aumid.contains(k.as_str()) || source == *k))
        })
        .map(|a| a.id.clone())
}

fn rule<'a>(rules: &'a [NotifApp], aumid: &str, name: &str) -> Option<&'a NotifApp> {
    rules.iter().find(|r| {
        let m = r.matches.to_lowercase();
        !m.is_empty() && (aumid.to_lowercase().contains(&m) || name.to_lowercase() == m)
    })
}

fn to_radio(app: &AppHandle, raw: &Raw) -> Option<RadioMsg> {
    let cfg = state::cfg(app);
    let texts = toast_texts(&raw.payload);
    if texts.is_empty() {
        return None;
    }
    let source = app_name(&raw.aumid);
    let r = rule(&cfg.notifications.apps, &raw.aumid, &source);
    if r.is_some_and(|r| r.ignore) {
        return None;
    }
    let source = r.and_then(|r| (!r.name.is_empty()).then(|| r.name.clone())).unwrap_or(source);
    let (from, message) = if texts.len() == 1 {
        (source.clone(), texts[0].clone())
    } else {
        (texts[0].clone(), texts[1..].join(" · "))
    };
    let mut msg = radio::from_incoming(
        app,
        radio::Incoming {
            from: from.clone(),
            message,
            source: source.to_lowercase(),
            urgent: r.is_some_and(|r| r.urgent),
            color: r.and_then(|r| (!r.color.is_empty()).then(|| r.color.clone())),
            ..Default::default()
        },
    );
    // Bez kontaktu: kolor kategorii „komunikacja”, numer = inicjały nadawcy.
    if msg.color == "#a1a1aa" && !cfg.contacts.iter().any(|c| c.name == msg.name) {
        msg.color = if source == "Windows" || source == "Defender" { "#a1a1aa".into() } else { "#fbbf24".into() };
    }
    if !raw.aumid.is_empty() {
        msg.open_target = Some(format!("shell:AppsFolder\\{}", raw.aumid));
    }
    msg.app = Some(source);
    msg.at = Local::now();
    Some(msg)
}

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut conn: Option<Connection> = None;
        let mut last_id: i64 = -1;
        loop {
            std::thread::sleep(Duration::from_millis(1000));
            if !state::cfg(&app).notifications.enabled {
                continue;
            }
            if conn.is_none() {
                match open() {
                    Ok(c) => {
                        if last_id < 0 {
                            // Start: nie odtwarzamy historii, tylko nowe powiadomienia.
                            last_id = max_id(&c).unwrap_or(0);
                        }
                        conn = Some(c);
                        set_status(&app, "ok");
                    }
                    Err(err) => {
                        set_status(&app, &err);
                        std::thread::sleep(Duration::from_secs(10));
                        continue;
                    }
                }
            }
            let Some(c) = conn.as_ref() else { continue };
            match fetch_new(c, last_id) {
                Ok(rows) => {
                    for raw in rows {
                        last_id = last_id.max(raw.id);
                        if let Some(msg) = to_radio(&app, &raw) {
                            if let Some(id) = dock_app_for(&app, &raw.aumid, msg.app.as_deref().unwrap_or("")) {
                                state::update(&app, false, |rt| *rt.badges.entry(id).or_insert(0) += 1);
                            }
                            radio::deliver(&app, msg);
                        }
                    }
                }
                Err(err) => {
                    set_status(&app, &err);
                    conn = None;
                }
            }
        }
    });
}

fn set_status(app: &AppHandle, status: &str) {
    if state::read(app, |rt| rt.notif_status != status) {
        let status = status.to_string();
        state::update(app, false, |rt| rt.notif_status = status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toast_texts() {
        let xml = r#"<toast launch="x"><visual><binding template="ToastGeneric"><text>Kasia</text><text hint-maxLines="2">Masz chwilę &amp; kawę?</text><text placement="attribution">via Teams</text><image src="a"/></binding></visual></toast>"#;
        assert_eq!(toast_texts(xml), vec!["Kasia".to_string(), "Masz chwilę & kawę?".to_string()]);
    }

    #[test]
    fn names_apps() {
        assert_eq!(app_name("MSTeams_8wekyb3d8bbwe!MSTeams"), "Teams");
        assert_eq!(app_name("5319275A.WhatsAppDesktop_cv1g1gvanyjgm!App"), "WhatsApp");
        assert_eq!(app_name("Acme.Notes_abc!App"), "Notes");
    }
}
