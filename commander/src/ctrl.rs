//! HTTP control API for test automation.
//!
//! Runs a tiny HTTP server on COMMANDER_HTTP (default 127.0.0.1:7700) and
//! forwards commands to the app thread, which answers within one frame.
//!
//! Endpoints (GET or POST, query-string parameters):
//!   /state                          → world state as JSON
//!   /key?k=l                        → inject a key press (egui key names)
//!   /text?s=albion                  → inject text into the focused field
//!   /click?x=..&y=..[&double=1][&world=1]
//!                                   → synthetic canvas click (screen px, or
//!                                     world coords with world=1)
//!   /place?x=..&y=..&name=..        → establish a base centered at world x,y
//!   /destroy?i=0                    → destroy base i (archives its record; no confirm)
//!   /link?a=0&b=1                   → toggle a link between two bases
//!   /decide?d=0&o=1                 → commit option o of decision d
//!   /capture?s=..                   → drop a capture note
//!   /module_add?i=0&name=..&path=.. → install a wasm program in base i
//!       [&interval=60&fuel=50000000&http=4&http_kib=256]
//!   /module_rm?i=0&name=..          → uninstall a wasm program
//!   /module_toggle?i=0&name=..      → enable/disable a wasm program
//!   /pylon?i=0&title=..[&x=..&y=..][&state=todo|doing|done|blocked][&notes=..]
//!                                   → upsert a goal pylon on base i (title = short
//!                                     name shown on the map; notes = the body)
//!   /question?i=0&text=..[&x=..&y=..][&resolved=1][&notes=..]
//!                                   → upsert a question sensor array on base i
//!   /cfg?struct_scale=1.5           → set the substructure size ratio
//!   /band?x1=..&y1=..&x2=..&y2=..[&world=1]
//!                                   → rectangle group-select substructures
//!   /base?i=0[&cwd=/repo][&sandbox=workspace-write][&model=..]
//!                                   → set the repo / codex options units of base i use
//!   /dispatch?i=0&title=..[&agent=cx-1][&prompt=..]
//!                                   → send a unit (idle one, or a new one) to work
//!                                     the pylon `title` via codex; pylon → doing
//!   /tell?i=0&agent=cx-1&s=..       → follow-up order: resume the unit's codex thread
//!   /halt?i=0&agent=cx-1            → kill the unit's running codex turn
//!   /fire?i=0&agent=cx-1            → remove the unit from the garrison

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

pub enum Cmd {
    /// `ctrl` marks the key as a ctrl/command chord (e.g. ctrl+v = paste)
    Key { name: String, ctrl: bool },
    Text(String),
    Click { x: f32, y: f32, double: bool, world: bool, ui: bool },
    Place { x: f32, y: f32, name: String },
    Destroy { i: usize },
    Link { a: usize, b: usize },
    Decide { d: usize, o: usize },
    Capture(String),
    ModuleAdd { i: usize, cfg: crate::model::ModuleCfg },
    ModuleRm { i: usize, name: String },
    ModuleToggle { i: usize, name: String },
    Cfg { struct_scale: Option<f32> },
    Band { x1: f32, y1: f32, x2: f32, y2: f32, world: bool },
    Pylon { i: usize, title: String, pos: Option<(f32, f32)>, state: Option<String>, notes: Option<String> },
    Question { i: usize, text: String, pos: Option<(f32, f32)>, resolved: Option<bool>, notes: Option<String> },
    Base { i: usize, cwd: Option<String>, sandbox: Option<String>, model: Option<String> },
    Dispatch { i: usize, title: String, agent: Option<String>, prompt: Option<String> },
    Tell { i: usize, agent: String, text: String },
    Halt { i: usize, agent: String },
    Fire { i: usize, agent: String },
    State,
}

pub struct CtrlReq {
    pub cmd: Cmd,
    pub reply: Sender<String>,
}

fn urldecode(s: &str) -> String {
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

fn params(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (urldecode(k), urldecode(v))
        })
        .collect()
}

