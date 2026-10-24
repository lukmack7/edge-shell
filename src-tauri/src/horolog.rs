//! Most do Horologa: plan dnia, strumień zmian (SSE), zapisywanie myśli,
//! oznaczanie ukończenia i przeplanowanie.
//!
//! Horolog wysyła w strumieniu tylko sygnał „plan się zmienił”, więc po każdym
//! zdarzeniu `plan` pobieramy cały plan od nowa. Ostatni plan trzymamy na dysku,
//! żeby panel działał też bez sieci.

use crate::state::{self, DayItem};
use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveTime, TimeZone};
use futures_util::StreamExt;
use serde::Deserialize;
use std::time::Duration;
use tauri::AppHandle;

#[derive(Deserialize)]
struct BlockOut {
    intent_id: String,
    title: String,
    kind: String,
    priority: Option<u8>,
    occurrence: Option<i64>,
    chunk: Option<i64>,
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
    moved_from: Option<DateTime<FixedOffset>>,
}

#[derive(Deserialize)]
struct BusyOut {
    #[serde(default)]
    label: String,
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
    #[serde(default)]
    source: String,
}

#[derive(Deserialize)]
struct PlanOut {
    blocks: Vec<BlockOut>,
    #[serde(default)]
    busy: Vec<BusyOut>,
}

#[derive(Deserialize)]
struct Analytics {
    focus_minutes: i64,
}

fn base(app: &AppHandle) -> String {
    state::cfg(app).horolog.url.trim_end_matches('/').to_string()
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default()
}

fn is_today(dt: &DateTime<Local>) -> bool {
    let now = Local::now();
    dt.year() == now.year() && dt.ordinal() == now.ordinal()
}

pub async fn fetch_plan(app: &AppHandle) -> Result<(), String> {
    let url = format!("{}/api/plan", base(app));
    let plan: PlanOut = client()
        .get(&url)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let horizon = Local::now() + chrono::Duration::days(8);
    let mut items: Vec<DayItem> = Vec::new();
    for b in plan.blocks {
        let start = b.start.with_timezone(&Local);
        let end = b.end.with_timezone(&Local);
        if end < Local::now() - chrono::Duration::hours(24) || start > horizon {
            continue;
        }
        items.push(DayItem {
            key: format!("{}:{}:{}", b.intent_id, b.occurrence.unwrap_or(0), b.chunk.unwrap_or(0)),
            intent_id: Some(b.intent_id),
            title: b.title,
            kind: b.kind,
            priority: b.priority,
            start,
            end,
            moved: b.moved_from.is_some(),
        });
    }
    for (n, e) in plan.busy.into_iter().enumerate() {
        let start = e.start.with_timezone(&Local);
        let end = e.end.with_timezone(&Local);
        if end < Local::now() - chrono::Duration::hours(24) || start > horizon {
            continue;
        }
        let buffer = e.source == "buffer";
        let title = if buffer {
            "Bufor — odejdź od biurka".to_string()
        } else if e.label.is_empty() {
            "Zajęte".to_string()
        } else {
            e.label.clone()
        };
        items.push(DayItem {
            key: format!("busy:{}:{}", start.timestamp(), n),
            intent_id: None,
            title,
            kind: if buffer { "buffer".into() } else { "meeting".into() },
            priority: None,
            start,
            end,
            moved: false,
        });
    }
    items.sort_by_key(|i| i.start);
    let week = items.clone();
    let items: Vec<DayItem> = items.into_iter().filter(|i| is_today(&i.start) || is_today(&i.end)).collect();

    state::update(app, true, |rt| {
        rt.plan = items.clone();
        rt.week = week.clone();
        rt.store.week_cache = week;
        rt.store.plan_cache = items;
        rt.store.plan_cached_at = Some(Local::now());
        rt.connected = true;
        rt.horolog_error.clear();
        rt.last_sync = Some(Local::now());
    });

    if let Ok(a) = fetch_analytics(app).await {
        state::update(app, true, |rt| rt.store.week_focus_min = a.focus_minutes);
    }
    Ok(())
}

async fn fetch_analytics(app: &AppHandle) -> Result<Analytics, String> {
    client()
        .get(format!("{}/api/analytics", base(app)))
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
}

