//! Pogoda z Open-Meteo (bez klucza API) i kursy walut z NBP.
//!
//! Obie pętle chodzą w tle i zapisują wynik do stanu (`Runtime.weather`,
//! `Runtime.rates`), więc dock i paleta tylko czytają gotowe dane — żadne
//! żądanie HTTP nie blokuje interfejsu. Gdy lat/lon puste, moduł śpi i
//! sprawdza co pół godziny, czy konfiguracja się pojawiła.

use crate::state;
use chrono::TimeZone;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::AppHandle;

/// Ostatnia pogoda — trafia do snapshotu jako `weather`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Weather {
    /// Temperatura w °C.
    pub temp: f64,
    /// Podpis z konfiguracji (miasto).
    pub label: String,
    /// Tekst po polsku, np. `deszcz · ok. 16:00`.
    pub text: String,
    pub emoji: String,
}

#[derive(Deserialize)]
struct MeteoOut {
    current: MeteoNow,
    #[serde(default)]
    hourly: MeteoHourly,
}

#[derive(Deserialize)]
struct MeteoNow {
    temperature_2m: f64,
    weather_code: i64,
    #[serde(default)]
    time: String,
}

#[derive(Default, Deserialize)]
struct MeteoHourly {
    #[serde(default)]
    time: Vec<String>,
    #[serde(default)]
    precipitation: Vec<f64>,
}

#[derive(Deserialize)]
struct NbpTable {
    #[serde(default)]
    rates: Vec<NbpRate>,
}

#[derive(Deserialize)]
struct NbpRate {
    code: String,
    mid: f64,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default()
}

/// Kod pogody WMO → (emoji, opis po polsku).
fn describe(code: i64) -> (&'static str, &'static str) {
    match code {
        0 => ("☀️", "słonecznie"),
        1 => ("🌤️", "mało chmur"),
        2 => ("⛅", "przejściowe zachmurzenie"),
        3 => ("☁️", "pochmurno"),
        45 | 48 => ("🌫️", "mgła"),
        51 | 53 | 55 => ("🌦️", "mżawka"),
        56 | 57 => ("🌧️", "mżawka marznąca"),
        61 | 63 | 65 => ("🌧️", "deszcz"),
        66 | 67 => ("🌧️", "deszcz marznący"),
        71 | 73 | 75 | 77 => ("❄️", "śnieg"),
        80 | 81 | 82 => ("🌦️", "przelotne deszcze"),
        85 | 86 => ("🌨️", "opady śniegu"),
        95 => ("⛈️", "burza"),
        96 | 99 => ("⛈️", "burza z gradem"),
        _ => ("🌡️", "pogoda"),
    }
}

/// Godzina najbliższych opadów (>0,2 mm) w perspektywie 12 h, np. `16:00`.
fn rain_at(hourly: &MeteoHourly, now: &chrono::DateTime<chrono::Local>) -> Option<String> {
    for (i, t) in hourly.time.iter().enumerate() {
        let ok = hourly.precipitation.get(i).copied().unwrap_or(0.0) > 0.2;
        if !ok {
            continue;
        }
        let Ok(when) = chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M") else { continue };
        let when = chrono::Local.from_local_datetime(&when).single()?;
        if when > *now && when < *now + chrono::Duration::hours(12) {
            return Some(when.format("%H:%M").to_string());
        }
    }
    None
}

async fn fetch_one(app: &AppHandle) -> Result<(), String> {
    let (lat, lon, label) = {
        let cfg = state::cfg(app);
        (cfg.weather.lat.trim().to_string(), cfg.weather.lon.trim().to_string(), cfg.weather.label.clone())
    };
    if lat.is_empty() || lon.is_empty() {
        return Err("brak lokalizacji".into());
    }
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}\
         &current=temperature_2m,weather_code&hourly=precipitation&forecast_days=2&timezone=auto"
    );
    let out: MeteoOut = client()
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

    let now = chrono::Local::now();
    let (emoji, desc) = describe(out.current.weather_code);
    let text = match rain_at(&out.hourly, &now) {
        Some(h) if desc.contains("deszcz") || desc.contains("mżawka") => format!("{desc} · już {h}"),
        Some(h) => format!("{desc} · deszcz ~{h}"),
        None => desc.to_string(),
    };
    let weather = Weather { temp: out.current.temperature_2m, label: if label.is_empty() { "Pogoda".into() } else { label }, text, emoji: emoji.into() };
    state::update(app, false, |rt| rt.weather = Some(weather));
    Ok(())
}

/// Pętla pogodowa: co 30 minut, a po błędzie próba co 5 minut.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            match fetch_one(&app).await {
                Ok(()) => tokio::time::sleep(Duration::from_secs(1800)).await,
                Err(_) => tokio::time::sleep(Duration::from_secs(300)).await,
            }
        }
    });
}

/// Odświeżenie na żądanie (paleta: Enter na „pog”).
pub async fn refresh(app: AppHandle) {
    let _ = fetch_one(&app).await;
}

/// Kursy walut z NBP (tabela A): kod → PLN za 1 jednostkę.
async fn fetch_rates() -> Result<std::collections::HashMap<String, f64>, String> {
    let url = "https://api.nbp.pl/api/exchangerates/tables/a/?format=json";
    let tables: Vec<NbpTable> = client()
        .get(url)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let mut rates: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    rates.insert("PLN".into(), 1.0);
    for r in tables.into_iter().next().map(|t| t.rates).unwrap_or_default() {
        rates.insert(r.code, r.mid);
    }
    Ok(rates)
}

/// Pętla kursów: odświeżanie co 6 h, po błędzie próba co 15 min.
pub fn start_rates(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            match fetch_rates().await {
                Ok(rates) => {
                    if rates.len() > 1 {
                        state::update(&app, false, |rt| rt.rates = rates);
                    }
                    tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
                }
                Err(_) => tokio::time::sleep(Duration::from_secs(900)).await,
            }
        }
    });
}