pub fn spawn(addr: String) -> Receiver<CtrlReq> {
    let (tx, rx) = channel::<CtrlReq>();
    std::thread::spawn(move || {
        let server = match tiny_http::Server::http(addr.as_str()) {
            Ok(s) => {
                eprintln!("control api listening on http://{}", addr);
                s
            }
            Err(e) => {
                eprintln!("control api failed to bind {}: {}", addr, e);
                return;
            }
        };
        for request in server.incoming_requests() {
            let url = request.url().to_string();
            let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
            let q = params(query);
            let f32p = |k: &str| q.get(k).and_then(|v| v.parse::<f32>().ok());
            let usizep = |k: &str| q.get(k).and_then(|v| v.parse::<usize>().ok());
            let boolp = |k: &str| matches!(q.get(k).map(|s| s.as_str()), Some("1") | Some("true"));
            let cmd = match path {
                "/state" => Ok(Cmd::State),
                "/key" => q.get("k").map(|k| Cmd::Key { name: k.clone(), ctrl: boolp("ctrl") }).ok_or("missing k"),
                "/text" => q.get("s").map(|s| Cmd::Text(s.clone())).ok_or("missing s"),
                "/click" => match (f32p("x"), f32p("y")) {
                    (Some(x), Some(y)) => Ok(Cmd::Click { x, y, double: boolp("double"), world: boolp("world"), ui: boolp("ui") }),
                    _ => Err("missing x/y"),
                },
                "/place" => match (f32p("x"), f32p("y"), q.get("name")) {
                    (Some(x), Some(y), Some(n)) => Ok(Cmd::Place { x, y, name: n.clone() }),
                    _ => Err("missing x/y/name"),
                },
                "/destroy" => usizep("i").map(|i| Cmd::Destroy { i }).ok_or("missing i"),
                "/link" => match (usizep("a"), usizep("b")) {
                    (Some(a), Some(b)) => Ok(Cmd::Link { a, b }),
                    _ => Err("missing a/b"),
                },
                "/decide" => match (usizep("d"), usizep("o")) {
                    (Some(d), Some(o)) => Ok(Cmd::Decide { d, o }),
                    _ => Err("missing d/o"),
                },
                "/capture" => q.get("s").map(|s| Cmd::Capture(s.clone())).ok_or("missing s"),
                "/module_add" => match (usizep("i"), q.get("name"), q.get("path")) {
                    (Some(i), Some(name), Some(path)) => {
                        let mut cfg = crate::model::ModuleCfg::new(name.clone(), path.clone());
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
                "/cfg" => Ok(Cmd::Cfg { struct_scale: f32p("struct_scale") }),
                "/band" => match (f32p("x1"), f32p("y1"), f32p("x2"), f32p("y2")) {
                    (Some(x1), Some(y1), Some(x2), Some(y2)) => Ok(Cmd::Band { x1, y1, x2, y2, world: boolp("world") }),
                    _ => Err("missing x1/y1/x2/y2"),
                },
                "/pylon" => match (usizep("i"), q.get("title")) {
                    (Some(i), Some(title)) => Ok(Cmd::Pylon {
                        i,
                        title: title.clone(),
                        pos: match (f32p("x"), f32p("y")) {
                            (Some(x), Some(y)) => Some((x, y)),
                            _ => None,
                        },
                        state: q.get("state").cloned(),
                        notes: q.get("notes").cloned(),
                    }),
                    _ => Err("missing i/title"),
                },
                "/question" => match (usizep("i"), q.get("text")) {
                    (Some(i), Some(text)) => Ok(Cmd::Question {
                        i,
                        text: text.clone(),
                        pos: match (f32p("x"), f32p("y")) {
                            (Some(x), Some(y)) => Some((x, y)),
                            _ => None,
                        },
                        resolved: q.get("resolved").map(|v| v == "1" || v == "true"),
                        notes: q.get("notes").cloned(),
                    }),
                    _ => Err("missing i/text"),
                },
                "/base" => usizep("i")
                    .map(|i| Cmd::Base { i, cwd: q.get("cwd").cloned(), sandbox: q.get("sandbox").cloned(), model: q.get("model").cloned() })
                    .ok_or("missing i"),
                "/dispatch" => match (usizep("i"), q.get("title")) {
                    (Some(i), Some(title)) => {
                        Ok(Cmd::Dispatch { i, title: title.clone(), agent: q.get("agent").cloned(), prompt: q.get("prompt").cloned() })
                    }
                    _ => Err("missing i/title"),
                },
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
                _ => Err("unknown endpoint"),
            };
            let json = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
            match cmd {
                Ok(cmd) => {
                    let (rtx, rrx) = channel();
                    if tx.send(CtrlReq { cmd, reply: rtx }).is_err() {
                        break;
                    }
                    let body = rrx
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap_or_else(|_| "{\"err\":\"app timeout\"}".to_string());
                    let _ = request.respond(tiny_http::Response::from_string(body).with_header(json));
                }
                Err(e) => {
                    let _ = request.respond(
                        tiny_http::Response::from_string(format!("{{\"err\":\"{}\"}}", e))
                            .with_status_code(400)
                            .with_header(json),
                    );
                }
            }
        }
    });
    rx
}
