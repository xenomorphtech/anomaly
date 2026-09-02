//! Wasm module host — spacetimedb-style programs per building.
//!
//! Each building (project) can install any number of wasm modules. A module is
//! a .wasm/.wat file exporting `memory` and `tick() -> i32`; the host calls
//! `tick` on the module's interval with a fuel budget, and the module talks
//! back through host imports (module name "commander"):
//!
//!   log(ptr, len)                   debug line → stderr + status
//!   signal(ptr, len)                event into the building's feed (pings, deltas)
//!   reduce(ptr, len)                JSON reducer command applied to building state
//!   state() -> i64                  load building state JSON into the host scratch,
//!                                   returns its length
//!   http(mp, ml, up, ul, bp, bl) -> i64
//!                                   perform METHOD url [body]; scratch = response
//!                                   body; returns length, -1 on error, -2 when the
//!                                   tick's http budget is spent
//!   read(ptr, cap) -> i32           copy the scratch into guest memory
//!
//! Budgets are enforced per tick: wasmtime fuel (out-of-fuel traps the tick),
//! an http call counter, and a response size cap. Modules run on a dedicated
//! thread; the app thread syncs building snapshots in and drains outputs
//! (signals / reducer commands / run reports) back out. Edited module files
//! hot-reload on the next tick (mtime watch).

use crate::model::ModuleCfg;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant, SystemTime};

/// per-building snapshot pushed from the app thread
pub struct Building {
    pub name: String,
    pub state_json: String,
    pub modules: Vec<ModuleCfg>,
}

/// outputs flowing back to the app thread
pub enum Out {
    Signal { proj: String, module: String, text: String },
    Reduce { proj: String, module: String, cmd: serde_json::Value },
    Log { proj: String, module: String, text: String },
    Ran { proj: String, module: String, fuel_used: u64, http_used: u32, ms: f64, error: Option<String> },
}

/// last-run report the app keeps per (building, module) for the UI
#[derive(Clone, Default)]
pub struct ModStatus {
    pub ticks: u64,
    pub fuel_used: u64,
    pub http_used: u32,
    pub ms: f64,
    pub error: Option<String>,
    pub last_log: Option<String>,
}

pub struct Host {
    tx: Sender<Vec<Building>>,
    rx: Receiver<Out>,
}

impl Host {
    pub fn spawn() -> Host {
        let (tx, sync_rx) = channel::<Vec<Building>>();
        let (out_tx, rx) = channel::<Out>();
        std::thread::spawn(move || runtime(sync_rx, out_tx));
        Host { tx, rx }
    }

    pub fn sync(&self, buildings: Vec<Building>) {
        let _ = self.tx.send(buildings);
    }

    pub fn drain(&self) -> Vec<Out> {
        let mut out = vec![];
        while let Ok(o) = self.rx.try_recv() {
            out.push(o);
        }
        out
    }
}

// ---------- runtime thread ----------

/// per-instance host context (wasmtime store data)
struct Ctx {
    proj: String,
    module: String,
    state_json: String,
    scratch: Vec<u8>,
    http_used: u32,
    http_max: u32,
    http_resp_max: usize,
    outs: Vec<Out>,
}

const MAX_OUTS_PER_TICK: usize = 64;
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

struct Slot {
    store: wasmtime::Store<Ctx>,
    tick: wasmtime::TypedFunc<(), i32>,
    cfg: ModuleCfg,
    mtime: Option<SystemTime>,
    next_run: Instant,
}

/// a build/instantiate failure we won't retry until the file or config changes
struct Failed {
    cfg: ModuleCfg,
    mtime: Option<SystemTime>,
}

fn mtime(path: &str) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn mem_of(caller: &mut wasmtime::Caller<'_, Ctx>) -> Option<wasmtime::Memory> {
    caller.get_export("memory").and_then(|e| e.into_memory())
}

fn guest_bytes(caller: &mut wasmtime::Caller<'_, Ctx>, ptr: u32, len: u32) -> Option<Vec<u8>> {
    let mem = mem_of(caller)?;
    let data = mem.data(&caller);
    data.get(ptr as usize..(ptr as usize).checked_add(len as usize)?).map(|s| s.to_vec())
}