/// Pętla połączenia: pobierz plan, słuchaj strumienia, przy zerwaniu ponów.
pub fn start(app: AppHandle) {
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut backoff = 2u64;
        loop {
            match listen(&app).await {
                Ok(()) => backoff = 2,
                Err(err) => {
                    state::update(&app, false, |rt| {
                        rt.connected = false;
                        rt.horolog_error = err.clone();
                    });
                }
            }
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(60);
        }
    });

    // Odświeżanie co 5 minut i o północy — na wypadek zgubionych zdarzeń.
    tauri::async_runtime::spawn(async move {
        let mut last_day = Local::now().ordinal();
        loop {
            tokio::time::sleep(Duration::from_secs(300)).await;
            let today = Local::now().ordinal();
            if today != last_day {
                last_day = today;
                // Zamknięcie dnia: niezaznaczone bloki trafiają do historii jako pominięte.
                state::update(&app2, true, |rt| {
                    let date = rt.plan.first().map(|i| i.start.format("%Y-%m-%d").to_string()).unwrap_or_default();
                    for i in rt.plan.iter().filter(|i| matches!(i.kind.as_str(), "task" | "focus" | "habit")) {
                        if !rt.store.marks.contains_key(&i.key) {
                            rt.store.history.push(crate::state::HistEntry {
                                date: date.clone(),
                                title: i.title.clone(),
                                kind: i.kind.clone(),
                                start: i.start,
                                end: i.end,
                                status: "skipped".into(),
                                delta_min: 0,
                            });
                        }
                    }
                    rt.store.marks.clear();
                    let len = rt.store.history.len();
                    if len > 600 {
                        rt.store.history.drain(..len - 600);
                    }
                });
            }
            let _ = fetch_plan(&app2).await;
        }
    });
}

async fn listen(app: &AppHandle) -> Result<(), String> {
    fetch_plan(app).await?;
    flush_offline(app).await;

    let resp = client()
        .get(format!("{}/api/stream", base(app)))
        .header("Accept", "text/event-stream")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut event = String::new();
    loop {
        // Horolog wysyła „ping” co 20 s; cisza dłuższa niż minuta = martwe połączenie.
        let chunk = tokio::time::timeout(Duration::from_secs(60), stream.next())
            .await
            .map_err(|_| "strumień Horologa zamilkł".to_string())?;
        let Some(chunk) = chunk else {
            return Err("Horolog zamknął strumień".into());
        };
        let chunk = chunk.map_err(|e| e.to_string())?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find('\n') {
            let line: String = buf.drain(..=pos).collect();
            let line = line.trim_end_matches(['\r', '\n']);
            if let Some(name) = line.strip_prefix("event:") {
                event = name.trim().to_string();
            } else if line.is_empty() {
                if event == "plan" {
                    let _ = fetch_plan(app).await;
                }
                event.clear();
            }
        }
    }
}

/// Wrzuca myśl do Horologa (język naturalny → zadanie w planie).
/// Bez połączenia odkłada ją do kolejki i wysyła po powrocie sieci.
pub async fn capture(app: &AppHandle, text: String) -> Result<String, String> {
    match post_capture(app, &text).await {
        Ok(msg) => Ok(msg),
        Err(err) if err.starts_with("offline:") => {
            state::update(app, true, |rt| rt.store.offline_captures.push(text));
            Ok("Brak połączenia — zapisane, wyślę po powrocie sieci".into())
        }
        Err(err) => Err(err),
    }
}

async fn post_capture(app: &AppHandle, text: &str) -> Result<String, String> {
    let resp = client()
        .post(format!("{}/api/capture", base(app)))
        .timeout(Duration::from_secs(60))
        .json(&serde_json::json!({ "text": text }))
        .send()
        .await
        .map_err(|e| format!("offline:{e}"))?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    if status.is_success() {
        let title = body["intent"]["title"].as_str().unwrap_or(text).to_string();
        Ok(format!("W planie: {title}"))
    } else if status.as_u16() == 503 {
        Err(format!("offline:{}", body["detail"].as_str().unwrap_or("model niedostępny")))
    } else {
        Err(body["detail"].as_str().unwrap_or("Horolog nie zrozumiał tej myśli").to_string())
    }
}

async fn flush_offline(app: &AppHandle) {
    let pending = state::read(app, |rt| rt.store.offline_captures.clone());
    if pending.is_empty() {
        return;
    }
    let mut left = Vec::new();
    for text in pending {
        if post_capture(app, &text).await.is_err() {
            left.push(text);
        }
    }
    state::update(app, true, |rt| rt.store.offline_captures = left);
}

pub async fn complete(app: &AppHandle, intent_id: &str) -> Result<(), String> {
    let resp = client()
        .post(format!("{}/api/intents/{}/complete", base(app), intent_id))
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    // 422 = cykliczny nawyk; Horolog nie oznacza pojedynczych wystąpień — wtedy
    // wystarcza nam lokalne oznaczenie, więc to nie jest błąd.
    if resp.status().is_success() || resp.status().as_u16() == 422 {
        Ok(())
    } else {
        Err(format!("Horolog: {}", resp.status()))
    }
}

pub async fn replan(app: &AppHandle) -> Result<(), String> {
    client()
        .post(format!("{}/api/plan/solve", base(app)))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    fetch_plan(app).await
}

// ---------------------------------------------------------------------------
// Edycja zadań: przesuwanie bloków i korekta czasu
// ---------------------------------------------------------------------------

const SLOT_MINUTES: i64 = 15;

async fn get_json(app: &AppHandle, path: &str) -> Result<serde_json::Value, String> {
    client()
        .get(format!("{}{}", base(app), path))
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
}

