//! HTTP control API of the frontend, for test automation.
//!
//! Listens on COMMANDER_UI_HTTP (default 127.0.0.1:7701). Input injection is
//! answered by the app thread within one frame; everything else is forwarded
//! to the daemon, so a test script can point at this one port:
//!
//!   /key?k=l[&ctrl=1]               → inject a key press (egui key names)
//!   /text?s=albion                  → inject text into the focused field
//!   /click?x=..&y=..[&double=1][&world=1]
//!                                   → synthetic canvas click (screen px, or
//!                                     world coords with world=1)
//!   /band?x1=..&y1=..&x2=..&y2=..[&world=1]
//!                                   → rectangle group-select substructures
//!   /state                          → the daemon's world state plus this
//!                                     frontend's view (sel, cam, sroom, …)
//!   /<anything else>                → forwarded to the daemon verbatim

use commander_core::proto::params;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

pub enum UiCmd {
    /// `ctrl` marks the key as a ctrl/command chord (e.g. ctrl+v = paste)
    Key { name: String, ctrl: bool },
    Text(String),
    Click { x: f32, y: f32, double: bool, world: bool },
    Band { x1: f32, y1: f32, x2: f32, y2: f32, world: bool },
    /// this frontend's view state as JSON
    View,
}

pub struct CtrlReq {
    pub cmd: UiCmd,
    pub reply: Sender<String>,
}

pub fn spawn(addr: String, backend: String) -> Receiver<CtrlReq> {
    let (tx, rx) = channel::<CtrlReq>();
    std::thread::spawn(move || {
        let server = match tiny_http::Server::http(addr.as_str()) {
            Ok(s) => {
                eprintln!("ui control api listening on http://{} (forwarding to {})", addr, backend);
                s
            }
            Err(e) => {
                eprintln!("ui control api failed to bind {}: {}", addr, e);
                return;
            }
        };
        let ask = |tx: &Sender<CtrlReq>, cmd: UiCmd| -> Option<String> {
            let (rtx, rrx) = channel();
            tx.send(CtrlReq { cmd, reply: rtx }).ok()?;
            Some(rrx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| "{\"err\":\"app timeout\"}".to_string()))
        };
        for request in server.incoming_requests() {
            let url = request.url().to_string();
            let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
            let q = params(query);
            let f32p = |k: &str| q.get(k).and_then(|v| v.parse::<f32>().ok());
            let boolp = |k: &str| matches!(q.get(k).map(|s| s.as_str()), Some("1") | Some("true"));
            let json = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
            let cmd = match path {
                "/key" => q.get("k").map(|k| UiCmd::Key { name: k.clone(), ctrl: boolp("ctrl") }).ok_or("missing k"),
                "/text" => q.get("s").map(|s| UiCmd::Text(s.clone())).ok_or("missing s"),
                "/click" => match (f32p("x"), f32p("y")) {
                    (Some(x), Some(y)) => Ok(UiCmd::Click { x, y, double: boolp("double"), world: boolp("world") }),
                    _ => Err("missing x/y"),
                },
                "/band" => match (f32p("x1"), f32p("y1"), f32p("x2"), f32p("y2")) {
                    (Some(x1), Some(y1), Some(x2), Some(y2)) => Ok(UiCmd::Band { x1, y1, x2, y2, world: boolp("world") }),
                    _ => Err("missing x1/y1/x2/y2"),
                },
                "/state" => {
                    // the daemon's document with this frontend's view merged in
                    let mut doc = match crate::api::get(&backend, "/state", Duration::from_secs(5)) {
                        Ok(v) => v,
                        Err(e) => serde_json::json!({ "err": format!("daemon: {}", e) }),
                    };
                    let view = ask(&tx, UiCmd::View).unwrap_or_else(|| "{}".into());
                    if let (Some(d), Ok(serde_json::Value::Object(v))) = (doc.as_object_mut(), serde_json::from_str::<serde_json::Value>(&view)) {
                        for (k, val) in v {
                            d.insert(k, val);
                        }
                    }
                    let _ = request.respond(tiny_http::Response::from_string(doc.to_string()).with_header(json));
                    continue;
                }
                _ => {
                    // not ours: hand it to the daemon
                    let target = format!("{}{}", backend, url);
                    let (status, body) = match ureq::get(&target).timeout(Duration::from_secs(30)).call() {
                        Ok(r) => (r.status(), r.into_string().unwrap_or_default()),
                        Err(ureq::Error::Status(code, r)) => (code, r.into_string().unwrap_or_default()),
                        Err(e) => (502, format!("{{\"err\":\"daemon unreachable: {}\"}}", e.to_string().replace('"', "'"))),
                    };
                    let _ = request.respond(tiny_http::Response::from_string(body).with_status_code(status).with_header(json));
                    continue;
                }
            };
            match cmd {
                Ok(cmd) => match ask(&tx, cmd) {
                    Some(body) => {
                        let _ = request.respond(tiny_http::Response::from_string(body).with_header(json));
                    }
                    None => break,
                },
                Err(e) => {
                    let _ = request.respond(
                        tiny_http::Response::from_string(format!("{{\"err\":\"{}\"}}", e)).with_status_code(400).with_header(json),
                    );
                }
            }
        }
    });
    rx
}
