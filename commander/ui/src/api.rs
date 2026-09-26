//! The frontend's line to the daemon.
//!
//! Two background threads: one long-polls `/snapshot` and hands every new
//! world version to the UI thread, one sends the UI's commands (`Req`) in
//! order and hands the replies back. The UI never blocks on the network — it
//! renders its mirror of the last snapshot, applies its own edits on top
//! optimistically, and lets the next snapshot settle them.

use commander_core::model::TaskState;
use commander_core::proto::{urlencode, Snapshot, Target};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

/// a command the UI sends; mirrors the daemon endpoints it uses
#[derive(Clone, Debug)]
pub enum Req {
    Place { x: f32, y: f32, name: String },
    Destroy { i: usize },
    Link { a: usize, b: usize },
    Unlink { li: usize },
    Decide { d: usize, o: usize },
    Capture { text: String, x: f32, y: f32 },
    FileCapture { cap: usize, proj: usize },
    DiscardCapture { cap: usize },
    ModuleRm { i: usize, name: String },
    ModuleToggle { i: usize, name: String },
    Cfg { struct_scale: Option<f32>, show_rail: Option<bool> },
    Pylon { i: usize, title: String, x: f32, y: f32 },
    Question { i: usize, text: String, x: f32, y: f32 },
    Struct {
        i: usize,
        at: Target,
        state: Option<TaskState>,
        resolved: Option<bool>,
        notes: Option<String>,
        effort: Option<Option<String>>,
        pos: Option<(f32, f32)>,
        read: bool,
    },
    RemoveStructs { list: Vec<(usize, Target)> },
    MoveBase { i: usize, x: f32, y: f32 },
    MoveCapture { ci: usize, x: f32, y: f32 },
    Dispatch { i: usize, at: Target, cont: bool },
    Halt { i: usize, agent: String },
    Visit { i: usize },
}

impl Req {
    /// a bare edit of a substructure
    pub fn edit(i: usize, at: Target) -> Req {
        Req::Struct { i, at, state: None, resolved: None, notes: None, effort: None, pos: None, read: false }
    }

    pub fn state(mut self, st: TaskState) -> Req {
        if let Req::Struct { state, .. } = &mut self {
            *state = Some(st);
        }
        self
    }
    pub fn resolved(mut self, r: bool) -> Req {
        if let Req::Struct { resolved, .. } = &mut self {
            *resolved = Some(r);
        }
        self
    }
    pub fn notes(mut self, n: String) -> Req {
        if let Req::Struct { notes, .. } = &mut self {
            *notes = Some(n);
        }
        self
    }
    pub fn effort(mut self, e: Option<String>) -> Req {
        if let Req::Struct { effort, .. } = &mut self {
            *effort = Some(e);
        }
        self
    }
    pub fn pos(mut self, x: f32, y: f32) -> Req {
        if let Req::Struct { pos, .. } = &mut self {
            *pos = Some((x, y));
        }
        self
    }
    pub fn read(mut self) -> Req {
        if let Req::Struct { read, .. } = &mut self {
            *read = true;
        }
        self
    }

    /// path + query, relative to the daemon base url
    pub fn url(&self) -> String {
        fn target(at: Target) -> String {
            match at {
                Target::Pylon(ti) => format!("pylon={}", ti),
                Target::Question(qi) => format!("question={}", qi),
            }
        }
        match self {
            Req::Place { x, y, name } => format!("/place?x={}&y={}&name={}", x, y, urlencode(name)),
            Req::Destroy { i } => format!("/destroy?i={}", i),
            Req::Link { a, b } => format!("/link?a={}&b={}", a, b),
            Req::Unlink { li } => format!("/unlink?li={}", li),
            Req::Decide { d, o } => format!("/decide?d={}&o={}", d, o),
            Req::Capture { text, x, y } => format!("/capture?s={}&x={}&y={}", urlencode(text), x, y),
            Req::FileCapture { cap, proj } => format!("/file_capture?cap={}&proj={}", cap, proj),
            Req::DiscardCapture { cap } => format!("/discard_capture?cap={}", cap),
            Req::ModuleRm { i, name } => format!("/module_rm?i={}&name={}", i, urlencode(name)),
            Req::ModuleToggle { i, name } => format!("/module_toggle?i={}&name={}", i, urlencode(name)),
            Req::Cfg { struct_scale, show_rail } => {
                let mut q = vec![];
                if let Some(s) = struct_scale {
                    q.push(format!("struct_scale={}", s));
                }
                if let Some(r) = show_rail {
                    q.push(format!("show_rail={}", if *r { 1 } else { 0 }));
                }
                format!("/cfg?{}", q.join("&"))
            }
            Req::Pylon { i, title, x, y } => format!("/pylon?i={}&title={}&x={}&y={}", i, urlencode(title), x, y),
            Req::Question { i, text, x, y } => format!("/question?i={}&text={}&x={}&y={}", i, urlencode(text), x, y),
            Req::Struct { i, at, state, resolved, notes, effort, pos, read } => {
                let mut q = vec![format!("i={}", i), target(*at)];
                if let Some(s) = state {
                    q.push(format!("state={}", s.label()));
                }
                if let Some(r) = resolved {
                    q.push(format!("resolved={}", if *r { 1 } else { 0 }));
                }
                if let Some(n) = notes {
                    q.push(format!("notes={}", urlencode(n)));
                }
                if let Some(e) = effort {
                    q.push(format!("effort={}", urlencode(e.as_deref().unwrap_or(""))));
                }
                if let Some((x, y)) = pos {
                    q.push(format!("x={}&y={}", x, y));
                }
                if *read {
                    q.push("read=1".into());
                }
                format!("/struct?{}", q.join("&"))
            }
            Req::RemoveStructs { list } => {
                let items: Vec<String> = list
                    .iter()
                    .map(|(i, at)| match at {
                        Target::Pylon(ti) => format!("{}p{}", i, ti),
                        Target::Question(qi) => format!("{}q{}", i, qi),
                    })
                    .collect();
                format!("/remove_structs?s={}", items.join(","))
            }
            Req::MoveBase { i, x, y } => format!("/move?i={}&x={}&y={}", i, x, y),
            Req::MoveCapture { ci, x, y } => format!("/move_capture?ci={}&x={}&y={}", ci, x, y),
            Req::Dispatch { i, at, cont } => format!("/{}?i={}&{}", if *cont { "continue" } else { "dispatch" }, i, target(*at)),
            Req::Halt { i, agent } => format!("/halt?i={}&agent={}", i, urlencode(agent)),
            Req::Visit { i } => format!("/visit?i={}", i),
        }
    }
}

