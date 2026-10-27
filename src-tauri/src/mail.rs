//! Poczta z Thunderbirda przez dodatek „Edge Shell Bridge”.
//!
//! Dodatek rozmawia z lokalnym serwerem radia (127.0.0.1):
//! - `POST /mail/event`  — nowy mail → reguły (od razu / paczka / pominięty) → notch,
//! - `POST /mail/state`  — nieprzeczytane na konto (licznik w docku) i oflagowane,
//! - `POST /mail/result` — odpowiedź na polecenie z `reqId` (szukaj, kontakty, skrzynka),
//! - `GET  /mail/commands` — co ~0,8 s odbiera polecenia (to też sygnał „jestem”).

use crate::radio;
use crate::state::{self, MailRef, MailTask};
use chrono::Local;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::AppHandle;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub name: String,
    pub unread: u32,
}

#[derive(Default)]
struct Bridge {
    last_seen: Option<Instant>,
    accounts: Vec<Account>,
    flagged: Vec<Value>,
    inbox: Vec<Value>,
    inbox_at: Option<Instant>,
    queue: Vec<Value>,
    results: HashMap<String, Value>,
    skipped_today: (String, u32),
    seq: u64,
}

static BRIDGE: Mutex<Option<Bridge>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Bridge) -> R) -> R {
    let mut g = BRIDGE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Bridge::default))
}

/// Dodatek odezwał się w ciągu ostatnich 10 s.
pub fn alive() -> bool {
    with(|b| b.last_seen.is_some_and(|t| t.elapsed() < Duration::from_secs(10)))
}

// ---------------------------------------------------------------------------
// HTTP (wołane z serwera radia)
// ---------------------------------------------------------------------------

/// GET /mail/commands — oddaje kolejkę poleceń.
pub fn take_commands(app: &AppHandle) -> String {
    let (cmds, came_back) = with(|b| {
        let came_back = b.last_seen.map_or(true, |t| t.elapsed() > Duration::from_secs(10));
        b.last_seen = Some(Instant::now());
        (std::mem::take(&mut b.queue), came_back)
    });
    if came_back {
        state::emit_snapshot(app);
    }
    serde_json::to_string(&cmds).unwrap_or_else(|_| "[]".into())
}

/// POST /mail/state — liczniki i oflagowane.
pub fn on_state(app: &AppHandle, v: Value) {
    let accounts: Vec<Account> = v["accounts"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| Account {
                    id: x["id"].as_str().unwrap_or_default().into(),
                    name: x["name"].as_str().unwrap_or_default().into(),
                    unread: x["unread"].as_u64().unwrap_or(0) as u32,
                })
                .collect()
        })
        .unwrap_or_default();
    let flagged = v["flagged"].as_array().cloned().unwrap_or_default();
    with(|b| {
        b.accounts = accounts;
        b.flagged = flagged;
        b.last_seen = Some(Instant::now());
    });
    state::emit_snapshot(app);
}

/// POST /mail/result — odpowiedź na `request`.
pub fn on_result(v: Value) {
    if let Some(id) = v["reqId"].as_str().map(str::to_string) {
        with(|b| {
            b.results.insert(id, v);
        });
    }
}

/// „Jan Kowalski <jan@firma.pl>” → („Jan Kowalski”, „jan@firma.pl”).
fn split_author(author: &str) -> (String, String) {
    let a = author.trim();
    if let (Some(lt), Some(gt)) = (a.rfind('<'), a.rfind('>')) {
        if lt < gt {
            let email = a[lt + 1..gt].trim().to_string();
            let name = a[..lt].trim().trim_matches('"').trim().to_string();
            return (if name.is_empty() { email.clone() } else { name }, email);
        }
    }
    (a.to_string(), if a.contains('@') { a.to_string() } else { String::new() })
}

pub fn mail_ref(m: &Value) -> MailRef {
    let (from_name, from_email) = split_author(m["author"].as_str().unwrap_or_default());
    MailRef {
        id: m["id"].as_i64().unwrap_or(-1),
        mid: m["mid"].as_str().unwrap_or_default().into(),
        account: m["accountName"].as_str().unwrap_or_default().into(),
        from_name: crate::notifications::clean_text(&from_name),
        from_email,
        subject: crate::notifications::clean_text(m["subject"].as_str().unwrap_or_default()),
        snippet: crate::notifications::clean_text(m["snippet"].as_str().unwrap_or_default()),
    }
}

