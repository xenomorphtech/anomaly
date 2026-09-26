//! Wire protocol between the commander daemon and its frontends.
//!
//! One daemon owns one space (the world, its wasm building programs, its codex
//! units). Any number of frontends attach to it: they read the world through
//! `/snapshot` (a long-poll on a version counter — the daemon publishes a fresh
//! `Snapshot` whenever anything changed) and change it through commands, one
//! HTTP request each (`Cmd`, parsed from path + query by `parse`). Everything a
//! frontend keeps for itself — camera, selection, open rooms, toasts — never
//! reaches the daemon.

use crate::model::*;
use crate::store::Prefs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// last-run report the engine keeps per (building, module) for the UI
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ModStatus {
    pub ticks: u64,
    pub fuel_used: u64,
    pub http_used: u32,
    pub ms: f64,
    pub error: Option<String>,
    pub last_log: Option<String>,
}

/// codex subscription usage — the supply counter shown top-center
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub pct_left: f32,
    pub resets_at: i64, // unix seconds
    pub window_minutes: i64,
    /// mtime of the rollout the numbers came from (unix seconds) — staleness
    pub read_from: i64,
}

/// something the engine wants the commander to notice: a toast, a map ping,
/// or both. frontends replay every notice newer than the last one they saw.
#[derive(Clone, Serialize, Deserialize)]
pub struct Notice {
    pub seq: u64,
    pub proj: Option<usize>,
    pub head: String,
    pub body: String,
    pub sub: String,
    pub ok: bool,
    /// show a toast (false = ping only, e.g. routine worker traffic)
    pub loud: bool,
    /// pulse the base on the map and minimap
    pub ping: bool,
    /// an agent report: the frontend decides the sub-line from whether the
    /// base is in view ("updated in place" vs "SPACE jumps to it")
    pub report: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ModuleStatusRec {
    pub proj: String,
    pub module: String,
    pub status: ModStatus,
}

/// the whole world as one frontend-facing document
#[derive(Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u64,
    pub world: World,
    pub rt: Vec<Rt>,
    /// bases with unseen events, oldest first (SPACE jumps to the last)
    pub unseen: Vec<usize>,
    pub prefs: Prefs,
    pub modules: Vec<ModuleStatusRec>,
    /// (base name, unit id) pairs whose codex turn is running right now
    pub running: Vec<(String, String)>,
    pub codex: Option<Usage>,
    /// the newest notices (bounded tail)
    pub notices: Vec<Notice>,
    pub space_path: String,
    pub archive_path: String,
}

impl Snapshot {
    pub fn module_status(&self) -> HashMap<(String, String), ModStatus> {
        self.modules.iter().map(|m| ((m.proj.clone(), m.module.clone()), m.status.clone())).collect()
    }
}

/// a substructure of a base, by index
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Target {
    Pylon(usize),
    Question(usize),
}

/// what a dispatch is aimed at
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DispatchTarget {
    /// pylon title first, then question text (the historic `/dispatch?title=`)
    Title(String),
    At(Target),
}

/// one command against the world. `parse` maps an HTTP request onto it.
#[derive(Clone, Debug)]
pub enum Cmd {
    State,
    /// establish a base centered at world x,y
    Place { x: f32, y: f32, name: String },
    Destroy { i: usize },
    /// toggle a link between two bases
    Link { a: usize, b: usize },
    /// sever link `li`
    Unlink { li: usize },
    Decide { d: usize, o: usize },
    /// drop a capture note; position = where the commander was looking
    Capture { text: String, pos: Option<(f32, f32)> },
    FileCapture { cap: usize, proj: usize },
    DiscardCapture { cap: usize },
    ModuleAdd { i: usize, cfg: ModuleCfg },
    ModuleRm { i: usize, name: String },
    ModuleToggle { i: usize, name: String },
    Cfg { struct_scale: Option<f32>, show_rail: Option<bool> },
    /// upsert a pylon by title
    Pylon { i: usize, title: String, pos: Option<(f32, f32)>, state: Option<String>, notes: Option<String>, effort: Option<String> },
    /// upsert a sensor array by question text
    Question { i: usize, text: String, pos: Option<(f32, f32)>, resolved: Option<bool>, notes: Option<String>, effort: Option<String> },
    /// edit a substructure in place, by index
    Struct {
        i: usize,
        at: Target,
        state: Option<TaskState>,
        resolved: Option<bool>,
        notes: Option<String>,
        /// Some(None) clears the override (base default)
        effort: Option<Option<String>>,
        pos: Option<(f32, f32)>,
        /// the commander opened the room: a finished report has been read
        read: bool,
    },
    /// demolish substructures (any bases)
    RemoveStructs { list: Vec<(usize, Target)> },
    MoveBase { i: usize, x: f32, y: f32 },
    MoveCapture { ci: usize, x: f32, y: f32 },
    Base { i: usize, cwd: Option<String>, sandbox: Option<String>, model: Option<String>, effort: Option<String> },
    /// `cont`: continuation — the prompt carries the structure's last report
    Dispatch { i: usize, at: DispatchTarget, agent: Option<String>, prompt: Option<String>, cont: bool },
    Tell { i: usize, agent: String, text: String },
    Halt { i: usize, agent: String },
    Fire { i: usize, agent: String },
    /// the commander looked at base i: its delta is consumed
    Visit { i: usize },
    /// save the space now
    Save,
    /// save and exit the daemon
    Shutdown,
}

