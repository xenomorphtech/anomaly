use eframe::egui::{
    self, Align2, Area, Button, CentralPanel, Color32, CornerRadius, CursorIcon, FontId, Frame,
    Id, Key, Margin, Painter, Pos2, Rect, RichText, ScrollArea, Sense, SidePanel, Stroke,
    StrokeKind, TopBottomPanel, pos2, vec2,
};

use crate::ctrl::{Cmd, CtrlReq};
use crate::model::*;
use std::sync::mpsc::Receiver;

// ---------- palette ----------
const BG: Color32 = Color32::from_rgb(0x07, 0x0b, 0x08);
const PANEL: Color32 = Color32::from_rgb(0x0d, 0x13, 0x0e);
const PANEL2: Color32 = Color32::from_rgb(0x11, 0x1a, 0x13);
const LINE: Color32 = Color32::from_rgb(0x1d, 0x2b, 0x20);
const LINE_HI: Color32 = Color32::from_rgb(0x3a, 0x55, 0x40);
const TXT: Color32 = Color32::from_rgb(0xc8, 0xd6, 0xc9);
const DIM: Color32 = Color32::from_rgb(0x6e, 0x82, 0x72);
const FAINT: Color32 = Color32::from_rgb(0x44, 0x54, 0x3f);
const GREEN: Color32 = Color32::from_rgb(0x7d, 0xff, 0x9a);
const GREEN_DK: Color32 = Color32::from_rgb(0x2f, 0x7a, 0x44);
const AMBER: Color32 = Color32::from_rgb(0xe8, 0xb2, 0x41);
const RED: Color32 = Color32::from_rgb(0xff, 0x5c, 0x5c);
const CYAN: Color32 = Color32::from_rgb(0x6f, 0xd8, 0xff);

const MMW: f32 = 216.0;
const MMH: f32 = 144.0;
const MMS: f32 = MMW / WW;

// ---------- building icon sheet (strategy_building.jpeg) ----------
// cell edges measured from the sheet's grid gaps; rows 0-2 hold the structure icons
const ICON_SHEET_W: f32 = 735.0;
const ICON_SHEET_H: f32 = 1200.0;
const ICON_COL_EDGES: [f32; 9] = [61.5, 138.0, 214.5, 291.5, 368.0, 445.0, 521.5, 598.0, 673.5];
const ICON_ROW_EDGES: [f32; 4] = [426.0, 494.0, 570.0, 645.0];
pub const ICON_COUNT: usize = 24; // 3 rows x 8 cols

fn icon_uv(idx: usize) -> Rect {
    let idx = idx % ICON_COUNT;
    let (r, c) = (idx / 8, idx % 8);
    let inset = 5.0; // trim the dark frame + neighbor slivers
    Rect::from_min_max(
        pos2(
            (ICON_COL_EDGES[c] + inset) / ICON_SHEET_W,
            (ICON_ROW_EDGES[r] + inset) / ICON_SHEET_H,
        ),
        pos2(
            (ICON_COL_EDGES[c + 1] - inset) / ICON_SHEET_W,
            (ICON_ROW_EDGES[r + 1] - inset) / ICON_SHEET_H,
        ),
    )
}