pub struct Reply {
    pub req: Req,
    pub result: Result<Value, String>,
}

/// GET base+path; an {"err":..} body (any status) is the Err
pub fn get(base: &str, path: &str, timeout: Duration) -> Result<Value, String> {
    let url = format!("{}{}", base, path);
    let body = match ureq::get(&url).timeout(timeout).call() {
        Ok(r) => r.into_string().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(_, r)) => r.into_string().map_err(|e| e.to_string())?,
        Err(e) => return Err(e.to_string()),
    };
    let v: Value = serde_json::from_str(&body).map_err(|e| format!("bad reply: {} ({})", e, body.chars().take(80).collect::<String>()))?;
    match v.get("err").and_then(|e| e.as_str()) {
        Some(e) => Err(e.to_string()),
        None => Ok(v),
    }
}

pub struct Client {
    base: String,
    tx: Sender<Req>,
    replies: Receiver<Reply>,
    snaps: Receiver<Snapshot>,
    online: Arc<AtomicBool>,
}

impl Client {
    /// attach to the daemon at `base` (e.g. http://127.0.0.1:7700)
    pub fn connect(base: String) -> Client {
        let online = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel::<Req>();
        let (rtx, replies) = channel::<Reply>();
        let (stx, snaps) = channel::<Snapshot>();
        {
            let base = base.clone();
            std::thread::spawn(move || {
                for req in rx {
                    let result = get(&base, &req.url(), Duration::from_secs(15));
                    if rtx.send(Reply { req, result }).is_err() {
                        return;
                    }
                }
            });
        }
        {
            let base = base.clone();
            let online = online.clone();
            std::thread::spawn(move || {
                let agent = ureq::AgentBuilder::new().timeout_read(Duration::from_secs(40)).timeout_connect(Duration::from_secs(3)).build();
                let mut version = 0u64;
                loop {
                    let url = format!("{}/snapshot?since={}", base, version);
                    match agent.get(&url).call() {
                        Ok(r) => match r.into_string().map_err(|e| e.to_string()).and_then(|b| serde_json::from_str::<Snapshot>(&b).map_err(|e| e.to_string())) {
                            Ok(s) => {
                                online.store(true, Ordering::Relaxed);
                                if s.version > version {
                                    version = s.version;
                                    if stx.send(s).is_err() {
                                        return;
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("snapshot parse failed: {}", e);
                                std::thread::sleep(Duration::from_secs(1));
                            }
                        },
                        Err(e) => {
                            if online.swap(false, Ordering::Relaxed) {
                                eprintln!("daemon unreachable: {}", e);
                            }
                            // a restarted daemon starts its versions over
                            version = 0;
                            std::thread::sleep(Duration::from_secs(1));
                        }
                    }
                }
            });
        }
        Client { base, tx, replies, snaps, online }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    pub fn send(&self, req: Req) {
        let _ = self.tx.send(req);
    }

    /// send and wait — for the last words before exit
    pub fn send_sync(&self, req: Req) -> Result<Value, String> {
        get(&self.base, &req.url(), Duration::from_secs(5))
    }

    pub fn replies(&self) -> Vec<Reply> {
        let mut out = vec![];
        while let Ok(r) = self.replies.try_recv() {
            out.push(r);
        }
        out
    }

    /// the newest snapshot since the last call, if any
    pub fn snapshot(&self) -> Option<Snapshot> {
        let mut latest = None;
        while let Ok(s) = self.snaps.try_recv() {
            latest = Some(s);
        }
        latest
    }
}

/// make sure a daemon answers at `base`: if none does, start `commanderd`
/// (next to this executable, or on PATH) detached, so it outlives this
/// frontend, and wait for it to come up. returns whether one is reachable.
pub fn ensure_daemon(base: &str, spawn: bool) -> bool {
    let ping = || get(base, "/state", Duration::from_secs(2)).is_ok();
    if ping() {
        return true;
    }
    if !spawn {
        return false;
    }
    let Some(addr) = base.strip_prefix("http://") else { return false };
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("commanderd")));
    let bin = match exe {
        Some(p) if p.is_file() => p,
        _ => std::path::PathBuf::from("commanderd"),
    };
    eprintln!("no daemon at {} — starting {}", base, bin.display());
    let mut cmd = std::process::Command::new(&bin);
    cmd.env("COMMANDER_HTTP", addr)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    // own session: closing this frontend must not take the daemon down
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    if let Err(e) = cmd.spawn() {
        eprintln!("could not start {}: {}", bin.display(), e);
        return false;
    }
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));
        if ping() {
            return true;
        }
    }
    false
}
