//! commander — the Commander frontend.
//!
//! A view onto one daemon (`commanderd`). Env:
//!   COMMANDER_API      daemon url (default http://127.0.0.1:7700)
//!   COMMANDER_UI_HTTP  this frontend's control api (default 127.0.0.1:7701)
//!   COMMANDER_SPAWN=0  never start a daemon when none answers
//! When no daemon answers, one is started detached (it outlives this window)
//! with COMMANDER_HTTP set from COMMANDER_API; COMMANDER_SPACE passes through.

mod api;
mod app;
mod ctrl;

fn main() -> eframe::Result {
    // the app usually runs inside a nested weston whose clipboard is isolated
    // from the host X11 session the user copies from. the launcher strips
    // DISPLAY so winit picks wayland; put the host display back (winit still
    // prefers WAYLAND_DISPLAY) so the app can reach the host clipboard.
    // must happen before any thread is spawned.
    if std::env::var_os("DISPLAY").map_or(true, |v| v.is_empty()) {
        let host = std::env::var_os("COMMANDER_HOST_DISPLAY").unwrap_or_else(|| ":0".into());
        std::env::set_var("DISPLAY", host);
    }
    let backend = std::env::var("COMMANDER_API").unwrap_or_else(|_| "http://127.0.0.1:7700".into());
    let backend = backend.trim_end_matches('/').to_string();
    let spawn = std::env::var("COMMANDER_SPAWN").map_or(true, |v| v != "0");
    if !api::ensure_daemon(&backend, spawn) {
        eprintln!("no daemon at {} — the view stays empty until one answers", backend);
    }
    let ui_addr = std::env::var("COMMANDER_UI_HTTP").unwrap_or_else(|_| "127.0.0.1:7701".into());
    let ctrl_rx = ctrl::spawn(ui_addr, backend.clone());
    let client = api::Client::connect(backend);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 900.0])
            .with_title("Commander — RTS Context HQ (POC)"),
        ..Default::default()
    };
    eframe::run_native(
        "commander-poc",
        options,
        Box::new(|cc| Ok(Box::new(app::CommanderApp::new(cc, ctrl_rx, client)))),
    )
}
