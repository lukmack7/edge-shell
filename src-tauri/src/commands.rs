//! Komendy wywoływane z okien (przez `invoke`).

use crate::config::{expand_env, Config};
use crate::state::{self, Adhoc, ShelfItem, Stash, StashWindow};
use crate::{engine, horolog, layout, platform, radio, spotify};
use chrono::{Duration as CDuration, Local};
use serde::Serialize;
use serde_json::Value;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

type Res<T> = Result<T, String>;

#[tauri::command]
pub fn get_snapshot(app: AppHandle) -> Value {
    state::snapshot(&app)
}

#[tauri::command]
pub async fn set_mode(app: AppHandle, id: String) -> Res<()> {
    let cfg = state::cfg(&app);
    if !cfg.modes.iter().any(|m| m.id == id) {
        return Err(format!("nieznany tryb {id}"));
    }
    state::update(&app, true, |rt| rt.store.mode = id.clone());
    let mode = cfg.mode(&id);
    if !mode.quiet {
        radio::release_batch(&app);
    }
    // Tapeta i jasność trybu — wolne wywołania, więc w tle.
    let (wallpaper, brightness) = (expand_env(&mode.wallpaper), mode.brightness);
    std::thread::spawn(move || {
        if !wallpaper.trim().is_empty() {
            crate::sys::set_wallpaper(&wallpaper);
        }
        if let Some(b) = brightness {
            crate::sys::set_brightness(b);
        }
    });
    let spotify_on = !cfg.spotify.client_id.is_empty() && state::read(&app, |rt| rt.store.spotify.is_some());
    if spotify_on && cfg.spotify.play_on_mode_change && !mode.playlist.is_empty() {
        spotify::play_playlist(&app, &mode.playlist, None).await;
    }
    Ok(())
}

#[tauri::command]
pub fn toggle_panel(app: AppHandle, label: String) {
    layout::toggle_panel(&app, &label);
}

#[tauri::command]
pub fn set_panel(app: AppHandle, label: String, open: bool) {
    layout::set_panel(&app, &label, open);
}

#[tauri::command]
pub fn hide_window(app: AppHandle, label: String) {
    if label == "drawer" || label == "now" {
        layout::set_panel(&app, &label, false);
    } else {
        layout::hide_window(&app, &label);
    }
}

#[tauri::command]
pub fn show_capture(app: AppHandle) {
    layout::show_window(&app, "capture", true);
}

#[tauri::command]
pub fn dock_resize(app: AppHandle, width: f64) {
    layout::place_dock(&app, Some(width));
}

fn open_target(app: &AppHandle, path: &str, args: &[String]) -> Res<()> {
    let target = expand_env(path);
    if !args.is_empty() {
        std::process::Command::new(&target).args(args).spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }
    let is_uri = target.starts_with("http://")
        || target.starts_with("https://")
        || (target.contains(':') && !target.contains('\\') && !target.contains('/'));
    if is_uri {
        app.opener().open_url(target, None::<&str>).map_err(|e| e.to_string())
    } else {
        app.opener().open_path(target, None::<&str>).map_err(|e| e.to_string())
    }
}

/// Uruchamia aplikację z docka. Wyciszona w bieżącym trybie → żółta flaga.
#[tauri::command]
pub fn launch(app: AppHandle, id: String) -> Res<String> {
    let cfg = state::cfg(&app);
    let Some(entry) = cfg.dock.iter().find(|a| a.id == id) else { return Err("nie ma takiej aplikacji".into()) };
    let mode = cfg.mode(&state::read(&app, |rt| rt.store.mode.clone()));
    if mode.dim.contains(&id) {
        state::update(&app, false, |rt| rt.friction_app = Some(id.clone()));
        layout::show_window(&app, "friction", true);
        return Ok("friction".into());
    }
    // Aplikacja już działa: przełącz do jej okna (kolejne kliknięcia przechodzą
    // po oknach), zamiast otwierać nową instancję.
    let wins = crate::previews::windows_of(&app, &id);
    if !wins.is_empty() {
        let fg = platform::foreground_hwnd();
        let next = match wins.iter().position(|w| w.hwnd == fg) {
            Some(i) => &wins[(i + 1) % wins.len()],
            None => &wins[0],
        };
        crate::sys::activate(next.hwnd);
        return Ok("activated".into());
    }
    open_target(&app, &entry.path, &entry.args)?;
    Ok("ok".into())
}

/// Nowe okno aplikacji (środkowy klik albo Shift+klik w docku).
#[tauri::command]
pub fn launch_new(app: AppHandle, id: String) -> Res<()> {
    let cfg = state::cfg(&app);
    let Some(entry) = cfg.dock.iter().find(|a| a.id == id) else { return Err("nie ma takiej aplikacji".into()) };
    open_target(&app, &entry.path, &entry.args)
}

#[tauri::command]
pub fn launch_force(app: AppHandle) -> Res<()> {
    let id = state::update(&app, false, |rt| rt.friction_app.take());
    layout::hide_window(&app, "friction");
    let cfg = state::cfg(&app);
    if let Some(entry) = id.and_then(|id| cfg.dock.into_iter().find(|a| a.id == id)) {
        open_target(&app, &entry.path, &entry.args)?;
    }
    Ok(())
}