pub fn urldecode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(c) => {
                        out.push(c);
                        i += 3;
                        continue;
                    }
                    None => out.push(b[i]),
                }
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

pub fn params(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (urldecode(k), urldecode(v))
        })
        .collect()
}

/// HTTP path + query → command. The endpoints:
///
///   /state                          → world state as JSON
///   /place?x=..&y=..&name=..        → establish a base centered at world x,y
///   /destroy?i=0                    → destroy base i (archives its record; no confirm)
///   /link?a=0&b=1                   → toggle a link between two bases
///   /unlink?li=0                    → sever link li
///   /decide?d=0&o=1                 → commit option o of decision d
///   /capture?s=..[&x=..&y=..]       → drop a capture note
///   /file_capture?cap=0&proj=1      → file capture 0 into base 1
///   /discard_capture?cap=0          → drop capture 0
///   /module_add?i=0&name=..&path=.. → install a wasm program in base i
///       [&interval=60&fuel=50000000&http=4&http_kib=256]
///   /module_rm?i=0&name=..          → uninstall a wasm program
///   /module_toggle?i=0&name=..      → enable/disable a wasm program
///   /cfg[?struct_scale=1.5][&show_rail=1]
///                                   → display prefs persisted with the space
///   /pylon?i=0&title=..[&x=..&y=..][&state=todo|doing|done|blocked][&notes=..][&effort=..]
///                                   → upsert a goal pylon on base i (title = short
///                                     name shown on the map; notes = the body;
///                                     effort = minimal|low|medium|high|xhigh for
///                                     units sent here, empty = base default)
///   /question?i=0&text=..[&x=..&y=..][&resolved=1][&notes=..][&effort=..]
///                                   → upsert a question sensor array on base i
///   /struct?i=0&pylon=2|question=1[&state=..][&resolved=1][&notes=..][&effort=..][&x=..&y=..][&read=1]
///                                   → edit one substructure by index
///   /remove_structs?s=0p3,0q1,1p0   → demolish substructures (base index, p|q, index)
///   /move?i=0&x=..&y=..             → move base i (top-left, world coords)
///   /move_capture?ci=0&x=..&y=..    → move capture ci
///   /base?i=0[&cwd=/repo][&sandbox=workspace-write][&model=..][&effort=..]
///                                   → set the repo / codex options units of base i use
///   /dispatch?i=0&title=..|pylon=2|question=1[&agent=cx-1][&prompt=..]
///                                   → send a unit (idle one, or a new one) to work
///                                     the pylon (pylon → doing) or scout the sensor
///                                     array (read-only by default; DONE resolves it)
///   /continue?i=0&title=..[&s=..]   → same, with the last report folded in
///   /tell?i=0&agent=cx-1&s=..       → follow-up order: resume the unit's codex thread
///   /halt?i=0&agent=cx-1            → kill the unit's running codex turn
///   /fire?i=0&agent=cx-1            → remove the unit from the garrison
///   /visit?i=0                      → the commander looked at base i
///   /save                           → write the space file now
///   /shutdown                       → save and stop the daemon
pub fn parse(path: &str, query: &str) -> Result<Cmd, &'static str> {
    let q = params(query);
    let f32p = |k: &str| q.get(k).and_then(|v| v.parse::<f32>().ok());
    let usizep = |k: &str| q.get(k).and_then(|v| v.parse::<usize>().ok());
    let boolp = |k: &str| matches!(q.get(k).map(|s| s.as_str()), Some("1") | Some("true"));
    let posp = || match (f32p("x"), f32p("y")) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };
    let target = || match (usizep("pylon"), usizep("question")) {
        (Some(ti), _) => Some(Target::Pylon(ti)),
        (None, Some(qi)) => Some(Target::Question(qi)),
        _ => None,
    };
    match path {
        "/state" => Ok(Cmd::State),
        "/save" => Ok(Cmd::Save),
        "/shutdown" => Ok(Cmd::Shutdown),
        "/place" => match (f32p("x"), f32p("y"), q.get("name")) {
            (Some(x), Some(y), Some(n)) => Ok(Cmd::Place { x, y, name: n.clone() }),
            _ => Err("missing x/y/name"),
        },
        "/destroy" => usizep("i").map(|i| Cmd::Destroy { i }).ok_or("missing i"),
        "/link" => match (usizep("a"), usizep("b")) {
            (Some(a), Some(b)) => Ok(Cmd::Link { a, b }),
            _ => Err("missing a/b"),
        },
        "/unlink" => usizep("li").map(|li| Cmd::Unlink { li }).ok_or("missing li"),
        "/decide" => match (usizep("d"), usizep("o")) {
            (Some(d), Some(o)) => Ok(Cmd::Decide { d, o }),
            _ => Err("missing d/o"),
        },
        "/capture" => q.get("s").map(|s| Cmd::Capture { text: s.clone(), pos: posp() }).ok_or("missing s"),
        "/file_capture" => match (usizep("cap"), usizep("proj")) {
            (Some(cap), Some(proj)) => Ok(Cmd::FileCapture { cap, proj }),
            _ => Err("missing cap/proj"),
        },
        "/discard_capture" => usizep("cap").map(|cap| Cmd::DiscardCapture { cap }).ok_or("missing cap"),
        "/module_add" => match (usizep("i"), q.get("name"), q.get("path")) {
            (Some(i), Some(name), Some(path)) => {
                let mut cfg = ModuleCfg::new(name.clone(), path.clone());
                if let Some(v) = f32p("interval") {
                    cfg.interval_sec = v as f64;
                }
                if let Some(v) = q.get("fuel").and_then(|v| v.parse::<u64>().ok()) {
                    cfg.fuel_per_tick = v;
                }
                if let Some(v) = q.get("http").and_then(|v| v.parse::<u32>().ok()) {
                    cfg.max_http_per_tick = v;
                }
                if let Some(v) = q.get("http_kib").and_then(|v| v.parse::<u32>().ok()) {
                    cfg.max_http_resp_kib = v;
                }
                Ok(Cmd::ModuleAdd { i, cfg })
            }
            _ => Err("missing i/name/path"),
        },
        "/module_rm" => match (usizep("i"), q.get("name")) {
            (Some(i), Some(n)) => Ok(Cmd::ModuleRm { i, name: n.clone() }),
            _ => Err("missing i/name"),
        },
        "/module_toggle" => match (usizep("i"), q.get("name")) {
            (Some(i), Some(n)) => Ok(Cmd::ModuleToggle { i, name: n.clone() }),
            _ => Err("missing i/name"),
        },
        "/cfg" => Ok(Cmd::Cfg { struct_scale: f32p("struct_scale"), show_rail: q.get("show_rail").map(|_| boolp("show_rail")) }),
        "/pylon" => match (usizep("i"), q.get("title")) {
            (Some(i), Some(title)) => Ok(Cmd::Pylon {
                i,
                title: title.clone(),
                pos: posp(),
                state: q.get("state").cloned(),
                notes: q.get("notes").cloned(),
                effort: q.get("effort").cloned(),
            }),
            _ => Err("missing i/title"),
        },
        "/question" => match (usizep("i"), q.get("text")) {
            (Some(i), Some(text)) => Ok(Cmd::Question {
                i,
                text: text.clone(),
                pos: posp(),
                resolved: q.get("resolved").map(|_| boolp("resolved")),
                notes: q.get("notes").cloned(),
                effort: q.get("effort").cloned(),
            }),
            _ => Err("missing i/text"),
        },
        "/struct" => match (usizep("i"), target()) {
            (Some(i), Some(at)) => {
                // effort: absent → None; "" → Some(None) (clear); level → Some(Some(level))
                let effort = match q.get("effort") {
                    None => None,
                    Some(e) => Some(parse_effort(e).ok_or("bad effort")?),
                };
                let state = match q.get("state") {
                    None => None,
                    Some(s) => Some(TaskState::parse(s).ok_or("bad state")?),
                };
                Ok(Cmd::Struct {
                    i,
                    at,
                    state,
                    resolved: q.get("resolved").map(|_| boolp("resolved")),
                    notes: q.get("notes").cloned(),
                    effort,
                    pos: posp(),
                    read: boolp("read"),
                })
            }
            _ => Err("missing i and pylon|question"),
        },
        "/remove_structs" => {
            let mut list = vec![];
            for item in q.get("s").map(|s| s.as_str()).unwrap_or("").split(',').filter(|s| !s.is_empty()) {
                let (kind_pos, _) = item.char_indices().find(|(_, c)| *c == 'p' || *c == 'q').ok_or("bad s item")?;
                let i: usize = item[..kind_pos].parse().map_err(|_| "bad s item")?;
                let idx: usize = item[kind_pos + 1..].parse().map_err(|_| "bad s item")?;
                let at = if &item[kind_pos..kind_pos + 1] == "p" { Target::Pylon(idx) } else { Target::Question(idx) };
                list.push((i, at));
            }
            if list.is_empty() {
                return Err("missing s");
            }
            Ok(Cmd::RemoveStructs { list })
        }
        "/move" => match (usizep("i"), f32p("x"), f32p("y")) {
            (Some(i), Some(x), Some(y)) => Ok(Cmd::MoveBase { i, x, y }),
            _ => Err("missing i/x/y"),
        },
        "/move_capture" => match (usizep("ci"), f32p("x"), f32p("y")) {
            (Some(ci), Some(x), Some(y)) => Ok(Cmd::MoveCapture { ci, x, y }),
            _ => Err("missing ci/x/y"),
        },
        "/base" => usizep("i")
            .map(|i| Cmd::Base {
                i,
                cwd: q.get("cwd").cloned(),
                sandbox: q.get("sandbox").cloned(),
                model: q.get("model").cloned(),
                effort: q.get("effort").cloned(),
            })
            .ok_or("missing i"),
        "/dispatch" | "/continue" => {
            let i = usizep("i").ok_or("missing i")?;
            let at = match (q.get("title"), target()) {
                (Some(t), _) => DispatchTarget::Title(t.clone()),
                (None, Some(t)) => DispatchTarget::At(t),
                _ => return Err("missing title|pylon|question"),
            };
            Ok(Cmd::Dispatch {
                i,
                at,
                agent: q.get("agent").cloned(),
                // /continue?s= is the commander's note riding along with the last report
                prompt: q.get("prompt").or_else(|| q.get("s")).cloned(),
                cont: path == "/continue",
            })
        }
        "/tell" => match (usizep("i"), q.get("agent"), q.get("s")) {
            (Some(i), Some(agent), Some(s)) => Ok(Cmd::Tell { i, agent: agent.clone(), text: s.clone() }),
            _ => Err("missing i/agent/s"),
        },
        "/halt" => match (usizep("i"), q.get("agent")) {
            (Some(i), Some(agent)) => Ok(Cmd::Halt { i, agent: agent.clone() }),
            _ => Err("missing i/agent"),
        },
        "/fire" => match (usizep("i"), q.get("agent")) {
            (Some(i), Some(agent)) => Ok(Cmd::Fire { i, agent: agent.clone() }),
            _ => Err("missing i/agent"),
        },
        "/visit" => usizep("i").map(|i| Cmd::Visit { i }).ok_or("missing i"),
        _ => Err("unknown endpoint"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_roundtrip() {
        let s = "a b&c=d/é";
        assert_eq!(urldecode(&urlencode(s)), s);
    }

    #[test]
    fn parses_struct_and_remove() {
        match parse("/struct", "i=1&pylon=2&state=done&effort=&read=1").unwrap() {
            Cmd::Struct { i, at, state, effort, read, .. } => {
                assert_eq!((i, at, state, read), (1, Target::Pylon(2), Some(TaskState::Done), true));
                assert_eq!(effort, Some(None));
            }
            c => panic!("{:?}", c),
        }
        match parse("/struct", "i=1&question=0&effort=high").unwrap() {
            Cmd::Struct { effort, .. } => assert_eq!(effort, Some(Some("high".into()))),
            c => panic!("{:?}", c),
        }
        match parse("/remove_structs", "s=0p3,1q0").unwrap() {
            Cmd::RemoveStructs { list } => assert_eq!(list, vec![(0, Target::Pylon(3)), (1, Target::Question(0))]),
            c => panic!("{:?}", c),
        }
        assert!(parse("/nope", "").is_err());
    }
}
