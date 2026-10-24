//! Spotify przez Web API (logowanie PKCE, bez sekretu w aplikacji).
//!
//! Sterowanie odtwarzaniem w Web API wymaga konta Premium. Bez Premium widget
//! nadal pokazuje, co gra.

use crate::state::{self, SpotifyTokens};
use base64::Engine;
use chrono::Local;
use sha2::Digest;
use std::time::Duration;
use tauri::AppHandle;

const SCOPES: &str = "user-read-playback-state user-modify-playback-state user-read-currently-playing playlist-read-private playlist-read-collaborative user-library-read user-library-modify";

fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/callback")
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Otwiera przeglądarkę z logowaniem i czeka na powrót na lokalny port.
pub fn login(app: AppHandle) -> Result<(), String> {
    let cfg = state::cfg(&app).spotify;
    if cfg.client_id.is_empty() {
        return Err("Uzupełnij spotify.clientId w config.json".into());
    }
    let verifier: String = (0..64)
        .map(|_| {
            let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
            chars[rand::random::<usize>() % chars.len()] as char
        })
        .collect();
    let challenge = b64(&sha2::Sha256::digest(verifier.as_bytes()));
    let state_tok = b64(&rand::random::<[u8; 16]>());
    let redirect = redirect_uri(cfg.redirect_port);
    let url = format!(
        "https://accounts.spotify.com/authorize?response_type=code&client_id={}&scope={}&redirect_uri={}&code_challenge_method=S256&code_challenge={}&state={}",
        urlencoding::encode(&cfg.client_id),
        urlencoding::encode(SCOPES),
        urlencoding::encode(&redirect),
        challenge,
        state_tok
    );

    let server = tiny_http::Server::http(("127.0.0.1", cfg.redirect_port)).map_err(|e| e.to_string())?;
    {
        use tauri_plugin_opener::OpenerExt;
        app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())?;
    }

    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(300);
        while std::time::Instant::now() < deadline {
            let Ok(Some(req)) = server.recv_timeout(Duration::from_secs(5)) else { continue };
            let query = req.url().split_once('?').map(|(_, q)| q.to_string()).unwrap_or_default();
            let param = |name: &str| {
                query.split('&').find_map(|kv| {
                    let (k, v) = kv.split_once('=')?;
                    (k == name).then(|| urlencoding::decode(v).map(|s| s.into_owned()).unwrap_or_default())
                })
            };
            let ok = param("state").as_deref() == Some(state_tok.as_str());
            let code = param("code");
            let page = if ok && code.is_some() {
                "<h2 style='font-family:sans-serif'>Spotify połączone z Edge Shell. Możesz zamknąć tę kartę.</h2>"
            } else {
                "<h2 style='font-family:sans-serif'>Logowanie przerwane.</h2>"
            };
            let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).ok();
            let mut resp = tiny_http::Response::from_string(page);
            if let Some(h) = header {
                resp = resp.with_header(h);
            }
            let _ = req.respond(resp);
            if let (true, Some(code)) = (ok, code) {
                let app2 = app.clone();
                let verifier = verifier.clone();
                let redirect = redirect.clone();
                tauri::async_runtime::spawn(async move {
                    let params = [
                        ("grant_type", "authorization_code".to_string()),
                        ("code", code),
                        ("redirect_uri", redirect),
                        ("client_id", state::cfg(&app2).spotify.client_id),
                        ("code_verifier", verifier),
                    ];
                    if let Err(err) = token_request(&app2, &params).await {
                        state::update(&app2, false, |rt| rt.music.error = err);
                    }
                });
            }
            break;
        }
    });
    Ok(())
}

