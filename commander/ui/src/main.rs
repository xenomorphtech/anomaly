//! commander — the Commander frontend.
//!
//! A view onto one daemon (`commanderd`). It opens on a connect screen
//! prefilled with the last address used (saved in the user's config dir, see
//! `settings`). Env:
//!   COMMANDER_API      daemon url; when set, connects to it right away
//!   COMMANDER_UI_HTTP  this frontend's control api (default 127.0.0.1:7701)
//!   COMMANDER_SPAWN=0  never start a daemon when none answers
//! When the address is local and no daemon answers, one is started detached
//! (it outlives this window) with COMMANDER_HTTP set from the address;
//! COMMANDER_SPACE passes through.

mod api;
mod app;
mod connect;
mod ctrl;
mod settings;

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
    let env_api = std::env::var("COMMANDER_API").ok().filter(|v| !v.trim().is_empty());
    let auto = env_api.is_some();
    let saved = settings::load().api;
    let addr = env_api.unwrap_or(if saved.is_empty() { settings::DEFAULT_API.into() } else { saved });
    let spawn = std::env::var("COMMANDER_SPAWN").map_or(true, |v| v != "0");
    let ui_addr = std::env::var("COMMANDER_UI_HTTP").unwrap_or_else(|_| "127.0.0.1:7701".into());
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 900.0])
            .with_title("Commander — RTS Context HQ (POC)"),
        ..Default::default()
    };
    eframe::run_native(
        "commander-poc",
        options,
        Box::new(move |cc| Ok(Box::new(connect::ConnectApp::new(cc, addr, ui_addr, spawn, auto)))),
    )
}