fn a(c: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

struct Camera {
    pos: Pos2,
    scale: f32,
    target_pos: Pos2,
    target_scale: f32,
}

struct Toast {
    head: String,
    body: String,
    sub: String,
    ok: bool,
    created: f64,
    proj: Option<usize>,
}

struct Ping {
    proj: usize,
    color: Color32,
    created: f64,
}

/// interior view of a single structure (entered by double-clicking it)
#[derive(Clone, Copy, PartialEq)]
enum SRoom {
    Pylon(usize, usize),    // (proj, task index)
    Question(usize, usize), // (proj, question index)
}

#[derive(Clone)]
enum Act {
    Focus { proj: usize, scale: f32, from_space: bool },
    SelectBase(usize),
    SelectCapture(usize),
    FileCapture { cap: usize, proj: usize },
    DiscardCapture(usize),
    Unit { proj: usize, agent: String },
    OpenDecision(usize),
    CommitDecision(usize, usize),
    CloseBriefing,
    OpenRecovery(usize),
    CloseRecovery,
    CycleIdle,
    OpenCapture,
    CommitCapture(String),
    PlaceBase(Pos2),
    CommitBase(String),
    DestroyBase(usize),
    PlacePylon(Pos2),
    CommitPylon(String),
    SetPylon(usize, usize, TaskState),
    /// send a unit to work pylon (proj, task) via codex
    Dispatch(usize, usize),
    HaltUnit(usize, String),
    PlaceQuestion(Pos2),
    CommitQuestion(String),
    ToggleQuestion(usize, usize),
    SetQuestion(usize, usize, bool),
    DestroyStructs,
    StartLink,
    ToggleLink(usize, usize),
    DeleteLink(usize),
    SpaceJump,
    Back,
    EnterInterior(usize),
    ExitInterior,
    MinimapGoto(Pos2),
    Center,
}

#[derive(Clone)]
enum ClickZone {
    FocusBase(usize),
    Unit(usize, String),
    Decision(usize),
    Capture(usize),
    Link(usize),
    Pylon(usize, usize),    // (proj, task index)
    Question(usize, usize), // (proj, question index)
    SetTask(usize, usize, TaskState), // state chip inside a pylon room
    SetQuest(usize, usize, bool),     // resolve chip inside a question room
    Dispatch(usize, usize),           // "dispatch worker" chip inside a pylon room
    HaltUnit(usize, String),          // "halt" chip for the unit working a pylon
    RailToggle,                       // show/hide the base enumeration rail
    ExitInterior,
}

pub struct CommanderApp {
    world: World,
    rt: Vec<Rt>,
    cam: Camera,
    sel: Option<usize>,
    interior: Option<usize>,
    back: Vec<(Pos2, f32, Option<usize>)>,
    space_crumb: Option<String>,
    unseen: Vec<usize>,
    idle_idx: usize,
    highlight: Option<(usize, String)>,
    toasts: Vec<Toast>,
    wpings: Vec<Ping>,
    mpings: Vec<Ping>,
    capture_open: bool,
    capture_text: String,
    capture_focus: bool,
    sel_capture: Option<usize>,
    build_open: bool,
    build_pos: Pos2,
    build_text: String,
    build_focus: bool,
    pylon_open: bool,
    pylon_pos: Pos2,
    pylon_text: String,
    pylon_focus: bool,
    /// next frame, give the structure room's brief editor keyboard focus (enter in a room)
    brief_focus: bool,
    /// the brief editor held focus last frame — an esc that just released it must
    /// not also leave the room
    brief_focused: bool,
    /// the host (X11) clipboard. eframe only talks to the wayland clipboard of
    /// the compositor it runs under, and the nested weston's clipboard is
    /// isolated from the host session — so ctrl+v would paste nothing and
    /// ctrl+c would copy into a void. None when the host display is unreachable.
    host_clip: Option<arboard::Clipboard>,
    quest_open: bool,
    quest_pos: Pos2,
    quest_text: String,
    quest_focus: bool,
    drag_pylon: Option<(usize, usize)>,
    drag_quest: Option<(usize, usize)>,
    build_menu: bool,
    sroom: Option<SRoom>,
    prefs: crate::store::Prefs,
    codex: crate::codex::Shared,    // codex subscription usage (supply counter)
    sel_structs: Vec<SRoom>,        // group-selected substructures (ctrl+drag band)
    band: Option<(Pos2, Pos2)>,     // in-progress ctrl+drag selection rectangle (screen)
    sdestroy_arm: Option<f64>,      // time D was pressed with a structure group selected
    link_from: Option<usize>,
    destroy_arm: Option<(usize, f64)>, // (base, time armed) — destroy needs a second press
    drag_base: Option<usize>,
    drag_capture: Option<usize>,
    briefing: Option<usize>,
    recovery: Option<usize>,
    now_min: f64,
    last_num: Option<usize>,
    last_num_t: f64,
    time: f64,
    started: bool,
    viewport: Rect,
    clicks: Vec<(Rect, ClickZone)>,
    acts: Vec<Act>,
    ctrl: Receiver<CtrlReq>,
    icons_tex: Option<egui::TextureHandle>,
    space_path: String,
    dirty: bool,
    last_save: f64,
    wasm: crate::wasm::Host,
    mod_status: std::collections::HashMap<(String, String), crate::wasm::ModStatus>,
    last_wasm_sync: f64,
    /// codex worker processes (one turn per unit at a time)
    workers: crate::worker::Host,
}

impl CommanderApp {
    pub fn new(cc: &eframe::CreationContext<'_>, ctrl: Receiver<CtrlReq>) -> Self {
        let mut v = egui::Visuals::dark();
        v.panel_fill = PANEL;
        v.window_fill = Color32::from_rgb(0x0f, 0x18, 0x10);
        v.window_stroke = Stroke::new(1.0, LINE_HI);
        v.widgets.noninteractive.fg_stroke.color = TXT;
        v.widgets.noninteractive.bg_stroke.color = LINE;
        v.widgets.inactive.bg_fill = Color32::from_rgb(0x17, 0x23, 0x1a);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x17, 0x23, 0x1a);
        v.widgets.inactive.fg_stroke.color = Color32::from_rgb(0xa8, 0xc2, 0xab);
        v.widgets.hovered.bg_fill = Color32::from_rgb(0x1d, 0x2d, 0x20);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x1d, 0x2d, 0x20);
        v.widgets.hovered.fg_stroke.color = GREEN;
        v.widgets.hovered.bg_stroke.color = GREEN_DK;
        v.widgets.active.bg_fill = Color32::from_rgb(0x22, 0x33, 0x25);
        v.selection.bg_fill = a(GREEN_DK, 120);
        cc.egui_ctx.set_visuals(v);

        let icons_tex = match image::load_from_memory(include_bytes!("../strategy_building.jpeg")) {
            Ok(img) => {
                let img = img.to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let cimg = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                Some(cc.egui_ctx.load_texture("building_icons", cimg, egui::TextureOptions::LINEAR))
            }
            Err(e) => {
                eprintln!("failed to load strategy_building.jpeg: {e}");
                None
            }
        };

        let space_path = crate::store::path();
        let (world, rt, prefs) = match crate::store::load(&space_path) {
            Some((mut world, visits, prefs)) => {
                // a unit that was mid-turn when we last quit lost its process
                for p in world.projects.iter_mut() {
                    for ag in p.agents.iter_mut() {
                        if ag.state == AgentState::Working {
                            ag.state = AgentState::Idle;
                        }
                    }
                }
                eprintln!("loaded space from {} ({} bases)", space_path, world.projects.len());
                let now = now_min();
                let rt = visits.iter().map(|&v| new_rt(if v > 0.0 { v } else { now })).collect();
                (world, rt, prefs)
            }
            None => {
                let world = initial_world();
                let rt = initial_rt(&world);
                (world, rt, crate::store::Prefs::default())
            }
        };
        CommanderApp {
            world,
            rt,
            cam: Camera {
                pos: pos2(WW / 2.0, WH / 2.0),
                scale: 0.45,
                target_pos: pos2(WW / 2.0, WH / 2.0),
                target_scale: 0.45,
            },
            sel: None,
            interior: None,
            back: vec![],
            space_crumb: None,
            unseen: vec![],
            idle_idx: usize::MAX,
            highlight: None,
            toasts: vec![],
            wpings: vec![],
            mpings: vec![],
            capture_open: false,
            capture_text: String::new(),
            capture_focus: false,
            sel_capture: None,
            build_open: false,
            build_pos: pos2(WW / 2.0, WH / 2.0),
            build_text: String::new(),
            build_focus: false,
            pylon_open: false,
            pylon_pos: pos2(WW / 2.0, WH / 2.0),
            pylon_text: String::new(),
            pylon_focus: false,
            brief_focus: false,
            brief_focused: false,
            host_clip: match arboard::Clipboard::new() {
                Ok(c) => Some(c),
                Err(e) => {
                    eprintln!("host clipboard unavailable ({e}); paste falls back to the compositor clipboard");
                    None
                }
            },
            quest_open: false,
            quest_pos: pos2(WW / 2.0, WH / 2.0),
            quest_text: String::new(),
            quest_focus: false,
            drag_pylon: None,
            drag_quest: None,
            build_menu: false,
            sroom: None,
            link_from: None,
            destroy_arm: None,
            drag_base: None,
            drag_capture: None,
            briefing: None,
            recovery: None,
            now_min: now_min(),
            last_num: None,
            last_num_t: -10.0,
            time: 0.0,
            started: false,
            viewport: Rect::from_min_size(Pos2::ZERO, vec2(1150.0, 850.0)),
            clicks: vec![],
            acts: vec![],
            ctrl,
            icons_tex,
            space_path,
            dirty: false,
            last_save: 0.0,
            wasm: crate::wasm::Host::spawn(),
            workers: crate::worker::Host::spawn(),
            mod_status: std::collections::HashMap::new(),
            last_wasm_sync: -10.0,
            prefs,
            codex: crate::codex::spawn(),
            sel_structs: vec![],
            band: None,
            sdestroy_arm: None,
        }
    }

    fn save_space(&mut self) {
        if let Err(e) = crate::store::save(&self.space_path, &self.world, &self.rt, &self.prefs) {
            eprintln!("failed to save {}: {}", self.space_path, e);
        }
        self.dirty = false;
        self.last_save = self.time;
    }

    // ---------- time / formatting ----------
    fn clock(&self) -> String {
        let m = (self.now_min as i64).rem_euclid(1440);
        format!("{:02}:{:02}", m / 60, m % 60)
    }
    fn age_min(&self, i: usize) -> f64 {
        self.now_min - self.rt[i].last_visit_min
    }
    fn tier(&self, i: usize) -> Tier {
        Tier::from_age_min(self.age_min(i))
    }
    fn age_str(&self, i: usize) -> String {
        fmt_age(self.age_min(i))
    }
    fn base_center(&self, i: usize) -> Pos2 {
        let (x, y) = self.world.projects[i].pos;
        pos2(x + 180.0, y + 130.0)
    }
    /// world anchor of task ti's pylon: stored pos, or an auto slot on the arc under the base
    fn pylon_world_pos(&self, pi: usize, ti: usize) -> (f32, f32) {
        if let Some(p) = self.world.projects[pi].tasks.get(ti).and_then(|t| t.pos) {
            return p;
        }
        let c = self.base_center(pi);
        let ang = 1.05 + ti as f32 * 0.5; // sweeps down-right → down-left
        ((c.x + ang.cos() * 300.0).clamp(0.0, WW), (c.y + ang.sin() * 230.0).clamp(0.0, WH))
    }
    /// world anchor of question qi's sensor array: stored pos, or an auto slot on the arc above
    fn question_world_pos(&self, pi: usize, qi: usize) -> (f32, f32) {
        if let Some(p) = self.world.projects[pi].questions.get(qi).and_then(|q| q.pos) {
            return p;
        }
        let c = self.base_center(pi);
        let ang = -1.05 - qi as f32 * 0.5; // sweeps up-right → up-left
        ((c.x + ang.cos() * 300.0).clamp(0.0, WW), (c.y + ang.sin() * 230.0).clamp(0.0, WH))
    }
    fn screen_to_world(&self, p: Pos2) -> Pos2 {
        self.cam.pos + (p - self.viewport.center()) / self.cam.scale
    }
    fn world_to_screen(&self, wp: Pos2) -> Pos2 {
        self.viewport.center() + (wp - self.cam.pos) * self.cam.scale
    }

    /// shared click routing for real pointer clicks and injected test clicks
    fn canvas_click(&mut self, p: Pos2, double: bool) {
        let hit = self.clicks.iter().rev().find(|(r, _)| r.contains(p)).map(|(_, z)| z.clone());
        if self.sroom.is_some() {
            match hit {
                Some(ClickZone::SetTask(pi, ti, st)) => self.acts.push(Act::SetPylon(pi, ti, st)),
                Some(ClickZone::SetQuest(pi, qi, r)) => self.acts.push(Act::SetQuestion(pi, qi, r)),
                Some(ClickZone::Dispatch(pi, ti)) => self.acts.push(Act::Dispatch(pi, ti)),
                Some(ClickZone::HaltUnit(pi, aid)) => self.acts.push(Act::HaltUnit(pi, aid)),
                Some(ClickZone::ExitInterior) => self.sroom = None,
                _ => {
                    if double {
                        self.sroom = None;
                    }
                }
            }
            return;
        }
        if self.interior.is_some() {
            match hit {
                Some(ClickZone::Unit(pi, aid)) => self.highlight = Some((pi, aid)),
                Some(ClickZone::Decision(di)) => self.acts.push(Act::OpenDecision(di)),
                Some(ClickZone::ExitInterior) => self.acts.push(Act::ExitInterior),
                _ => {
                    // double-click on the floor walks back out to the map
                    if double {
                        self.acts.push(Act::ExitInterior);
                    }
                }
            }
            return;
        }
        if double {
            match hit {
                Some(ClickZone::FocusBase(i)) => self.acts.push(Act::EnterInterior(i)),
                Some(ClickZone::Pylon(pi, ti)) => self.sroom = Some(SRoom::Pylon(pi, ti)),
                Some(ClickZone::Question(pi, qi)) => self.sroom = Some(SRoom::Question(pi, qi)),
                Some(ClickZone::RailToggle) => {}
                None => {
                    let wp = self.screen_to_world(p);
                    self.acts.push(Act::PlaceBase(wp));
                }
                _ => {}
            }
        } else {
            match hit {
                Some(ClickZone::FocusBase(i)) | Some(ClickZone::Unit(i, _)) if self.link_from.is_some() => {
                    let from = self.link_from.take().unwrap();
                    self.acts.push(Act::ToggleLink(from, i));
                }
                Some(ClickZone::FocusBase(i)) => self.acts.push(Act::SelectBase(i)),
                Some(ClickZone::Unit(i, aid)) => self.acts.push(Act::Unit { proj: i, agent: aid }),
                Some(ClickZone::Decision(di)) => self.acts.push(Act::OpenDecision(di)),
                Some(ClickZone::Capture(ci)) => self.acts.push(Act::SelectCapture(ci)),
                Some(ClickZone::Link(li)) => self.acts.push(Act::DeleteLink(li)),
                // single click selects the structure (dd demolishes, 2×click enters);
                // structure and base selection are mutually exclusive
                Some(ClickZone::Pylon(pi, ti)) => {
                    self.sel_structs = vec![SRoom::Pylon(pi, ti)];
                    self.sdestroy_arm = None;
                    self.sel = None;
                }
                Some(ClickZone::Question(pi, qi)) => {
                    self.sel_structs = vec![SRoom::Question(pi, qi)];
                    self.sdestroy_arm = None;
                    self.sel = None;
                }
                Some(ClickZone::SetTask(..)) | Some(ClickZone::SetQuest(..)) => {}
                Some(ClickZone::Dispatch(..)) | Some(ClickZone::HaltUnit(..)) => {}
                Some(ClickZone::RailToggle) => {
                    self.prefs.show_rail = !self.prefs.show_rail;
                    self.dirty = true;
                }
                Some(ClickZone::ExitInterior) => {}
                None => {
                    self.sel_capture = None;
                    self.link_from = None;
                    self.sel_structs.clear();
                    self.sdestroy_arm = None;
                }
            }
        }
    }

    // ---------- camera / navigation ----------
    fn push_back(&mut self) {
        self.back.push((self.cam.target_pos, self.cam.target_scale, self.sel));
        if self.back.len() > 24 {
            self.back.remove(0);
        }
    }
    /// frame the content (bases + captures) rather than the whole — mostly empty — world
    fn fit_all(&mut self) {
        let mut min = pos2(f32::MAX, f32::MAX);
        let mut max = pos2(f32::MIN, f32::MIN);
        for p in &self.world.projects {
            min = min.min(pos2(p.pos.0, p.pos.1));
            max = max.max(pos2(p.pos.0 + BASE_W, p.pos.1 + 260.0));
        }
        for c in &self.world.captures {
            min = min.min(pos2(c.pos.0, c.pos.1));
            max = max.max(pos2(c.pos.0 + 200.0, c.pos.1 + 90.0));
        }
        if min.x > max.x {
            // empty space: settle on the world center at a workable scale
            self.cam.target_pos = pos2(WW / 2.0, WH / 2.0);
            self.cam.target_scale = 0.45;
            return;
        }
        let pad = 160.0;
        let w = (max.x - min.x) + pad * 2.0;
        let h = (max.y - min.y) + pad * 2.0;
        let s = (self.viewport.width() / w).min(self.viewport.height() / h).clamp(0.05, 1.0);
        self.cam.target_pos = pos2((min.x + max.x) / 2.0, (min.y + max.y) / 2.0);
        self.cam.target_scale = s;
    }
    fn visit(&mut self, i: usize, fresh: bool) {
        if fresh {
            let age = self.age_min(i);
            let rt = &mut self.rt[i];
            rt.shown = rt.delta.clone();
            rt.shown_age_min = age;
            rt.shown_age = fmt_age(age);
        }
        let rt = &mut self.rt[i];
        rt.delta.clear();
        rt.unseen_events = 0;
        rt.last_visit_min = self.now_min;
        self.unseen.retain(|&p| p != i);
        self.dirty = true;
    }
    fn focus(&mut self, i: usize, scale: f32, from_space: bool) {
        if i >= self.world.projects.len() {
            return;
        }
        self.interior = None;
        if !from_space {
            self.space_crumb = None;
        }
        self.push_back();
        let fresh = self.sel != Some(i);
        self.sel = Some(i);
        self.sel_structs.clear();
        self.sdestroy_arm = None;
        self.cam.target_pos = self.base_center(i);
        self.cam.target_scale = scale;
        self.visit(i, fresh);
        if fresh {
            self.highlight = None;
        }
    }
    /// select a base without moving the camera (plain click)
    fn select_base(&mut self, i: usize) {
        if i >= self.world.projects.len() {
            return;
        }
        self.space_crumb = None;
        let fresh = self.sel != Some(i);
        self.sel = Some(i);
        self.sel_structs.clear();
        self.sdestroy_arm = None;
        self.visit(i, fresh);
        if fresh {
            self.highlight = None;
        }
    }
    fn go_back(&mut self) {
        self.space_crumb = None;
        match self.back.pop() {
            Some((p, s, sel)) => {
                self.cam.target_pos = p;
                self.cam.target_scale = s;
                self.sel = sel;
            }
            None => {
                self.sel = None;
                self.fit_all();
            }
        }
    }
    fn location_name(&self) -> String {
        match self.sel {
            Some(i) => self.world.projects[i].name.to_string(),
            None => "theater overview".into(),
        }
    }

    // ---------- toasts / pings ----------
    fn toast(&mut self, head: &str, body: &str, sub: &str, ok: bool, proj: Option<usize>) {
        self.toasts.push(Toast {
            head: head.into(),
            body: body.into(),
            sub: sub.into(),
            ok,
            created: self.time,
            proj,
        });
    }
    fn ping(&mut self, proj: usize) {
        let color = self.world.projects[proj].color;
        self.wpings.push(Ping { proj, color, created: self.time });
        self.mpings.push(Ping { proj, color, created: self.time });
    }

    /// ingest an agent report into the world (event log, staleness deltas, pings, toasts)
    fn report(&mut self, proj: usize, agent: Option<&str>, text: &str) {
        self.report_ex(proj, agent, text, true);
    }

    /// quiet variant: event log + ping + delta, no toast (routine worker traffic)
    fn report_quiet(&mut self, proj: usize, agent: Option<&str>, text: &str) {
        self.report_ex(proj, agent, text, false);
    }

    fn report_ex(&mut self, proj: usize, agent: Option<&str>, text: &str, loud: bool) {
        if proj >= self.world.projects.len() {
            return;
        }
        let ts = self.clock();
        self.world.events.push(Event {
            ts: ts.clone(),
            proj: Some(proj),
            agent: agent.map(String::from),
            text: text.into(),
        });
        if let Some(aid) = agent {
            if let Some(ag) = self.world.projects[proj].agents.iter_mut().find(|a| a.id == aid) {
                ag.last_report = ts.clone();
            }
        }
        self.ping(proj);
        self.dirty = true;
        let who = agent.map(String::from).unwrap_or_else(|| self.world.projects[proj].name.clone());
        if self.sel == Some(proj) {
            // focused project: update in place, never steal focus
            self.rt[proj].last_visit_min = self.now_min;
            if !loud {
                return;
            }
            self.toast(
                &format!("📡 {} · {}", who, ts),
                text,
                "in current view — updated in place",
                true,
                Some(proj),
            );
        } else {
            self.rt[proj].delta.push(format!("{}: {} ({})", who, text, ts));
            self.rt[proj].unseen_events += 1;
            self.unseen.push(proj);
            if !loud {
                return;
            }
            self.toast(
                &format!("📡 {} · {}", who, ts),
                text,
                "SPACE jumps to it · click this toast",
                false,
                Some(proj),
            );
        }
    }

    // ---------- wasm module host ----------

    fn proj_by_name(&self, name: &str) -> Option<usize> {
        self.world.projects.iter().position(|p| p.name == name)
    }

    /// push fresh building snapshots to the wasm runtime thread
    fn wasm_sync(&mut self) {
        let buildings = self
            .world
            .projects
            .iter()
            .map(|p| crate::wasm::Building {
                name: p.name.clone(),
                state_json: serde_json::to_string(p).unwrap_or_default(),
                modules: p.modules.clone(),
            })
            .collect();
        self.wasm.sync(buildings);
        self.last_wasm_sync = self.time;
    }

    /// drain module outputs: signals feed the event log, reducers mutate state
    fn wasm_pump(&mut self) {
        use crate::wasm::Out;
        for out in self.wasm.drain() {
            match out {
                Out::Signal { proj, module, text } => {
                    if let Some(i) = self.proj_by_name(&proj) {
                        self.report(i, Some(&format!("⚙{}", module)), &text);
                    }
                }
                Out::Reduce { proj, module, cmd } => {
                    if let Some(i) = self.proj_by_name(&proj) {
                        self.apply_wasm_reduce(i, &module, &cmd);
                    }
                }
                Out::Log { proj, module, text } => {
                    eprintln!("[wasm {}/{}] {}", proj, module, text);
                    self.mod_status.entry((proj, module)).or_default().last_log = Some(text);
                }
                Out::Ran { proj, module, fuel_used, http_used, ms, error } => {
                    if let Some(e) = &error {
                        eprintln!("[wasm {}/{}] tick error: {}", proj, module, e);
                    }
                    let st = self.mod_status.entry((proj, module)).or_default();
                    st.ticks += 1;
                    st.fuel_used = fuel_used;
                    st.http_used = http_used;
                    st.ms = ms;
                    st.error = error;
                }
            }
        }
    }

    /// apply one reducer command from a module to building i's state
    fn apply_wasm_reduce(&mut self, i: usize, module: &str, cmd: &serde_json::Value) {
        let s = |k: &str| cmd.get(k).and_then(|v| v.as_str()).map(String::from);
        let op = s("op").unwrap_or_default();
        let ts = self.clock();
        let p = &mut self.world.projects[i];
        match op.as_str() {
            "status" => {
                if let Some(v) = s("value") {
                    p.status = v;
                }
            }
            "goal" => {
                if let Some(v) = s("value") {
                    p.goal = v;
                }
            }
            "task" => {
                let pos = match (cmd.get("x").and_then(|v| v.as_f64()), cmd.get("y").and_then(|v| v.as_f64())) {
                    (Some(x), Some(y)) => Some((x as f32, y as f32)),
                    _ => None,
                };
                let notes = s("notes");
                if let (Some(title), Some(state)) = (s("title"), s("state").as_deref().and_then(TaskState::parse)) {
                    match p.tasks.iter_mut().find(|t| t.title == title) {
                        Some(t) => {
                            t.state = state;
                            if pos.is_some() {
                                t.pos = pos;
                            }
                            if let Some(n) = notes {
                                t.notes = n;
                            }
                        }
                        None => p.tasks.push(Task { title, state, pos, notes: notes.unwrap_or_default() }),
                    }
                }
            }
            "task_remove" => {
                if let Some(title) = s("title") {
                    p.tasks.retain(|t| t.title != title);
                }
            }
            "question" => {
                let pos = match (cmd.get("x").and_then(|v| v.as_f64()), cmd.get("y").and_then(|v| v.as_f64())) {
                    (Some(x), Some(y)) => Some((x as f32, y as f32)),
                    _ => None,
                };
                let resolved = cmd.get("resolved").and_then(|v| v.as_bool());
                let notes = s("notes");
                if let Some(text) = s("text") {
                    match p.questions.iter_mut().find(|q| q.text == text) {
                        Some(q) => {
                            if let Some(r) = resolved {
                                q.resolved = r;
                            }
                            if pos.is_some() {
                                q.pos = pos;
                            }
                            if let Some(n) = notes {
                                q.notes = n;
                            }
                        }
                        None => p.questions.push(Question { text, resolved: resolved.unwrap_or(false), pos, notes: notes.unwrap_or_default() }),
                    }
                }
            }
            "question_remove" => {
                if let Some(text) = s("text") {
                    p.questions.retain(|q| q.text != text);
                }
            }
            "agent" => {
                if let Some(id) = s("id") {
                    let state = s("state").as_deref().and_then(AgentState::parse);
                    match p.agents.iter_mut().find(|a| a.id == id) {
                        Some(a) => {
                            if let Some(st) = state {
                                a.state = st;
                            }
                            if let Some(t) = s("task") {
                                a.task = t;
                            }
                            if let Some(b) = s("blocked_on") {
                                a.blocked_on = Some(b);
                            }
                            a.last_report = ts;
                        }
                        None => {
                            let mut ag = Agent::new(id);
                            ag.state = state.unwrap_or(AgentState::Idle);
                            ag.task = s("task").unwrap_or_default();
                            ag.last_report = ts;
                            ag.blocked_on = s("blocked_on");
                            p.agents.push(ag);
                        }
                    }
                }
            }
            "agent_remove" => {
                if let Some(id) = s("id") {
                    p.agents.retain(|a| a.id != id);
                }
            }
            other => {
                eprintln!("[wasm {}/{}] unknown reduce op '{}'", p.name, module, other);
                return;
            }
        }
        self.dirty = true;
    }

    fn idle_agents(&self) -> Vec<(usize, String)> {
        let mut out = vec![];
        for (pi, p) in self.world.projects.iter().enumerate() {
            for ag in &p.agents {
                if ag.state != AgentState::Working {
                    out.push((pi, ag.id.to_string()));
                }
            }
        }
        out
    }

    fn commit_decision(&mut self, di: usize, oi: usize) {
        if self.world.decisions[di].resolved {
            return;
        }
        let ts = self.clock();
        let chosen = self.world.decisions[di].options[oi].clone();
        let name = chosen.split(':').next().unwrap_or(&chosen).to_string();
        let proj = self.world.decisions[di].proj;
        let title = self.world.decisions[di].title.clone();
        let dec_id = self.world.decisions[di].id.clone();
        self.world.decisions[di].resolved = true;
        self.world.decisions[di].chosen = Some(chosen);
        self.world.events.push(Event {
            ts: ts.clone(),
            proj: Some(proj),
            agent: None,
            text: format!("DECIDED: {} → {}", title, name),
        });
        // release any agents holding on this decision
        let mut released = vec![];
        for ag in self.world.projects[proj].agents.iter_mut() {
            if ag.blocked_on.as_deref() == Some(dec_id.as_str()) {
                ag.state = AgentState::Working;
                ag.blocked_on = None;
                ag.last_report = ts.clone();
                released.push(ag.id.clone());
            }
        }
        for aid in &released {
            self.world.events.push(Event {
                ts: ts.clone(),
                proj: Some(proj),
                agent: Some(aid.clone()),
                text: format!("Unblocked — resuming with {} strategy", name.to_lowercase()),
            });
        }
        let sub = if released.is_empty() {
            String::new()
        } else {
            format!("{} released · minimap marker cleared", released.join(", "))
        };
        self.toast(
            &format!("✓ ORDER COMMITTED · {}", ts),
            &format!("{} → {}", title, name),
            &sub,
            true,
            Some(proj),
        );
        self.ping(proj);
        self.dirty = true;
        self.idle_idx = usize::MAX;
        self.briefing = None;
    }

    /// demolish base i: archive its record and history to disk, then remove it
    /// from the world, shifting every live project index above i down by one
    fn destroy_base(&mut self, i: usize) {
        if i >= self.world.projects.len() {
            return;
        }
        let ts = self.clock();
        let name = self.world.projects[i].name.clone();

        // archive first — the base is only demolished once its record is safely on disk
        let rec = crate::store::ArchivedBase {
            t: "archived_base",
            ts: ts.clone(),
            project: self.world.projects[i].clone(),
            last_visit_min: self.rt[i].last_visit_min,
            decisions: self.world.decisions.iter().filter(|d| d.proj == i).cloned().collect(),
            events: self.world.events.iter().filter(|e| e.proj == Some(i)).cloned().collect(),
        };
        let apath = crate::store::archive_path(&self.space_path);
        if let Err(e) = crate::store::append_archive(&apath, &rec) {
            self.toast("💥 DESTROY ABORTED", &format!("could not archive {}: {}", name, e), "the base still stands", false, Some(i));
            self.destroy_arm = None;
            return;
        }

        // remove the base and every record tied to it
        self.world.projects.remove(i);
        self.rt.remove(i);
        self.world.decisions.retain(|d| d.proj != i);
        for d in &mut self.world.decisions {
            if d.proj > i {
                d.proj -= 1;
            }
        }
        self.world.events.retain(|e| e.proj != Some(i));
        for e in &mut self.world.events {
            if let Some(p) = &mut e.proj {
                if *p > i {
                    *p -= 1;
                }
            }
        }
        self.world.links.retain(|l| l.a != i && l.b != i);
        for l in &mut self.world.links {
            if l.a > i {
                l.a -= 1;
            }
            if l.b > i {
                l.b -= 1;
            }
        }

        // fix up every live index that pointed at or past the removed base
        let shift = |v: usize| if v > i { v - 1 } else { v };
        self.sel = self.sel.and_then(|s| if s == i { None } else { Some(shift(s)) });
        self.interior = self.interior.and_then(|s| if s == i { None } else { Some(shift(s)) });
        self.link_from = self.link_from.and_then(|s| if s == i { None } else { Some(shift(s)) });
        self.drag_base = self.drag_base.and_then(|s| if s == i { None } else { Some(shift(s)) });
        self.recovery = self.recovery.and_then(|s| if s == i { None } else { Some(shift(s)) });
        self.briefing = None; // decision indices shifted — close any open briefing
        self.highlight = self.highlight.take().and_then(|(p, a)| if p == i { None } else { Some((shift(p), a)) });
        for (_, _, s) in &mut self.back {
            *s = s.and_then(|v| if v == i { None } else { Some(shift(v)) });
        }
        self.unseen.retain(|&p| p != i);
        for p in &mut self.unseen {
            *p = shift(*p);
        }
        self.toasts.retain(|t| t.proj != Some(i));
        for t in &mut self.toasts {
            t.proj = t.proj.and_then(|p| if p == i { None } else { Some(shift(p)) });
        }
        self.wpings.retain(|p| p.proj != i);
        for p in &mut self.wpings {
            p.proj = shift(p.proj);
        }
        self.mpings.retain(|p| p.proj != i);
        for p in &mut self.mpings {
            p.proj = shift(p.proj);
        }
        self.last_num = None;
        self.idle_idx = usize::MAX;
        self.destroy_arm = None;
        self.sroom = self.sroom.take().and_then(|r| match r {
            SRoom::Pylon(pi, ti) => (pi != i).then(|| SRoom::Pylon(shift(pi), ti)),
            SRoom::Question(pi, qi) => (pi != i).then(|| SRoom::Question(shift(pi), qi)),
        });
        self.sel_structs.retain(|r| match r {
            SRoom::Pylon(pi, _) | SRoom::Question(pi, _) => *pi != i,
        });
        for r in &mut self.sel_structs {
            match r {
                SRoom::Pylon(pi, _) | SRoom::Question(pi, _) => *pi = shift(*pi),
            }
        }

        self.world.events.push(Event {
            ts: ts.clone(),
            proj: None,
            agent: None,
            text: format!("base destroyed: {} (record archived)", name),
        });
        self.toast(
            &format!("💥 BASE DESTROYED · {}", ts),
            &name,
            &format!("record archived → {}", apath),
            true,
            None,
        );
        self.dirty = true;
    }

    fn apply(&mut self, act: Act) {
        match act {
            Act::Focus { proj, scale, from_space } => self.focus(proj, scale, from_space),
            Act::SelectBase(i) => self.select_base(i),
            Act::SelectCapture(ci) => self.sel_capture = Some(ci),
            Act::FileCapture { cap, proj } => {
                if cap < self.world.captures.len() {
                    let c = self.world.captures.remove(cap);
                    let ts = self.clock();
                    let pname = self.world.projects[proj].name.to_string();
                    self.world.events.push(Event {
                        ts: ts.clone(),
                        proj: Some(proj),
                        agent: None,
                        text: format!("filed capture: {}", c.text),
                    });
                    if self.sel != Some(proj) {
                        self.rt[proj].delta.push(format!("you filed: {} ({})", c.text, ts));
                        self.rt[proj].unseen_events += 1;
                    }
                    self.toast(&format!("⚡ FILED · {}", ts), &c.text, &format!("→ {}", pname), true, Some(proj));
                    self.dirty = true;
                }
                self.sel_capture = None;
            }
            Act::DiscardCapture(ci) => {
                if ci < self.world.captures.len() {
                    let c = self.world.captures.remove(ci);
                    self.toast("🗑 DISCARDED", &c.text, "", true, None);
                    self.dirty = true;
                }
                self.sel_capture = None;
            }
            Act::Unit { proj, agent } => {
                self.focus(proj, 0.95, false);
                self.highlight = Some((proj, agent));
            }
            Act::OpenDecision(di) => {
                self.recovery = None;
                self.briefing = Some(di);
            }
            Act::CommitDecision(di, oi) => self.commit_decision(di, oi),
            Act::CloseBriefing => self.briefing = None,
            Act::OpenRecovery(pi) => {
                self.briefing = None;
                self.recovery = Some(pi);
            }
            Act::CloseRecovery => self.recovery = None,
            Act::CycleIdle => {
                let list = self.idle_agents();
                if list.is_empty() {
                    self.toast("🛌 NO IDLE UNITS", "Every unit is tasked. Commander, the line holds.", "", true, None);
                } else {
                    self.idle_idx = self.idle_idx.wrapping_add(1) % list.len();
                    let (pi, aid) = list[self.idle_idx].clone();
                    self.focus(pi, 1.1, false);
                    self.highlight = Some((pi, aid));
                }
            }
            Act::OpenCapture => {
                self.capture_open = true;
                self.capture_focus = true;
            }
            Act::PlaceBase(wp) => {
                self.build_pos = pos2(wp.x.clamp(0.0, WW - BASE_W), wp.y.clamp(0.0, WH - 200.0));
                self.build_open = true;
                self.build_focus = true;
            }
            Act::CommitBase(name) => {
                let idx = self.world.projects.len();
                let color = PROJ_COLORS[idx % PROJ_COLORS.len()];
                let pos = (
                    (self.build_pos.x - 180.0).clamp(0.0, WW - BASE_W),
                    (self.build_pos.y - 130.0).clamp(0.0, WH - 200.0),
                );
                self.world.projects.push(Project {
                    name: name.clone(),
                    color,
                    icon: idx % ICON_COUNT,
                    status: "active".into(),
                    goal: String::new(),
                    agents: vec![],
                    tasks: vec![],
                    pos,
                    modules: vec![],
                    questions: vec![],
                    cwd: None,
                    sandbox: None,
                    model: None,
                });
                self.rt.push(new_rt(self.now_min));
                let ts = self.clock();
                self.world.events.push(Event {
                    ts: ts.clone(),
                    proj: Some(idx),
                    agent: None,
                    text: format!("base established: {}", name),
                });
                self.toast(
                    &format!("⌂ BASE ESTABLISHED · {}", ts),
                    &name,
                    "drag to reposition · L links it to another base",
                    true,
                    Some(idx),
                );
                self.build_open = false;
                self.build_text.clear();
                self.dirty = true;
                self.focus(idx, 0.95, false);
            }
            Act::PlacePylon(wp) => match self.sel {
                Some(_) => {
                    self.pylon_pos = pos2(wp.x.clamp(0.0, WW), wp.y.clamp(0.0, WH));
                    self.pylon_open = true;
                    self.pylon_focus = true;
                }
                None => self.toast("◆ PYLON", "Select a base first (1–4 or click one), then press P.", "pylons are goals owned by a base", false, None),
            },
            Act::CommitPylon(title) => {
                if let Some(i) = self.sel {
                    let ts = self.clock();
                    self.world.projects[i].tasks.push(Task {
                        title: title.clone(),
                        state: TaskState::Todo,
                        pos: Some((self.pylon_pos.x, self.pylon_pos.y)),
                        notes: String::new(),
                    });
                    self.world.events.push(Event {
                        ts: ts.clone(),
                        proj: Some(i),
                        agent: None,
                        text: format!("pylon warped in: {}", title),
                    });
                    self.toast(
                        &format!("◆ PYLON WARPED IN · {}", ts),
                        &title,
                        "click it to cycle todo → doing → done · drag to reposition",
                        true,
                        Some(i),
                    );
                    self.ping(i);
                    self.dirty = true;
                }
                self.pylon_open = false;
                self.pylon_text.clear();
            }
            Act::SetPylon(pi, ti, st) => {
                if let Some(t) = self.world.projects.get_mut(pi).and_then(|p| p.tasks.get_mut(ti)) {
                    if t.state != st {
                        t.state = st;
                        let title = t.title.clone();
                        let ts = self.clock();
                        self.world.events.push(Event {
                            ts,
                            proj: Some(pi),
                            agent: None,
                            text: format!("pylon {} → {}", title, st.label()),
                        });
                        self.dirty = true;
                    }
                }
            }
            Act::Dispatch(pi, ti) => {
                if let Err(e) = self.dispatch(pi, ti, None, None) {
                    self.toast("⚠ DISPATCH FAILED", &e, "set the base repo: /base?i=..&cwd=/path", false, Some(pi));
                }
            }
            Act::HaltUnit(pi, aid) => {
                self.halt(pi, &aid);
            }
            Act::SetQuestion(pi, qi, r) => {
                if let Some(q) = self.world.projects.get_mut(pi).and_then(|p| p.questions.get_mut(qi)) {
                    if q.resolved != r {
                        q.resolved = r;
                        let text = q.text.clone();
                        let ts = self.clock();
                        self.world.events.push(Event {
                            ts,
                            proj: Some(pi),
                            agent: None,
                            text: if r { format!("question resolved: {}", text) } else { format!("question reopened: {}", text) },
                        });
                        self.dirty = true;
                    }
                }
            }
            Act::PlaceQuestion(wp) => match self.sel {
                Some(_) => {
                    self.quest_pos = pos2(wp.x.clamp(0.0, WW), wp.y.clamp(0.0, WH));
                    self.quest_open = true;
                    self.quest_focus = true;
                }
                None => self.toast("⌖ SENSOR ARRAY", "Select a base first (1–4 or click one), then press Q.", "sensor arrays are open questions owned by a base", false, None),
            },
            Act::CommitQuestion(text) => {
                if let Some(i) = self.sel {
                    let ts = self.clock();
                    self.world.projects[i].questions.push(Question {
                        text: text.clone(),
                        resolved: false,
                        pos: Some((self.quest_pos.x, self.quest_pos.y)),
                        notes: String::new(),
                    });
                    self.world.events.push(Event {
                        ts: ts.clone(),
                        proj: Some(i),
                        agent: None,
                        text: format!("sensor array raised: {}", text),
                    });
                    self.toast(
                        &format!("⌖ SENSOR ARRAY ONLINE · {}", ts),
                        &text,
                        "scanning — click it when the question is answered",
                        true,
                        Some(i),
                    );
                    self.ping(i);
                    self.dirty = true;
                }
                self.quest_open = false;
                self.quest_text.clear();
            }
            Act::ToggleQuestion(pi, qi) => {
                if let Some(q) = self.world.projects.get_mut(pi).and_then(|p| p.questions.get_mut(qi)) {
                    q.resolved = !q.resolved;
                    let (text, resolved) = (q.text.clone(), q.resolved);
                    let ts = self.clock();
                    self.world.events.push(Event {
                        ts,
                        proj: Some(pi),
                        agent: None,
                        text: if resolved {
                            format!("question resolved: {}", text)
                        } else {
                            format!("question reopened: {}", text)
                        },
                    });
                    self.dirty = true;
                }
            }
            Act::DestroyStructs => {
                // demolish the group-selected substructures; remove per project in
                // descending index order so earlier removals don't shift later ones
                let ts = self.clock();
                let sel = std::mem::take(&mut self.sel_structs);
                let mut tasks: Vec<(usize, usize)> = vec![];
                let mut quests: Vec<(usize, usize)> = vec![];
                for r in sel {
                    match r {
                        SRoom::Pylon(pi, ti) => tasks.push((pi, ti)),
                        SRoom::Question(pi, qi) => quests.push((pi, qi)),
                    }
                }
                tasks.sort_by(|a0, b0| b0.cmp(a0));
                tasks.dedup();
                quests.sort_by(|a0, b0| b0.cmp(a0));
                quests.dedup();
                let mut n = 0usize;
                for (pi, ti) in tasks {
                    if let Some(p) = self.world.projects.get_mut(pi) {
                        if ti < p.tasks.len() {
                            let t0 = p.tasks.remove(ti);
                            self.world.events.push(Event {
                                ts: ts.clone(),
                                proj: Some(pi),
                                agent: None,
                                text: format!("pylon demolished: {}", t0.title),
                            });
                            n += 1;
                        }
                    }
                }
                for (pi, qi) in quests {
                    if let Some(p) = self.world.projects.get_mut(pi) {
                        if qi < p.questions.len() {
                            let q = p.questions.remove(qi);
                            self.world.events.push(Event {
                                ts: ts.clone(),
                                proj: Some(pi),
                                agent: None,
                                text: format!("sensor array decommissioned: {}", q.text),
                            });
                            n += 1;
                        }
                    }
                }
                // indices shifted — drop anything that might point at removed slots
                self.sroom = None;
                self.drag_pylon = None;
                self.drag_quest = None;
                self.sdestroy_arm = None;
                self.toast(
                    &format!("💥 DEMOLISHED · {}", ts),
                    &format!("{} structure{} removed", n, if n == 1 { "" } else { "s" }),
                    "logged in the event feed",
                    true,
                    None,
                );
                self.dirty = true;
            }
            Act::DestroyBase(i) => {
                if i < self.world.projects.len() {
                    let armed = self.destroy_arm.map_or(false, |(p, t)| p == i && self.time - t < 4.0);
                    if armed {
                        self.destroy_base(i);
                    } else {
                        self.destroy_arm = Some((i, self.time));
                        let name = self.world.projects[i].name.clone();
                        self.toast(
                            "💥 CONFIRM DESTROY",
                            &format!("{} will be demolished — its record is archived, not lost.", name),
                            "press D / click destroy again within 4s to confirm",
                            false,
                            Some(i),
                        );
                    }
                }
            }
            Act::StartLink => match self.sel {
                Some(i) => {
                    self.link_from = Some(i);
                    self.toast(
                        "⛓ LINK MODE",
                        &format!("from {} — click the target base", self.world.projects[i].name),
                        "esc cancels · clicking a linked base severs the link",
                        true,
                        None,
                    );
                }
                None => self.toast("⛓ LINK MODE", "Select a base first (1–4 or click one), then press L.", "", false, None),
            },
            Act::ToggleLink(from, to) => {
                if from == to {
                    self.toast("⛓ LINK", "A base cannot link to itself.", "", false, None);
                } else if from < self.world.projects.len() && to < self.world.projects.len() {
                    let existing = self
                        .world
                        .links
                        .iter()
                        .position(|l| (l.a == from && l.b == to) || (l.a == to && l.b == from));
                    let ts = self.clock();
                    let names = format!("{} ⟷ {}", self.world.projects[from].name, self.world.projects[to].name);
                    match existing {
                        Some(k) => {
                            self.world.links.remove(k);
                            self.toast(&format!("⛓ LINK SEVERED · {}", ts), &names, "", true, None);
                        }
                        None => {
                            self.world.links.push(Link { a: from, b: to });
                            self.world.events.push(Event {
                                ts: ts.clone(),
                                proj: Some(from),
                                agent: None,
                                text: format!("link established: {}", names),
                            });
                            self.toast(&format!("⛓ LINK ESTABLISHED · {}", ts), &names, "click the ◆ midpoint node to sever", true, None);
                        }
                    }
                    self.dirty = true;
                }
            }
            Act::DeleteLink(li) => {
                if li < self.world.links.len() {
                    let l = self.world.links.remove(li);
                    let names = format!(
                        "{} ⟷ {}",
                        self.world.projects.get(l.a).map(|p| p.name.as_str()).unwrap_or("?"),
                        self.world.projects.get(l.b).map(|p| p.name.as_str()).unwrap_or("?"),
                    );
                    self.toast(&format!("⛓ LINK SEVERED · {}", self.clock()), &names, "", true, None);
                    self.dirty = true;
                }
            }
            Act::CommitCapture(text) => {
                let ts = self.clock();
                let n = self.world.captures.len() as f32;
                let pos = (
                    (self.cam.target_pos.x - 220.0 + (n % 3.0) * 150.0).clamp(0.0, WW - 200.0),
                    (self.cam.target_pos.y + 120.0 + ((n / 3.0).floor() % 3.0) * 100.0).clamp(0.0, WH - 80.0),
                );
                self.world.captures.push(CaptureNote { text: text.clone(), ts: ts.clone(), pos });
                self.world.events.push(Event { ts: ts.clone(), proj: None, agent: None, text: text.clone() });
                self.toast(
                    &format!("⚡ CAPTURED · {}", ts),
                    &text,
                    "drifting unsorted mid-map — file it whenever",
                    true,
                    None,
                );
                self.capture_open = false;
                self.capture_text.clear();
                self.dirty = true;
            }
            Act::SpaceJump => {
                if self.unseen.is_empty() {
                    self.toast("📡 ALL CLEAR", "No unseen alerts — theater is quiet.", "", true, None);
                } else {
                    let pid = *self.unseen.last().unwrap();
                    self.space_crumb = Some(self.location_name());
                    self.focus(pid, 0.95, true);
                }
            }
            Act::EnterInterior(i) => {
                if i < self.world.projects.len() {
                    self.select_base(i);
                    self.interior = Some(i);
                }
            }
            Act::ExitInterior => self.interior = None,
            Act::Back => self.go_back(),
            Act::MinimapGoto(p) => {
                self.cam.target_pos = p;
            }
            Act::Center => {
                if let Some(i) = self.sel {
                    self.focus(i, 1.35, false);
                }
            }
        }
    }

    // ---------- host clipboard ----------
    /// ctrl+v: serve the paste from the host clipboard. runs before the UI is
    /// built so this frame's text editors see the Paste event. any Paste the
    /// windowing layer produced from the compositor clipboard is replaced —
    /// copies are mirrored to the host (host_copy), so the host is never staler.
    fn host_paste(&mut self, ctx: &egui::Context) {
        let chord = ctx.input(|i| {
            i.events.iter().any(|e| match e {
                egui::Event::Key { key: Key::V, pressed: true, modifiers, .. } => modifiers.command || modifiers.ctrl,
                egui::Event::Key { key: Key::Paste, pressed: true, .. } => true,
                _ => false,
            })
        });
        if !chord {
            return;
        }
        let Some(clip) = self.host_clip.as_mut() else { return };
        match clip.get_text() {
            Ok(text) if !text.is_empty() => {
                let text = text.replace("\r\n", "\n");
                ctx.input_mut(|i| {
                    i.events.retain(|e| !matches!(e, egui::Event::Paste(_)));
                    i.events.push(egui::Event::Paste(text));
                });
            }
            Ok(_) => {}
            Err(e) => eprintln!("host clipboard read failed: {e}"),
        }
    }

    /// ctrl+c / ctrl+x inside a text editor: mirror what egui copied into the
    /// host clipboard as well. runs after the UI so the frame's copy commands
    /// are already in the platform output.
    fn host_copy(&mut self, ctx: &egui::Context) {
        let Some(clip) = self.host_clip.as_mut() else { return };
        let copied: Vec<String> = ctx.output(|o| {
            o.commands
                .iter()
                .filter_map(|c| match c {
                    egui::OutputCommand::CopyText(s) => Some(s.clone()),
                    _ => None,
                })
                .collect()
        });
        for text in copied {
            if let Err(e) = clip.set_text(text) {
                eprintln!("host clipboard write failed: {e}");
            }
        }
    }

    // ---------- input ----------
    fn keyboard(&mut self, ctx: &egui::Context) {
        let typing = ctx.memory(|m| m.focused().is_some());
        if typing {
            return;
        }
        if self.brief_focused && ctx.input(|i| i.key_pressed(Key::Escape)) {
            // this esc pulled the cursor out of the brief editor; the room stays
            self.brief_focused = false;
            return;
        }
        ctx.input(|inp| {
            // build submenu: B arms it, then B/P/Q choose what to place (SC-style
            // command card — the hint bar switches to this level while armed)
            if self.build_menu {
                let wp = inp
                    .pointer
                    .hover_pos()
                    .filter(|p| self.viewport.contains(*p))
                    .map(|p| self.screen_to_world(p))
                    .unwrap_or(self.cam.target_pos);
                if inp.key_pressed(Key::B) {
                    self.acts.push(Act::PlaceBase(wp));
                    self.build_menu = false;
                } else if inp.key_pressed(Key::P) {
                    self.acts.push(Act::PlacePylon(wp));
                    self.build_menu = false;
                } else if inp.key_pressed(Key::Q) {
                    self.acts.push(Act::PlaceQuestion(wp));
                    self.build_menu = false;
                } else if inp.key_pressed(Key::Escape) {
                    self.build_menu = false;
                }
                return;
            }
            let nums = [Key::Num1, Key::Num2, Key::Num3, Key::Num4];
            for (i, k) in nums.iter().enumerate() {
                if inp.key_pressed(*k) {
                    let dbl = self.last_num == Some(i) && self.time - self.last_num_t < 0.45;
                    self.last_num = Some(i);
                    self.last_num_t = self.time;
                    self.acts.push(Act::Focus { proj: i, scale: if dbl { 1.35 } else { 0.95 }, from_space: false });
                }
            }
            if inp.key_pressed(Key::Space) {
                self.acts.push(Act::SpaceJump);
            }
            if inp.key_pressed(Key::Escape) || inp.key_pressed(Key::Num0) {
                if self.briefing.is_some() {
                    self.acts.push(Act::CloseBriefing);
                } else if self.recovery.is_some() {
                    self.acts.push(Act::CloseRecovery);
                } else if self.link_from.is_some() {
                    self.link_from = None;
                } else if self.sel_capture.is_some() {
                    self.sel_capture = None;
                } else if !self.sel_structs.is_empty() {
                    self.sel_structs.clear();
                    self.sdestroy_arm = None;
                } else if self.sroom.is_some() {
                    self.sroom = None;
                } else if self.interior.is_some() {
                    self.acts.push(Act::ExitInterior);
                } else {
                    self.acts.push(Act::Back);
                }
            }
            if inp.key_pressed(Key::C) {
                self.acts.push(Act::OpenCapture);
            }
            if inp.key_pressed(Key::Enter) && self.sroom.is_some() {
                // inside a structure room, enter drops the cursor into its brief
                self.brief_focus = true;
            }
            if inp.key_pressed(Key::W) {
                if let Some(SRoom::Pylon(pi, ti)) = self.sroom {
                    self.acts.push(Act::Dispatch(pi, ti));
                }
            }
            if inp.key_pressed(Key::I) {
                self.acts.push(Act::CycleIdle);
            }
            if inp.key_pressed(Key::B) {
                // arm the build submenu; the hint bar switches to its level
                self.build_menu = true;
            }
            if inp.key_pressed(Key::L) {
                self.acts.push(Act::StartLink);
            }
            if inp.key_pressed(Key::D) {
                if !self.sel_structs.is_empty() {
                    // dd demolishes the group-selected pylons/sensors
                    let armed = self.sdestroy_arm.map_or(false, |t0| self.time - t0 < 4.0);
                    if armed {
                        self.acts.push(Act::DestroyStructs);
                    } else {
                        self.sdestroy_arm = Some(self.time);
                        self.toast(
                            "💥 CONFIRM DEMOLISH",
                            &format!("{} selected structure(s) will be removed.", self.sel_structs.len()),
                            "press D again within 4s to confirm",
                            false,
                            None,
                        );
                    }
                } else {
                    match self.sel {
                        Some(i) => self.acts.push(Act::DestroyBase(i)),
                        None => self.toast("💥 DESTROY", "Select a base first (1–4 or click one), then press D.", "", false, None),
                    }
                }
            }
        });
    }

    // ---------- world canvas ----------
    fn world_canvas(&mut self, ctx: &egui::Context) {
        CentralPanel::default()
            .frame(Frame::new().fill(BG))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                self.viewport = rect;
                if !self.started {
                    self.started = true;
                    self.fit_all();
                    self.cam.pos = self.cam.target_pos;
                    self.cam.scale = self.cam.target_scale;
                }
                let resp = ui.allocate_rect(rect, Sense::click_and_drag());
                if self.interior.map_or(false, |ii| ii >= self.world.projects.len()) {
                    self.interior = None;
                }
                if let Some(room) = self.sroom {
                    // structure space: validate the target still exists, then draw it
                    let ok = match room {
                        SRoom::Pylon(pi, ti) => self.world.projects.get(pi).map_or(false, |p| ti < p.tasks.len()),
                        SRoom::Question(pi, qi) => self.world.projects.get(pi).map_or(false, |p| qi < p.questions.len()),
                    };
                    if !ok {
                        self.sroom = None;
                    } else {
                        let painter = ui.painter_at(rect);
                        let brief = self.draw_struct_room(&painter, rect, room);
                        self.brief_editor(ui, brief, room);
                        if resp.double_clicked() {
                            if let Some(p) = resp.interact_pointer_pos() {
                                self.canvas_click(p, true);
                            }
                        } else if resp.clicked() {
                            if let Some(p) = resp.interact_pointer_pos() {
                                self.canvas_click(p, false);
                            }
                        }
                        return;
                    }
                }
                if let Some(ii) = self.interior {
                    let painter = ui.painter_at(rect);
                    self.draw_interior(&painter, rect, ii);
                    if resp.double_clicked() {
                        if let Some(p) = resp.interact_pointer_pos() {
                            self.canvas_click(p, true);
                        }
                    } else if resp.clicked() {
                        if let Some(p) = resp.interact_pointer_pos() {
                            self.canvas_click(p, false);
                        }
                    }
                    return;
                }
                if resp.drag_started() {
                    // ctrl+drag opens a rectangle group-select over substructures
                    if ctx.input(|i| i.modifiers.ctrl || i.modifiers.command) {
                        if let Some(p) = resp.interact_pointer_pos() {
                            self.band = Some((p, p));
                        }
                    }
                    // hit-test against last frame's zones: drag a base or capture, else pan
                    if self.band.is_none() {
                        if let Some(p) = resp.interact_pointer_pos() {
                            let hit = self.clicks.iter().rev().find(|(r, _)| r.contains(p)).map(|(_, z)| z.clone());
                            match hit {
                            Some(ClickZone::FocusBase(i)) | Some(ClickZone::Unit(i, _)) => self.drag_base = Some(i),
                            Some(ClickZone::Capture(ci)) => self.drag_capture = Some(ci),
                            Some(ClickZone::Pylon(pi, ti)) => {
                                // pin the auto slot so the drag delta has an anchor
                                let wp = self.pylon_world_pos(pi, ti);
                                if let Some(t) = self.world.projects[pi].tasks.get_mut(ti) {
                                    t.pos = Some(wp);
                                }
                                self.drag_pylon = Some((pi, ti));
                            }
                            Some(ClickZone::Question(pi, qi)) => {
                                let wp = self.question_world_pos(pi, qi);
                                if let Some(q) = self.world.projects[pi].questions.get_mut(qi) {
                                    q.pos = Some(wp);
                                }
                                self.drag_quest = Some((pi, qi));
                            }
                            _ => {}
                            }
                        }
                    }
                }
                if resp.dragged() {
                    let d = resp.drag_delta();
                    if let Some((_, end)) = &mut self.band {
                        *end += d;
                    } else if let Some(i) = self.drag_base {
                        if i < self.world.projects.len() {
                            let pr = &mut self.world.projects[i];
                            pr.pos.0 = (pr.pos.0 + d.x / self.cam.scale).clamp(0.0, WW - BASE_W);
                            pr.pos.1 = (pr.pos.1 + d.y / self.cam.scale).clamp(0.0, WH - 200.0);
                        }
                        ui.output_mut(|o| o.cursor_icon = CursorIcon::Move);
                    } else if let Some(ci) = self.drag_capture {
                        if ci < self.world.captures.len() {
                            let c = &mut self.world.captures[ci];
                            c.pos.0 = (c.pos.0 + d.x / self.cam.scale).clamp(0.0, WW - 200.0);
                            c.pos.1 = (c.pos.1 + d.y / self.cam.scale).clamp(0.0, WH - 80.0);
                        }
                        ui.output_mut(|o| o.cursor_icon = CursorIcon::Move);
                    } else if let Some((pi, ti)) = self.drag_pylon {
                        if let Some(t) = self.world.projects.get_mut(pi).and_then(|p| p.tasks.get_mut(ti)) {
                            if let Some(pos) = &mut t.pos {
                                pos.0 = (pos.0 + d.x / self.cam.scale).clamp(0.0, WW);
                                pos.1 = (pos.1 + d.y / self.cam.scale).clamp(0.0, WH);
                            }
                        }
                        ui.output_mut(|o| o.cursor_icon = CursorIcon::Move);
                    } else if let Some((pi, qi)) = self.drag_quest {
                        if let Some(q) = self.world.projects.get_mut(pi).and_then(|p| p.questions.get_mut(qi)) {
                            if let Some(pos) = &mut q.pos {
                                pos.0 = (pos.0 + d.x / self.cam.scale).clamp(0.0, WW);
                                pos.1 = (pos.1 + d.y / self.cam.scale).clamp(0.0, WH);
                            }
                        }
                        ui.output_mut(|o| o.cursor_icon = CursorIcon::Move);
                    } else {
                        self.cam.pos -= d / self.cam.scale;
                        self.cam.target_pos = self.cam.pos;
                        self.cam.target_scale = self.cam.scale;
                        ui.output_mut(|o| o.cursor_icon = CursorIcon::Grabbing);
                    }
                }
                if resp.drag_stopped() {
                    if let Some((a0, b0)) = self.band.take() {
                        // band select: every substructure whose zone touches the rectangle
                        let r = Rect::from_two_pos(a0, b0);
                        self.sel_structs = self
                            .clicks
                            .iter()
                            .filter_map(|(zr, z)| match z {
                                ClickZone::Pylon(pi, ti) if r.intersects(*zr) => Some(SRoom::Pylon(*pi, *ti)),
                                ClickZone::Question(pi, qi) if r.intersects(*zr) => Some(SRoom::Question(*pi, *qi)),
                                _ => None,
                            })
                            .collect();
                        self.sdestroy_arm = None;
                        if !self.sel_structs.is_empty() {
                            self.sel = None; // structure and base selection are exclusive
                        }
                    }
                    if self.drag_base.is_some() || self.drag_capture.is_some() || self.drag_pylon.is_some() || self.drag_quest.is_some() {
                        self.dirty = true;
                    }
                    self.drag_base = None;
                    self.drag_capture = None;
                    self.drag_pylon = None;
                    self.drag_quest = None;
                }
                if resp.hovered() {
                    let scroll = ctx.input(|i| i.raw_scroll_delta.y);
                    if scroll.abs() > 0.1 {
                        let f = if scroll > 0.0 { 1.12 } else { 0.89 };
                        let ns = (self.cam.scale * f).clamp(0.05, 1.9);
                        if let Some(mp) = resp.hover_pos() {
                            let m = mp - rect.center();
                            self.cam.pos = self.cam.pos + m / self.cam.scale - m / ns;
                        }
                        self.cam.scale = ns;
                        self.cam.target_pos = self.cam.pos;
                        self.cam.target_scale = ns;
                    }
                }
                let painter = ui.painter_at(rect);
                self.draw_world(&painter, rect);
                if resp.double_clicked() {
                    if let Some(p) = resp.interact_pointer_pos() {
                        self.canvas_click(p, true);
                    }
                } else if resp.clicked() {
                    if let Some(p) = resp.interact_pointer_pos() {
                        self.canvas_click(p, false);
                    }
                }
            });
    }

    fn draw_world(&mut self, p: &Painter, rect: Rect) {
        self.clicks.clear();
        let s = self.cam.scale;
        let cam = self.cam.pos;
        let t = self.time;
        let to = |wp: Pos2| rect.center() + (wp - cam) * s;

        // grid — unbounded: covers whatever the camera can see
        let half = rect.size() / (2.0 * s);
        let wmin = cam - half;
        let wmax = cam + half;
        let grid = |step: f32, alpha: u8, painter: &Painter| {
            let mut x = (wmin.x / step).floor() * step;
            while x <= wmax.x {
                painter.line_segment(
                    [to(pos2(x, wmin.y)), to(pos2(x, wmax.y))],
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(60, 90, 64, alpha)),
                );
                x += step;
            }
            let mut y = (wmin.y / step).floor() * step;
            while y <= wmax.y {
                painter.line_segment(
                    [to(pos2(wmin.x, y)), to(pos2(wmax.x, y))],
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(60, 90, 64, alpha)),
                );
                y += step;
            }
        };
        if s > 0.55 {
            grid(48.0, 10, p);
        }
        grid(240.0, 22, p);

        // placement marker while a new base is being named
        if self.build_open {
            let c = to(self.build_pos);
            let pulse = (120.0 + 100.0 * (t * 4.0).sin()) as u8;
            p.circle_stroke(c, 14.0 * s, Stroke::new(1.5, a(GREEN, pulse.max(80))));
            p.line_segment([c - vec2(22.0 * s, 0.0), c + vec2(22.0 * s, 0.0)], Stroke::new(1.0, a(GREEN, 160)));
            p.line_segment([c - vec2(0.0, 22.0 * s), c + vec2(0.0, 22.0 * s)], Stroke::new(1.0, a(GREEN, 160)));
            p.text(c + vec2(0.0, -30.0 * s), Align2::CENTER_CENTER, "⌂ new base", FontId::monospace((10.0 * s).max(6.0)), GREEN);
        }

        // links between bases (drawn beneath the base cards)
        let links: Vec<(usize, usize)> = self.world.links.iter().map(|l| (l.a, l.b)).collect();
        for (li, (la, lb)) in links.into_iter().enumerate() {
            if la >= self.world.projects.len() || lb >= self.world.projects.len() {
                continue;
            }
            let ca = to(self.base_center(la));
            let cb = to(self.base_center(lb));
            let mid = ca.lerp(cb, 0.5);
            let col_a = self.world.projects[la].color;
            let col_b = self.world.projects[lb].color;
            p.line_segment([ca, mid], Stroke::new(1.5, a(col_a, 140)));
            p.line_segment([mid, cb], Stroke::new(1.5, a(col_b, 140)));
            // traffic pulse
            let f = ((t * 0.25 + li as f64 * 0.37).fract()) as f32;
            p.circle_filled(ca.lerp(cb, f), (3.0 * s).max(1.5), a(GREEN, 180));
            // severable midpoint node
            p.circle_stroke(mid, 5.0, Stroke::new(1.0, a(DIM, 170)));
            p.text(mid, Align2::CENTER_CENTER, "◆", FontId::proportional(7.0), a(DIM, 200));
            self.clicks.push((Rect::from_center_size(mid, vec2(16.0, 16.0)), ClickZone::Link(li)));
        }

        // live link preview while choosing a target
        if let Some(from) = self.link_from {
            if from < self.world.projects.len() {
                if let Some(mp) = p.ctx().pointer_hover_pos() {
                    let c = to(self.base_center(from));
                    p.line_segment([c, mp], Stroke::new(1.5, a(GREEN, 150)));
                    p.circle_filled(mp, 3.0, a(GREEN, 200));
                }
            }
        }

        // drifting capture cards (selectable, draggable)
        let mut capture_zones: Vec<(Rect, usize)> = vec![];
        for i in 0..self.world.captures.len() {
            let cp_text = self.world.captures[i].text.clone();
            let cp_ts = self.world.captures[i].ts.clone();
            let (sx, sy) = self.world.captures[i].pos;
            let drift = vec2(
                ((t * 0.35 + i as f64 * 1.7).sin() as f32) * 10.0,
                ((t * 0.27 + i as f64 * 0.9).cos() as f32) * 7.0,
            );
            let origin = to(pos2(sx, sy)) + drift * s;
            let wrap = 180.0 * s;
            let galley = p.layout(cp_text, FontId::proportional((10.5 * s).max(6.0)), Color32::from_rgb(0x9f, 0xb3, 0xa1), wrap);
            let ts_galley = p.layout(
                format!("⚡ {} · unsorted", cp_ts),
                FontId::monospace((8.5 * s).max(5.0)),
                FAINT,
                wrap,
            );
            let pad = 8.0 * s;
            let card = Rect::from_min_size(
                origin,
                vec2(
                    galley.size().x.max(ts_galley.size().x) + pad * 2.0,
                    galley.size().y + ts_galley.size().y + pad * 2.0 + 3.0 * s,
                ),
            );
            let selected = self.sel_capture == Some(i);
            p.rect_filled(card, CornerRadius::same(3), Color32::from_rgba_unmultiplied(20, 26, 21, 230));
            let border = if selected {
                let pulse = (128.0 + 90.0 * (t * 4.0).sin()) as u8;
                a(GREEN, pulse.max(90))
            } else {
                a(Color32::from_rgb(0x3a, 0x4a, 0x3c), 200)
            };
            p.rect_stroke(card, CornerRadius::same(3), Stroke::new(1.0, border), StrokeKind::Middle);
            p.galley(origin + vec2(pad, pad), galley, TXT);
            p.galley(origin + vec2(pad, pad + 3.0 * s) + vec2(0.0, card.height() - pad * 2.0 - ts_galley.size().y - 3.0 * s), ts_galley, FAINT);
            capture_zones.push((card, i));
        }
        for (r, i) in capture_zones {
            self.clicks.push((r, ClickZone::Capture(i)));
        }

        // bases
        for i in 0..self.world.projects.len() {
            let (px, py) = self.world.projects[i].pos;
            self.draw_base(p, i, to(pos2(px, py)), s, t);
        }

        // structures owned by bases: pylons = goals/tasks, sensor arrays = questions.
        // drawn (and their zones pushed) after the bases so they win the click
        // hit-test; ss applies the configurable substructure size ratio.
        let ss = s * self.prefs.struct_scale;
        let mut zones: Vec<(Rect, ClickZone)> = vec![];
        for pi in 0..self.world.projects.len() {
            let base_sel = self.sel == Some(pi);
            let show_label = base_sel || s >= 0.75;
            let bc = to(self.base_center(pi));
            let bcol = self.world.projects[pi].color;
            // faint dashed feed line from each structure back to its base
            let feed = |p: &Painter, o: Pos2, selected: bool| {
                let alpha = if base_sel || selected { 130 } else { 48 };
                p.extend(egui::Shape::dashed_line(&[bc, o], Stroke::new(1.0, a(bcol, alpha)), 5.0, 7.0));
            };
            for ti in 0..self.world.projects[pi].tasks.len() {
                let wp = self.pylon_world_pos(pi, ti);
                let o = to(pos2(wp.0, wp.1));
                if !rect.expand(140.0).contains(o) {
                    continue;
                }
                let (state, title) = {
                    let tk = &self.world.projects[pi].tasks[ti];
                    (tk.state, tk.title.clone())
                };
                let selected = self.sel_structs.contains(&SRoom::Pylon(pi, ti));
                feed(p, o, selected);
                let phase = (pi * 7 + ti) as f64 * 1.31;
                let zr = Self::draw_pylon(p, o, ss, t, phase, state, (show_label || selected).then_some(title.as_str()));
                if selected {
                    Self::draw_sel_ring(p, zr, t);
                }
                zones.push((zr, ClickZone::Pylon(pi, ti)));
            }
            for qi in 0..self.world.projects[pi].questions.len() {
                let wp = self.question_world_pos(pi, qi);
                let o = to(pos2(wp.0, wp.1));
                if !rect.expand(140.0).contains(o) {
                    continue;
                }
                let (resolved, text) = {
                    let q = &self.world.projects[pi].questions[qi];
                    (q.resolved, q.text.clone())
                };
                let selected = self.sel_structs.contains(&SRoom::Question(pi, qi));
                feed(p, o, selected);
                let phase = (pi * 5 + qi) as f64 * 1.73;
                let zr = Self::draw_sensor(p, o, ss, t, phase, resolved, (show_label || selected).then_some(text.as_str()));
                if selected {
                    Self::draw_sel_ring(p, zr, t);
                }
                zones.push((zr, ClickZone::Question(pi, qi)));
            }
        }
        self.clicks.extend(zones);

        // rail toggle tab (canvas-drawn so both real and injected clicks reach it)
        {
            let label = if self.prefs.show_rail {
                "◂ hide bases".to_string()
            } else {
                format!("☰ bases {}", self.world.projects.len())
            };
            let g = p.layout_no_wrap(label, FontId::monospace(10.0), DIM);
            let tab = Rect::from_min_size(pos2(rect.min.x + 10.0, rect.min.y + 10.0), g.size() + vec2(16.0, 10.0));
            p.rect_filled(tab, CornerRadius::same(3), a(PANEL2, 230));
            p.rect_stroke(tab, CornerRadius::same(3), Stroke::new(1.0, LINE), StrokeKind::Middle);
            p.galley(tab.min + vec2(8.0, 5.0), g, DIM);
            self.clicks.push((tab, ClickZone::RailToggle));
        }

        // ctrl+drag selection band
        if let Some((a0, b0)) = self.band {
            let r = Rect::from_two_pos(a0, b0);
            p.rect_filled(r, CornerRadius::ZERO, a(GREEN, 14));
            p.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, a(GREEN, 170)), StrokeKind::Middle);
        }
        // group-selection readout
        if !self.sel_structs.is_empty() {
            let txt = format!("▣ {} structure{} selected · [d][d] demolish · esc clears", self.sel_structs.len(), if self.sel_structs.len() > 1 { "s" } else { "" });
            p.text(pos2(rect.center().x, rect.max.y - 72.0), Align2::CENTER_BOTTOM, txt, FontId::monospace(11.0), GREEN);
        }

        // placement markers while a new structure is being named
        if self.pylon_open {
            let c = to(self.pylon_pos);
            let pulse = (120.0 + 100.0 * (t * 4.0).sin()) as u8;
            let pts = vec![c + vec2(0.0, -16.0 * s), c + vec2(9.0 * s, 0.0), c + vec2(0.0, 12.0 * s), c + vec2(-9.0 * s, 0.0)];
            p.add(egui::Shape::convex_polygon(pts, a(CYAN, 30), Stroke::new(1.5, a(CYAN, pulse.max(90)))));
            p.text(c + vec2(0.0, -30.0 * s), Align2::CENTER_CENTER, "◆ new pylon — goal", FontId::monospace((10.0 * s).max(6.0)), CYAN);
        }
        if self.quest_open {
            let c = to(self.quest_pos);
            let pulse = (120.0 + 100.0 * (t * 4.0).sin()) as u8;
            p.circle_stroke(c, 12.0 * s, Stroke::new(1.5, a(AMBER, pulse.max(90))));
            p.text(c, Align2::CENTER_CENTER, "?", FontId::monospace((11.0 * s).max(7.0)), a(AMBER, 230));
            p.text(c + vec2(0.0, -30.0 * s), Align2::CENTER_CENTER, "⌖ new sensor — question", FontId::monospace((10.0 * s).max(6.0)), AMBER);
        }

        // world pings (expanding rings)
        let nproj = self.world.projects.len();
        self.wpings.retain(|pg| t - pg.created < 7.0 && pg.proj < nproj);
        for pg in &self.wpings {
            let age = (t - pg.created) as f32;
            let c = to(self.base_center(pg.proj));
            let r = (12.0 + (age % 2.4) / 2.4 * 46.0) * s;
            let alpha = ((1.0 - age / 7.0) * 180.0) as u8;
            p.circle_stroke(c, r, Stroke::new(2.0, a(pg.color, alpha)));
        }
    }

    fn draw_base(&mut self, p: &Painter, i: usize, origin: Pos2, s: f32, t: f64) {
        let proj = &self.world.projects[i];
        let tier = self.tier(i);
        let sel = self.sel == Some(i);
        let pending_dec: Vec<usize> = self
            .world
            .decisions
            .iter()
            .enumerate()
            .filter(|(_, d)| d.proj == i && !d.resolved)
            .map(|(di, _)| di)
            .collect();

        let pt = |x: f32, y: f32| origin + vec2(x * s, y * s);

        // pre-layout decision flag to know its height
        let mut dflag_galleys = vec![];
        for &di in &pending_dec {
            let d = &self.world.decisions[di];
            let g = p.layout(
                format!("◆ DECISION PENDING — {} (due {})", d.title, d.due),
                FontId::monospace((10.0 * s).max(5.5)),
                AMBER,
                330.0 * s,
            );
            dflag_galleys.push(g);
        }
        let dflag_h: f32 = dflag_galleys.iter().map(|g| g.size().y / s + 12.0).sum();

        let agents_h = if proj.agents.is_empty() { 22.0 } else { 66.0 };
        let body_h = 108.0 + agents_h + dflag_h + 8.0;
        let body = Rect::from_min_size(origin, vec2(BASE_W * s, body_h * s));

        // body
        p.rect_filled(body, CornerRadius::same(4), Color32::from_rgba_unmultiplied(15, 22, 16, 238));
        let border = if sel {
            let pulse = (128.0 + 90.0 * (t * 4.0).sin()) as u8;
            a(GREEN, pulse.max(90))
        } else {
            LINE
        };
        p.rect_stroke(body, CornerRadius::same(4), Stroke::new(1.0, border), StrokeKind::Middle);

        // header
        p.rect_filled(
            Rect::from_min_size(origin, vec2(BASE_W * s, 28.0 * s)),
            CornerRadius::same(4),
            Color32::from_rgba_unmultiplied(0x2e, 0x44, 0x33, 70),
        );
        // large building portrait (strategy_building.jpeg sheet) filling the card's left column
        if let Some(tex) = &self.icons_tex {
            let ir = Rect::from_min_size(pt(10.0, 36.0), vec2(64.0 * s, 64.0 * s));
            p.rect_filled(ir, CornerRadius::same(3), Color32::from_rgb(0x0a, 0x10, 0x0b));
            p.image(tex.id(), ir, icon_uv(proj.icon), Color32::WHITE);
            p.rect_stroke(ir, CornerRadius::same(3), Stroke::new(1.5, a(proj.color, 220)), StrokeKind::Middle);
        } else {
            p.rect_filled(Rect::from_min_size(pt(10.0, 36.0), vec2(10.0 * s, 64.0 * s)), CornerRadius::ZERO, proj.color);
        }
        // title centered in the header
        p.text(pt(BASE_W / 2.0, 14.0), Align2::CENTER_CENTER, &proj.name, FontId::proportional((13.0 * s).max(7.0)), TXT);
        // status as a play/pause glyph at the header's right
        let status_col = match proj.status.as_str() {
            "deadline" => AMBER,
            "background" => FAINT,
            _ => GREEN,
        };
        let sc = pt(BASE_W - 18.0, 14.0);
        if proj.status == "background" {
            // paused: two bars
            for dx in [-2.6, 2.6] {
                p.rect_filled(
                    Rect::from_center_size(sc + vec2(dx * s, 0.0), vec2(2.6 * s, 10.0 * s)),
                    CornerRadius::ZERO,
                    status_col,
                );
            }
        } else {
            // running: play triangle
            let r = 5.5 * s;
            p.add(egui::Shape::convex_polygon(
                vec![sc + vec2(-r * 0.7, -r), sc + vec2(-r * 0.7, r), sc + vec2(r, 0.0)],
                status_col,
                Stroke::NONE,
            ));
        }
        // whole card selects the base (units/decision flags pushed later win on overlap)
        self.clicks.push((body, ClickZone::FocusBase(i)));

        // goal
        let goal_txt = if proj.goal.is_empty() { "🎯 no objective set".to_string() } else { format!("🎯 {}", proj.goal) };
        p.text(pt(84.0, 40.0), Align2::LEFT_CENTER, goal_txt, FontId::proportional((11.0 * s).max(6.0)), DIM);

        // tasks (structures) — laid out right of the portrait column
        for (ti, task) in proj.tasks.iter().enumerate() {
            let bx = 84.0 + ti as f32 * 74.0;
            let box_r = Rect::from_min_size(pt(bx + 4.0, 54.0), vec2(60.0 * s, 24.0 * s));
            let (fill, stroke, glyph, gcol) = match task.state {
                TaskState::Done => (Color32::from_rgb(0x15, 0x24, 0x17), Color32::from_rgb(0x3a, 0x6b, 0x44), "✓", GREEN),
                TaskState::Doing => (Color32::from_rgb(0x1a, 0x2a, 0x1c), GREEN_DK, "", GREEN),
                TaskState::Todo => (Color32::from_rgba_unmultiplied(20, 30, 22, 120), FAINT, "·", FAINT),
                TaskState::Blocked => (Color32::from_rgb(0x2a, 0x14, 0x14), Color32::from_rgb(0x7a, 0x2f, 0x2f), "✖", RED),
            };
            p.rect_filled(box_r, CornerRadius::same(2), fill);
            p.rect_stroke(box_r, CornerRadius::same(2), Stroke::new(1.0, stroke), StrokeKind::Middle);
            if task.state == TaskState::Doing {
                // animated build bar
                let frac = ((t * 0.55).fract()) as f32;
                let bar = Rect::from_min_size(
                    box_r.left_bottom() + vec2(3.0 * s, -6.0 * s),
                    vec2((box_r.width() - 6.0 * s) * frac, 3.5 * s),
                );
                p.rect_filled(bar, CornerRadius::same(2), a(GREEN, 200));
            } else {
                p.text(box_r.center(), Align2::CENTER_CENTER, glyph, FontId::proportional((12.0 * s).max(6.0)), gcol);
            }
            let label = p.layout(task.title.clone(), FontId::proportional((8.5 * s).max(5.0)), DIM, 68.0 * s);
            let lx = pt(bx + 34.0, 82.0).x - label.size().x / 2.0;
            p.galley(pos2(lx, pt(0.0, 81.0).y), label, DIM);
        }

        // agents (units)
        let ay = 108.0;
        if proj.agents.is_empty() {
            p.text(
                pt(12.0, ay + 8.0),
                Align2::LEFT_CENTER,
                "no units garrisoned — manual theater",
                FontId::monospace((10.0 * s).max(5.5)),
                FAINT,
            );
        } else {
            for (ai, ag) in proj.agents.iter().enumerate() {
                let cx = 46.0 + ai as f32 * 92.0;
                let c = pt(cx, ay + 20.0);
                let r = 16.0 * s;
                p.circle_filled(c, r, Color32::from_rgb(0x16, 0x22, 0x18));
                let (ring, icon) = match ag.state {
                    AgentState::Working => {
                        let glow = (40.0 + 50.0 * (t * 3.0).sin().abs()) as u8;
                        p.circle_stroke(c, r + 5.0 * s, Stroke::new(1.0, a(GREEN, glow)));
                        (GREEN_DK, "⚙")
                    }
                    AgentState::Blocked => {
                        let blink = if (t * 1.8).fract() < 0.5 { 255 } else { 90 };
                        (a(RED, blink), "✖")
                    }
                    AgentState::Idle => (a(Color32::from_rgb(0x4a, 0x5a, 0x4c), 200), "☕"),
                };
                p.circle_stroke(c, r, Stroke::new((2.0 * s).max(1.0), ring));
                p.text(c, Align2::CENTER_CENTER, icon, FontId::proportional((14.0 * s).max(7.0)), TXT);
                if ag.state == AgentState::Blocked {
                    p.text(c + vec2(r * 0.9, -r * 1.1), Align2::CENTER_CENTER, "⚠", FontId::proportional((10.0 * s).max(6.0)), AMBER);
                }
                p.text(
                    c + vec2(0.0, r + 8.0 * s),
                    Align2::CENTER_CENTER,
                    &ag.id,
                    FontId::monospace((9.0 * s).max(5.0)),
                    Color32::from_rgb(0xa8, 0xc2, 0xab),
                );
                p.text(
                    c + vec2(0.0, r + 18.0 * s),
                    Align2::CENTER_CENTER,
                    format!("{} · {}", ag.state.label(), ag.last_report),
                    FontId::proportional((8.0 * s).max(5.0)),
                    DIM,
                );
                self.clicks.push((
                    Rect::from_center_size(c + vec2(0.0, 6.0 * s), vec2(84.0 * s, 62.0 * s)),
                    ClickZone::Unit(i, ag.id.to_string()),
                ));
            }
        }

        // decision flags
        let mut dy = ay + agents_h;
        for (gi, &di) in pending_dec.iter().enumerate() {
            let g = &dflag_galleys[gi];
            let fh = g.size().y + 8.0 * s;
            let fr = Rect::from_min_size(pt(10.0, dy), vec2((BASE_W - 20.0) * s, fh));
            let pulse = (90.0 + 60.0 * (t * 2.8).sin()) as u8;
            p.rect_filled(fr, CornerRadius::same(3), a(AMBER, 24));
            p.rect_stroke(fr, CornerRadius::same(3), Stroke::new(1.0, a(AMBER, pulse.max(80))), StrokeKind::Middle);
            p.galley(fr.min + vec2(6.0 * s, 4.0 * s), g.clone(), AMBER);
            self.clicks.push((fr, ClickZone::Decision(di)));
            dy += fh / s + 4.0;
        }

        // fog of staleness
        let fog = tier.fog_alpha();
        if fog > 0 {
            p.rect_filled(body, CornerRadius::same(4), Color32::from_rgba_unmultiplied(5, 8, 6, fog));
            let chip = format!("⌚ {}", self.age_str(i));
            let g = p.layout_no_wrap(chip, FontId::monospace((10.0 * s).max(6.0)), Color32::from_rgb(0x8f, 0xa3, 0x92));
            let cr = Rect::from_min_size(
                body.right_bottom() - vec2(g.size().x + 18.0 * s, g.size().y + 12.0 * s),
                g.size() + vec2(12.0 * s, 8.0 * s),
            );
            p.rect_filled(cr, CornerRadius::same(2), Color32::from_rgba_unmultiplied(7, 11, 8, 220));
            p.rect_stroke(cr, CornerRadius::same(2), Stroke::new(1.0, Color32::from_rgb(0x2a, 0x3a, 0x2e)), StrokeKind::Middle);
            let chip_col = if tier == Tier::Cold { AMBER } else { Color32::from_rgb(0x8f, 0xa3, 0x92) };
            p.galley(cr.min + vec2(6.0 * s, 4.0 * s), g, chip_col);
        }

        // selection brackets
        if sel {
            let l = 16.0 * s.max(0.5);
            let off = 6.0;
            let stroke = Stroke::new(2.0, GREEN);
            let corners = [
                (body.left_top() + vec2(-off, -off), vec2(l, 0.0), vec2(0.0, l)),
                (body.right_top() + vec2(off, -off), vec2(-l, 0.0), vec2(0.0, l)),
                (body.left_bottom() + vec2(-off, off), vec2(l, 0.0), vec2(0.0, -l)),
                (body.right_bottom() + vec2(off, off), vec2(-l, 0.0), vec2(0.0, -l)),
            ];
            for (c, dx, dyv) in corners {
                p.line_segment([c, c + dx], stroke);
                p.line_segment([c, c + dyv], stroke);
            }
        }
    }

    // ---------- building interior ----------
    /// pulsing corner brackets around a group-selected substructure
    fn draw_sel_ring(p: &Painter, r: Rect, t: f64) {
        let r = r.expand(3.0);
        let pulse = (150.0 + 80.0 * (t * 3.0).sin()) as u8;
        let col = a(GREEN, pulse);
        let l = (r.width().min(r.height()) * 0.3).clamp(4.0, 10.0);
        let st = Stroke::new(1.5, col);
        for (c, dx, dy) in [
            (r.left_top(), 1.0, 1.0),
            (r.right_top(), -1.0, 1.0),
            (r.left_bottom(), 1.0, -1.0),
            (r.right_bottom(), -1.0, -1.0),
        ] {
            p.line_segment([c, c + vec2(dx * l, 0.0)], st);
            p.line_segment([c, c + vec2(0.0, dy * l)], st);
        }
    }

    /// starcraft-style pylon: floating crystal over a ground plate; represents one
    /// goal/task of its base. powered look follows the task state.
    fn draw_pylon(p: &Painter, o: Pos2, s: f32, t: f64, phase: f64, state: TaskState, label: Option<&str>) -> Rect {
        let col = match state {
            TaskState::Todo => Color32::from_rgb(0x8f, 0xa8, 0xb8),
            TaskState::Doing => CYAN,
            TaskState::Done => GREEN,
            TaskState::Blocked => RED,
        };
        let powered = matches!(state, TaskState::Doing | TaskState::Blocked);
        let bob = (if powered { (t * 2.4 + phase).sin() * 3.0 } else { (t * 1.1 + phase).sin() * 1.5 }) as f32 * s;
        let cc = o + vec2(0.0, -18.0 * s + bob);

        // power field (doing) / alarm ring (blocked)
        if state == TaskState::Doing {
            let pulse = (0.5 + 0.5 * (t * 1.8 + phase).sin()) as f32;
            let r = 84.0 * s;
            p.circle_filled(o, r, a(col, 7));
            p.circle_stroke(o, r, Stroke::new(1.0, a(col, 36 + (pulse * 40.0) as u8)));
        }
        if state == TaskState::Blocked {
            let flicker = ((t * 6.0 + phase).sin() * 0.5 + 0.5) as f32;
            p.circle_stroke(o, 60.0 * s, Stroke::new(1.2, a(col, 40 + (flicker * 90.0) as u8)));
        }

        // ground plate
        let plate = vec![o + vec2(-11.0 * s, 0.0), o + vec2(0.0, 4.5 * s), o + vec2(11.0 * s, 0.0), o + vec2(0.0, -4.5 * s)];
        p.add(egui::Shape::convex_polygon(plate, a(col, 26), Stroke::new(1.0, a(col, 80))));

        // crystal glow
        if powered {
            p.circle_filled(cc, 16.0 * s, a(col, 16));
            p.circle_filled(cc, 8.0 * s, a(col, 28));
        }
        // crystal body + inner facet
        let body = vec![cc + vec2(0.0, -17.0 * s), cc + vec2(8.5 * s, -4.0 * s), cc + vec2(0.0, 13.0 * s), cc + vec2(-8.5 * s, -4.0 * s)];
        p.add(egui::Shape::convex_polygon(
            body,
            a(col, if powered { 110 } else { 45 }),
            Stroke::new(1.3, a(col, if powered { 230 } else { 140 })),
        ));
        let facet = vec![cc + vec2(0.0, -10.0 * s), cc + vec2(4.2 * s, -4.0 * s), cc + vec2(0.0, 5.0 * s), cc + vec2(-4.2 * s, -4.0 * s)];
        p.add(egui::Shape::convex_polygon(facet, a(col, if powered { 190 } else { 80 }), Stroke::NONE));

        if let Some(title) = label {
            let glyph = match state {
                TaskState::Todo => "▫",
                TaskState::Doing => "◐",
                TaskState::Done => "✓",
                TaskState::Blocked => "✖",
            };
            let mut txt: String = title.chars().take(24).collect();
            if title.chars().count() > 24 {
                txt.push('…');
            }
            p.text(
                o + vec2(0.0, 12.0 * s),
                Align2::CENTER_TOP,
                format!("{} {}", glyph, txt),
                FontId::monospace((9.0 * s).max(6.0)),
                a(col, 220),
            );
        }
        // zone spans crystal top to ground plate so small structures stay clickable
        Rect::from_center_size(o + vec2(0.0, -16.0 * s), vec2((30.0 * s).max(20.0), (52.0 * s).max(30.0)))
    }

    /// sensor array: mast + dish with a sweeping radar beam; represents one open
    /// question / research thread of its base. resolved arrays go quiet and green.
    fn draw_sensor(p: &Painter, o: Pos2, s: f32, t: f64, phase: f64, resolved: bool, label: Option<&str>) -> Rect {
        let col = if resolved { GREEN } else { AMBER };
        let m = o + vec2(0.0, -14.0 * s); // mast top / dish pivot

        // ground plate + mast
        let plate = vec![o + vec2(-10.0 * s, 0.0), o + vec2(0.0, 4.0 * s), o + vec2(10.0 * s, 0.0), o + vec2(0.0, -4.0 * s)];
        p.add(egui::Shape::convex_polygon(plate, a(col, 24), Stroke::new(1.0, a(col, 80))));
        p.line_segment([o, m], Stroke::new(1.3, a(col, 170)));

        // dish: upward-opening arc on the mast
        let dish: Vec<Pos2> = (0..=10)
            .map(|k| {
                let ang = std::f32::consts::PI * (1.15 + 0.7 * k as f32 / 10.0); // ~207°..333°
                m + vec2(ang.cos() * 9.0 * s, ang.sin() * 7.0 * s)
            })
            .collect();
        p.add(egui::Shape::line(dish, Stroke::new(1.3, a(col, 210))));

        if resolved {
            p.text(m + vec2(0.0, -12.0 * s), Align2::CENTER_CENTER, "✓", FontId::monospace((10.0 * s).max(6.0)), a(col, 200));
        } else {
            // sweeping radar beam + expanding scan ring
            let ang = ((t * 1.4 + phase) % std::f64::consts::TAU) as f32;
            let dir = vec2(ang.cos(), ang.sin() * 0.55);
            p.line_segment([m, m + dir * 30.0 * s], Stroke::new(1.2, a(col, 170)));
            let trail = ang - 0.4;
            p.line_segment([m, m + vec2(trail.cos(), trail.sin() * 0.55) * 26.0 * s], Stroke::new(1.0, a(col, 70)));
            p.circle_filled(m + dir * 30.0 * s, 2.0 * s.max(0.8), a(col, 220));
            let ring = ((t * 0.6 + phase).fract()) as f32;
            p.circle_stroke(m, (10.0 + ring * 55.0) * s, Stroke::new(1.0, a(col, ((1.0 - ring) * 90.0) as u8)));
            // floating question glyph
            let bob = ((t * 2.0 + phase).sin() * 2.5) as f32 * s;
            p.text(m + vec2(0.0, -16.0 * s + bob), Align2::CENTER_CENTER, "?", FontId::monospace((11.0 * s).max(7.0)), a(col, 230));
        }

        if let Some(text) = label {
            let glyph = if resolved { "✓" } else { "?" };
            let mut txt: String = text.chars().take(24).collect();
            if text.chars().count() > 24 {
                txt.push('…');
            }
            p.text(
                o + vec2(0.0, 10.0 * s),
                Align2::CENTER_TOP,
                format!("{} {}", glyph, txt),
                FontId::monospace((9.0 * s).max(6.0)),
                a(col, 220),
            );
        }
        Rect::from_center_size(o + vec2(0.0, -14.0 * s), vec2((32.0 * s).max(20.0), (48.0 * s).max(30.0)))
    }

    /// interior space of a single structure: the pylon's goal or the sensor's
    /// question, blown up with state controls, its editable brief and its slice
    /// of the event feed. returns the brief panel's rect: the caller mounts the
    /// text editor there (painter-only drawing can't host a widget).
    fn draw_struct_room(&mut self, p: &Painter, rect: Rect, room: SRoom) -> Rect {
        self.clicks.clear();
        let t = self.time;
        let (pi, title, col, sub, is_pylon, cur_state, resolved) = match room {
            SRoom::Pylon(pi, ti) => {
                let tk = &self.world.projects[pi].tasks[ti];
                let col = match tk.state {
                    TaskState::Todo => Color32::from_rgb(0x8f, 0xa8, 0xb8),
                    TaskState::Doing => CYAN,
                    TaskState::Done => GREEN,
                    TaskState::Blocked => RED,
                };
                (pi, tk.title.clone(), col, format!("goal pylon · state {}", tk.state.label().to_uppercase()), true, tk.state, false)
            }
            SRoom::Question(pi, qi) => {
                let q = &self.world.projects[pi].questions[qi];
                let col = if q.resolved { GREEN } else { AMBER };
                (
                    pi,
                    q.text.clone(),
                    col,
                    format!("research sensor · {}", if q.resolved { "RESOLVED" } else { "SCANNING" }),
                    false,
                    TaskState::Todo,
                    q.resolved,
                )
            }
        };
        let base = self.world.projects[pi].name.clone();
        let base_col = self.world.projects[pi].color;

        // floor + faint tile grid
        p.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(0x08, 0x0d, 0x09));
        let step = 42.0;
        let mut gx = rect.min.x + step;
        while gx < rect.max.x {
            p.line_segment([pos2(gx, rect.min.y), pos2(gx, rect.max.y)], Stroke::new(1.0, Color32::from_rgba_unmultiplied(60, 90, 64, 14)));
            gx += step;
        }
        let mut gy = rect.min.y + step;
        while gy < rect.max.y {
            p.line_segment([pos2(rect.min.x, gy), pos2(rect.max.x, gy)], Stroke::new(1.0, Color32::from_rgba_unmultiplied(60, 90, 64, 14)));
            gy += step;
        }

        // walls in the structure's state color
        let inner = rect.shrink(16.0);
        p.rect_stroke(inner, CornerRadius::same(6), Stroke::new(2.0, a(col, 200)), StrokeKind::Middle);
        p.rect_stroke(inner.shrink(5.0), CornerRadius::same(5), Stroke::new(1.0, a(col, 55)), StrokeKind::Middle);

        // header (indented past the base rail overlay)
        // small structure icon at the top-left of the header — enough to stay oriented,
        // leaving the room itself to the worker's report.
        let ibox = Rect::from_min_size(pos2(inner.min.x + 200.0, inner.min.y + 12.0), vec2(56.0, 62.0));
        p.rect_filled(ibox, CornerRadius::same(4), Color32::from_rgb(0x0a, 0x10, 0x0b));
        p.rect_stroke(ibox, CornerRadius::same(4), Stroke::new(1.0, a(col, 160)), StrokeKind::Middle);
        {
            let ip = p.with_clip_rect(ibox.shrink(1.0));
            let ground = pos2(ibox.center().x, ibox.max.y - 7.0);
            match room {
                SRoom::Pylon(pi2, ti) => {
                    let state = self.world.projects[pi2].tasks[ti].state;
                    Self::draw_pylon(&ip, ground, 1.3, t, 0.0, state, None);
                }
                SRoom::Question(pi2, qi) => {
                    let r = self.world.projects[pi2].questions[qi].resolved;
                    Self::draw_sensor(&ip, ground, 1.3, t, 0.0, r, None);
                }
            }
        }
        let head = pos2(ibox.max.x + 14.0, inner.min.y + 18.0);
        let glyph = if is_pylon { "◆ PYLON" } else { "⌖ SENSOR ARRAY" };
        p.text(head, Align2::LEFT_TOP, format!("{} — {}", glyph, title.to_uppercase()), FontId::proportional(20.0), TXT);
        p.text(head + vec2(0.0, 28.0), Align2::LEFT_TOP, sub, FontId::monospace(10.0), a(col, 220));
        p.text(head + vec2(0.0, 44.0), Align2::LEFT_TOP, format!("owned by base ⌂ {}", base), FontId::monospace(10.0), a(base_col, 220));

        // exit door (top-right)
        let g = p.layout_no_wrap("⤺ EXIT TO MAP · esc / 2×click floor".into(), FontId::monospace(11.0), TXT);
        let chip = Rect::from_min_size(pos2(inner.max.x - g.size().x - 34.0, inner.min.y + 14.0), g.size() + vec2(20.0, 12.0));
        p.rect_filled(chip, CornerRadius::same(3), Color32::from_rgba_unmultiplied(16, 24, 17, 235));
        p.rect_stroke(chip, CornerRadius::same(3), Stroke::new(1.0, LINE_HI), StrokeKind::Middle);
        p.galley(chip.min + vec2(10.0, 6.0), g, TXT);
        self.clicks.push((chip, ClickZone::ExitInterior));

        // state chips — click to set
        let chips: Vec<(String, Color32, ClickZone, bool)> = if is_pylon {
            let (pi2, ti) = match room {
                SRoom::Pylon(a0, b0) => (a0, b0),
                _ => unreachable!(),
            };
            [TaskState::Todo, TaskState::Doing, TaskState::Done, TaskState::Blocked]
                .into_iter()
                .map(|st| {
                    let c = match st {
                        TaskState::Todo => Color32::from_rgb(0x8f, 0xa8, 0xb8),
                        TaskState::Doing => CYAN,
                        TaskState::Done => GREEN,
                        TaskState::Blocked => RED,
                    };
                    (st.label().to_uppercase(), c, ClickZone::SetTask(pi2, ti, st), st == cur_state)
                })
                .collect()
        } else {
            let (pi2, qi) = match room {
                SRoom::Question(a0, b0) => (a0, b0),
                _ => unreachable!(),
            };
            vec![
                ("SCANNING".into(), AMBER, ClickZone::SetQuest(pi2, qi, false), !resolved),
                ("RESOLVED".into(), GREEN, ClickZone::SetQuest(pi2, qi, true), resolved),
            ]
        };
        let mut widths = vec![];
        let mut total = 0.0;
        for (label, ..) in &chips {
            let w = p.layout_no_wrap(label.clone(), FontId::monospace(11.0), TXT).size().x + 26.0;
            widths.push(w);
            total += w + 10.0;
        }
        let mut cx = inner.center().x - (total - 10.0) / 2.0;
        let cy = inner.min.y + 88.0;
        for ((label, c, zone, active), w) in chips.into_iter().zip(widths) {
            let r = Rect::from_min_size(pos2(cx, cy), vec2(w, 26.0));
            p.rect_filled(r, CornerRadius::same(3), if active { a(c, 55) } else { Color32::from_rgba_unmultiplied(16, 24, 17, 235) });
            let pulse = if active { (150.0 + 70.0 * (t * 2.5).sin()) as u8 } else { 90 };
            p.rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0, a(c, pulse)), StrokeKind::Middle);
            p.text(r.center(), Align2::CENTER_CENTER, label, FontId::monospace(11.0), if active { c } else { DIM });
            self.clicks.push((r, zone));
            cx += w + 10.0;
        }

        // worker row (pylons only): who is on this pylon, dispatch / halt
        let mut log_top = cy + 48.0;
        if let SRoom::Pylon(pi2, ti) = room {
            let title2 = self.world.projects[pi2].tasks[ti].title.clone();
            let pname = self.world.projects[pi2].name.clone();
            let unit = self.world.projects[pi2].agents.iter().find(|ag| ag.task == title2).cloned();
            let running = unit.as_ref().map_or(false, |ag| self.workers.running(&pname, &ag.id));
            let has_repo = self.world.projects[pi2].cwd.is_some();
            let mut wchips: Vec<(String, Color32, ClickZone)> = vec![];
            match &unit {
                Some(ag) if running => wchips.push((format!("■ HALT {}", ag.id), RED, ClickZone::HaltUnit(pi2, ag.id.clone()))),
                Some(ag) => wchips.push((format!("⚙ RE-DISPATCH {}", ag.id), CYAN, ClickZone::Dispatch(pi2, ti))),
                None => wchips.push(("⚙ DISPATCH WORKER  [w]".into(), CYAN, ClickZone::Dispatch(pi2, ti))),
            }
            let mut ws = vec![];
            let mut wtotal = 0.0;
            for (label, ..) in &wchips {
                let w = p.layout_no_wrap(label.clone(), FontId::monospace(11.0), TXT).size().x + 26.0;
                ws.push(w);
                wtotal += w + 10.0;
            }
            let mut wx = inner.center().x - (wtotal - 10.0) / 2.0;
            let wy = cy + 36.0;
            for ((label, c, zone), w) in wchips.into_iter().zip(ws) {
                let r = Rect::from_min_size(pos2(wx, wy), vec2(w, 26.0));
                p.rect_filled(r, CornerRadius::same(3), Color32::from_rgba_unmultiplied(16, 24, 17, 235));
                let pulse = if running { (150.0 + 70.0 * (t * 2.5).sin()) as u8 } else { 140 };
                p.rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0, a(c, pulse)), StrokeKind::Middle);
                p.text(r.center(), Align2::CENTER_CENTER, label, FontId::monospace(11.0), c);
                self.clicks.push((r, zone));
                wx += w + 10.0;
            }
            let repo_line = match &self.world.projects[pi2].cwd {
                Some(c) => format!("repo {}", c),
                None => "no repo set for this base — /base?i=..&cwd=/path".into(),
            };
            p.text(pos2(inner.center().x, wy + 34.0), Align2::CENTER_TOP, repo_line, FontId::monospace(9.5), if has_repo { FAINT } else { AMBER });
            log_top = wy + 60.0;
            if let Some(ag) = &unit {
                let head = format!(
                    "🪖 {} · {}{} · {} turns · {} tok{}",
                    ag.id,
                    ag.state.label(),
                    if running { " (codex running)" } else { "" },
                    ag.turns,
                    ag.tokens,
                    ag.thread_id.as_ref().map(|t| format!(" · thread {}", t.chars().take(8).collect::<String>())).unwrap_or_default()
                );
                p.text(pos2(inner.min.x + 22.0, log_top), Align2::LEFT_TOP, head, FontId::monospace(10.0), if running { CYAN } else { FAINT });
                log_top += 18.0;
                if !ag.last_msg.is_empty() {
                    let g = p.layout(ag.last_msg.clone(), FontId::monospace(10.5), Color32::from_rgb(0xb6, 0xc8, 0xb8), inner.width() - 44.0);
                    let max_h = (inner.max.y - log_top - 110.0).max(40.0);
                    let clip = Rect::from_min_size(pos2(inner.min.x + 22.0, log_top), vec2(inner.width() - 44.0, g.size().y.min(max_h)));
                    p.with_clip_rect(clip).galley(clip.min, g, TXT);
                    log_top = clip.max.y + 12.0;
                }
            }
        }

        // brief: the structure's body text. the title above is only its name on
        // the map; this surface holds what it actually means. editable in place —
        // the caller mounts a TextEdit over `brief` (see world_canvas).
        let brief_label = if is_pylon {
            "▤ BRIEF — what this pylon means · the title is just its name · click or [enter] to edit · esc leaves"
        } else {
            "▤ BRIEF — context, findings, the answer · the title is just its name · click or [enter] to edit · esc leaves"
        };
        p.text(pos2(inner.min.x + 22.0, log_top), Align2::LEFT_TOP, brief_label, FontId::monospace(10.0), FAINT);
        let avail = (inner.max.y - 10.0 - (log_top + 20.0)).max(120.0);
        let brief_h = (avail * 0.42).clamp(90.0, 320.0);
        let brief = Rect::from_min_size(pos2(inner.min.x + 22.0, log_top + 20.0), vec2(inner.width() - 44.0, brief_h));
        p.rect_filled(brief, CornerRadius::same(4), Color32::from_rgba_unmultiplied(12, 19, 14, 235));
        p.rect_stroke(brief, CornerRadius::same(4), Stroke::new(1.0, a(col, 90)), StrokeKind::Middle);
        log_top = brief.max.y + 16.0;

        // comms wall: this structure's slice of the base's event feed
        let related: Vec<(String, String)> = self
            .world
            .events
            .iter()
            .filter(|e| {
                e.proj == Some(pi)
                    && (e.text.to_lowercase().contains(&title.to_lowercase())
                        || matches!((room, &e.agent), (SRoom::Pylon(pi2, ti), Some(aid)) if self.world.projects[pi2].agents.iter().any(|ag| &ag.id == aid && ag.task == self.world.projects[pi2].tasks[ti].title)))
            })
            .rev()
            .take(12)
            .map(|e| {
                // one line per event: fold embedded newlines so a multi-line command can't overprint the next row
                let text = e.text.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ⏎ ");
                (format!("{}{}", e.ts, e.agent.as_ref().map(|a| format!(" {}", a)).unwrap_or_default()), text)
            })
            .collect();
        p.text(pos2(inner.min.x + 22.0, log_top), Align2::LEFT_TOP, "▤ RELATED TRAFFIC", FontId::monospace(10.0), FAINT);
        let mut ly = log_top + 20.0;
        if related.is_empty() {
            p.text(pos2(inner.min.x + 22.0, ly), Align2::LEFT_TOP, "no events mention this structure yet", FontId::monospace(10.5), FAINT);
        }
        for (ts, text) in related {
            if ly + 16.0 > inner.max.y - 10.0 {
                break;
            }
            p.text(pos2(inner.min.x + 22.0, ly), Align2::LEFT_TOP, format!("{}  {}", ts, text), FontId::monospace(10.5), Color32::from_rgb(0x9f, 0xb3, 0xa1));
            ly += 17.0;
        }
        brief
    }

    /// the editable surface inside a structure room: a frameless multiline
    /// editor mounted over the brief panel, writing straight into the model.
    /// while it has focus the hotkey layer is muted (keyboard() checks focus).
    fn brief_editor(&mut self, ui: &mut egui::Ui, r: Rect, room: SRoom) {
        let (hint, salt, col) = match room {
            SRoom::Pylon(pi, ti) => (
                "what this pylon actually is — the brief a dispatched worker receives along with the title…",
                ("brief-pylon", pi, ti),
                CYAN,
            ),
            SRoom::Question(pi, qi) => ("what is being asked, what has been found, the answer once it lands…", ("brief-quest", pi, qi), AMBER),
        };
        let notes: &mut String = match room {
            SRoom::Pylon(pi, ti) => &mut self.world.projects[pi].tasks[ti].notes,
            SRoom::Question(pi, qi) => &mut self.world.projects[pi].questions[qi].notes,
        };
        let pad = r.shrink2(vec2(10.0, 8.0));
        let rows = ((pad.height() / 15.0).floor() as usize).max(3);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(pad).layout(egui::Layout::top_down(egui::Align::Min)));
        let out = ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]).show(&mut child, |ui| {
            let resp = ui.add(
                egui::TextEdit::multiline(notes)
                    .frame(false)
                    .desired_width(f32::INFINITY)
                    .desired_rows(rows)
                    .font(FontId::monospace(11.5))
                    .text_color(Color32::from_rgb(0xc6, 0xd8, 0xc8))
                    .hint_text(RichText::new(hint).monospace().size(11.0).color(a(col, 110))),
            );
            resp
        });
        if self.brief_focus {
            out.inner.request_focus();
            self.brief_focus = false;
        }
        let focused = out.inner.has_focus();
        if focused && ui.input(|i| i.key_pressed(Key::Escape)) {
            // esc steps out of the editor (egui does this from raw input too; this
            // also covers injected key events, and keyboard() reads brief_focused
            // so the same press doesn't leave the room)
            out.inner.surrender_focus();
        }
        self.brief_focused = focused;
        if out.inner.changed() {
            self.dirty = true;
        }
    }

    fn draw_interior(&mut self, p: &Painter, rect: Rect, i: usize) {
        self.clicks.clear();
        let t = self.time;
        let proj = &self.world.projects[i];
        let color = proj.color;
        let name = proj.name.clone();
        let status = proj.status.clone();
        let goal = proj.goal.clone();
        let icon = proj.icon;
        let tasks = proj.tasks.clone();
        let agents = proj.agents.clone();
        let pending: Vec<(usize, String, String)> = self
            .world
            .decisions
            .iter()
            .enumerate()
            .filter(|(_, d)| d.proj == i && !d.resolved)
            .map(|(di, d)| (di, d.title.clone(), d.due.clone()))
            .collect();
        let events: Vec<(String, Option<String>, String)> = self
            .world
            .events
            .iter()
            .filter(|e| e.proj == Some(i))
            .rev()
            .take(8)
            .map(|e| (e.ts.clone(), e.agent.clone(), e.text.clone()))
            .collect();

        // floor + faint tile grid
        p.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(0x08, 0x0d, 0x09));
        let step = 42.0;
        let mut gx = rect.min.x + step;
        while gx < rect.max.x {
            p.line_segment([pos2(gx, rect.min.y), pos2(gx, rect.max.y)], Stroke::new(1.0, Color32::from_rgba_unmultiplied(60, 90, 64, 14)));
            gx += step;
        }
        let mut gy = rect.min.y + step;
        while gy < rect.max.y {
            p.line_segment([pos2(rect.min.x, gy), pos2(rect.max.x, gy)], Stroke::new(1.0, Color32::from_rgba_unmultiplied(60, 90, 64, 14)));
            gy += step;
        }

        // walls in the base's color
        let inner = rect.shrink(16.0);
        p.rect_stroke(inner, CornerRadius::same(6), Stroke::new(2.0, a(color, 200)), StrokeKind::Middle);
        p.rect_stroke(inner.shrink(5.0), CornerRadius::same(5), Stroke::new(1.0, a(color, 55)), StrokeKind::Middle);

        // header: portrait + name + status + goal
        let hpad = 22.0;
        let head = pos2(inner.min.x + hpad, inner.min.y + 18.0);
        if let Some(tex) = &self.icons_tex {
            let ir = Rect::from_min_size(head, vec2(84.0, 84.0));
            p.rect_filled(ir, CornerRadius::same(4), Color32::from_rgb(0x0a, 0x10, 0x0b));
            p.image(tex.id(), ir, icon_uv(icon), Color32::WHITE);
            p.rect_stroke(ir, CornerRadius::same(4), Stroke::new(2.0, a(color, 230)), StrokeKind::Middle);
        }
        let tx = head + vec2(100.0, 6.0);
        p.text(tx, Align2::LEFT_TOP, format!("◈ {}", name.to_uppercase()), FontId::proportional(22.0), TXT);
        p.text(
            tx + vec2(0.0, 30.0),
            Align2::LEFT_TOP,
            format!("INTERIOR VIEW · base {} · {} · visited {}", i + 1, status, self.age_str(i)),
            FontId::monospace(10.0),
            DIM,
        );
        let goal_txt = if goal.is_empty() { "🎯 no objective set".to_string() } else { format!("🎯 {}", goal) };
        p.text(tx + vec2(0.0, 48.0), Align2::LEFT_TOP, goal_txt, FontId::proportional(12.5), Color32::from_rgb(0xa8, 0xc2, 0xab));

        // exit door (top-right)
        let g = p.layout_no_wrap("⤺ EXIT TO MAP · esc / 2×click floor".into(), FontId::monospace(11.0), TXT);
        let chip = Rect::from_min_size(pos2(inner.max.x - g.size().x - 34.0, inner.min.y + 14.0), g.size() + vec2(20.0, 12.0));
        p.rect_filled(chip, CornerRadius::same(3), Color32::from_rgba_unmultiplied(16, 24, 17, 235));
        p.rect_stroke(chip, CornerRadius::same(3), Stroke::new(1.0, LINE_HI), StrokeKind::Middle);
        p.galley(chip.min + vec2(10.0, 6.0), g, TXT);
        self.clicks.push((chip, ClickZone::ExitInterior));

        // pending decisions — war-table banners under the header
        let mut dy = inner.min.y + 112.0;
        for (di, title, due) in &pending {
            let g = p.layout(
                format!("◆ DECISION PENDING — {} (due {}) · click to open briefing", title, due),
                FontId::monospace(11.0),
                AMBER,
                inner.width() - hpad * 2.0 - 16.0,
            );
            let fr = Rect::from_min_size(pos2(inner.min.x + hpad, dy), vec2(inner.width() - hpad * 2.0, g.size().y + 12.0));
            let pulse = (90.0 + 60.0 * (t * 2.8).sin()) as u8;
            p.rect_filled(fr, CornerRadius::same(3), a(AMBER, 22));
            p.rect_stroke(fr, CornerRadius::same(3), Stroke::new(1.0, a(AMBER, pulse.max(80))), StrokeKind::Middle);
            p.galley(fr.min + vec2(8.0, 6.0), g, AMBER);
            self.clicks.push((fr, ClickZone::Decision(*di)));
            dy += fr.height() + 8.0;
        }

        // body split: structures wing (left) · garrison (right) · comms wall (bottom)
        let log_h = (inner.height() * 0.26).clamp(120.0, 210.0);
        let body = Rect::from_min_max(pos2(inner.min.x + hpad, dy + 8.0), pos2(inner.max.x - hpad, inner.max.y - log_h - 14.0));
        let split = body.min.x + body.width() * 0.44;

        // structures wing
        let sw = Rect::from_min_max(body.min, pos2(split - 10.0, body.max.y));
        p.rect_stroke(sw, CornerRadius::same(4), Stroke::new(1.0, LINE), StrokeKind::Middle);
        p.text(sw.min + vec2(10.0, 10.0), Align2::LEFT_TOP, "🏗 STRUCTURES — TASKS", FontId::monospace(10.0), FAINT);
        let cols = 2usize;
        let cw = (sw.width() - 30.0) / cols as f32;
        for (ti, task) in tasks.iter().enumerate() {
            let cx = sw.min.x + 10.0 + (ti % cols) as f32 * (cw + 10.0);
            let cy = sw.min.y + 32.0 + (ti / cols) as f32 * 64.0;
            if cy + 54.0 > sw.max.y {
                break;
            }
            let room = Rect::from_min_size(pos2(cx, cy), vec2(cw, 54.0));
            let (fill, stroke, glyph, gcol) = match task.state {
                TaskState::Done => (Color32::from_rgb(0x15, 0x24, 0x17), Color32::from_rgb(0x3a, 0x6b, 0x44), "✓", GREEN),
                TaskState::Doing => (Color32::from_rgb(0x1a, 0x2a, 0x1c), GREEN_DK, "◐", GREEN),
                TaskState::Todo => (Color32::from_rgba_unmultiplied(20, 30, 22, 120), FAINT, "·", FAINT),
                TaskState::Blocked => (Color32::from_rgb(0x2a, 0x14, 0x14), Color32::from_rgb(0x7a, 0x2f, 0x2f), "✖", RED),
            };
            p.rect_filled(room, CornerRadius::same(3), fill);
            p.rect_stroke(room, CornerRadius::same(3), Stroke::new(1.0, stroke), StrokeKind::Middle);
            p.text(room.min + vec2(8.0, 8.0), Align2::LEFT_TOP, glyph, FontId::proportional(13.0), gcol);
            let label = p.layout(task.title.clone(), FontId::proportional(10.5), TXT, cw - 34.0);
            p.galley(room.min + vec2(26.0, 8.0), label, TXT);
            if task.state == TaskState::Doing {
                let frac = ((t * 0.55).fract()) as f32;
                let bar = Rect::from_min_size(room.left_bottom() + vec2(6.0, -9.0), vec2((room.width() - 12.0) * frac, 4.0));
                p.rect_filled(bar, CornerRadius::same(2), a(GREEN, 200));
            } else {
                p.text(room.left_bottom() + vec2(8.0, -6.0), Align2::LEFT_BOTTOM, task.state.label().to_uppercase(), FontId::monospace(8.0), FAINT);
            }
        }
        if tasks.is_empty() {
            p.text(sw.center(), Align2::CENTER_CENTER, "no structures", FontId::monospace(10.0), FAINT);
        }

        // garrison wing
        let gw = Rect::from_min_max(pos2(split + 10.0, body.min.y), body.max);
        p.rect_stroke(gw, CornerRadius::same(4), Stroke::new(1.0, LINE), StrokeKind::Middle);
        p.text(gw.min + vec2(10.0, 10.0), Align2::LEFT_TOP, "🪖 GARRISON — UNITS", FontId::monospace(10.0), FAINT);
        if agents.is_empty() {
            p.text(gw.center(), Align2::CENTER_CENTER, "no units garrisoned — manual theater", FontId::monospace(10.0), FAINT);
        }
        let per_row = (((gw.width() - 20.0) / 150.0).floor() as usize).max(1);
        for (ai, ag) in agents.iter().enumerate() {
            let cx = gw.min.x + 80.0 + (ai % per_row) as f32 * 150.0;
            let cy = gw.min.y + 78.0 + (ai / per_row) as f32 * 128.0;
            if cy + 50.0 > gw.max.y {
                break;
            }
            let c = pos2(cx, cy);
            let r = 26.0;
            p.circle_filled(c, r, Color32::from_rgb(0x16, 0x22, 0x18));
            let (ring, uicon) = match ag.state {
                AgentState::Working => {
                    let glow = (40.0 + 50.0 * (t * 3.0).sin().abs()) as u8;
                    p.circle_stroke(c, r + 7.0, Stroke::new(1.5, a(GREEN, glow)));
                    (GREEN_DK, "⚙")
                }
                AgentState::Blocked => {
                    let blink = if (t * 1.8).fract() < 0.5 { 255 } else { 90 };
                    (a(RED, blink), "✖")
                }
                AgentState::Idle => (a(Color32::from_rgb(0x4a, 0x5a, 0x4c), 200), "☕"),
            };
            p.circle_stroke(c, r, Stroke::new(2.5, ring));
            p.text(c, Align2::CENTER_CENTER, uicon, FontId::proportional(20.0), TXT);
            if ag.state == AgentState::Blocked {
                p.text(c + vec2(r * 0.9, -r * 1.1), Align2::CENTER_CENTER, "⚠", FontId::proportional(13.0), AMBER);
            }
            p.text(c + vec2(0.0, r + 12.0), Align2::CENTER_CENTER, &ag.id, FontId::monospace(11.0), Color32::from_rgb(0xa8, 0xc2, 0xab));
            p.text(
                c + vec2(0.0, r + 26.0),
                Align2::CENTER_CENTER,
                format!("{} · {}", ag.state.label(), ag.last_report),
                FontId::proportional(9.5),
                DIM,
            );
            let task_g = p.layout(ag.task.clone(), FontId::proportional(9.5), FAINT, 140.0);
            p.galley(pos2(c.x - task_g.size().x / 2.0, c.y + r + 38.0), task_g, FAINT);
            let zone = Rect::from_center_size(c + vec2(0.0, 14.0), vec2(144.0, 120.0));
            if self.highlight.as_ref().map_or(false, |(pi, id)| *pi == i && *id == ag.id) {
                p.rect_stroke(zone, CornerRadius::same(4), Stroke::new(1.5, GREEN), StrokeKind::Middle);
            }
            self.clicks.push((zone, ClickZone::Unit(i, ag.id.clone())));
        }

        // comms wall — recent signals for this base
        let lw = Rect::from_min_max(pos2(inner.min.x + hpad, body.max.y + 10.0), pos2(inner.max.x - hpad, inner.max.y - 12.0));
        p.rect_filled(lw, CornerRadius::same(4), Color32::from_rgba_unmultiplied(13, 19, 14, 220));
        p.rect_stroke(lw, CornerRadius::same(4), Stroke::new(1.0, LINE), StrokeKind::Middle);
        p.text(lw.min + vec2(10.0, 8.0), Align2::LEFT_TOP, "📜 COMMS WALL — RECENT SIGNALS", FontId::monospace(10.0), FAINT);
        if events.is_empty() {
            p.text(lw.min + vec2(10.0, 30.0), Align2::LEFT_TOP, "no signals on record", FontId::monospace(10.0), FAINT);
        }
        let mut ey = lw.min.y + 30.0;
        for (ts, agent, text) in &events {
            let who = agent.clone().map(|a| format!("  {}", a)).unwrap_or_default();
            let g = p.layout(
                format!("{}{}  {}", ts, who, text),
                FontId::monospace(10.0),
                Color32::from_rgb(0xb6, 0xc8, 0xb8),
                lw.width() - 20.0,
            );
            if ey + g.size().y > lw.max.y - 8.0 {
                break;
            }
            let gh = g.size().y;
            p.galley(pos2(lw.min.x + 10.0, ey), g, Color32::from_rgb(0xb6, 0xc8, 0xb8));
            ey += gh + 4.0;
        }
    }

    // ---------- top bar ----------
    fn topbar(&mut self, ctx: &egui::Context) {
        TopBottomPanel::top("topbar")
            .exact_height(46.0)
            .frame(Frame::new().fill(Color32::from_rgb(0x0f, 0x17, 0x10)).inner_margin(Margin::symmetric(12, 4)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new("◈ COMMANDER").monospace().size(14.0).color(GREEN));
                        ui.label(RichText::new("rust/egui POC — dynamic RTS context HQ").size(9.5).color(DIM));
                    });
                    ui.add_space(10.0);

                    let warm = (0..self.world.projects.len()).filter(|&i| self.tier(i) == Tier::Warm).count();
                    let dec = self.world.decisions.iter().filter(|d| !d.resolved).count();
                    let idle = self.idle_agents().len();
                    let unseen = self.unseen.len();
                    let res = |ui: &mut egui::Ui, n: usize, label: &str, col: Color32| {
                        Frame::new()
                            .fill(Color32::from_rgb(0x10, 0x19, 0x11))
                            .stroke(Stroke::new(1.0, Color32::from_rgb(0x1e, 0x2d, 0x1f)))
                            .corner_radius(CornerRadius::same(3))
                            .inner_margin(Margin::symmetric(10, 4))
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.label(RichText::new(n.to_string()).monospace().size(14.0).color(col));
                                ui.label(RichText::new(label).monospace().size(9.0).color(DIM));
                            });
                    };
                    res(ui, warm, "WARM CTX", GREEN);
                    res(ui, dec, "DECISIONS", if dec > 0 { AMBER } else { GREEN });
                    res(ui, idle, "IDLE UNITS", if idle > 0 { RED } else { GREEN });
                    res(ui, unseen, "UNSEEN", if unseen > 0 { AMBER } else { GREEN });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(self.clock()).monospace().size(14.0).color(TXT));
                        ui.add_space(14.0);
                        // substructure size ratio (pylons / sensor arrays)
                        ui.spacing_mut().slider_width = 90.0;
                        let resp = ui.add(
                            egui::Slider::new(&mut self.prefs.struct_scale, 0.6..=3.0)
                                .fixed_decimals(1)
                                .show_value(true),
                        );
                        if resp.changed() {
                            self.dirty = true;
                        }
                        ui.label(RichText::new("◆ SIZE ×").monospace().size(9.0).color(DIM));
                    });
                });
            });
    }

    /// codex subscription supply counter, top center (RTS resource style)
    fn codex_meter(&self, ctx: &egui::Context) {
        let usage = self.codex.lock().unwrap().clone();
        let Some(u) = usage else { return };
        let col = if u.pct_left > 50.0 {
            GREEN
        } else if u.pct_left > 20.0 {
            AMBER
        } else {
            RED
        };
        Area::new(Id::new("codex_meter")).anchor(Align2::CENTER_TOP, vec2(0.0, 6.0)).show(ctx, |ui| {
            Frame::new()
                .fill(Color32::from_rgb(0x10, 0x19, 0x11))
                .stroke(Stroke::new(1.0, Color32::from_rgb(0x1e, 0x2d, 0x1f)))
                .corner_radius(CornerRadius::same(3))
                .inner_margin(Margin::symmetric(10, 5))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 7.0;
                        let label = if u.window_minutes >= 10080 { "◆ CODEX WEEKLY" } else { "◆ CODEX" };
                        ui.label(RichText::new(label).monospace().size(9.0).color(DIM));
                        // supply bar
                        let (bar, _) = ui.allocate_exact_size(vec2(96.0, 9.0), Sense::hover());
                        let p = ui.painter();
                        p.rect_filled(bar, CornerRadius::same(2), Color32::from_rgb(0x0a, 0x10, 0x0b));
                        let fill = Rect::from_min_size(bar.min, vec2(bar.width() * (u.pct_left / 100.0), bar.height()));
                        p.rect_filled(fill, CornerRadius::same(2), a(col, 190));
                        p.rect_stroke(bar, CornerRadius::same(2), Stroke::new(1.0, a(col, 90)), StrokeKind::Middle);
                        ui.label(RichText::new(format!("{:.0}%", u.pct_left)).monospace().size(12.0).color(col));
                        ui.label(
                            RichText::new(format!("↻ {}", crate::codex::eta(u.resets_at)))
                                .monospace()
                                .size(9.5)
                                .color(DIM),
                        );
                    });
                });
        });
    }

    // ---------- project rail ----------
    fn rail(&mut self, ctx: &egui::Context) {
        // hidden by default — the canvas-drawn tab (draw_world) toggles it
        if !self.prefs.show_rail {
            return;
        }
        Area::new(Id::new("rail"))
            .anchor(Align2::LEFT_TOP, vec2(10.0, 82.0))
            .show(ctx, |ui| {
                for i in 0..self.world.projects.len() {
                    let proj_name = self.world.projects[i].name.clone();
                    let color = self.world.projects[i].color;
                    let tier = self.tier(i);
                    let age = self.age_str(i);
                    let n = self.rt[i].unseen_events;
                    let blocked = self.world.projects[i].agents.iter().any(|a| a.state == AgentState::Blocked);
                    let pend = self.world.decisions.iter().any(|d| d.proj == i && !d.resolved);
                    let selected = self.sel == Some(i);
                    let ir = Frame::new()
                        .fill(PANEL2)
                        .stroke(Stroke::new(1.0, if selected { GREEN_DK } else { LINE }))
                        .corner_radius(CornerRadius::same(3))
                        .inner_margin(Margin::symmetric(8, 6))
                        .show(ui, |ui| {
                            ui.set_width(168.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 7.0;
                                let (kr, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                                ui.painter().rect_filled(kr, CornerRadius::same(2), color);
                                ui.painter().text(
                                    kr.center(),
                                    Align2::CENTER_CENTER,
                                    (i + 1).to_string(),
                                    FontId::monospace(11.0),
                                    Color32::from_rgb(0x0a, 0x0f, 0x0a),
                                );
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    ui.label(RichText::new(proj_name).size(11.0).color(TXT));
                                    ui.label(RichText::new(format!("{} · {}", tier.label(), age)).monospace().size(8.5).color(DIM));
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if n > 0 {
                                        ui.label(RichText::new(format!("+{}", n)).monospace().size(9.5).color(AMBER));
                                    }
                                    if pend {
                                        ui.label(RichText::new("◆").size(10.0).color(AMBER));
                                    }
                                    if blocked {
                                        ui.label(RichText::new("▲").size(10.0).color(RED));
                                    }
                                });
                            });
                        });
                    let resp = ir.response.interact(Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
                    if resp.clicked() {
                        self.acts.push(Act::Focus { proj: i, scale: 0.95, from_space: false });
                    }
                    ui.add_space(4.0);
                }
            });
    }

    // ---------- minimap + idle button ----------
    fn minimap_cluster(&mut self, ctx: &egui::Context) {
        Area::new(Id::new("mmcluster"))
            .anchor(Align2::LEFT_BOTTOM, vec2(10.0, -10.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    Frame::new()
                        .fill(PANEL)
                        .stroke(Stroke::new(1.0, LINE))
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(Margin::same(6))
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 3.0;
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("TACTICAL").monospace().size(8.5).color(FAINT));
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(RichText::new(self.clock()).monospace().size(8.5).color(FAINT));
                                    });
                                });
                                let (mrect, mresp) = ui.allocate_exact_size(vec2(MMW, MMH), Sense::click_and_drag());
                                self.draw_minimap(ui.painter(), mrect);
                                if mresp.dragged() || mresp.drag_started() {
                                    // scrub the scope: camera follows the pointer
                                    if let Some(cp) = mresp.interact_pointer_pos() {
                                        let wp = pos2((cp.x - mrect.min.x) / MMS, (cp.y - mrect.min.y) / MMS);
                                        self.cam.target_pos = wp;
                                        self.cam.pos = wp;
                                    }
                                }
                                if mresp.clicked() {
                                    if let Some(cp) = mresp.interact_pointer_pos() {
                                        // base hit first
                                        let mut hit_base = None;
                                        for i in 0..self.world.projects.len() {
                                            let c = self.base_center(i);
                                            let bc = mrect.min + vec2(c.x * MMS, c.y * MMS);
                                            if (cp - bc).length() < 10.0 {
                                                hit_base = Some(i);
                                            }
                                        }
                                        match hit_base {
                                            Some(i) => self.acts.push(Act::Focus { proj: i, scale: 0.95, from_space: false }),
                                            None => {
                                                let wp = pos2((cp.x - mrect.min.x) / MMS, (cp.y - mrect.min.y) / MMS);
                                                self.acts.push(Act::MinimapGoto(wp));
                                            }
                                        }
                                    }
                                }
                            });
                        });
                    // idle button
                    let idle = self.idle_agents().len();
                    let label = RichText::new(format!("🛌 {}\nIDLE (i)", idle))
                        .monospace()
                        .size(11.0)
                        .color(if idle > 0 { RED } else { Color32::from_rgb(0x9f, 0xdc, 0xaa) });
                    if ui
                        .add_sized([58.0, 58.0], Button::new(label).stroke(Stroke::new(1.0, if idle > 0 { RED } else { LINE })))
                        .on_hover_text("Cycle idle / blocked agents (i)")
                        .clicked()
                    {
                        self.acts.push(Act::CycleIdle);
                    }
                });
            });
    }

    fn draw_minimap(&mut self, p: &Painter, rect: Rect) {
        let t = self.time;
        p.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(0x0a, 0x0f, 0x0b));
        p.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_rgb(0x22, 0x33, 0x1f)), StrokeKind::Middle);
        let step = MMW / 10.0;
        for k in 1..10 {
            let x = rect.min.x + k as f32 * step;
            p.line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], Stroke::new(1.0, Color32::from_rgba_unmultiplied(60, 90, 64, 30)));
        }
        for k in 1..7 {
            let y = rect.min.y + k as f32 * step;
            if y < rect.max.y {
                p.line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], Stroke::new(1.0, Color32::from_rgba_unmultiplied(60, 90, 64, 30)));
            }
        }
        // links
        for l in &self.world.links {
            if l.a >= self.world.projects.len() || l.b >= self.world.projects.len() {
                continue;
            }
            let ca = self.base_center(l.a);
            let cb = self.base_center(l.b);
            p.line_segment(
                [rect.min + vec2(ca.x * MMS, ca.y * MMS), rect.min + vec2(cb.x * MMS, cb.y * MMS)],
                Stroke::new(1.0, a(GREEN_DK, 140)),
            );
        }
        // bases + markers
        for i in 0..self.world.projects.len() {
            let bc = self.base_center(i);
            let c = rect.min + vec2(bc.x * MMS, bc.y * MMS);
            let tier = self.tier(i);
            let alpha = match tier {
                Tier::Warm => 255,
                Tier::Cooling => 190,
                Tier::Cold => 115,
                Tier::Frozen => 64,
            };
            p.rect_filled(Rect::from_center_size(c, vec2(14.0, 10.0)), CornerRadius::same(2), a(self.world.projects[i].color, alpha));
            let blocked = self.world.projects[i].agents.iter().any(|ag| ag.state == AgentState::Blocked);
            let pend = self.world.decisions.iter().any(|d| d.proj == i && !d.resolved);
            if blocked {
                let blink = if (t * 1.8).fract() < 0.5 { 255 } else { 100 };
                p.text(c + vec2(-12.0, -9.0), Align2::CENTER_CENTER, "▲", FontId::proportional(9.0), a(RED, blink));
            }
            if pend {
                p.text(c + vec2(10.0, -9.0), Align2::CENTER_CENTER, "◆", FontId::proportional(9.0), AMBER);
            }
        }
        // pings
        let nproj = self.world.projects.len();
        self.mpings.retain(|pg| t - pg.created < 30.0 && pg.proj < nproj);
        for pg in &self.mpings {
            let age = (t - pg.created) as f32;
            let bc = self.base_center(pg.proj);
            let c = rect.min + vec2(bc.x * MMS, bc.y * MMS);
            let fade = (1.0 - age / 30.0).clamp(0.0, 1.0);
            p.circle_filled(c, 4.0, a(pg.color, (fade * 220.0) as u8));
            if age < 6.0 {
                let r = 4.0 + (age % 1.5) / 1.5 * 12.0;
                p.circle_stroke(c, r, Stroke::new(1.5, a(pg.color, ((1.0 - (age % 1.5) / 1.5) * 255.0) as u8)));
            }
        }
        // viewport rect
        let vw = self.viewport.width() / self.cam.scale * MMS;
        let vh = self.viewport.height() / self.cam.scale * MMS;
        let w = vw.min(MMW);
        let h = vh.min(MMH);
        let x = (self.cam.pos.x * MMS - vw / 2.0).clamp(0.0, (MMW - w).max(0.0));
        let y = (self.cam.pos.y * MMS - vh / 2.0).clamp(0.0, (MMH - h).max(0.0));
        p.rect_stroke(
            Rect::from_min_size(rect.min + vec2(x, y), vec2(w, h)),
            CornerRadius::ZERO,
            Stroke::new(1.0, a(GREEN, 200)),
            StrokeKind::Middle,
        );
    }

    // ---------- hint bar / crumb / toasts ----------
    fn hint_bar(&self, ctx: &egui::Context) {
        Area::new(Id::new("hintbar"))
            .anchor(Align2::CENTER_BOTTOM, vec2(40.0, -10.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(a(PANEL, 230))
                    .stroke(Stroke::new(1.0, LINE))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(Margin::symmetric(14, 5))
                    .show(ui, |ui| {
                        if self.build_menu {
                            // submenu level: the command card switched down one tier
                            ui.label(
                                RichText::new("▸ BUILD  ·  [b] base  ·  [p] pylon — goal  ·  [q] sensor — question  ·  [esc] cancel")
                                    .monospace()
                                    .size(10.5)
                                    .color(GREEN),
                            );
                        } else {
                            ui.label(
                                RichText::new("[1–4] base · [b] build… · click structure selects (ctrl+drag = group) · 2×click = enter · [d][d] demolish · [l] link · [space] alert · [esc] back · [c] capture · [i] idle · drag moves · wheel zoom")
                                    .monospace()
                                    .size(10.0)
                                    .color(DIM),
                            );
                        }
                    });
            });
    }

    fn crumb(&self, ctx: &egui::Context) {
        if let Some(from) = self.link_from {
            if from < self.world.projects.len() {
                Area::new(Id::new("linkmode"))
                    .anchor(Align2::CENTER_TOP, vec2(0.0, 56.0))
                    .show(ctx, |ui| {
                        Frame::new()
                            .fill(Color32::from_rgba_unmultiplied(8, 20, 10, 235))
                            .stroke(Stroke::new(1.0, GREEN_DK))
                            .corner_radius(CornerRadius::same(3))
                            .inner_margin(Margin::symmetric(14, 5))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(format!("⛓ LINK FROM {} — click target base · esc cancels", self.world.projects[from].name))
                                        .monospace()
                                        .size(11.0)
                                        .color(GREEN),
                                );
                            });
                    });
            }
            return;
        }
        if let Some(from) = &self.space_crumb {
            Area::new(Id::new("crumb"))
                .anchor(Align2::CENTER_TOP, vec2(0.0, 56.0))
                .show(ctx, |ui| {
                    Frame::new()
                        .fill(Color32::from_rgba_unmultiplied(20, 16, 6, 235))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(0x6b, 0x54, 0x20)))
                        .corner_radius(CornerRadius::same(3))
                        .inner_margin(Margin::symmetric(14, 5))
                        .show(ui, |ui| {
                            ui.label(RichText::new(format!("⤺ ESC — camera back to {}", from)).monospace().size(11.0).color(AMBER));
                        });
                });
        }
    }

    fn toasts_ui(&mut self, ctx: &egui::Context) {
        let t = self.time;
        self.toasts.retain(|to| t - to.created < 7.0);
        let items: Vec<(usize, String, String, String, bool, f64, Option<usize>)> = self
            .toasts
            .iter()
            .enumerate()
            .map(|(k, to)| (k, to.head.clone(), to.body.clone(), to.sub.clone(), to.ok, to.created, to.proj))
            .collect();
        Area::new(Id::new("toasts"))
            .anchor(Align2::RIGHT_TOP, vec2(-334.0, 56.0))
            .show(ctx, |ui| {
                ui.set_width(290.0);
                for (_k, head, body, sub, ok, created, proj) in items {
                    let fade = (((7.0 - (t - created)) / 0.6).clamp(0.0, 1.0) * 255.0) as u8;
                    let border = if ok { Color32::from_rgb(0x3a, 0x6b, 0x44) } else { Color32::from_rgb(0x6b, 0x54, 0x20) };
                    let ir = Frame::new()
                        .fill(a(Color32::from_rgb(0x14, 0x1c, 0x10), fade.min(240)))
                        .stroke(Stroke::new(1.0, a(border, fade)))
                        .corner_radius(CornerRadius::same(3))
                        .inner_margin(Margin::symmetric(11, 8))
                        .show(ui, |ui| {
                            ui.set_width(268.0);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(RichText::new(head).monospace().size(10.0).color(a(if ok { GREEN } else { AMBER }, fade)));
                            ui.label(RichText::new(body).size(11.0).color(a(TXT, fade)));
                            if !sub.is_empty() {
                                ui.label(RichText::new(sub).monospace().size(9.0).color(a(DIM, fade)));
                            }
                        });
                    let resp = ir.response.interact(Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
                    if resp.clicked() {
                        if let Some(pi) = proj {
                            self.acts.push(Act::Focus { proj: pi, scale: 0.95, from_space: true });
                            self.space_crumb = Some(self.location_name());
                        }
                    }
                    ui.add_space(6.0);
                }
            });
    }

    // ---------- command card (right panel) ----------
    fn section(ui: &mut egui::Ui, text: &str) {
        ui.add_space(10.0);
        ui.label(RichText::new(text.to_uppercase()).monospace().size(9.5).color(FAINT));
        ui.separator();
    }

    fn card_panel(&mut self, ctx: &egui::Context) {
        SidePanel::right("card")
            .exact_width(322.0)
            .resizable(false)
            .frame(Frame::new().fill(Color32::from_rgb(0x0c, 0x13, 0x0d)).inner_margin(Margin::same(0)))
            .show(ctx, |ui| {
                TopBottomPanel::bottom("cmdgrid")
                    .frame(Frame::new().fill(Color32::from_rgb(0x10, 0x19, 0x11)).inner_margin(Margin::same(10)))
                    .show_inside(ui, |ui| self.cmd_grid(ui));
                CentralPanel::default()
                    .frame(Frame::new().inner_margin(Margin::symmetric(12, 8)))
                    .show_inside(ui, |ui| {
                        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            self.card_capture(ui);
                            match self.sel {
                                Some(i) => self.card_project(ui, i),
                                None => self.card_overview(ui),
                            }
                        });
                    });
            });
    }

    fn cmd_grid(&mut self, ui: &mut egui::Ui) {
        let has_sel = self.sel.is_some();
        let has_pend = self.sel.map_or(false, |i| self.world.decisions.iter().any(|d| d.proj == i && !d.resolved));
        let is_cold = self.sel.map_or(false, |i| self.rt[i].shown_age_min > 120.0);
        let bw4 = (ui.available_width() - 18.0) / 4.0;
        let mk = |txt: &str, col: Color32, w: f32| Button::new(RichText::new(txt).size(10.0).color(col)).min_size(vec2(w, 40.0));
        let btn4 = |txt: &str, col: Color32| mk(txt, col, bw4);
        let destroy_armed = self.destroy_arm.map_or(false, |(p, t)| Some(p) == self.sel && self.time - t < 4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if ui.add(btn4("⚡\ncapture  C", TXT)).clicked() {
                self.acts.push(Act::OpenCapture);
            }
            if ui.add(btn4("⌂\nnew base  B", GREEN)).clicked() {
                self.acts.push(Act::PlaceBase(self.cam.target_pos));
            }
            if ui.add(btn4("🛌\nidle  I", TXT)).clicked() {
                self.acts.push(Act::CycleIdle);
            }
            let dtxt = if destroy_armed { "💥\nconfirm?  D" } else { "💥\ndestroy  D" };
            if ui.add_enabled(has_sel, btn4(dtxt, RED)).clicked() {
                if let Some(i) = self.sel {
                    self.acts.push(Act::DestroyBase(i));
                }
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if ui.add_enabled(has_pend, btn4("◆\nbriefing", AMBER)).clicked() {
                if let Some(i) = self.sel {
                    if let Some((di, _)) = self.world.decisions.iter().enumerate().find(|(_, d)| d.proj == i && !d.resolved) {
                        self.acts.push(Act::OpenDecision(di));
                    }
                }
            }
            if ui.add_enabled(has_sel, btn4("⛓\nlink  L", TXT)).clicked() {
                self.acts.push(Act::StartLink);
            }
            if ui.add_enabled(has_sel && is_cold, btn4("⟲\nrecovery", AMBER)).clicked() {
                if let Some(i) = self.sel {
                    self.acts.push(Act::OpenRecovery(i));
                }
            }
            if ui.add_enabled(has_sel, btn4("◎\ncenter", TXT)).clicked() {
                self.acts.push(Act::Center);
            }
        });
    }

    fn card_capture(&mut self, ui: &mut egui::Ui) {
        let Some(ci) = self.sel_capture else { return };
        if ci >= self.world.captures.len() {
            self.sel_capture = None;
            return;
        }
        let text = self.world.captures[ci].text.clone();
        let ts = self.world.captures[ci].ts.clone();
        let proj_names: Vec<(usize, String, Color32)> = self
            .world
            .projects
            .iter()
            .enumerate()
            .map(|(pi, p)| (pi, p.name.to_string(), p.color))
            .collect();
        Frame::new()
            .fill(a(GREEN, 12))
            .stroke(Stroke::new(1.0, GREEN_DK))
            .corner_radius(CornerRadius::same(3))
            .inner_margin(Margin::same(9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(RichText::new("⚡ CAPTURE SELECTED").monospace().size(10.0).color(GREEN));
                ui.label(RichText::new(&text).size(11.5).color(TXT));
                ui.label(RichText::new(format!("captured {} · unsorted", ts)).monospace().size(8.5).color(FAINT));
                ui.add_space(4.0);
                ui.label(RichText::new("FILE TO:").monospace().size(8.5).color(DIM));
                ui.horizontal_wrapped(|ui| {
                    for (pi, name, color) in &proj_names {
                        if ui
                            .add(Button::new(RichText::new(format!("{} {}", pi + 1, name)).size(9.5).color(*color)).stroke(Stroke::new(1.0, LINE)))
                            .clicked()
                        {
                            self.acts.push(Act::FileCapture { cap: ci, proj: *pi });
                        }
                    }
                });
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    if ui.add(Button::new(RichText::new("🗑 discard").size(9.5).color(RED)).stroke(Stroke::new(1.0, LINE))).clicked() {
                        self.acts.push(Act::DiscardCapture(ci));
                    }
                    ui.label(RichText::new("esc = deselect").monospace().size(8.5).color(FAINT));
                });
            });
        ui.add_space(8.0);
    }

    fn card_project(&mut self, ui: &mut egui::Ui, i: usize) {
        let name = self.world.projects[i].name.clone();
        let color = self.world.projects[i].color;
        let status = self.world.projects[i].status.clone();
        let goal = self.world.projects[i].goal.clone();
        let age = self.age_str(i);

        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(3), color);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new(name).size(14.0).color(TXT).strong());
                ui.label(
                    RichText::new(format!("{} · base {} · visited {}", status, i + 1, age))
                        .monospace()
                        .size(9.0)
                        .color(DIM),
                );
            });
        });

        // delta-first resume header
        let shown = self.rt[i].shown.clone();
        let shown_age = self.rt[i].shown_age.clone();
        let cold = self.rt[i].shown_age_min > 120.0;
        let (rb, rf) = if cold {
            (Color32::from_rgba_unmultiplied(0x6b, 0x54, 0x20, 255), AMBER)
        } else {
            (LINE_HI, GREEN)
        };
        ui.add_space(6.0);
        Frame::new()
            .fill(Color32::from_rgba_unmultiplied(0x2e, 0x44, 0x33, 50))
            .stroke(Stroke::new(1.0, rb))
            .corner_radius(CornerRadius::same(3))
            .inner_margin(Margin::same(9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if !shown.is_empty() {
                    ui.label(
                        RichText::new(format!("⟳ SINCE YOU LEFT {} — {} event{}", shown_age, shown.len(), if shown.len() > 1 { "s" } else { "" }))
                            .monospace()
                            .size(10.5)
                            .color(rf),
                    );
                    ui.add_space(3.0);
                    for line in &shown {
                        ui.label(RichText::new(format!("▸ {}", line)).size(11.0).color(Color32::from_rgb(0xb6, 0xc8, 0xb8)));
                    }
                } else {
                    ui.label(RichText::new("⟳ RESUME").monospace().size(10.5).color(rf));
                    ui.label(RichText::new("You are current — no new events since your last visit.").size(11.0).color(DIM).italics());
                }
                if cold {
                    ui.add_space(6.0);
                    if ui
                        .add(Button::new(RichText::new("⟲ COLD CONTEXT — open deep recovery briefing").monospace().size(10.5).color(AMBER)).stroke(Stroke::new(1.0, rb)))
                        .clicked()
                    {
                        self.acts.push(Act::OpenRecovery(i));
                    }
                }
            });

        Self::section(ui, "🎯 objective");
        if goal.is_empty() {
            ui.label(RichText::new("no objective set").size(11.5).color(FAINT).italics());
        } else {
            ui.label(RichText::new(goal).size(11.5).color(Color32::from_rgb(0xa8, 0xc2, 0xab)));
        }

        Self::section(ui, "◆ pylons — goals/tasks");
        if self.world.projects[i].tasks.is_empty() {
            ui.label(RichText::new("no pylons — press P over the map to warp one in").monospace().size(10.0).color(FAINT));
        }
        for t in &self.world.projects[i].tasks {
            let (ico, col) = match t.state {
                TaskState::Done => ("✓", DIM),
                TaskState::Doing => ("◐", GREEN),
                TaskState::Todo => ("▫", TXT),
                TaskState::Blocked => ("✖", Color32::from_rgb(0xe0, 0x9a, 0x9a)),
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(ico).size(11.0).color(col));
                let title = if t.state == TaskState::Done {
                    RichText::new(&t.title).size(11.5).color(DIM).strikethrough()
                } else {
                    RichText::new(&t.title).size(11.5).color(col)
                };
                ui.label(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(t.state.label().to_uppercase()).monospace().size(8.5).color(FAINT));
                });
            });
        }

        // research: questions rendered as sensor arrays around the base
        let quests: Vec<(usize, String, bool)> = self.world.projects[i]
            .questions
            .iter()
            .enumerate()
            .map(|(qi, q)| (qi, q.text.clone(), q.resolved))
            .collect();
        if !quests.is_empty() {
            Self::section(ui, "⌖ sensor arrays — questions");
            for (qi, text, resolved) in quests {
                let (ico, col) = if resolved { ("✓", DIM) } else { ("?", AMBER) };
                let row = ui.horizontal(|ui| {
                    ui.label(RichText::new(ico).monospace().size(11.0).color(col));
                    let label = if resolved {
                        RichText::new(&text).size(11.5).color(DIM).strikethrough()
                    } else {
                        RichText::new(&text).size(11.5).color(Color32::from_rgb(0xd8, 0xc2, 0x8f))
                    };
                    ui.label(label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(if resolved { "RESOLVED" } else { "SCANNING" }).monospace().size(8.5).color(if resolved { FAINT } else { AMBER }));
                    });
                });
                if row.response.interact(Sense::click()).on_hover_text("toggle resolved").clicked() {
                    self.acts.push(Act::ToggleQuestion(i, qi));
                }
            }
        }

        Self::section(ui, "🪖 units — agents");
        if self.world.projects[i].agents.is_empty() {
            ui.label(RichText::new("no units garrisoned — manual theater").monospace().size(10.0).color(FAINT));
        }
        let agent_rows: Vec<(String, AgentState, String, String, bool)> = self.world.projects[i]
            .agents
            .iter()
            .map(|ag| {
                (
                    ag.id.to_string(),
                    ag.state,
                    ag.task.clone(),
                    ag.last_report.clone(),
                    ag.blocked_on.is_some(),
                )
            })
            .collect();
        for (aid, state, task, last, has_block) in agent_rows {
            let hl = self.highlight.as_ref().map_or(false, |(pi, id)| *pi == i && *id == aid);
            let (dot, stc) = match state {
                AgentState::Working => (GREEN, GREEN),
                AgentState::Blocked => (RED, RED),
                AgentState::Idle => (Color32::from_rgb(0x8f, 0xa3, 0x92), Color32::from_rgb(0x8f, 0xa3, 0x92)),
            };
            let ir = Frame::new()
                .fill(Color32::from_rgba_unmultiplied(17, 26, 19, 160))
                .stroke(Stroke::new(1.0, if hl { GREEN } else { LINE }))
                .corner_radius(CornerRadius::same(3))
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("●").size(9.0).color(dot));
                        ui.label(RichText::new(&aid).monospace().size(11.0).color(TXT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(state.label().to_uppercase()).monospace().size(9.0).color(stc));
                        });
                    });
                    ui.label(RichText::new(&task).size(10.5).color(DIM));
                    ui.label(
                        RichText::new(format!("last report {}{}", last, if has_block { " · blocked on decision" } else { "" }))
                            .monospace()
                            .size(9.0)
                            .color(FAINT),
                    );
                    if hl {
                        let (why, action) = self.suggest_for(&aid);
                        ui.add_space(3.0);
                        Frame::new()
                            .fill(a(GREEN, 15))
                            .stroke(Stroke::new(1.0, LINE_HI))
                            .corner_radius(CornerRadius::same(3))
                            .inner_margin(Margin::same(7))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new(format!("WHY: {}", why)).size(10.5).color(Color32::from_rgb(0xa8, 0xd4, 0xae)));
                                ui.label(RichText::new(format!("SUGGESTED ORDER: {}", action)).size(10.5).color(Color32::from_rgb(0xa8, 0xd4, 0xae)));
                            });
                    }
                });
            let resp = ir.response.interact(Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
            if resp.clicked() {
                self.highlight = Some((i, aid.clone()));
            }
            ui.add_space(4.0);
        }

        // installed wasm programs (signals / reducers / pollers) with budgets + last run
        let pname = self.world.projects[i].name.clone();
        let mod_rows: Vec<ModuleCfg> = self.world.projects[i].modules.clone();
        if !mod_rows.is_empty() {
            Self::section(ui, "⚙ programs — wasm modules");
            for m in mod_rows {
                let st = self.mod_status.get(&(pname.clone(), m.name.clone())).cloned().unwrap_or_default();
                let (dot, stc) = if !m.enabled {
                    (FAINT, FAINT)
                } else if st.error.is_some() {
                    (RED, RED)
                } else {
                    (GREEN, GREEN)
                };
                Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(17, 26, 19, 160))
                    .stroke(Stroke::new(1.0, LINE))
                    .corner_radius(CornerRadius::same(3))
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("⚙").size(10.0).color(dot));
                            ui.label(RichText::new(&m.name).monospace().size(11.0).color(TXT));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .add(Button::new(RichText::new("✕").monospace().size(9.0).color(DIM)).frame(false))
                                    .on_hover_text("uninstall program")
                                    .clicked()
                                {
                                    self.world.projects[i].modules.retain(|x| x.name != m.name);
                                    self.mod_status.remove(&(pname.clone(), m.name.clone()));
                                    self.dirty = true;
                                    self.wasm_sync();
                                }
                                let tog = if m.enabled { "⏸" } else { "▶" };
                                if ui
                                    .add(Button::new(RichText::new(tog).monospace().size(9.0).color(DIM)).frame(false))
                                    .on_hover_text(if m.enabled { "disable" } else { "enable" })
                                    .clicked()
                                {
                                    if let Some(x) = self.world.projects[i].modules.iter_mut().find(|x| x.name == m.name) {
                                        x.enabled = !x.enabled;
                                    }
                                    self.dirty = true;
                                    self.wasm_sync();
                                }
                                let state = if !m.enabled {
                                    "PAUSED".to_string()
                                } else if st.error.is_some() {
                                    "FAULT".to_string()
                                } else {
                                    format!("{} TICKS", st.ticks)
                                };
                                ui.label(RichText::new(state).monospace().size(9.0).color(stc));
                            });
                        });
                        ui.label(RichText::new(&m.path).monospace().size(9.0).color(DIM));
                        ui.label(
                            RichText::new(format!(
                                "budget: {:.0}s · {}M fuel · {} http · last: {:.1}ms, {}M fuel, {} http",
                                m.interval_sec,
                                m.fuel_per_tick / 1_000_000,
                                m.max_http_per_tick,
                                st.ms,
                                st.fuel_used / 1_000_000,
                                st.http_used,
                            ))
                            .monospace()
                            .size(9.0)
                            .color(FAINT),
                        );
                        if let Some(e) = &st.error {
                            ui.label(RichText::new(format!("⚠ {}", e)).monospace().size(9.0).color(RED));
                        }
                        if let Some(l) = &st.last_log {
                            ui.label(RichText::new(format!("log: {}", l)).monospace().size(9.0).color(FAINT));
                        }
                    });
                ui.add_space(4.0);
            }
        }

        let decs: Vec<(usize, String, String, bool, Option<String>)> = self
            .world
            .decisions
            .iter()
            .enumerate()
            .filter(|(_, d)| d.proj == i)
            .map(|(di, d)| (di, d.title.to_string(), d.due.to_string(), d.resolved, d.chosen.clone()))
            .collect();
        if !decs.is_empty() {
            Self::section(ui, "◆ intel — decisions");
            for (di, title, due, resolved, chosen) in decs {
                if !resolved {
                    let ir = Frame::new()
                        .fill(a(AMBER, 15))
                        .stroke(Stroke::new(1.0, a(AMBER, 110)))
                        .corner_radius(CornerRadius::same(3))
                        .inner_margin(Margin::same(8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(format!("◆ {}", title)).size(11.0).color(AMBER));
                            ui.label(RichText::new(format!("PENDING · due {} · click to open briefing", due)).monospace().size(8.5).color(DIM));
                        });
                    if ir.response.interact(Sense::click()).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        self.acts.push(Act::OpenDecision(di));
                    }
                } else {
                    Frame::new()
                        .fill(a(GREEN, 12))
                        .stroke(Stroke::new(1.0, LINE))
                        .corner_radius(CornerRadius::same(3))
                        .inner_margin(Margin::same(8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(format!("✓ {}", title)).size(11.0).color(GREEN));
                            ui.label(RichText::new(format!("RESOLVED → {}", chosen.unwrap_or_default())).monospace().size(8.5).color(DIM));
                        });
                }
                ui.add_space(4.0);
            }
        }
    }

    fn card_overview(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Theater Overview").size(14.0).color(TXT).strong());
        ui.label(RichText::new("no base selected · 1–4 to focus").monospace().size(9.0).color(DIM));
        ui.add_space(6.0);
        Frame::new()
            .fill(Color32::from_rgba_unmultiplied(0x2e, 0x44, 0x33, 50))
            .stroke(Stroke::new(1.0, LINE_HI))
            .corner_radius(CornerRadius::same(3))
            .inner_margin(Margin::same(9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("⟳ COMMANDER'S SWEEP").monospace().size(10.5).color(GREEN));
                ui.label(
                    RichText::new("Press 1 2 3 4 for a ~10-second sweep: each base opens delta-first, and its fog burns off as you visit.")
                        .size(11.0)
                        .color(DIM),
                );
            });

        Self::section(ui, "🗺 bases");
        if self.world.projects.is_empty() {
            ui.label(RichText::new("no theaters — double-click the map or press B to establish a base").monospace().size(10.0).color(FAINT));
        }
        for i in 0..self.world.projects.len() {
            let name = self.world.projects[i].name.clone();
            let color = self.world.projects[i].color;
            let tier = self.tier(i);
            let n = self.rt[i].unseen_events;
            let ir = ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(9.0, 9.0), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(2), color);
                ui.label(RichText::new(format!("{} · {}", i + 1, name)).size(11.5).color(TXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{}{}", tier.label(), if n > 0 { format!(" · +{}", n) } else { String::new() }))
                            .monospace()
                            .size(9.0)
                            .color(DIM),
                    );
                });
            });
            if ir.response.interact(Sense::click()).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                self.acts.push(Act::Focus { proj: i, scale: 0.95, from_space: false });
            }
        }

        Self::section(ui, "◆ pending decisions");
        let pend: Vec<(usize, String, String, String)> = self
            .world
            .decisions
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.resolved)
            .map(|(di, d)| (di, d.title.to_string(), self.world.projects[d.proj].name.to_string(), d.due.to_string()))
            .collect();
        if pend.is_empty() {
            ui.label(RichText::new("none — all orders committed").monospace().size(10.0).color(FAINT));
        }
        for (di, title, pname, due) in pend {
            let ir = Frame::new()
                .fill(a(AMBER, 15))
                .stroke(Stroke::new(1.0, a(AMBER, 110)))
                .corner_radius(CornerRadius::same(3))
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(format!("◆ {}", title)).size(11.0).color(AMBER));
                    ui.label(RichText::new(format!("{} · due {}", pname, due)).monospace().size(8.5).color(DIM));
                });
            if ir.response.interact(Sense::click()).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                self.acts.push(Act::OpenDecision(di));
            }
            ui.add_space(4.0);
        }

        Self::section(ui, &format!("⚡ unsorted captures ({})", self.world.captures.len()));
        for c in self.world.captures.iter().rev().take(4) {
            ui.label(RichText::new(format!("⚡ {}  ·  {}", c.text, c.ts)).size(10.5).color(Color32::from_rgb(0x9f, 0xb3, 0xa1)));
        }
    }

    fn suggest_for(&self, aid: &str) -> (String, String) {
        for (pi, p) in self.world.projects.iter().enumerate() {
            if let Some(ag) = p.agents.iter().find(|a| a.id == aid) {
                if let Some(dep) = &ag.blocked_on {
                    if let Some(d) = self.world.decisions.iter().find(|d| d.id == *dep && !d.resolved) {
                        return (
                            format!("Holding on pending decision '{}' (due {}).", d.title, d.due),
                            "Resolve that decision — one commit releases this unit.".into(),
                        );
                    }
                    return (format!("Holding on '{}'.", dep), "Clear the dependency or reassign the unit.".into());
                }
                if ag.state == AgentState::Idle {
                    return (
                        format!("Idle in {} — no task in progress.", self.world.projects[pi].name),
                        "Assign a structure, or garrison the unit.".into(),
                    );
                }
            }
        }
        ("No blocking condition on record.".into(), "Review last report and reassign.".into())
    }

    // ---------- overlays ----------
    fn briefing_window(&mut self, ctx: &egui::Context) {
        let Some(di) = self.briefing else { return };
        let d = &self.world.decisions[di];
        let proj_name = self.world.projects[d.proj].name.to_string();
        let title = d.title.to_string();
        let due = d.due.to_string();
        let resolved = d.resolved;
        let chosen = d.chosen.clone();
        let options: Vec<String> = d.options.clone();
        let h = ctx.screen_rect().height();

        egui::Window::new("briefing")
            .title_bar(false)
            .resizable(false)
            .fixed_size(vec2(440.0, (h - 120.0).max(300.0)))
            .anchor(Align2::RIGHT_TOP, vec2(-330.0, 52.0))
            .frame(Frame::new().fill(Color32::from_rgb(0x0f, 0x18, 0x10)).stroke(Stroke::new(1.0, LINE_HI)).inner_margin(Margin::same(14)).corner_radius(CornerRadius::same(4)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("◆ INTEL BRIEFING · {}", proj_name.to_uppercase())).monospace().size(13.0).color(AMBER));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(Button::new(RichText::new("✕").size(13.0).color(DIM))).clicked() {
                            self.acts.push(Act::CloseBriefing);
                        }
                    });
                });
                ui.separator();
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(if resolved { "RESOLVED".to_string() } else { format!("◆ PENDING · due {}", due) })
                                .monospace()
                                .size(9.5)
                                .color(if resolved { GREEN } else { AMBER }),
                        );
                        ui.label(RichText::new(&proj_name).monospace().size(9.5).color(DIM));
                    });
                    ui.add_space(6.0);
                    ui.label(RichText::new(&title).size(14.0).color(Color32::from_rgb(0xff, 0xe9, 0xb0)).strong());
                    ui.add_space(8.0);

                    if !resolved {
                        ui.label(RichText::new("Choose an option:").size(10.5).color(DIM).italics());
                    }
                    ui.add_space(6.0);

                    if !resolved {
                        for (oi, opt) in options.iter().enumerate() {
                            let name = opt.split(':').next().unwrap_or(opt).to_uppercase();
                            let b = Button::new(
                                RichText::new(format!("✓ CHOOSE {}\n{}", name, opt)).size(10.5).color(GREEN),
                            )
                            .min_size(vec2(ui.available_width(), 44.0))
                            .stroke(Stroke::new(1.0, LINE_HI));
                            if ui.add(b).clicked() {
                                self.acts.push(Act::CommitDecision(di, oi));
                            }
                            ui.add_space(4.0);
                        }
                    } else {
                        ui.label(RichText::new(format!("✓ RESOLVED → {}", chosen.unwrap_or_default())).size(11.0).color(GREEN));
                    }
                });
            });
    }

    fn recovery_window(&mut self, ctx: &egui::Context) {
        let Some(pi) = self.recovery else { return };
        let proj_name = self.world.projects[pi].name.clone();
        let status = self.world.projects[pi].status.clone();
        let goal = self.world.projects[pi].goal.clone();
        let age = if self.rt[pi].shown_age.is_empty() { self.age_str(pi) } else { self.rt[pi].shown_age.clone() };
        let h = ctx.screen_rect().height();

        egui::Window::new("recovery")
            .title_bar(false)
            .resizable(false)
            .fixed_size(vec2(440.0, (h - 120.0).max(300.0)))
            .anchor(Align2::RIGHT_TOP, vec2(-330.0, 52.0))
            .frame(Frame::new().fill(Color32::from_rgb(0x0f, 0x18, 0x10)).stroke(Stroke::new(1.0, LINE_HI)).inner_margin(Margin::same(14)).corner_radius(CornerRadius::same(4)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("⟲ DEEP RECOVERY · {}", proj_name.to_uppercase())).monospace().size(13.0).color(AMBER));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(Button::new(RichText::new("✕").size(13.0).color(DIM))).clicked() {
                            self.acts.push(Act::CloseRecovery);
                        }
                    });
                });
                ui.separator();
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("🧊 {} SINCE LAST CONTACT", age.to_uppercase())).monospace().size(9.5).color(AMBER));
                        ui.label(RichText::new(&status).monospace().size(9.5).color(DIM));
                        ui.label(
                            RichText::new(format!("due-pressure: {}", if status == "deadline" { "HIGH" } else { "low" }))
                                .monospace()
                                .size(9.5)
                                .color(if status == "deadline" { RED } else { DIM }),
                        );
                    });
                    ui.add_space(4.0);
                    ui.label(RichText::new("Cold context detected. Full situational re-read before you issue orders:").size(10.5).color(DIM).italics());

                    Self::section(ui, "🎯 what this theater is for");
                    ui.label(RichText::new(goal).size(12.0).color(TXT));

                    Self::section(ui, "🏗 structure status");
                    for t in &self.world.projects[pi].tasks {
                        let (ico, col) = match t.state {
                            TaskState::Done => ("✓", DIM),
                            TaskState::Doing => ("◐", GREEN),
                            TaskState::Todo => ("▫", TXT),
                            TaskState::Blocked => ("✖", Color32::from_rgb(0xe0, 0x9a, 0x9a)),
                        };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(ico).size(11.0).color(col));
                            ui.label(RichText::new(&t.title).size(11.5).color(col));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(t.state.label().to_uppercase()).monospace().size(8.5).color(FAINT));
                            });
                        });
                    }

                    Self::section(ui, "📜 last known signals");
                    let evs: Vec<(String, Option<String>, String)> = self
                        .world
                        .events
                        .iter()
                        .filter(|e| e.proj == Some(pi))
                        .map(|e| (e.ts.clone(), e.agent.clone(), e.text.clone()))
                        .collect();
                    if evs.is_empty() {
                        ui.label(RichText::new("no signals on record — this theater has been fully manual").monospace().size(10.0).color(FAINT));
                    }
                    for (ts, ag, text) in evs.iter().rev().take(5).rev() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(ts).monospace().size(9.5).color(FAINT));
                            if let Some(agn) = ag {
                                ui.label(RichText::new(agn).monospace().size(10.0).color(GREEN));
                            }
                            ui.label(RichText::new(text).size(11.0).color(Color32::from_rgb(0xb6, 0xc8, 0xb8)));
                        });
                        ui.add_space(2.0);
                    }

                    let pend: Vec<(usize, String, String)> = self
                        .world
                        .decisions
                        .iter()
                        .enumerate()
                        .filter(|(_, d)| d.proj == pi && !d.resolved)
                        .map(|(di, d)| (di, d.title.to_string(), d.due.to_string()))
                        .collect();
                    if !pend.is_empty() {
                        Self::section(ui, "◆ waiting on you");
                        for (di, title, due) in pend {
                            let ir = Frame::new()
                                .fill(a(AMBER, 15))
                                .stroke(Stroke::new(1.0, a(AMBER, 110)))
                                .corner_radius(CornerRadius::same(3))
                                .inner_margin(Margin::same(8))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(RichText::new(format!("◆ {}", title)).size(11.0).color(AMBER));
                                    ui.label(RichText::new(format!("due {} · open briefing", due)).monospace().size(8.5).color(DIM));
                                });
                            if ir.response.interact(Sense::click()).clicked() {
                                self.acts.push(Act::OpenDecision(di));
                            }
                        }
                    }

                    ui.add_space(12.0);
                    if ui
                        .add(Button::new(RichText::new("✓ CAUGHT UP — RESUME COMMAND").monospace().size(12.0).color(GREEN)).min_size(vec2(ui.available_width(), 36.0)).stroke(Stroke::new(1.0, Color32::from_rgb(0x3a, 0x6b, 0x44))))
                        .clicked()
                    {
                        self.acts.push(Act::CloseRecovery);
                    }
                });
            });
    }

    // ---------- control api ----------
    fn handle_cmd(&mut self, cmd: Cmd, ctx: &egui::Context) -> String {
        match cmd {
            Cmd::Key { name, ctrl } => {
                // the windowing layer turns ctrl+c / ctrl+x into Copy / Cut events
                // (and ctrl+v into a Paste that host_paste replaces); mimic that
                if ctrl && (name.eq_ignore_ascii_case("c") || name.eq_ignore_ascii_case("x")) {
                    let ev = if name.eq_ignore_ascii_case("c") { egui::Event::Copy } else { egui::Event::Cut };
                    ctx.input_mut(|i| i.events.push(ev));
                    return "{\"ok\":true}".into();
                }
                let cap = {
                    let mut c = name.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => String::new(),
                    }
                };
                match egui::Key::from_name(&name).or_else(|| egui::Key::from_name(&cap)) {
                    Some(key) => {
                        ctx.input_mut(|i| {
                            i.events.push(egui::Event::Key {
                                key,
                                physical_key: None,
                                pressed: true,
                                repeat: false,
                                modifiers: if ctrl { egui::Modifiers::CTRL | egui::Modifiers::COMMAND } else { Default::default() },
                            });
                        });
                        "{\"ok\":true}".into()
                    }
                    None => format!("{{\"err\":\"unknown key '{}'\"}}", name),
                }
            }
            Cmd::Text(s) => {
                ctx.input_mut(|i| i.events.push(egui::Event::Text(s)));
                "{\"ok\":true}".into()
            }
            Cmd::Click { x, y, double, world, ui: _ } => {
                let p = if world { self.world_to_screen(pos2(x, y)) } else { pos2(x, y) };
                self.canvas_click(p, double);
                "{\"ok\":true}".into()
            }
            Cmd::Place { x, y, name } => {
                self.build_pos = pos2(x, y);
                self.apply(Act::CommitBase(name));
                format!("{{\"ok\":true,\"proj\":{}}}", self.world.projects.len() - 1)
            }
            Cmd::Destroy { i } => {
                if i >= self.world.projects.len() {
                    "{\"err\":\"no such base\"}".into()
                } else {
                    self.destroy_base(i);
                    "{\"ok\":true}".into()
                }
            }
            Cmd::Link { a, b } => {
                if a >= self.world.projects.len() || b >= self.world.projects.len() {
                    "{\"err\":\"no such base\"}".into()
                } else {
                    self.apply(Act::ToggleLink(a, b));
                    "{\"ok\":true}".into()
                }
            }
            Cmd::Decide { d, o } => {
                if d >= self.world.decisions.len() || o >= self.world.decisions[d].options.len() {
                    "{\"err\":\"no such decision/option\"}".into()
                } else {
                    self.apply(Act::CommitDecision(d, o));
                    "{\"ok\":true}".into()
                }
            }
            Cmd::Capture(s) => {
                self.apply(Act::CommitCapture(s));
                "{\"ok\":true}".into()
            }
            Cmd::ModuleAdd { i, cfg } => {
                if i >= self.world.projects.len() {
                    "{\"err\":\"no such base\"}".into()
                } else if self.world.projects[i].modules.iter().any(|m| m.name == cfg.name) {
                    "{\"err\":\"module name already installed\"}".into()
                } else {
                    let ts = self.clock();
                    let name = cfg.name.clone();
                    self.world.projects[i].modules.push(cfg);
                    self.world.events.push(Event {
                        ts,
                        proj: Some(i),
                        agent: None,
                        text: format!("program installed: ⚙{}", name),
                    });
                    self.dirty = true;
                    self.wasm_sync();
                    "{\"ok\":true}".into()
                }
            }
            Cmd::ModuleRm { i, name } => {
                if i >= self.world.projects.len() {
                    "{\"err\":\"no such base\"}".into()
                } else {
                    let before = self.world.projects[i].modules.len();
                    self.world.projects[i].modules.retain(|m| m.name != name);
                    if self.world.projects[i].modules.len() == before {
                        "{\"err\":\"no such module\"}".into()
                    } else {
                        let pname = self.world.projects[i].name.clone();
                        self.mod_status.remove(&(pname, name.clone()));
                        let ts = self.clock();
                        self.world.events.push(Event {
                            ts,
                            proj: Some(i),
                            agent: None,
                            text: format!("program uninstalled: ⚙{}", name),
                        });
                        self.dirty = true;
                        self.wasm_sync();
                        "{\"ok\":true}".into()
                    }
                }
            }
            Cmd::ModuleToggle { i, name } => {
                match self.world.projects.get_mut(i).and_then(|p| p.modules.iter_mut().find(|m| m.name == name)) {
                    Some(m) => {
                        m.enabled = !m.enabled;
                        let enabled = m.enabled;
                        self.dirty = true;
                        self.wasm_sync();
                        format!("{{\"ok\":true,\"enabled\":{}}}", enabled)
                    }
                    None => "{\"err\":\"no such base/module\"}".into(),
                }
            }
            Cmd::Band { x1, y1, x2, y2, world } => {
                let (a0, b0) = if world {
                    (self.world_to_screen(pos2(x1, y1)), self.world_to_screen(pos2(x2, y2)))
                } else {
                    (pos2(x1, y1), pos2(x2, y2))
                };
                let r = Rect::from_two_pos(a0, b0);
                self.sel_structs = self
                    .clicks
                    .iter()
                    .filter_map(|(zr, z)| match z {
                        ClickZone::Pylon(pi, ti) if r.intersects(*zr) => Some(SRoom::Pylon(*pi, *ti)),
                        ClickZone::Question(pi, qi) if r.intersects(*zr) => Some(SRoom::Question(*pi, *qi)),
                        _ => None,
                    })
                    .collect();
                self.sdestroy_arm = None;
                if !self.sel_structs.is_empty() {
                    self.sel = None;
                }
                format!("{{\"ok\":true,\"selected\":{}}}", self.sel_structs.len())
            }
            Cmd::Cfg { struct_scale } => {
                if let Some(v) = struct_scale {
                    self.prefs.struct_scale = v.clamp(0.3, 5.0);
                    self.dirty = true;
                }
                format!("{{\"ok\":true,\"struct_scale\":{}}}", self.prefs.struct_scale)
            }
            Cmd::Pylon { i, title, pos, state, notes } => {
                if i >= self.world.projects.len() {
                    "{\"err\":\"no such base\"}".into()
                } else {
                    let st = state.as_deref().and_then(TaskState::parse).unwrap_or(TaskState::Todo);
                    let ts = self.clock();
                    match self.world.projects[i].tasks.iter_mut().find(|t| t.title == title) {
                        Some(t) => {
                            t.state = st;
                            if pos.is_some() {
                                t.pos = pos;
                            }
                            if let Some(n) = notes {
                                t.notes = n;
                            }
                        }
                        None => {
                            self.world.projects[i].tasks.push(Task { title: title.clone(), state: st, pos, notes: notes.unwrap_or_default() });
                            self.world.events.push(Event {
                                ts,
                                proj: Some(i),
                                agent: None,
                                text: format!("pylon warped in: {}", title),
                            });
                            self.ping(i);
                        }
                    }
                    self.dirty = true;
                    "{\"ok\":true}".into()
                }
            }
            Cmd::Question { i, text, pos, resolved, notes } => {
                if i >= self.world.projects.len() {
                    "{\"err\":\"no such base\"}".into()
                } else {
                    let ts = self.clock();
                    match self.world.projects[i].questions.iter_mut().find(|q| q.text == text) {
                        Some(q) => {
                            if let Some(r) = resolved {
                                q.resolved = r;
                            }
                            if pos.is_some() {
                                q.pos = pos;
                            }
                            if let Some(n) = notes {
                                q.notes = n;
                            }
                        }
                        None => {
                            self.world.projects[i].questions.push(Question {
                                text: text.clone(),
                                resolved: resolved.unwrap_or(false),
                                pos,
                                notes: notes.unwrap_or_default(),
                            });
                            self.world.events.push(Event {
                                ts,
                                proj: Some(i),
                                agent: None,
                                text: format!("sensor array raised: {}", text),
                            });
                            self.ping(i);
                        }
                    }
                    self.dirty = true;
                    "{\"ok\":true}".into()
                }
            }
            Cmd::Base { i, cwd, sandbox, model } => match self.world.projects.get_mut(i) {
                None => "{\"err\":\"no such base\"}".into(),
                Some(p) => {
                    if let Some(c) = cwd {
                        p.cwd = if c.is_empty() { None } else { Some(c) };
                    }
                    if let Some(sb) = sandbox {
                        p.sandbox = if sb.is_empty() { None } else { Some(sb) };
                    }
                    if let Some(m) = model {
                        p.model = if m.is_empty() { None } else { Some(m) };
                    }
                    self.dirty = true;
                    "{\"ok\":true}".into()
                }
            },
            Cmd::Dispatch { i, title, agent, prompt } => {
                let ti = self.world.projects.get(i).and_then(|p| p.tasks.iter().position(|t| t.title == title));
                match ti {
                    None => "{\"err\":\"no such base/pylon\"}".into(),
                    Some(ti) => match self.dispatch(i, ti, agent, prompt) {
                        Ok(aid) => format!("{{\"ok\":true,\"agent\":\"{}\"}}", aid),
                        Err(e) => format!("{{\"err\":\"{}\"}}", e.replace('"', "'")),
                    },
                }
            }
            Cmd::Tell { i, agent, text } => match self.tell(i, &agent, &text) {
                Ok(()) => "{\"ok\":true}".into(),
                Err(e) => format!("{{\"err\":\"{}\"}}", e.replace('"', "'")),
            },
            Cmd::Halt { i, agent } => {
                if self.halt(i, &agent) {
                    "{\"ok\":true}".into()
                } else {
                    "{\"err\":\"unit is not working\"}".into()
                }
            }
            Cmd::Fire { i, agent } => match self.world.projects.get_mut(i) {
                None => "{\"err\":\"no such base\"}".into(),
                Some(p) => {
                    let name = p.name.clone();
                    let before = p.agents.len();
                    p.agents.retain(|a| a.id != agent);
                    if p.agents.len() == before {
                        "{\"err\":\"no such unit\"}".into()
                    } else {
                        self.workers.halt(&name, &agent);
                        self.dirty = true;
                        "{\"ok\":true}".into()
                    }
                }
            },
            Cmd::State => self.state_json(),
        }
    }

    // ---------- codex workers ----------

    /// send a unit to work pylon `ti` of base `pi`: picks `agent` (or the first
    /// idle unit, or hires a new one), starts a fresh codex thread, pylon → doing
    fn dispatch(&mut self, pi: usize, ti: usize, agent: Option<String>, extra: Option<String>) -> Result<String, String> {
        let proj = self.world.projects.get(pi).ok_or("no such base")?;
        let task = proj.tasks.get(ti).ok_or("no such pylon")?;
        let (title, notes) = (task.title.clone(), task.notes.clone());
        let cwd = proj.cwd.clone().ok_or_else(|| format!("base {} has no repo (cwd) set", proj.name))?;
        let name = proj.name.clone();
        // a pylon already being worked is not handed to a second unit
        if let Some(ag) = proj.agents.iter().find(|a| a.task == title && self.workers.running(&name, &a.id)) {
            return Err(format!("{} is already working this pylon", ag.id));
        }
        let aid = match agent {
            Some(a) => a,
            // the unit that last held this pylon keeps it (fresh thread); else an idle one; else hire
            None => match proj
                .agents
                .iter()
                .find(|a| a.task == title)
                .or_else(|| proj.agents.iter().find(|a| a.state == AgentState::Idle && !self.workers.running(&name, &a.id)))
            {
                Some(a) => a.id.clone(),
                None => {
                    let mut n = proj.agents.len() + 1;
                    while proj.agents.iter().any(|a| a.id == format!("cx-{}", n)) {
                        n += 1;
                    }
                    format!("cx-{}", n)
                }
            },
        };
        if self.workers.running(&name, &aid) {
            return Err(format!("{} is already working", aid));
        }
        let prompt = crate::worker::prompt(&aid, &name, &proj.goal, &title, &notes, extra.as_deref().unwrap_or(""));
        let job = crate::worker::Job {
            proj: name.clone(),
            agent: aid.clone(),
            cwd,
            sandbox: proj.sandbox.clone().unwrap_or_else(|| "workspace-write".into()),
            model: proj.model.clone(),
            prompt,
            resume: None,
        };
        self.workers.start(job)?;
        let ts = self.clock();
        let p = &mut self.world.projects[pi];
        let hired = !p.agents.iter().any(|a| a.id == aid);
        if hired {
            p.agents.push(Agent::new(aid.clone()));
        }
        let ag = p.agents.iter_mut().find(|a| a.id == aid).unwrap();
        ag.state = AgentState::Working;
        ag.task = title.clone();
        ag.blocked_on = None;
        ag.thread_id = None;
        ag.last_msg.clear();
        ag.last_report = ts;
        p.tasks[ti].state = TaskState::Doing;
        self.report(pi, Some(&aid), &format!("{}dispatched → {}", if hired { "hired · " } else { "" }, title));
        Ok(aid)
    }

    /// follow-up order for a unit: resumes its codex thread with `text`
    fn tell(&mut self, pi: usize, aid: &str, text: &str) -> Result<(), String> {
        let proj = self.world.projects.get(pi).ok_or("no such base")?;
        let ag = proj.agents.iter().find(|a| a.id == aid).ok_or("no such unit")?;
        let thread = ag.thread_id.clone().ok_or("unit has no codex thread yet — dispatch it first")?;
        let cwd = proj.cwd.clone().ok_or("base has no repo (cwd) set")?;
        let name = proj.name.clone();
        let job = crate::worker::Job {
            proj: name,
            agent: aid.to_string(),
            cwd,
            sandbox: proj.sandbox.clone().unwrap_or_else(|| "workspace-write".into()),
            model: proj.model.clone(),
            prompt: format!(
                "Commander's follow-up order: {}\n\nSame closing rule as before: end with one DONE: / BLOCKED: / PARTIAL: line.",
                text
            ),
            resume: Some(thread),
        };
        self.workers.start(job)?;
        let ts = self.clock();
        let p = &mut self.world.projects[pi];
        let task = {
            let ag = p.agents.iter_mut().find(|a| a.id == aid).unwrap();
            ag.state = AgentState::Working;
            ag.blocked_on = None;
            ag.last_report = ts;
            ag.task.clone()
        };
        if let Some(t) = p.tasks.iter_mut().find(|t| t.title == task) {
            if t.state == TaskState::Blocked {
                t.state = TaskState::Doing;
            }
        }
        let short: String = text.chars().take(120).collect();
        self.report(pi, Some(aid), &format!("order: {}", short));
        Ok(())
    }

    fn halt(&mut self, pi: usize, aid: &str) -> bool {
        let Some(name) = self.world.projects.get(pi).map(|p| p.name.clone()) else { return false };
        let ok = self.workers.halt(&name, aid);
        if ok {
            self.report(pi, Some(aid), "halted by commander");
        }
        ok
    }

    fn agent_mut(&mut self, proj: &str, aid: &str) -> Option<(usize, &mut Agent)> {
        let pi = self.proj_by_name(proj)?;
        let ag = self.world.projects[pi].agents.iter_mut().find(|a| a.id == aid)?;
        Some((pi, ag))
    }

    /// drain codex event streams into units, pylons and the comms wall
    fn worker_pump(&mut self) {
        use crate::worker::{verdict, Out, Verdict};
        for out in self.workers.drain() {
            match out {
                Out::Started { proj, agent, thread_id } => {
                    if let Some((_, ag)) = self.agent_mut(&proj, &agent) {
                        ag.thread_id = Some(thread_id);
                        self.dirty = true;
                    }
                }
                Out::Cmd { proj, agent, command, exit_code, ok } => {
                    if let Some(pi) = self.proj_by_name(&proj) {
                        let short: String = command.chars().take(90).collect();
                        let tail = match (ok, exit_code) {
                            (true, _) => String::new(),
                            (false, Some(c)) => format!(" ✗ exit {}", c),
                            (false, None) => " ✗".into(),
                        };
                        self.report_quiet(pi, Some(&agent), &format!("$ {}{}", short, tail));
                    }
                }
                Out::Files { proj, agent, paths } => {
                    if let Some(pi) = self.proj_by_name(&proj) {
                        let names: Vec<String> = paths
                            .iter()
                            .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
                            .take(6)
                            .collect();
                        let more = if paths.len() > 6 { format!(" +{}", paths.len() - 6) } else { String::new() };
                        self.report_quiet(pi, Some(&agent), &format!("✎ {}{}", names.join(", "), more));
                    }
                }
                Out::Msg { proj, agent, text } => {
                    if let Some((pi, ag)) = self.agent_mut(&proj, &agent) {
                        ag.last_msg = text.clone();
                        let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
                        let short: String = first.chars().take(160).collect();
                        self.report_quiet(pi, Some(&agent), &short);
                    }
                }
                Out::Error { proj, agent, text } => {
                    if let Some((pi, ag)) = self.agent_mut(&proj, &agent) {
                        ag.state = AgentState::Blocked;
                        ag.blocked_on = Some(text.clone());
                        let task = ag.task.clone();
                        if let Some(t) = self.world.projects[pi].tasks.iter_mut().find(|t| t.title == task) {
                            t.state = TaskState::Blocked;
                        }
                        let short: String = text.chars().take(200).collect();
                        self.report(pi, Some(&agent), &format!("✗ {}", short));
                    }
                }
                Out::Turn { proj, agent, input_tokens, output_tokens } => {
                    if let Some((pi, ag)) = self.agent_mut(&proj, &agent) {
                        ag.turns += 1;
                        ag.tokens += input_tokens + output_tokens;
                        let task = ag.task.clone();
                        let v = verdict(&ag.last_msg);
                        let (st, summary) = match &v {
                            Some((Verdict::Done, s)) => (AgentState::Idle, format!("✓ DONE: {}", s)),
                            Some((Verdict::Blocked, s)) => (AgentState::Blocked, format!("⚠ BLOCKED: {}", s)),
                            Some((Verdict::Partial, s)) => (AgentState::Idle, format!("… PARTIAL: {}", s)),
                            None => (AgentState::Idle, "turn complete (no status line)".into()),
                        };
                        ag.state = st;
                        ag.blocked_on = match &v {
                            Some((Verdict::Blocked, s)) => Some(s.clone()),
                            _ => None,
                        };
                        let tstate = match &v {
                            Some((Verdict::Done, _)) => Some(TaskState::Done),
                            Some((Verdict::Blocked, _)) => Some(TaskState::Blocked),
                            _ => None,
                        };
                        if let Some(ts) = tstate {
                            if let Some(t) = self.world.projects[pi].tasks.iter_mut().find(|t| t.title == task) {
                                t.state = ts;
                            }
                        }
                        let short: String = summary.chars().take(220).collect();
                        self.report(pi, Some(&agent), &format!("{} · {} tok", short, input_tokens + output_tokens));
                    }
                }
                Out::Exited { proj, agent, code, stderr_tail } => {
                    if let Some((pi, ag)) = self.agent_mut(&proj, &agent) {
                        // still "working" here = process died without turn.completed
                        if ag.state == AgentState::Working {
                            ag.state = AgentState::Idle;
                            let last = stderr_tail.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
                            let short: String = last.chars().take(160).collect();
                            self.report(pi, Some(&agent), &format!("process exited (code {:?}) {}", code, short));
                        }
                        self.dirty = true;
                    }
                }
                Out::Eof { .. } => {}
            }
        }
    }

    fn state_json(&self) -> String {
        fn j(s: &str) -> String {
            let mut o = String::with_capacity(s.len() + 2);
            for c in s.chars() {
                match c {
                    '"' => o.push_str("\\\""),
                    '\\' => o.push_str("\\\\"),
                    '\n' => o.push_str("\\n"),
                    c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                    c => o.push(c),
                }
            }
            o
        }
        fn opt(v: Option<usize>) -> String {
            v.map(|x| x.to_string()).unwrap_or_else(|| "null".into())
        }
        let projects: Vec<String> = self
            .world
            .projects
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let agents: Vec<String> = p
                    .agents
                    .iter()
                    .map(|a| {
                        format!(
                            "{{\"id\":\"{}\",\"state\":\"{}\",\"task\":\"{}\",\"blocked_on\":{},\"running\":{},\"thread_id\":{},\"turns\":{},\"tokens\":{},\"last_report\":\"{}\",\"last_msg\":\"{}\"}}",
                            j(&a.id),
                            a.state.label(),
                            j(&a.task),
                            a.blocked_on.as_ref().map(|b| format!("\"{}\"", j(b))).unwrap_or_else(|| "null".into()),
                            self.workers.running(&p.name, &a.id),
                            a.thread_id.as_ref().map(|b| format!("\"{}\"", j(b))).unwrap_or_else(|| "null".into()),
                            a.turns,
                            a.tokens,
                            j(&a.last_report),
                            j(&a.last_msg),
                        )
                    })
                    .collect();
                let tasks: Vec<String> = p
                    .tasks
                    .iter()
                    .enumerate()
                    .map(|(ti, t)| {
                        let (x, y) = t.pos.unwrap_or_else(|| self.pylon_world_pos(i, ti));
                        format!("{{\"title\":\"{}\",\"state\":\"{}\",\"notes\":\"{}\",\"pos\":[{:.1},{:.1}]}}", j(&t.title), t.state.label(), j(&t.notes), x, y)
                    })
                    .collect();
                let questions: Vec<String> = p
                    .questions
                    .iter()
                    .enumerate()
                    .map(|(qi, q)| {
                        let (x, y) = q.pos.unwrap_or_else(|| self.question_world_pos(i, qi));
                        format!("{{\"text\":\"{}\",\"resolved\":{},\"notes\":\"{}\",\"pos\":[{:.1},{:.1}]}}", j(&q.text), q.resolved, j(&q.notes), x, y)
                    })
                    .collect();
                let modules: Vec<String> = p
                    .modules
                    .iter()
                    .map(|m| {
                        let st = self.mod_status.get(&(p.name.clone(), m.name.clone()));
                        format!(
                            "{{\"name\":\"{}\",\"path\":\"{}\",\"enabled\":{},\"interval_sec\":{},\"fuel_per_tick\":{},\"max_http_per_tick\":{},\"ticks\":{},\"fuel_used\":{},\"http_used\":{},\"ms\":{:.1},\"error\":{}}}",
                            j(&m.name),
                            j(&m.path),
                            m.enabled,
                            m.interval_sec,
                            m.fuel_per_tick,
                            m.max_http_per_tick,
                            st.map_or(0, |s| s.ticks),
                            st.map_or(0, |s| s.fuel_used),
                            st.map_or(0, |s| s.http_used),
                            st.map_or(0.0, |s| s.ms),
                            st.and_then(|s| s.error.as_ref()).map(|e| format!("\"{}\"", j(e))).unwrap_or_else(|| "null".into()),
                        )
                    })
                    .collect();
                format!(
                    "{{\"i\":{},\"name\":\"{}\",\"status\":\"{}\",\"goal\":\"{}\",\"pos\":[{:.1},{:.1}],\"tier\":\"{}\",\"unseen\":{},\"cwd\":{},\"sandbox\":{},\"model\":{},\"agents\":[{}],\"tasks\":[{}],\"questions\":[{}],\"modules\":[{}]}}",
                    i,
                    j(&p.name),
                    j(&p.status),
                    j(&p.goal),
                    p.pos.0,
                    p.pos.1,
                    self.tier(i).label(),
                    self.rt[i].unseen_events,
                    p.cwd.as_ref().map(|b| format!("\"{}\"", j(b))).unwrap_or_else(|| "null".into()),
                    p.sandbox.as_ref().map(|b| format!("\"{}\"", j(b))).unwrap_or_else(|| "null".into()),
                    p.model.as_ref().map(|b| format!("\"{}\"", j(b))).unwrap_or_else(|| "null".into()),
                    agents.join(","),
                    tasks.join(","),
                    questions.join(","),
                    modules.join(","),
                )
            })
            .collect();
        let links: Vec<String> = self.world.links.iter().map(|l| format!("[{},{}]", l.a, l.b)).collect();
        let decisions: Vec<String> = self
            .world
            .decisions
            .iter()
            .enumerate()
            .map(|(di, d)| {
                format!(
                    "{{\"i\":{},\"id\":\"{}\",\"proj\":{},\"title\":\"{}\",\"due\":\"{}\",\"resolved\":{},\"chosen\":{}}}",
                    di,
                    j(&d.id),
                    d.proj,
                    j(&d.title),
                    j(&d.due),
                    d.resolved,
                    d.chosen.as_ref().map(|c| format!("\"{}\"", j(c))).unwrap_or_else(|| "null".into()),
                )
            })
            .collect();
        let captures: Vec<String> = self
            .world
            .captures
            .iter()
            .map(|c| format!("{{\"text\":\"{}\",\"ts\":\"{}\",\"pos\":[{:.1},{:.1}]}}", j(&c.text), j(&c.ts), c.pos.0, c.pos.1))
            .collect();
        let events: Vec<String> = self
            .world
            .events
            .iter()
            .rev()
            .take(10)
            .map(|e| {
                format!(
                    "{{\"ts\":\"{}\",\"proj\":{},\"agent\":{},\"text\":\"{}\"}}",
                    j(&e.ts),
                    opt(e.proj),
                    e.agent.as_ref().map(|a| format!("\"{}\"", j(a))).unwrap_or_else(|| "null".into()),
                    j(&e.text),
                )
            })
            .collect();
        format!(
            "{{\"clock\":\"{}\",\"sel\":{},\"interior\":{},\"link_from\":{},\"cam\":{{\"x\":{:.1},\"y\":{:.1},\"scale\":{:.3}}},\"viewport\":[{:.0},{:.0}],\"build_open\":{},\"build_menu\":{},\"struct_scale\":{},\"sel_structs\":{},\"codex\":{},\"workers_running\":{},\"sroom\":{},\"projects\":[{}],\"links\":[{}],\"decisions\":[{}],\"captures\":[{}],\"events\":[{}]}}",
            self.clock(),
            opt(self.sel),
            opt(self.interior),
            opt(self.link_from),
            self.cam.pos.x,
            self.cam.pos.y,
            self.cam.scale,
            self.viewport.width(),
            self.viewport.height(),
            self.build_open,
            self.build_menu,
            self.prefs.struct_scale,
            self.sel_structs.len(),
            match self.codex.lock().unwrap().clone() {
                Some(u) => format!(
                    "{{\"pct_left\":{:.1},\"resets_at\":{},\"eta\":\"{}\"}}",
                    u.pct_left,
                    u.resets_at,
                    crate::codex::eta(u.resets_at)
                ),
                None => "null".into(),
            },
            self.workers.running_count(),
            match self.sroom {
                Some(SRoom::Pylon(pi, ti)) => format!("{{\"kind\":\"pylon\",\"proj\":{},\"idx\":{}}}", pi, ti),
                Some(SRoom::Question(pi, qi)) => format!("{{\"kind\":\"question\",\"proj\":{},\"idx\":{}}}", pi, qi),
                None => "null".into(),
            },
            projects.join(","),
            links.join(","),
            decisions.join(","),
            captures.join(","),
            events.join(","),
        )
    }

    fn capture_window(&mut self, ctx: &egui::Context) {
        if !self.capture_open {
            return;
        }
        egui::Window::new("capture")
            .title_bar(false)
            .resizable(false)
            .fixed_size(vec2(420.0, 80.0))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -64.0))
            .frame(Frame::new().fill(Color32::from_rgb(0x0d, 0x14, 0x0e)).stroke(Stroke::new(1.0, LINE_HI)).inner_margin(Margin::same(10)).corner_radius(CornerRadius::same(4)))
            .show(ctx, |ui| {
                ui.label(RichText::new("⚡ QUICK CAPTURE — LANDS UNSORTED, ZERO FILING").monospace().size(9.5).color(GREEN));
                ui.add_space(4.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.capture_text)
                        .desired_width(f32::INFINITY)
                        .hint_text("type a thought, hit enter…"),
                );
                if self.capture_focus {
                    resp.request_focus();
                    self.capture_focus = false;
                }
                ui.label(RichText::new("enter = drop into the drift field · esc = cancel").monospace().size(9.0).color(DIM));
                let enter = ctx.input(|i| i.key_pressed(Key::Enter));
                let esc = ctx.input(|i| i.key_pressed(Key::Escape));
                if enter && !self.capture_text.trim().is_empty() {
                    let text = self.capture_text.trim().to_string();
                    self.acts.push(Act::CommitCapture(text));
                } else if esc || (enter && self.capture_text.trim().is_empty()) {
                    self.capture_open = false;
                    self.capture_text.clear();
                }
            });
    }

    fn build_window(&mut self, ctx: &egui::Context) {
        if !self.build_open {
            return;
        }
        egui::Window::new("build")
            .title_bar(false)
            .resizable(false)
            .fixed_size(vec2(420.0, 80.0))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -64.0))
            .frame(Frame::new().fill(Color32::from_rgb(0x0d, 0x14, 0x0e)).stroke(Stroke::new(1.0, LINE_HI)).inner_margin(Margin::same(10)).corner_radius(CornerRadius::same(4)))
            .show(ctx, |ui| {
                ui.label(RichText::new("⌂ ESTABLISH BASE — NAME THE THEATER").monospace().size(9.5).color(GREEN));
                ui.add_space(4.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.build_text)
                        .desired_width(f32::INFINITY)
                        .hint_text("theater name, hit enter…"),
                );
                if self.build_focus {
                    resp.request_focus();
                    self.build_focus = false;
                }
                ui.label(RichText::new("enter = establish at the marked spot · esc = cancel").monospace().size(9.0).color(DIM));
                let enter = ctx.input(|i| i.key_pressed(Key::Enter));
                let esc = ctx.input(|i| i.key_pressed(Key::Escape));
                if enter && !self.build_text.trim().is_empty() {
                    let name = self.build_text.trim().to_string();
                    self.acts.push(Act::CommitBase(name));
                } else if esc || (enter && self.build_text.trim().is_empty()) {
                    self.build_open = false;
                    self.build_text.clear();
                }
            });
    }

    fn pylon_window(&mut self, ctx: &egui::Context) {
        if !self.pylon_open {
            return;
        }
        egui::Window::new("pylon")
            .title_bar(false)
            .resizable(false)
            .fixed_size(vec2(420.0, 80.0))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -64.0))
            .frame(Frame::new().fill(Color32::from_rgb(0x0d, 0x14, 0x0e)).stroke(Stroke::new(1.0, LINE_HI)).inner_margin(Margin::same(10)).corner_radius(CornerRadius::same(4)))
            .show(ctx, |ui| {
                let base = self.sel.and_then(|i| self.world.projects.get(i)).map(|p| p.name.clone()).unwrap_or_default();
                ui.label(RichText::new(format!("◆ WARP IN PYLON — GOAL FOR {}", base.to_uppercase())).monospace().size(9.5).color(CYAN));
                ui.add_space(4.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.pylon_text)
                        .desired_width(f32::INFINITY)
                        .hint_text("goal / task title, hit enter…"),
                );
                if self.pylon_focus {
                    resp.request_focus();
                    self.pylon_focus = false;
                }
                ui.label(RichText::new("enter = warp in at the marked spot · esc = cancel").monospace().size(9.0).color(DIM));
                let enter = ctx.input(|i| i.key_pressed(Key::Enter));
                let esc = ctx.input(|i| i.key_pressed(Key::Escape));
                if enter && !self.pylon_text.trim().is_empty() {
                    let title = self.pylon_text.trim().to_string();
                    self.acts.push(Act::CommitPylon(title));
                } else if esc || (enter && self.pylon_text.trim().is_empty()) {
                    self.pylon_open = false;
                    self.pylon_text.clear();
                }
            });
    }

    fn quest_window(&mut self, ctx: &egui::Context) {
        if !self.quest_open {
            return;
        }
        egui::Window::new("question")
            .title_bar(false)
            .resizable(false)
            .fixed_size(vec2(420.0, 80.0))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -64.0))
            .frame(Frame::new().fill(Color32::from_rgb(0x0d, 0x14, 0x0e)).stroke(Stroke::new(1.0, LINE_HI)).inner_margin(Margin::same(10)).corner_radius(CornerRadius::same(4)))
            .show(ctx, |ui| {
                let base = self.sel.and_then(|i| self.world.projects.get(i)).map(|p| p.name.clone()).unwrap_or_default();
                ui.label(RichText::new(format!("⌖ RAISE SENSOR ARRAY — QUESTION FOR {}", base.to_uppercase())).monospace().size(9.5).color(AMBER));
                ui.add_space(4.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.quest_text)
                        .desired_width(f32::INFINITY)
                        .hint_text("open question / research thread, hit enter…"),
                );
                if self.quest_focus {
                    resp.request_focus();
                    self.quest_focus = false;
                }
                ui.label(RichText::new("enter = start scanning at the marked spot · esc = cancel").monospace().size(9.0).color(DIM));
                let enter = ctx.input(|i| i.key_pressed(Key::Enter));
                let esc = ctx.input(|i| i.key_pressed(Key::Escape));
                if enter && !self.quest_text.trim().is_empty() {
                    let text = self.quest_text.trim().to_string();
                    self.acts.push(Act::CommitQuestion(text));
                } else if esc || (enter && self.quest_text.trim().is_empty()) {
                    self.quest_open = false;
                    self.quest_text.clear();
                }
            });
    }
}