#[tauri::command]
pub fn friction_close(app: AppHandle) {
    state::update(&app, false, |rt| rt.friction_app = None);
    layout::hide_window(&app, "friction");
}

#[tauri::command]
pub async fn friction_to_buffer(app: AppHandle) -> Res<String> {
    let cfg = state::cfg(&app);
    let id = state::update(&app, false, |rt| rt.friction_app.take());
    layout::hide_window(&app, "friction");
    let name = id
        .and_then(|id| cfg.dock.iter().find(|a| a.id == id).map(|a| a.name.clone()))
        .unwrap_or_else(|| "aplikacja".into());
    horolog::capture(&app, format!("{name} — 15 minut w najbliższej przerwie")).await
}

#[tauri::command]
pub fn open_path(app: AppHandle, path: String) -> Res<()> {
    open_target(&app, &path, &[])
}

#[tauri::command]
pub fn open_horolog(app: AppHandle) -> Res<()> {
    let url = state::cfg(&app).horolog.web_url;
    open_target(&app, &url, &[])
}

#[tauri::command]
pub async fn capture(app: AppHandle, text: String) -> Res<String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("Pusta myśl".into());
    }
    horolog::capture(&app, text).await
}

#[tauri::command]
pub async fn replan(app: AppHandle) -> Res<()> {
    horolog::replan(&app).await
}

#[tauri::command]
pub async fn focus_toggle_pause(app: AppHandle) -> Res<bool> {
    let paused = state::update(&app, false, |rt| {
        if let Some(since) = rt.paused_since.take() {
            // Wznowienie sesji ad hoc przesuwa jej koniec o czas pauzy.
            if let Some(a) = rt.adhoc.as_mut() {
                if let Some(end) = a.end {
                    a.end = Some(end + (Local::now() - since));
                }
            }
            false
        } else {
            rt.paused_since = Some(Local::now());
            true
        }
    });
    let cfg = state::cfg(&app);
    if !cfg.spotify.client_id.is_empty() && state::read(&app, |rt| rt.store.spotify.is_some()) {
        let _ = spotify::control(&app, if paused { "pause" } else { "play" }, None).await;
    }
    Ok(paused)
}

#[tauri::command]
pub fn focus_start_adhoc(app: AppHandle, minutes: Option<i64>, title: Option<String>) {
    let cfg = state::cfg(&app);
    let mins = minutes.unwrap_or(cfg.focus.adhoc_minutes).clamp(5, 180);
    let now = Local::now();
    state::update(&app, false, |rt| {
        rt.adhoc = Some(Adhoc {
            title: title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "Fokus".into()),
            start: Some(now),
            end: Some(now + CDuration::minutes(mins)),
        });
        rt.paused_since = None;
    });
}

#[derive(Serialize)]
pub struct FinishResult {
    status: String,
    delta_min: i64,
}

#[tauri::command]
pub async fn focus_finish(app: AppHandle) -> Res<FinishResult> {
    let adhoc = state::update(&app, false, |rt| {
        rt.paused_since = None;
        rt.adhoc.take()
    });
    if adhoc.is_some() {
        radio::pit_wall(&app, "Meta! Sesja zakończona — zapisz jedno zdanie, gdzie skończyłeś.".into(), None);
        return Ok(FinishResult { status: "ontime".into(), delta_min: 0 });
    }
    let Some(item) = engine::current(&app) else { return Err("Teraz nic nie trwa".into()) };
    let mark = engine::mark_done(&app, &item);
    if let (Some(id), "task") = (&item.intent_id, item.kind.as_str()) {
        if let Err(err) = horolog::complete(&app, id).await {
            radio::pit_wall(&app, format!("Nie udało się odhaczyć w Horologu: {err}"), None);
        }
    }
    let msg = match mark.status.as_str() {
        "early" => format!("Fioletowy sektor! {} min przed planem. Masz zapas.", -mark.delta_min),
        "ontime" => "Zielony sektor — idealnie w czasie.".to_string(),
        _ => format!("Żółty sektor, +{} min. Horolog przesunie resztę.", mark.delta_min),
    };
    radio::pit_wall(&app, msg, None);
    Ok(FinishResult { status: mark.status, delta_min: mark.delta_min })
}

// --- Radio -----------------------------------------------------------------

#[tauri::command]
pub async fn radio_reply(app: AppHandle, text: String) -> Res<String> {
    radio::send_reply(&app, text).await
}

#[tauri::command]
pub fn radio_next(app: AppHandle) {
    radio::dismiss(&app, false);
}