/// POST /mail/event — nowy mail. Trzy tory: od razu (ważni), paczka, pominięty (newsletter).
pub fn on_event(app: &AppHandle, v: Value) {
    with(|b| b.last_seen = Some(Instant::now()));
    if v["kind"].as_str() != Some("new") {
        return;
    }
    let cfg = state::cfg(app);
    if !cfg.mail.enabled {
        return;
    }
    let m = &v["mail"];
    let r = mail_ref(m);
    let low = |s: &str| s.to_lowercase();
    let vip = cfg.mail.vip.iter().any(|x| {
        let x = low(x.trim());
        !x.is_empty() && (low(&r.from_email) == x || low(&r.from_name) == x)
    }) || cfg.contacts.iter().any(|c| {
        c.matches.iter().any(|x| {
            let x = low(x.trim());
            !x.is_empty() && (low(&r.from_email) == x || low(&r.from_name).contains(&x))
        })
    });
    if m["newsletter"].as_bool().unwrap_or(false) && cfg.mail.skip_newsletters && !vip {
        let today = state::today_key();
        with(|b| {
            if b.skipped_today.0 != today {
                b.skipped_today = (today, 0);
            }
            b.skipped_today.1 += 1;
        });
        state::emit_snapshot(app);
        return;
    }

    let mut msg = radio::from_incoming(
        app,
        radio::Incoming {
            from: r.from_name.clone(),
            message: if r.subject.is_empty() { "(bez tematu)".into() } else { r.subject.clone() },
            source: if r.account.is_empty() { "poczta".into() } else { format!("poczta · {}", r.account) },
            urgent: vip,
            ..Default::default()
        },
    );
    // Kontakt z konfiguracji dopasowany po adresie (from_incoming zna tylko nazwę).
    if let Some(c) = cfg.contacts.iter().find(|c| c.matches.iter().any(|x| low(x.trim()) == low(&r.from_email) && !r.from_email.is_empty())) {
        msg.name = c.name.clone();
        msg.number = c.number.clone();
        msg.color = c.color.clone();
    } else if msg.color == "#a1a1aa" {
        msg.color = "#38bdf8".into();
    }
    msg.app = Some("Thunderbird".into());
    msg.mail = Some(r.clone());
    msg.at = Local::now();

    // Konto firmowe poza trybem pracy → paczka (chyba że ważny nadawca).
    let mode = state::read(app, |rt| rt.store.mode.clone());
    let work = cfg.mail.work_accounts.iter().any(|a| a.eq_ignore_ascii_case(&r.account));
    if work && !vip && !mode.eq_ignore_ascii_case(&cfg.mail.work_mode) {
        state::update(app, true, |rt| rt.store.waiting.push(msg));
        return;
    }
    radio::deliver(app, msg);
}

// ---------------------------------------------------------------------------
// Polecenia do Thunderbirda
// ---------------------------------------------------------------------------

fn push(cmd: Value) -> Result<(), String> {
    if !alive() {
        return Err("Thunderbird nie jest połączony — zainstaluj dodatek Edge Shell Bridge (Ustawienia → Poczta)".into());
    }
    with(|b| b.queue.push(cmd));
    Ok(())
}

