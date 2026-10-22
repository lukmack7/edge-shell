//! Statystyki do górnego paska. Liczone w osobnym wątku i wysyłane tylko do
//! paska (zdarzenie `stats`), żeby nie przerysowywać co sekundę wszystkich okien.

use crate::state;
use crate::sys::{self, Stats};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

static LATEST: Mutex<Option<Stats>> = Mutex::new(None);

pub fn latest() -> Stats {
    LATEST.lock().ok().and_then(|g| g.clone()).unwrap_or_default()
}

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut prev_cpu = sys::cpu_times();
        let mut prev_net = sys::net_bytes();
        let mut prev_at = Instant::now();
        let mut stats = Stats::default();
        let mut tick: u64 = 0;
        loop {
            std::thread::sleep(Duration::from_millis(1000));
            tick += 1;
            let cfg = state::cfg(&app);

            // Pasek zadań Windows potrafi wrócić (np. po restarcie Eksploratora).
            if cfg.shell.hide_taskbar && tick % 2 == 0 && sys::taskbar_visible() {
                let restore = state::read(&app, |rt| rt.store.taskbar_state);
                sys::hide_taskbar(true, restore);
            }

            let now = Instant::now();
            let secs = now.duration_since(prev_at).as_secs_f64().max(0.2);
            prev_at = now;

            if let (Some((i1, k1, u1)), Some((i0, k0, u0))) = (sys::cpu_times(), prev_cpu) {
                let idle = i1.saturating_sub(i0) as f64;
                let total = (k1.saturating_sub(k0) + u1.saturating_sub(u0)) as f64;
                if total > 0.0 {
                    stats.cpu = Some((((total - idle) / total) * 100.0).clamp(0.0, 100.0) as f32);
                }
                prev_cpu = Some((i1, k1, u1));
            } else {
                prev_cpu = sys::cpu_times();
            }

            if let Some((pct, used, total)) = sys::memory() {
                stats.mem = Some(pct);
                stats.mem_used_gb = Some(used);
                stats.mem_total_gb = Some(total);
            }

            let net = sys::net_bytes();
            if let (Some((d1, u1)), Some((d0, u0))) = (net, prev_net) {
                stats.down_bps = Some((d1.saturating_sub(d0) as f64 / secs) as u64);
                stats.up_bps = Some((u1.saturating_sub(u0) as f64 / secs) as u64);
            }
            prev_net = net;

            if tick % 2 == 1 {
                if let Some((v, m)) = sys::volume() {
                    stats.volume = Some(v);
                    stats.muted = m;
                }
            }
            if tick % 15 == 1 {
                let (ssid, signal) = sys::wifi();
                stats.wifi_ssid = ssid;
                stats.wifi_signal = signal;
                let (present, connected) = sys::bluetooth();
                stats.bt_present = present;
                stats.bt_connected = connected;
            }
            if let Some((b, c)) = state::read(&app, |rt| rt.battery) {
                stats.battery = Some(b);
                stats.charging = c;
            } else {
                stats.battery = None;
            }

            if let Ok(mut g) = LATEST.lock() {
                *g = Some(stats.clone());
            }
            let _ = app.emit("stats", &stats);
        }
    });
}

/// Odświeża sam dźwięk od razu po zmianie (bez czekania na tick).
pub fn refresh_volume(app: &AppHandle) {
    if let Some((v, m)) = sys::volume() {
        let snapshot = {
            let mut g = match LATEST.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            let s = g.get_or_insert_with(Stats::default);
            s.volume = Some(v);
            s.muted = m;
            s.clone()
        };
        let _ = app.emit("stats", &snapshot);
    }
}