/// Otwiera aplikację, z której przyszło powiadomienie, i zdejmuje kartę.
#[tauri::command]
pub fn radio_open(app: AppHandle) -> Res<()> {
    let target = state::read(&app, |rt| rt.showing.front().and_then(|m| m.open_target.clone()));
    radio::dismiss(&app, false);
    match target {
        Some(t) => app.opener().open_path(t, None::<&str>).map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

#[tauri::command]
pub fn radio_later(app: AppHandle) {
    radio::dismiss(&app, true);
}

#[tauri::command]
pub fn radio_show_waiting(app: AppHandle) {
    radio::release_batch(&app);
}

// --- Półka, schowek, konteksty ----------------------------------------------

#[tauri::command]
pub fn shelf_add(app: AppHandle, paths: Vec<String>) {
    state::update(&app, true, |rt| {
        for p in paths {
            if rt.store.shelf.iter().any(|s| s.path == p) {
                continue;
            }
            let name = p.rsplit(['\\', '/']).next().unwrap_or(&p).to_string();
            rt.store.shelf.push(ShelfItem { name, path: p });
        }
        let len = rt.store.shelf.len();
        if len > 12 {
            rt.store.shelf.drain(..len - 12);
        }
    });
}

#[tauri::command]
pub fn shelf_remove(app: AppHandle, path: String) {
    state::update(&app, true, |rt| rt.store.shelf.retain(|s| s.path != path));
}

#[tauri::command]
pub fn reveal(app: AppHandle, path: String) -> Res<()> {
    app.opener().reveal_item_in_dir(path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn copy_text(app: AppHandle, text: String) -> Res<()> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

/// Odkłada wszystkie otwarte okna jako jeden kontekst i je minimalizuje.
#[tauri::command]
pub fn stash_create(app: AppHandle, name: String) -> Res<usize> {
    let windows = platform::app_windows();
    if windows.is_empty() {
        return Err("Nie ma otwartych okien do odłożenia".into());
    }
    for w in &windows {
        platform::minimize(w.hwnd);
    }
    let count = windows.len();
    let name = if name.trim().is_empty() { format!("Kontekst {}", Local::now().format("%H:%M")) } else { name };
    state::update(&app, true, |rt| {
        rt.store.stashes.insert(
            0,
            Stash {
                id: format!("{:x}", Local::now().timestamp_millis()),
                name,
                created: Local::now(),
                windows: windows
                    .into_iter()
                    .map(|w| StashWindow { hwnd: w.hwnd, exe: w.exe, title: w.title })
                    .collect(),
            },
        );
        rt.store.stashes.truncate(8);
    });
    Ok(count)
}

/// Przywraca odłożony kontekst: istniejące okna wracają, zamknięte aplikacje
/// uruchamiamy ponownie.
#[tauri::command]
pub fn stash_restore(app: AppHandle, id: String) -> Res<()> {
    let stash = state::update(&app, true, |rt| {
        let pos = rt.store.stashes.iter().position(|s| s.id == id)?;
        Some(rt.store.stashes.remove(pos))
    });
    let Some(stash) = stash else { return Err("Nie ma takiego kontekstu".into()) };
    let mut relaunched = std::collections::HashSet::new();
    for w in stash.windows {
        if !platform::restore(w.hwnd) && !w.exe.is_empty() && relaunched.insert(w.exe.clone()) {
            let _ = app.opener().open_path(w.exe, None::<&str>);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn stash_delete(app: AppHandle, id: String) {
    state::update(&app, true, |rt| rt.store.stashes.retain(|s| s.id != id));
}

// --- Wyszukiwarka -------------------------------------------------------------

#[derive(Serialize, Clone)]
pub struct SearchEntry {
    name: String,
    path: String,
    kind: String,
}

/// Skróty z menu Start (użytkownika i wspólne) — do wyszukiwarki w szufladzie.
#[tauri::command]
pub fn search_index() -> Vec<SearchEntry> {
    let mut roots = Vec::new();
    if let Some(d) = dirs::config_dir() {
        roots.push(d.join("Microsoft\\Windows\\Start Menu\\Programs"));
    }
    if let Ok(pd) = std::env::var("ProgramData") {
        roots.push(std::path::PathBuf::from(pd).join("Microsoft\\Windows\\Start Menu\\Programs"));
    }
    let mut out = Vec::new();
    let mut stack = roots;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk") || x.eq_ignore_ascii_case("url")) {
                let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let lower = name.to_lowercase();
                if lower.contains("uninstall") || lower.contains("odinstaluj") {
                    continue;
                }
                out.push(SearchEntry { name, path: p.to_string_lossy().to_string(), kind: "app".into() });
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    out
}

// --- Spotify -------------------------------------------------------------------

#[tauri::command]
pub fn spotify_login(app: AppHandle) -> Res<()> {
    spotify::login(app)
}

#[tauri::command]
pub async fn spotify_control(app: AppHandle, action: String) -> Res<()> {
    spotify::control(&app, &action, None).await
}

// --- Konfiguracja ---------------------------------------------------------------

#[tauri::command]
pub fn open_config(app: AppHandle) -> Res<()> {
    let path = crate::config::config_path();
    app.opener()
        .open_path(path.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reload_config(app: AppHandle) {
    let cfg = Config::load();
    if let Ok(mut c) = state::shared(&app).cfg.write() {
        *c = cfg;
    }
    crate::hotkeys::register(&app);
    layout::arrange(&app);
    state::emit_snapshot(&app);
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    layout::release(&app);
    app.exit(0);
}
