mod app;
mod codex;
mod ctrl;
mod model;
mod store;
mod wasm;
mod worker;

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
    let addr = std::env::var("COMMANDER_HTTP").unwrap_or_else(|_| "127.0.0.1:7700".into());
    let ctrl_rx = ctrl::spawn(addr);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 900.0])
            .with_title("Commander — RTS Context HQ (POC)"),
        ..Default::default()
    };
    eframe::run_native(
        "commander-poc",
        options,
        Box::new(|cc| Ok(Box::new(app::CommanderApp::new(cc, ctrl_rx)))),
    )
}
