//! commanderd — the Commander backend.
//!
//! Owns one space (COMMANDER_SPACE, default space.jsonl in the working
//! directory): the world, its wasm building programs and its codex units. Any
//! number of frontends attach over HTTP (COMMANDER_HTTP, default
//! 127.0.0.1:7700):
//!
//!   GET /snapshot[?since=N]   long-poll: returns the world once its version
//!                             passes N (or after ~25s, unchanged)
//!   GET /<command>?...        every command in commander_core::proto::parse;
//!                             the engine answers within one tick
//!
//! The engine runs on the main thread; each request gets its own thread that
//! either waits on the published snapshot or hands a command to the engine and
//! waits for the reply.

use commander_core::engine::Engine;
use commander_core::proto::{self, Cmd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

struct Req {
    cmd: Cmd,
    reply: Sender<String>,
}

/// the latest snapshot, versioned; waiters block on the condvar
struct Published {
    cur: Mutex<(u64, Arc<String>)>,
    cv: Condvar,
}

impl Published {
    /// block until version > since (or timeout), then return (version, json)
    fn wait(&self, since: u64, timeout: Duration) -> (u64, Arc<String>) {
        let deadline = Instant::now() + timeout;
        let mut g = self.cur.lock().unwrap();
        while g.0 <= since {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let (ng, _) = self.cv.wait_timeout(g, deadline - now).unwrap();
            g = ng;
        }
        (g.0, g.1.clone())
    }
    fn publish(&self, version: u64, json: String) {
        *self.cur.lock().unwrap() = (version, Arc::new(json));
        self.cv.notify_all();
    }
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn serve(addr: String, tx: Sender<Req>, pub_: Arc<Published>) {
    let server = match tiny_http::Server::http(addr.as_str()) {
        Ok(s) => {
            eprintln!("commanderd listening on http://{}", addr);
            s
        }
        Err(e) => {
            eprintln!("commanderd failed to bind {}: {}", addr, e);
            std::process::exit(1);
        }
    };
    for request in server.incoming_requests() {
        let tx = tx.clone();
        let pub_ = pub_.clone();
        std::thread::spawn(move || {
            let url = request.url().to_string();
            let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
            let json = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
            if path == "/snapshot" {
                let since = proto::params(query).get("since").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
                let (_, body) = pub_.wait(since, Duration::from_secs(25));
                let _ = request.respond(tiny_http::Response::from_string(body.as_str()).with_header(json));
                return;
            }
            match proto::parse(path, query) {
                Ok(cmd) => {
                    let (rtx, rrx) = channel();
                    if tx.send(Req { cmd, reply: rtx }).is_err() {
                        let _ = request.respond(tiny_http::Response::from_string("{\"err\":\"engine gone\"}").with_status_code(503).with_header(json));
                        return;
                    }
                    let body = rrx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| "{\"err\":\"engine timeout\"}".to_string());
                    let status = if body.starts_with("{\"err\"") { 400 } else { 200 };
                    let _ = request.respond(tiny_http::Response::from_string(body).with_status_code(status).with_header(json));
                }
                Err(e) => {
                    let _ = request.respond(
                        tiny_http::Response::from_string(format!("{{\"err\":\"{}\"}}", e)).with_status_code(400).with_header(json),
                    );
                }
            }
        });
    }
}

fn main() {
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as usize);
        libc::signal(libc::SIGTERM, on_signal as *const () as usize);
        // a frontend that spawned us and died must not take us down
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    let addr = std::env::var("COMMANDER_HTTP").unwrap_or_else(|_| "127.0.0.1:7700".into());
    let mut engine = Engine::open();
    let pub_ = Arc::new(Published { cur: Mutex::new((0, Arc::new(String::new()))), cv: Condvar::new() });
    let (tx, rx): (Sender<Req>, Receiver<Req>) = channel();
    {
        let pub_ = pub_.clone();
        std::thread::spawn(move || serve(addr, tx, pub_));
    }
    let mut version = 0u64;
    loop {
        // commands: block briefly for the first, then drain the rest. replies
        // wait until the snapshot that carries their effect is published and
        // name its version ("v"), so a frontend knows when its own edit has
        // landed and can drop its optimistic copy.
        let mut replies: Vec<(Sender<String>, serde_json::Value)> = vec![];
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(req) => {
                replies.push((req.reply, engine.handle(req.cmd)));
                while let Ok(req) = rx.try_recv() {
                    replies.push((req.reply, engine.handle(req.cmd)));
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if engine.tick() {
            version += 1;
            match serde_json::to_string(&engine.snapshot(version)) {
                Ok(json) => pub_.publish(version, json),
                Err(e) => eprintln!("snapshot serialize failed: {}", e),
            }
        }
        for (reply, mut body) in replies {
            if let Some(o) = body.as_object_mut() {
                o.insert("v".into(), serde_json::json!(version));
            }
            let _ = reply.send(body.to_string());
        }
        if engine.quitting() || STOP.load(Ordering::SeqCst) {
            break;
        }
    }
    engine.shutdown();
    eprintln!("commanderd: space saved, bye");
}