/// Pełne ciało `PUT /api/intents/{id}` z zapisanego zadania — tak jak robi to
/// interfejs WWW Horologa (API zastępuje cały obiekt, nie scala pól).
fn to_input(intent: &serde_json::Value, origin: DateTime<FixedOffset>) -> serde_json::Value {
    let slot = |v: &serde_json::Value| v.as_i64().map(|s| (origin + chrono::Duration::minutes(s * SLOT_MINUTES)).to_rfc3339());
    let window = &intent["daily_windows"][0];
    let mut out = serde_json::json!({
        "title": intent["title"],
        "kind": intent["kind"],
        "priority": intent["priority"],
        "minutes_per_period": intent["minutes_per_period"],
        "min_chunk_minutes": intent["min_chunk_minutes"],
        "max_chunk_minutes": intent["max_chunk_minutes"],
    });
    let o = out.as_object_mut().unwrap();
    for k in ["energy", "period_days", "max_per_day", "preferred_start_min"] {
        if !intent[k].is_null() {
            o.insert(k.into(), intent[k].clone());
        }
    }
    if window.is_object() {
        o.insert("window_start_min".into(), window["start_min"].clone());
        o.insert("window_end_min".into(), window["end_min"].clone());
    }
    if let Some(d) = slot(&intent["due_slot"]) {
        o.insert("due".into(), d.into());
    }
    if let Some(e) = slot(&intent["earliest_slot"]) {
        o.insert("earliest".into(), e.into());
    }
    out
}

/// Pobiera zadanie, zmienia je funkcją `f` i zapisuje (Horolog sam przeplanuje).
async fn update_intent(app: &AppHandle, id: &str, f: impl FnOnce(&mut serde_json::Value)) -> Result<(), String> {
    let intents = get_json(app, "/api/intents").await?;
    let intent = intents
        .as_array()
        .and_then(|a| a.iter().find(|i| i["id"].as_str() == Some(id)))
        .cloned()
        .ok_or("Horolog nie zna tego zadania")?;
    let plan = get_json(app, "/api/plan").await?;
    let origin = plan["origin"]
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .ok_or("Horolog nie podał początku planu")?;
    let mut body = to_input(&intent, origin);
    f(&mut body);
    let resp = client()
        .put(format!("{}/api/intents/{}", base(app), id))
        .timeout(Duration::from_secs(30))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let detail: serde_json::Value = resp.json().await.unwrap_or_default();
        return Err(detail["detail"].as_str().unwrap_or("Horolog odrzucił zmianę").to_string());
    }
    fetch_plan(app).await
}

/// Przesuwa blok: preferowana godzina + najwcześniej od tego dnia.
/// Horolog traktuje to jako życzenie — może dobrać najbliższe wolne miejsce.
pub async fn move_block(app: &AppHandle, intent_id: &str, start: DateTime<Local>) -> Result<(), String> {
    use chrono::Timelike;
    let minute = (start.hour() * 60 + start.minute()) as i64;
    let day_start = Local
        .from_local_datetime(&start.date_naive().and_hms_opt(0, 0, 0).unwrap_or_default())
        .single()
        .unwrap_or(start);
    let earliest = if day_start > Local::now() { day_start } else { Local::now() };
    update_intent(app, intent_id, |b| {
        b["preferred_start_min"] = minute.into();
        b["earliest"] = earliest.to_rfc3339().into();
        // Termin wcześniejszy niż nowy dzień zablokowałby przesunięcie.
        if let Some(due) = b["due"].as_str().and_then(|d| DateTime::parse_from_rfc3339(d).ok()) {
            if due.with_timezone(&Local) < start {
                b.as_object_mut().map(|o| o.remove("due"));
            }
        }
    })
    .await
}

/// Zmienia czas zadania/nawyku w skali (np. 1.5 = o połowę dłużej), w pełnych 15 min.
pub async fn scale_minutes(app: &AppHandle, intent_id: &str, chunk_minutes: i64) -> Result<(), String> {
    update_intent(app, intent_id, |b| {
        let round = |m: i64| ((m + 7) / 15 * 15).max(15);
        let per = b["minutes_per_period"].as_i64().unwrap_or(chunk_minutes);
        let old_max = b["max_chunk_minutes"].as_i64().unwrap_or(chunk_minutes).max(1);
        let ratio = chunk_minutes as f64 / old_max as f64;
        b["minutes_per_period"] = round((per as f64 * ratio) as i64).into();
        b["max_chunk_minutes"] = round(chunk_minutes).into();
        let min = b["min_chunk_minutes"].as_i64().unwrap_or(15);
        if min > chunk_minutes {
            b["min_chunk_minutes"] = round(chunk_minutes).into();
        }
    })
    .await
}

/// Godzina dzisiaj w lokalnej strefie (do testów reguł i paczek).
#[allow(dead_code)]
pub fn today_at(h: u32, m: u32) -> Option<DateTime<Local>> {
    let date = Local::now().date_naive();
    let t = NaiveTime::from_hms_opt(h, m, 0)?;
    Local.from_local_datetime(&date.and_time(t)).single()
}