fn guest_str(caller: &mut wasmtime::Caller<'_, Ctx>, ptr: u32, len: u32) -> Option<String> {
    guest_bytes(caller, ptr, len).map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn push_out(ctx: &mut Ctx, out: Out) {
    if ctx.outs.len() < MAX_OUTS_PER_TICK {
        ctx.outs.push(out);
    }
}

fn linker(engine: &wasmtime::Engine) -> wasmtime::Result<wasmtime::Linker<Ctx>> {
    let mut l = wasmtime::Linker::<Ctx>::new(engine);

    l.func_wrap("commander", "log", |mut caller: wasmtime::Caller<'_, Ctx>, ptr: u32, len: u32| {
        if let Some(s) = guest_str(&mut caller, ptr, len) {
            let ctx = caller.data_mut();
            let (proj, module) = (ctx.proj.clone(), ctx.module.clone());
            push_out(ctx, Out::Log { proj, module, text: s });
        }
    })?;

    l.func_wrap("commander", "signal", |mut caller: wasmtime::Caller<'_, Ctx>, ptr: u32, len: u32| {
        if let Some(s) = guest_str(&mut caller, ptr, len) {
            let ctx = caller.data_mut();
            let (proj, module) = (ctx.proj.clone(), ctx.module.clone());
            push_out(ctx, Out::Signal { proj, module, text: s });
        }
    })?;

    l.func_wrap("commander", "reduce", |mut caller: wasmtime::Caller<'_, Ctx>, ptr: u32, len: u32| {
        if let Some(s) = guest_str(&mut caller, ptr, len) {
            let ctx = caller.data_mut();
            let (proj, module) = (ctx.proj.clone(), ctx.module.clone());
            match serde_json::from_str::<serde_json::Value>(&s) {
                Ok(cmd) => push_out(ctx, Out::Reduce { proj, module, cmd }),
                Err(e) => push_out(ctx, Out::Log { proj, module, text: format!("bad reduce json: {}", e) }),
            }
        }
    })?;

    l.func_wrap("commander", "state", |mut caller: wasmtime::Caller<'_, Ctx>| -> i64 {
        let ctx = caller.data_mut();
        ctx.scratch = ctx.state_json.clone().into_bytes();
        ctx.scratch.len() as i64
    })?;

    l.func_wrap(
        "commander",
        "http",
        |mut caller: wasmtime::Caller<'_, Ctx>, mp: u32, ml: u32, up: u32, ul: u32, bp: u32, bl: u32| -> i64 {
            let method = match guest_str(&mut caller, mp, ml) {
                Some(m) if !m.is_empty() => m,
                _ => "GET".into(),
            };
            let url = match guest_str(&mut caller, up, ul) {
                Some(u) => u,
                None => return -1,
            };
            let body = if bl > 0 { guest_bytes(&mut caller, bp, bl).unwrap_or_default() } else { vec![] };
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return -1;
            }
            {
                let ctx = caller.data_mut();
                if ctx.http_used >= ctx.http_max {
                    return -2;
                }
                ctx.http_used += 1;
            }
            let cap = caller.data().http_resp_max;
            let agent = ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).build();
            let req = agent.request(&method, &url);
            let resp = if body.is_empty() { req.call() } else { req.send_bytes(&body) };
            let resp = match resp {
                Ok(r) => r,
                // non-2xx still has a body worth reading
                Err(ureq::Error::Status(_, r)) => r,
                Err(_) => return -1,
            };
            let mut buf = Vec::new();
            use std::io::Read;
            if resp.into_reader().take(cap as u64).read_to_end(&mut buf).is_err() {
                return -1;
            }
            let len = buf.len() as i64;
            caller.data_mut().scratch = buf;
            len
        },
    )?;

    l.func_wrap("commander", "read", |mut caller: wasmtime::Caller<'_, Ctx>, ptr: u32, cap: u32| -> i32 {
        let scratch = caller.data().scratch.clone();
        let n = scratch.len().min(cap as usize);
        let mem = match mem_of(&mut caller) {
            Some(m) => m,
            None => return 0,
        };
        let data = mem.data_mut(&mut caller);
        match data.get_mut(ptr as usize..(ptr as usize).saturating_add(n)) {
            Some(dst) => {
                dst.copy_from_slice(&scratch[..n]);
                n as i32
            }
            None => 0,
        }
    })?;

    Ok(l)
}

fn build_slot(
    engine: &wasmtime::Engine,
    lnk: &wasmtime::Linker<Ctx>,
    proj: &str,
    cfg: &ModuleCfg,
) -> Result<Slot, String> {
    let module = wasmtime::Module::from_file(engine, &cfg.path).map_err(|e| format!("compile {}: {}", cfg.path, e))?;
    let ctx = Ctx {
        proj: proj.to_string(),
        module: cfg.name.clone(),
        state_json: String::new(),
        scratch: vec![],
        http_used: 0,
        http_max: cfg.max_http_per_tick,
        http_resp_max: cfg.max_http_resp_kib as usize * 1024,
        outs: vec![],
    };
    let mut store = wasmtime::Store::new(engine, ctx);
    store.set_fuel(cfg.fuel_per_tick).map_err(|e| e.to_string())?;
    let instance = lnk.instantiate(&mut store, &module).map_err(|e| format!("instantiate: {}", e))?;
    let tick = instance
        .get_typed_func::<(), i32>(&mut store, "tick")
        .map_err(|e| format!("missing export tick() -> i32: {}", e))?;
    Ok(Slot { store, tick, cfg: cfg.clone(), mtime: mtime(&cfg.path), next_run: Instant::now() })
}