async fn token_request(app: &AppHandle, params: &[(&str, String)]) -> Result<(), String> {
    let resp: serde_json::Value = reqwest::Client::new()
        .post("https://accounts.spotify.com/api/token")
        .timeout(Duration::from_secs(15))
        .form(params)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let access = resp["access_token"].as_str().ok_or_else(|| {
        format!("Spotify: {}", resp["error_description"].as_str().unwrap_or("brak tokenu"))
    })?;
    let expires = resp["expires_in"].as_i64().unwrap_or(3600);
    let refresh = resp["refresh_token"].as_str().map(|s| s.to_string());
    let scope = resp["scope"].as_str().map(|s| s.to_string());
    state::update(app, true, |rt| {
        let old_refresh = rt.store.spotify.as_ref().map(|t| t.refresh_token.clone()).unwrap_or_default();
        let old_scope = rt.store.spotify.as_ref().map(|t| t.scope.clone()).unwrap_or_default();
        rt.store.spotify = Some(SpotifyTokens {
            access_token: access.to_string(),
            refresh_token: refresh.unwrap_or(old_refresh),
            expires_at: Local::now().timestamp() + expires - 60,
            scope: scope.unwrap_or(old_scope),
        });
        rt.music.connected = true;
        rt.music.error.clear();
    });
    Ok(())
}

async fn token(app: &AppHandle) -> Option<String> {
    let tokens = state::read(app, |rt| rt.store.spotify.clone())?;
    if tokens.expires_at > Local::now().timestamp() {
        return Some(tokens.access_token);
    }
    let params = [
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", tokens.refresh_token.clone()),
        ("client_id", state::cfg(app).spotify.client_id),
    ];
    if let Err(err) = token_request(app, &params).await {
        state::update(app, true, |rt| {
            rt.music.connected = false;
            rt.music.error = err;
            rt.store.spotify = None;
        });
        return None;
    }
    state::read(app, |rt| rt.store.spotify.as_ref().map(|t| t.access_token.clone()))
}

/// Wywołanie sterujące (play/pause/next...). Błąd zapisujemy, nie przerywamy.
pub async fn control(app: &AppHandle, action: &str, body: Option<serde_json::Value>) -> Result<(), String> {
    let Some(tok) = token(app).await else { return Err("Spotify nie jest połączone".into()) };
    let client = reqwest::Client::new();
    let base = "https://api.spotify.com/v1/me/player";
    let req = match action {
        "play" => client.put(format!("{base}/play")),
        "pause" => client.put(format!("{base}/pause")),
        "next" => client.post(format!("{base}/next")),
        "previous" => client.post(format!("{base}/previous")),
        a if a.starts_with("seek:") => client.put(format!("{base}/seek?position_ms={}", &a[5..])),
        a if a.starts_with("volume:") => client.put(format!("{base}/volume?volume_percent={}", &a[7..])),
        _ => return Err(format!("nieznana akcja {action}")),
    };
    let req = req.bearer_auth(tok).timeout(Duration::from_secs(10));
    let req = match body {
        Some(b) => req.json(&b),
        None => req.header("Content-Length", "0"),
    };
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.is_success() {
        poll_once(app).await;
        Ok(())
    } else {
        let msg = match status.as_u16() {
            403 => "Sterowanie wymaga Spotify Premium".to_string(),
            404 => "Brak aktywnego urządzenia — włącz Spotify".to_string(),
            _ => format!("Spotify: {status}"),
        };
        state::update(app, false, |rt| rt.music.error = msg.clone());
        Err(msg)
    }
}

pub async fn play_playlist(app: &AppHandle, uri: &str, volume: Option<u8>) {
    if uri.is_empty() {
        let _ = control(app, "play", None).await;
    } else {
        let _ = control(app, "play", Some(serde_json::json!({ "context_uri": uri }))).await;
    }
    if let Some(v) = volume {
        let _ = control(app, &format!("volume:{v}"), None).await;
    }
}

