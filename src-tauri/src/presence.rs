//! Obecność („body doubling”): co 30 s wysyłamy swój stan na serwer obecności
//! (server/presence na home serwerze) i pobieramy stany pozostałych osób.
//! W pasku widać, kto teraz pracuje — i oni widzą Ciebie.

use crate::state;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::AppHandle;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Person {
    pub name: String,
    /// focus | available | away | offline
    pub state: String,
    pub task: String,
    pub until: String,
    pub seen: i64,
}

fn my_state(app: &AppHandle) -> (String, String, String) {
    let cfg = state::cfg(app);
    if let Some(task) = crate::focus::focusing(app) {
        let until = state::read(app, |rt| rt.adhoc.as_ref().and_then(|a| a.end))
            .or_else(|| crate::engine::current(app).map(|c| c.end))
            .map(|t| t.to_rfc3339())
            .unwrap_or_default();
        let shown = if cfg.presence.share_task { task } else { String::new() };
        return ("focus".into(), shown, until);
    }
    if crate::platform::idle_secs() > 5 * 60 {
        return ("away".into(), String::new(), String::new());
    }
    ("available".into(), String::new(), String::new())
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(5)).build().unwrap_or_default();
        loop {
            let cfg = state::cfg(&app);
            let p = cfg.presence.clone();
            if p.enabled && !p.url.trim().is_empty() {
                let base = p.url.trim().trim_end_matches('/').to_string();
                let name = if p.name.trim().is_empty() { std::env::var("USERNAME").unwrap_or_else(|_| "Ja".into()) } else { p.name.trim().to_string() };
                let (st, task, until) = my_state(&app);
                let auth = |r: reqwest::RequestBuilder| if p.token.is_empty() { r } else { r.bearer_auth(&p.token) };
                let _ = auth(client.post(format!("{base}/status")))
                    .timeout(Duration::from_secs(8))
                    .json(&serde_json::json!({ "name": name, "state": st, "task": task, "until": until }))
                    .send()
                    .await;
                let people = match auth(client.get(format!("{base}/status"))).timeout(Duration::from_secs(8)).send().await {
                    Ok(r) if r.status().is_success() => r.json::<Vec<Person>>().await.ok(),
                    _ => None,
                };
                let me = name.clone();
                state::update(&app, false, |rt| {
                    rt.presence = people.map(|v| v.into_iter().filter(|x| x.name != me).collect()).unwrap_or_default();
                    rt.presence_at = Some(Local::now());
                });
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}
