//! Codex subscription usage — a supply counter for the HQ.
//!
//! A background thread polls the newest codex session rollout
//! (~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl) once a minute, pulls the last
//! `rate_limits` event, and shares the weekly limit as (percent left, reset
//! time). The topbar renders it top-center like an RTS resource.

use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub struct Usage {
    pub pct_left: f32,
    pub resets_at: i64, // unix seconds
    pub window_minutes: i64,
    /// mtime of the rollout the numbers came from (unix seconds) — staleness
    pub read_from: i64,
}

pub type Shared = Arc<Mutex<Option<Usage>>>;

pub fn spawn() -> Shared {
    let shared: Shared = Arc::new(Mutex::new(None));
    let out = shared.clone();
    std::thread::spawn(move || {
        let mut last_mtime = 0i64;
        loop {
            if let Some((path, mtime)) = newest_rollout() {
                if mtime != last_mtime {
                    if let Some(u) = read_usage(&path, mtime) {
                        last_mtime = mtime;
                        *out.lock().unwrap() = Some(u);
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(60));
        }
    });
    shared
}

fn sessions_dir() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(std::env::var("HOME").ok()?).join(".codex/sessions"))
}

/// newest rollout-*.jsonl under sessions/YYYY/MM/DD (by mtime)
fn newest_rollout() -> Option<(std::path::PathBuf, i64)> {
    let mut best: Option<(std::path::PathBuf, i64)> = None;
    let mut stack = vec![sessions_dir()?];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "jsonl") {
                let mtime = md
                    .modified()
                    .ok()
                    .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                if best.as_ref().map_or(true, |(_, bm)| mtime > *bm) {
                    best = Some((path, mtime));
                }
            }
        }
    }
    best
}

/// last rate_limits event in the rollout → weekly usage snapshot
fn read_usage(path: &std::path::Path, mtime: i64) -> Option<Usage> {
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().rev().find(|l| l.contains("\"rate_limits\""))?;
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let rl = &v["payload"]["rate_limits"];
    // prefer the window closest to weekly (10080 min); fall back to primary
    let pick = |w: &serde_json::Value| -> Option<(f32, i64, i64)> {
        Some((
            w.get("used_percent")?.as_f64()? as f32,
            w.get("resets_at").and_then(|x| x.as_i64()).unwrap_or(0),
            w.get("window_minutes").and_then(|x| x.as_i64()).unwrap_or(0),
        ))
    };
    let primary = pick(&rl["primary"]);
    let secondary = pick(&rl["secondary"]);
    let (used, resets_at, window) = match (primary, secondary) {
        (Some(p), Some(s)) => {
            if (s.2 - 10080).abs() < (p.2 - 10080).abs() {
                s
            } else {
                p
            }
        }
        (Some(p), None) => p,
        (None, Some(s)) => s,
        (None, None) => return None,
    };
    Some(Usage {
        pct_left: (100.0 - used).clamp(0.0, 100.0),
        resets_at,
        window_minutes: window,
        read_from: mtime,
    })
}

/// "6d 23h" / "23h 12m" / "55m" until the reset
pub fn eta(resets_at: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let s = (resets_at - now).max(0);
    let (d, h, m) = (s / 86400, (s % 86400) / 3600, (s % 3600) / 60);
    if d > 0 {
        format!("{}d {}h", d, h)
    } else if h > 0 {
        format!("{}h {}m", h, m)
    } else {
        format!("{}m", m)
    }
}