async fn poll_once(app: &AppHandle) {
    let Some(tok) = token(app).await else { return };
    let resp = reqwest::Client::new()
        .get("https://api.spotify.com/v1/me/player")
        .bearer_auth(tok)
        .timeout(Duration::from_secs(10))
        .send()
        .await;
    let Ok(resp) = resp else { return };
    if resp.status().as_u16() == 204 {
        state::update(app, false, |rt| {
            rt.music.playing = false;
            rt.music.connected = true;
        });
        return;
    }
    let Ok(v) = resp.json::<serde_json::Value>().await else { return };
    let item = &v["item"];
    let artists = item["artists"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    state::update(app, false, |rt| {
        rt.music.connected = true;
        rt.music.playing = v["is_playing"].as_bool().unwrap_or(false);
        rt.music.title = item["name"].as_str().unwrap_or("").to_string();
        rt.music.artist = artists;
        rt.music.art = item["album"]["images"][0]["url"].as_str().unwrap_or("").to_string();
        rt.music.progress_ms = v["progress_ms"].as_i64().unwrap_or(0);
        rt.music.duration_ms = item["duration_ms"].as_i64().unwrap_or(0);
        rt.music.fetched_at_ms = Local::now().timestamp_millis();
        rt.music.volume = v["device"]["volume_percent"].as_i64();
        rt.music.device = v["device"]["name"].as_str().unwrap_or("").to_string();
        rt.music.album = item["album"]["name"].as_str().unwrap_or("").to_string();
        rt.music.shuffle = v["shuffle_state"].as_bool().unwrap_or(false);
        rt.music.repeat = v["repeat_state"].as_str().unwrap_or("off").to_string();
        let uri = item["uri"].as_str().unwrap_or("").to_string();
        if rt.music.uri != uri {
            rt.music.liked = None;
        }
        rt.music.uri = uri;
    });
    // Serce: sprawdzamy raz na utwór.
    let (uri, known) = state::read(app, |rt| (rt.music.uri.clone(), rt.music.liked.is_some()));
    if !uri.is_empty() && !known {
        if let Ok(v) = get_json(app, &format!("https://api.spotify.com/v1/me/library/contains?uris={}", urlencoding::encode(&uri))).await {
            let liked = v.as_array().and_then(|a| a.first()).and_then(|b| b.as_bool());
            state::update(app, false, |rt| if rt.music.uri == uri { rt.music.liked = liked.or(Some(false)) });
        }
    }
}

async fn get_json(app: &AppHandle, url: &str) -> Result<serde_json::Value, String> {
    let Some(tok) = token(app).await else { return Err("Spotify nie jest połączone".into()) };
    let resp = reqwest::Client::new().get(url).bearer_auth(tok).timeout(Duration::from_secs(10)).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Spotify: {}", resp.status()));
    }
    resp.json().await.map_err(|e| e.to_string())
}

fn small_image(images: &serde_json::Value) -> String {
    images
        .as_array()
        .and_then(|a| a.iter().min_by_key(|i| i["width"].as_i64().unwrap_or(9999).abs_diff(64)))
        .and_then(|i| i["url"].as_str())
        .unwrap_or("")
        .to_string()
}

/// Dowolne wywołanie sterujące (PUT/DELETE) z kodem błędu po polsku.
pub async fn call(app: &AppHandle, method: reqwest::Method, url: &str, body: Option<serde_json::Value>) -> Result<(), String> {
    let Some(tok) = token(app).await else { return Err("Spotify nie jest połączone".into()) };
    let mut req = reqwest::Client::new().request(method, url).bearer_auth(tok).timeout(Duration::from_secs(10));
    req = match body {
        Some(b) => req.json(&b),
        None => req.header("Content-Length", "0"),
    };
    let resp = req.send().await.map_err(|e| e.to_string())?;
    match resp.status().as_u16() {
        200..=299 => Ok(()),
        401 | 403 => Err("Brak uprawnienia — połącz Spotify ponownie".into()),
        404 => Err("Brak aktywnego urządzenia — włącz Spotify".into()),
        s => Err(format!("Spotify: {s}")),
    }
}

