//! The first screen: which daemon to attach to.
//!
//! Prefilled with the last address that worked (see `settings`). Probing
//! happens off the UI thread; when the daemon answers, the address is saved
//! and the real app takes over the window. When the address is local and no
//! daemon answers, one is started here (unless COMMANDER_SPAWN=0).

use crate::{api, app::CommanderApp, ctrl, settings};
use eframe::egui::{self, Align2, Color32, FontId, Key, RichText, Vec2};
use std::sync::mpsc::{channel, Receiver};

enum Screen {
    Connect,
    Running(CommanderApp),
}

pub struct ConnectApp {
    screen: Screen,
    addr: String,
    error: Option<String>,
    probe: Option<Receiver<Result<String, String>>>,
    ui_addr: String,
    spawn_daemon: bool,
    /// connect without waiting for a click (COMMANDER_API was given)
    auto: bool,
}

impl ConnectApp {
    pub fn new(cc: &eframe::CreationContext<'_>, addr: String, ui_addr: String, spawn_daemon: bool, auto: bool) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        eprintln!("connect screen: {} ({})", addr, if auto { "auto" } else { "waiting for the user" });
        ConnectApp { screen: Screen::Connect, addr, error: None, probe: None, ui_addr, spawn_daemon, auto }
    }

    fn start_probe(&mut self) {
        let base = settings::normalize(&self.addr);
        self.addr = base.clone();
        self.error = None;
        let spawn = self.spawn_daemon && settings::is_local(&base);
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let r = if api::ensure_daemon(&base, spawn) {
                Ok(base)
            } else if spawn {
                Err(format!("no daemon at {} and none could be started", base))
            } else {
                Err(format!("no daemon answers at {}", base))
            };
            let _ = tx.send(r);
        });
        self.probe = Some(rx);
    }

    fn attach(&mut self, ctx: &egui::Context, base: String) {
        settings::save(&settings::Settings { api: base.clone() });
        let ctrl_rx = ctrl::spawn(self.ui_addr.clone(), base.clone());
        let client = api::Client::connect(base);
        self.screen = Screen::Running(CommanderApp::new(ctx, ctrl_rx, client));
    }

    fn connect_ui(&mut self, ctx: &egui::Context) {
        if self.auto {
            self.auto = false;
            self.start_probe();
        }
        if let Some(rx) = &self.probe {
            match rx.try_recv() {
                Ok(Ok(base)) => {
                    self.probe = None;
                    self.attach(ctx, base);
                    return;
                }
                Ok(Err(e)) => {
                    self.probe = None;
                    self.error = Some(e);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.probe = None;
                    self.error = Some("probe thread died".into());
                }
            }
        }
        let busy = self.probe.is_some();
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::from_rgb(0x0b, 0x12, 0x0d))).show(ctx, |ui| {
            let rect = ui.max_rect();
            let painter = ui.painter();
            painter.text(
                rect.center() - Vec2::new(0.0, 120.0),
                Align2::CENTER_CENTER,
                "COMMANDER",
                FontId::proportional(34.0),
                Color32::from_rgb(0x8a, 0xe2, 0x8a),
            );
            painter.text(
                rect.center() - Vec2::new(0.0, 84.0),
                Align2::CENTER_CENTER,
                "connect to a commanderd",
                FontId::proportional(15.0),
                Color32::from_rgb(0x6f, 0x8f, 0x72),
            );
            let panel = egui::Rect::from_center_size(rect.center(), Vec2::new(420.0, 130.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(panel).layout(egui::Layout::top_down(egui::Align::Min)));
            child.label(RichText::new("daemon address (host[:port] or url)").color(Color32::from_rgb(0xa8, 0xc2, 0xab)));
            child.add_space(4.0);
            let mut go = false;
            child.add_enabled_ui(!busy, |ui| {
                let resp = ui.add(egui::TextEdit::singleline(&mut self.addr).desired_width(f32::INFINITY).font(FontId::monospace(16.0)).hint_text(settings::DEFAULT_API));
                if !busy && self.error.is_none() && !resp.has_focus() && self.probe.is_none() && ui.memory(|m| m.focused().is_none()) {
                    resp.request_focus();
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    go = true;
                }
            });
            child.add_space(10.0);
            child.horizontal(|ui| {
                if ui.add_enabled(!busy, egui::Button::new(RichText::new("  connect  ").size(16.0))).clicked() {
                    go = true;
                }
                if busy {
                    ui.spinner();
                    ui.label(RichText::new(format!("reaching {} …", self.addr)).color(Color32::from_rgb(0x6f, 0x8f, 0x72)));
                }
            });
            if let Some(e) = &self.error {
                child.add_space(8.0);
                child.label(RichText::new(e).color(Color32::from_rgb(0xe8, 0x7a, 0x6a)));
            }
            if go {
                self.start_probe();
            }
        });
    }
}

impl eframe::App for ConnectApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        match &mut self.screen {
            Screen::Running(app) => app.update(ctx, frame),
            Screen::Connect => self.connect_ui(ctx),
        }
    }
}