fn runtime(sync_rx: Receiver<Vec<Building>>, out: Sender<Out>) {
    let mut config = wasmtime::Config::new();
    config.consume_fuel(true);
    let engine = match wasmtime::Engine::new(&config) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("wasm engine failed to start: {}", e);
            return;
        }
    };
    let lnk = match linker(&engine) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("wasm linker failed: {}", e);
            return;
        }
    };

    let mut desired: Vec<Building> = vec![];
    let mut slots: HashMap<(String, String), Slot> = HashMap::new();
    let mut failed: HashMap<(String, String), Failed> = HashMap::new();

    loop {
        // absorb the newest sync (coalescing any backlog)
        let mut got = None;
        loop {
            match sync_rx.try_recv() {
                Ok(s) => got = Some(s),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if let Some(s) = got {
            desired = s;
        }

        // reconcile: drop slots whose building/module is gone, (re)build changed ones
        let want: HashMap<(String, String), &ModuleCfg> = desired
            .iter()
            .flat_map(|b| b.modules.iter().map(move |m| ((b.name.clone(), m.name.clone()), m)))
            .collect();
        slots.retain(|k, _| want.contains_key(k));
        failed.retain(|k, _| want.contains_key(k));

        for (key, cfg) in &want {
            if !cfg.enabled {
                slots.remove(key);
                failed.remove(key);
                continue;
            }
            let mt = mtime(&cfg.path);
            let stale = |c: &ModuleCfg, m: &Option<SystemTime>| c != *cfg || *m != mt;
            if let Some(f) = failed.get(key) {
                if stale(&f.cfg, &f.mtime) {
                    failed.remove(key);
                } else {
                    continue;
                }
            }
            if let Some(s) = slots.get(key) {
                if !stale(&s.cfg, &s.mtime) {
                    continue;
                }
                slots.remove(key);
            }
            match build_slot(&engine, &lnk, &key.0, cfg) {
                Ok(slot) => {
                    slots.insert(key.clone(), slot);
                }
                Err(e) => {
                    failed.insert(key.clone(), Failed { cfg: (*cfg).clone(), mtime: mt });
                    let _ = out.send(Out::Ran {
                        proj: key.0.clone(),
                        module: key.1.clone(),
                        fuel_used: 0,
                        http_used: 0,
                        ms: 0.0,
                        error: Some(e),
                    });
                }
            }
        }

        // run every due module
        let now = Instant::now();
        for ((proj, module), slot) in slots.iter_mut() {
            if now < slot.next_run {
                continue;
            }
            slot.next_run = now + Duration::from_secs_f64(slot.cfg.interval_sec.max(1.0));
            let state_json = desired
                .iter()
                .find(|b| b.name == *proj)
                .map(|b| b.state_json.clone())
                .unwrap_or_default();
            {
                let ctx = slot.store.data_mut();
                ctx.state_json = state_json;
                ctx.scratch.clear();
                ctx.http_used = 0;
                ctx.outs.clear();
            }
            let _ = slot.store.set_fuel(slot.cfg.fuel_per_tick);
            let started = Instant::now();
            let result = slot.tick.call(&mut slot.store, ());
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            let fuel_left = slot.store.get_fuel().unwrap_or(0);
            let fuel_used = slot.cfg.fuel_per_tick.saturating_sub(fuel_left);
            let ctx = slot.store.data_mut();
            let http_used = ctx.http_used;
            for o in ctx.outs.drain(..) {
                let _ = out.send(o);
            }
            let error = match result {
                Ok(0) => None,
                Ok(code) => Some(format!("tick returned {}", code)),
                Err(e) => Some(match e.downcast_ref::<wasmtime::Trap>() {
                    Some(t) => format!("trap: {}", t),
                    None => format!("trap: {}", e.root_cause()),
                }),
            };
            let _ = out.send(Out::Ran {
                proj: proj.clone(),
                module: module.clone(),
                fuel_used,
                http_used,
                ms,
                error,
            });
        }

        std::thread::sleep(Duration::from_millis(200));
    }
}