/// Serce: zapisz / usuń bieżący utwór z biblioteki.
pub async fn like(app: &AppHandle, on: bool) -> Result<(), String> {
    let uri = state::read(app, |rt| rt.music.uri.clone());
    if uri.is_empty() {
        return Err("Nic nie gra".into());
    }
    let url = format!("https://api.spotify.com/v1/me/library?uris={}", urlencoding::encode(&uri));
    let method = if on { reqwest::Method::PUT } else { reqwest::Method::DELETE };
    call(app, method, &url, Some(serde_json::json!({ "uris": [uri] }))).await?;
    state::update(app, false, |rt| rt.music.liked = Some(on));
    Ok(())
}

pub async fn shuffle(app: &AppHandle, on: bool) -> Result<(), String> {
    call(app, reqwest::Method::PUT, &format!("https://api.spotify.com/v1/me/player/shuffle?state={on}"), None).await?;
    state::update(app, false, |rt| rt.music.shuffle = on);
    Ok(())
}

pub async fn repeat(app: &AppHandle, mode: &str) -> Result<(), String> {
    let mode = if matches!(mode, "off" | "context" | "track") { mode } else { "off" };
    call(app, reqwest::Method::PUT, &format!("https://api.spotify.com/v1/me/player/repeat?state={mode}"), None).await?;
    state::update(app, false, |rt| rt.music.repeat = mode.to_string());
    Ok(())
}

/// Kolejka: następne utwory (do 20).
pub async fn queue(app: &AppHandle) -> Result<Vec<serde_json::Value>, String> {
    let v = get_json(app, "https://api.spotify.com/v1/me/player/queue").await?;
    Ok(v["queue"]
        .as_array()
        .map(|a| {
            a.iter()
                .take(20)
                .map(|t| {
                    let artist = t["artists"].as_array().map(|x| x.iter().filter_map(|y| y["name"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_else(|| t["show"]["name"].as_str().unwrap_or("").to_string());
                    let art = if t["album"].is_object() { small_image(&t["album"]["images"]) } else { small_image(&t["images"]) };
                    serde_json::json!({ "title": t["name"], "artist": artist, "art": art, "uri": t["uri"], "ms": t["duration_ms"] })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Playlisty użytkownika (do 50) + czy trzeba połączyć ponownie, żeby widzieć prywatne.
pub async fn playlists(app: &AppHandle) -> Result<serde_json::Value, String> {
    let scope = state::read(app, |rt| rt.store.spotify.as_ref().map(|t| t.scope.clone()).unwrap_or_default());
    let needs_login = !scope.contains("playlist-read-private");
    let v = match get_json(app, "https://api.spotify.com/v1/me/playlists?limit=50").await {
        Ok(v) => v,
        // Token bez uprawnienia do playlist (logowanie sprzed tej funkcji) → poproś o ponowne połączenie.
        Err(e) if e.contains("403") || e.contains("401") => return Ok(serde_json::json!({ "items": [], "needsLogin": true })),
        Err(e) => return Err(e),
    };
    let items: Vec<serde_json::Value> = v["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|p| p.is_object())
                .map(|p| serde_json::json!({
                    "name": p["name"], "uri": p["uri"], "art": small_image(&p["images"]),
                    "owner": p["owner"]["display_name"],
                    // Od lutego 2026 Spotify zmienia nazwę pola `tracks` na `items`.
                    "tracks": if p["items"]["total"].is_number() { p["items"]["total"].clone() } else { p["tracks"]["total"].clone() },
                }))
                .collect()
        })
        .unwrap_or_default();
    Ok(serde_json::json!({ "items": items, "needsLogin": needs_login }))
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            if !state::cfg(&app).spotify.client_id.is_empty() && state::read(&app, |rt| rt.store.spotify.is_some()) {
                poll_once(&app).await;
            }
            tokio::time::sleep(Duration::from_secs(4)).await;
        }
    });
}