impl eframe::App for CommanderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        self.time = ctx.input(|i| i.time);
        self.now_min = now_min();

        // control api: apply injected commands before building the UI so
        // injected key/text events are seen by this frame's widgets
        while let Ok(req) = self.ctrl.try_recv() {
            let resp = self.handle_cmd(req.cmd, ctx);
            let _ = req.reply.send(resp);
        }

        // camera animation
        let k = 1.0 - (-8.0 * dt).exp();
        self.cam.pos = self.cam.pos.lerp(self.cam.target_pos, k);
        self.cam.scale += (self.cam.target_scale - self.cam.scale) * k;

        self.host_paste(ctx);
        self.keyboard(ctx);
        self.topbar(ctx);
        self.codex_meter(ctx);
        self.card_panel(ctx);
        self.world_canvas(ctx);
        self.rail(ctx);
        self.minimap_cluster(ctx);
        self.hint_bar(ctx);
        self.crumb(ctx);
        self.toasts_ui(ctx);
        self.briefing_window(ctx);
        self.recovery_window(ctx);
        self.capture_window(ctx);
        self.build_window(ctx);
        self.pylon_window(ctx);
        self.quest_window(ctx);
        self.host_copy(ctx);

        let acts: Vec<Act> = self.acts.drain(..).collect();
        for act in acts {
            self.apply(act);
        }

        // codex workers: ingest their event streams
        self.worker_pump();

        // wasm module host: ingest outputs, then refresh building snapshots ~1/s
        self.wasm_pump();
        if self.time - self.last_wasm_sync > 1.0 {
            self.wasm_sync();
        }

        // debounced autosave of the space
        if self.dirty && self.time - self.last_save > 2.0 {
            self.save_space();
        }

        ctx.request_repaint();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.dirty {
            self.save_space();
        }
    }
}