/// Polecenie z odpowiedzią (szukaj, kontakty, skrzynka).
pub async fn request(mut cmd: Value, timeout: Duration) -> Result<Value, String> {
    let id = with(|b| {
        b.seq += 1;
        format!("r{}", b.seq)
    });
    cmd["reqId"] = json!(id);
    push(cmd)?;
    let start = Instant::now();
    loop {
        if let Some(v) = with(|b| b.results.remove(&id)) {
            if v["ok"].as_bool() == Some(false) {
                return Err(v["error"].as_str().unwrap_or("Thunderbird: błąd").to_string());
            }
            return Ok(v);
        }
        if start.elapsed() > timeout {
            return Err("Thunderbird nie odpowiedział".into());
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

/// Zdejmuje mail z widoku (karta, przegapione, kolejka bloku poczty).
fn forget(app: &AppHandle, r: &MailRef) {
    let same = |m: &Option<MailRef>| m.as_ref().is_some_and(|x| if r.mid.is_empty() { x.id == r.id } else { x.mid == r.mid });
    let front = state::read(app, |rt| rt.showing.front().map(|m| same(&m.mail)).unwrap_or(false));
    if front {
        radio::dismiss(app, false);
    }
    state::update(app, true, |rt| {
        rt.showing.retain(|m| !same(&m.mail));
        rt.store.missed.retain(|m| !same(&m.mail));
        rt.store.waiting.retain(|m| !same(&m.mail));
    });
    with(|b| b.inbox.retain(|v| v["mid"].as_str() != Some(r.mid.as_str()) || r.mid.is_empty()));
}

fn ref_from(app: &AppHandle, id: i64, mid: &str) -> MailRef {
    // Szukamy pełnych danych w kartach, przegapionych, skrzynce bloku i oflagowanych.
    let found = state::read(app, |rt| {
        rt.showing
            .iter()
            .filter_map(|m| m.mail.clone())
            .chain(rt.store.missed.iter().filter_map(|m| m.mail.clone()))
            .chain(rt.store.waiting.iter().filter_map(|m| m.mail.clone()))
            .find(|m| (!mid.is_empty() && m.mid == mid) || m.id == id)
    });
    found.unwrap_or_else(|| {
        with(|b| {
            b.inbox
                .iter()
                .chain(b.flagged.iter())
                .find(|v| v["mid"].as_str() == Some(mid) || v["id"].as_i64() == Some(id))
                .map(mail_ref)
        })
        .unwrap_or(MailRef { id, mid: mid.into(), ..Default::default() })
    })
}

/// Archiwizuj / przeczytane / usuń / otwórz / flaga.
#[tauri::command]
pub fn mail_action(app: AppHandle, id: i64, mid: String, action: String) -> Result<(), String> {
    let cmd = match action.as_str() {
        "open" | "archive" | "read" | "delete" | "flag" | "unflag" => action.clone(),
        _ => return Err(format!("nieznana akcja {action}")),
    };
    push(json!({ "cmd": cmd, "id": id, "mid": mid }))?;
    let r = ref_from(&app, id, &mid);
    if matches!(action.as_str(), "archive" | "read" | "delete") {
        forget(&app, &r);
    } else if action == "open" {
        // Otwarty mail znika z karty, ale zostaje w skrzynce bloku.
        let front = state::read(&app, |rt| rt.showing.front().and_then(|m| m.mail.as_ref().map(|x| x.mid == r.mid)).unwrap_or(false));
        if front {
            radio::dismiss(&app, false);
        }
        state::update(&app, true, |rt| rt.store.missed.retain(|m| m.mail.as_ref().map_or(true, |x| x.mid != r.mid)));
    }
    if action == "unflag" {
        with(|b| b.flagged.retain(|v| v["mid"].as_str() != Some(mid.as_str())));
    }
    state::emit_snapshot(&app);
    Ok(())
}

/// Odpowiedź z notcha — wysyła Thunderbird z właściwego konta.
#[tauri::command]
pub async fn mail_reply(app: AppHandle, id: i64, mid: String, text: String) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Pusta odpowiedź".into());
    }
    request(json!({ "cmd": "reply", "id": id, "mid": mid, "text": text }), Duration::from_secs(30)).await?;
    let r = ref_from(&app, id, &mid);
    forget(&app, &r);
    state::emit_snapshot(&app);
    Ok(())
}

/// Mail → zadanie w Horologu. `when`: „dziś” albo „jutro”.
#[tauri::command]
pub async fn mail_task(app: AppHandle, id: i64, mid: String, when: Option<String>, archive: Option<bool>) -> Result<String, String> {
    let cfg = state::cfg(&app);
    let r = ref_from(&app, id, &mid);
    let who = if r.from_name.is_empty() { r.from_email.clone() } else { r.from_name.clone() };
    let subject = if r.subject.is_empty() { "mail".to_string() } else { r.subject.clone() };
    let when = when.unwrap_or_else(|| "dziś".into());
    let text = format!("Odpowiedzieć na mail od {who}: „{subject}” — {when}, {} min", cfg.mail.task_minutes);
    let (intent_id, title) = crate::horolog::capture_intent(&app, &text).await?;
    state::update(&app, true, |rt| {
        rt.store.mail_tasks.insert(0, MailTask { intent_id: intent_id.clone(), title: title.clone(), mail: r.clone() });
        rt.store.mail_tasks.truncate(50);
        if !r.mid.is_empty() && !rt.store.mail_planned.contains(&r.mid) {
            rt.store.mail_planned.insert(0, r.mid.clone());
            rt.store.mail_planned.truncate(200);
        }
    });
    if archive.unwrap_or(true) && alive() {
        let _ = push(json!({ "cmd": "archive", "id": r.id, "mid": r.mid }));
        forget(&app, &r);
    }
    state::emit_snapshot(&app);
    Ok(title)
}

/// Blok z Horologa powstał z maila → otwórz ten mail.
#[tauri::command]
pub fn mail_open_task(app: AppHandle, intent_id: String) -> Result<bool, String> {
    let t = state::read(&app, |rt| rt.store.mail_tasks.iter().find(|t| t.intent_id == intent_id).cloned());
    match t {
        Some(t) => {
            push(json!({ "cmd": "open", "id": t.mail.id, "mid": t.mail.mid }))?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Nowy mail (paleta „@ …”).
pub fn compose(to: &str, subject: &str) -> Result<(), String> {
    push(json!({ "cmd": "compose", "to": to, "subject": subject }))
}

/// Nieprzeczytane ze skrzynek — kolejka bloku poczty.
#[tauri::command]
pub async fn mail_inbox(app: AppHandle, fresh: Option<bool>) -> Result<Vec<Value>, String> {
    let cached = with(|b| b.inbox_at.filter(|t| t.elapsed() < Duration::from_secs(60)).map(|_| b.inbox.clone()));
    if let (Some(items), false) = (cached, fresh.unwrap_or(false)) {
        return Ok(items);
    }
    let v = request(json!({ "cmd": "inbox" }), Duration::from_secs(20)).await?;
    let mut items = v["items"].as_array().cloned().unwrap_or_default();
    for it in items.iter_mut() {
        let r = mail_ref(it);
        it["fromName"] = json!(r.from_name);
        it["fromEmail"] = json!(r.from_email);
        it["subject"] = json!(r.subject);
        it["snippet"] = json!(r.snippet);
    }
    with(|b| {
        b.inbox = items.clone();
        b.inbox_at = Some(Instant::now());
    });
    let _ = app;
    Ok(items)
}

#[tauri::command]
pub async fn mail_search(q: String) -> Result<Vec<Value>, String> {
    let v = request(json!({ "cmd": "search", "q": q }), Duration::from_secs(8)).await?;
    Ok(v["items"].as_array().cloned().unwrap_or_default())
}

pub async fn contacts(q: &str) -> Result<Vec<Value>, String> {
    let v = request(json!({ "cmd": "contacts", "q": q }), Duration::from_secs(3)).await?;
    Ok(v["items"].as_array().cloned().unwrap_or_default())
}

/// Ścieżka do pliku dodatku (.xpi) z instalatora — przycisk w ustawieniach.
#[tauri::command]
pub fn mail_addon_path(app: AppHandle) -> Result<String, String> {
    use tauri::Manager;
    let dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let p = dir.join("resources").join("edge-shell-bridge.xpi");
    if p.exists() {
        return Ok(p.to_string_lossy().to_string());
    }
    let p = dir.join("edge-shell-bridge.xpi");
    if p.exists() {
        Ok(p.to_string_lossy().to_string())
    } else {
        Err("Nie znalazłem pliku dodatku w instalacji Edge Shell".into())
    }
}

// ---------------------------------------------------------------------------
// Migawka dla interfejsu
// ---------------------------------------------------------------------------

/// Czy bieżący blok to „poczta” (słowa z ustawień w tytule).
pub fn mail_block(cfg: &crate::config::Config, title: &str) -> bool {
    let t = title.to_lowercase();
    cfg.mail.block_words.iter().any(|w| {
        let w = w.trim().to_lowercase();
        !w.is_empty() && t.split(|c: char| !c.is_alphanumeric() && c != '-').any(|x| x == w)
    })
}

pub fn snapshot(cfg: &crate::config::Config, planned: &[String]) -> Value {
    let today = state::today_key();
    with(|b| {
        let unread: u32 = b.accounts.iter().map(|a| a.unread).sum();
        let flagged: Vec<Value> = b
            .flagged
            .iter()
            .map(|v| {
                let r = mail_ref(v);
                json!({ "id": r.id, "mid": r.mid, "fromName": r.from_name, "subject": r.subject, "account": r.account,
                        "date": v["date"], "planned": planned.contains(&r.mid) })
            })
            .collect();
        json!({
            "enabled": cfg.mail.enabled,
            "connected": b.last_seen.is_some_and(|t| t.elapsed() < Duration::from_secs(10)),
            "accounts": b.accounts,
            "unread": unread,
            "flagged": flagged,
            "skippedToday": if b.skipped_today.0 == today { b.skipped_today.1 } else { 0 },
        })
    })
}

/// Licznik dla ikony Thunderbirda w docku.
pub fn unread() -> u32 {
    with(|b| b.accounts.iter().map(|a| a.unread).sum())
}
