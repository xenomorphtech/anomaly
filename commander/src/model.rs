use eframe::egui::Color32;
use serde::{Deserialize, Serialize};

pub const WW: f32 = 12000.0;
pub const WH: f32 = 8000.0;
pub const BASE_W: f32 = 360.0;

/// serialize Color32 as [r, g, b]
pub mod color_rgb {
    use eframe::egui::Color32;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(c: &Color32, s: S) -> Result<S::Ok, S::Error> {
        [c.r(), c.g(), c.b()].serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Color32, D::Error> {
        let [r, g, b] = <[u8; 3]>::deserialize(d)?;
        Ok(Color32::from_rgb(r, g, b))
    }
}

// color rotation for newly established bases
pub const PROJ_COLORS: [Color32; 8] = [
    Color32::from_rgb(0xe0, 0xa4, 0x58),
    Color32::from_rgb(0x6f, 0xb3, 0xd2),
    Color32::from_rgb(0xb5, 0x8e, 0xe0),
    Color32::from_rgb(0x7f, 0xc9, 0x8a),
    Color32::from_rgb(0xd2, 0x6f, 0x8e),
    Color32::from_rgb(0x8e, 0xd2, 0xc9),
    Color32::from_rgb(0xd2, 0xc9, 0x6f),
    Color32::from_rgb(0x9a, 0xa4, 0xe0),
];

/// current local time as a continuous minute counter (display wraps mod 1440)
pub fn now_min() -> f64 {
    let now = chrono::Local::now();
    use chrono::Offset;
    let offset_s = now.offset().fix().local_minus_utc() as f64;
    (now.timestamp() as f64 + offset_s) / 60.0
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Working,
    Blocked,
    Idle,
}

impl AgentState {
    pub fn parse(s: &str) -> Option<AgentState> {
        match s {
            "working" => Some(AgentState::Working),
            "blocked" => Some(AgentState::Blocked),
            "idle" => Some(AgentState::Idle),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            AgentState::Working => "working",
            AgentState::Blocked => "blocked",
            AgentState::Idle => "idle",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub state: AgentState,
    /// pylon title the unit is assigned to
    pub task: String,
    pub last_report: String,
    pub blocked_on: Option<String>,
    /// codex thread id — follow-up orders resume it so context carries over
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// the unit's latest message (its report from the last turn)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_msg: String,
    /// completed codex turns / tokens spent by this unit
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub turns: u32,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub tokens: i64,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}
fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}

impl Agent {
    pub fn new(id: String) -> Agent {
        Agent {
            id,
            state: AgentState::Idle,
            task: String::new(),
            last_report: String::new(),
            blocked_on: None,
            thread_id: None,
            last_msg: String::new(),
            turns: 0,
            tokens: 0,
        }
    }
}

// all states are constructible by a future data source, even if unused today
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Done,
    Doing,
    Todo,
    Blocked,
}

impl TaskState {
    pub fn parse(s: &str) -> Option<TaskState> {
        match s {
            "done" => Some(TaskState::Done),
            "doing" => Some(TaskState::Doing),
            "todo" => Some(TaskState::Todo),
            "blocked" => Some(TaskState::Blocked),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            TaskState::Done => "done",
            TaskState::Doing => "doing",
            TaskState::Todo => "todo",
            TaskState::Blocked => "blocked",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Task {
    /// short name — the visual cue on the map; the substance lives in `notes`
    pub title: String,
    pub state: TaskState,
    /// free-form body edited inside the pylon's room (what the goal actually is)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// world-space pylon anchor; None = auto slot in the ring around the base
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<(f32, f32)>,
}

/// an open question / research thread anchored to a building (sensor array)
#[derive(Clone, Serialize, Deserialize)]
pub struct Question {
    /// short name — the visual cue on the map; the substance lives in `notes`
    pub text: String,
    #[serde(default)]
    pub resolved: bool,
    /// free-form body edited inside the sensor's room (context, findings, answer)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// world-space anchor; None = auto slot in the arc around the base
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<(f32, f32)>,
}

/// a wasm program installed in a building (spacetimedb-style module + budgets)
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleCfg {
    pub name: String,
    /// path to a .wasm or .wat file, relative to the working directory
    pub path: String,
    /// minimum seconds between ticks
    #[serde(default = "d_interval")]
    pub interval_sec: f64,
    /// wasm fuel (≈ instructions) each tick may burn before being trapped
    #[serde(default = "d_fuel")]
    pub fuel_per_tick: u64,
    /// http/https requests allowed per tick
    #[serde(default = "d_http")]
    pub max_http_per_tick: u32,
    /// largest http response body kept, in KiB
    #[serde(default = "d_http_kib")]
    pub max_http_resp_kib: u32,
    #[serde(default = "d_true")]
    pub enabled: bool,
}

impl ModuleCfg {
    pub fn new(name: String, path: String) -> ModuleCfg {
        ModuleCfg {
            name,
            path,
            interval_sec: d_interval(),
            fuel_per_tick: d_fuel(),
            max_http_per_tick: d_http(),
            max_http_resp_kib: d_http_kib(),
            enabled: true,
        }
    }
}

fn d_interval() -> f64 {
    60.0
}
fn d_fuel() -> u64 {
    50_000_000
}
fn d_http() -> u32 {
    4
}
fn d_http_kib() -> u32 {
    256
}
fn d_true() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    #[serde(with = "color_rgb")]
    pub color: Color32,
    pub icon: usize, // index into the strategy_building.jpeg icon sheet
    pub status: String,
    pub goal: String,
    pub agents: Vec<Agent>,
    pub tasks: Vec<Task>,
    pub pos: (f32, f32), // world-space top-left of the base card
    /// wasm programs this building runs (signals, reducers, pollers)
    #[serde(default)]
    pub modules: Vec<ModuleCfg>,
    /// open questions / research threads (sensor arrays around the base)
    #[serde(default)]
    pub questions: Vec<Question>,
    /// repository the base's units work in (codex cwd); None = no workers
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// codex sandbox for units: read-only | workspace-write (default) | danger-full-access
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<String>,
    /// codex model override for units (None = ~/.codex/config.toml default)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub proj: usize,
    pub title: String,
    pub options: Vec<String>,
    pub due: String,
    pub resolved: bool,
    pub chosen: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CaptureNote {
    pub text: String,
    pub ts: String,
    pub pos: (f32, f32), // world-space anchor (drift animates around it)
}

/// undirected link between two bases (project indices)
#[derive(Clone, Serialize, Deserialize)]
pub struct Link {
    pub a: usize,
    pub b: usize,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Event {
    pub ts: String,
    pub proj: Option<usize>,
    pub agent: Option<String>,
    pub text: String,
}

pub struct World {
    pub projects: Vec<Project>,
    pub decisions: Vec<Decision>,
    pub captures: Vec<CaptureNote>,
    pub events: Vec<Event>,
    pub links: Vec<Link>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Warm,
    Cooling,
    Cold,
    Frozen,
}

impl Tier {
    pub fn from_age_min(age: f64) -> Tier {
        if age < 30.0 {
            Tier::Warm
        } else if age < 120.0 {
            Tier::Cooling
        } else if age < 1440.0 {
            Tier::Cold
        } else {
            Tier::Frozen
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Tier::Warm => "warm",
            Tier::Cooling => "cooling",
            Tier::Cold => "cold",
            Tier::Frozen => "frozen",
        }
    }
    pub fn fog_alpha(self) -> u8 {
        match self {
            Tier::Warm => 0,
            Tier::Cooling => 60,
            Tier::Cold => 130,
            Tier::Frozen => 190,
        }
    }
}

/// per-project runtime state (staleness, deltas, resume snapshot)
pub struct Rt {
    pub last_visit_min: f64, // absolute minute of last visit
    pub delta: Vec<String>,  // events accumulated while away
    pub unseen_events: u32,
    pub shown: Vec<String>, // resume snapshot rendered in the card
    pub shown_age_min: f64,
    pub shown_age: String,
}

pub fn initial_world() -> World {
    World {
        projects: vec![],
        decisions: vec![],
        captures: vec![],
        events: vec![],
        links: vec![],
    }
}

pub fn new_rt(now: f64) -> Rt {
    Rt {
        last_visit_min: now,
        delta: vec![],
        unseen_events: 0,
        shown: vec![],
        shown_age_min: 0.0,
        shown_age: String::new(),
    }
}

pub fn initial_rt(world: &World) -> Vec<Rt> {
    let now = now_min();
    world.projects.iter().map(|_| new_rt(now)).collect()
}

pub fn fmt_age(age_min: f64) -> String {
    let m = age_min.max(0.0) as i64;
    if m < 1 {
        "just now".into()
    } else if m < 60 {
        format!("{}m ago", m)
    } else if m < 1440 {
        format!("{}h ago", m / 60)
    } else {
        format!("{}d {}h ago", m / 1440, (m % 1440) / 60)
    }
}
