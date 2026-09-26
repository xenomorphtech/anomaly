//! The engine — one space, owned by the daemon.
//!
//! `Engine` holds the world, its staleness bookkeeping, the wasm building
//! programs and the codex units, and is the only thing that mutates any of it.
//! Frontends drive it with `Cmd`s (`handle`) and watch it through `Snapshot`s;
//! `tick` pumps the hosts, refreshes building snapshots and autosaves. Every
//! change flips `changed`, which the daemon turns into a new published
//! snapshot version.
//!
//! Anything a frontend would show as a toast or a map ping is emitted as a
//! `Notice` so that every attached frontend sees it, not just the one whose
//! command caused it.

use crate::model::*;
use crate::proto::*;
use crate::store::{self, Prefs};
use crate::{codex, wasm, worker};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

const MAX_DELTA: usize = 200;
const MAX_NOTICES: usize = 64;
/// events kept in a snapshot (the file keeps them all)
const SNAPSHOT_EVENTS: usize = 600;

pub struct Engine {
    pub world: World,
    pub rt: Vec<Rt>,
    pub unseen: Vec<usize>,
    pub prefs: Prefs,
    pub space_path: String,
    now_min: f64,
    dirty: bool,
    changed: bool,
    last_save: Instant,
    wasm: wasm::Host,
    mod_status: HashMap<(String, String), ModStatus>,
    last_wasm_sync: Instant,
    workers: worker::Host,
    codex: codex::Shared,
    last_codex: Option<Usage>,
    notices: VecDeque<Notice>,
    notice_seq: u64,
    next_id: u64,
    quit: bool,
}

impl Engine {
    /// load the space at COMMANDER_SPACE (default space.jsonl) and start the hosts
    pub fn open() -> Engine {
        let space_path = store::path();
        let (mut world, rt, prefs) = match store::load(&space_path) {
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
                (world, rt, Prefs::default())
            }
        };
        // stable ids for records that predate them
        let mut next_id = world.projects.iter().map(|p| p.id).max().unwrap_or(0) + 1;
        let mut dirty = false;
        for p in world.projects.iter_mut() {
            if p.id == 0 {
                p.id = next_id;
                next_id += 1;
                dirty = true;
            }
        }
        Engine {
            world,
            rt,
            unseen: vec![],
            prefs,
            space_path,
            now_min: now_min(),
            dirty,
            changed: true,
            last_save: Instant::now(),
            wasm: wasm::Host::spawn(),
            mod_status: HashMap::new(),
            last_wasm_sync: Instant::now() - Duration::from_secs(10),
            workers: worker::Host::spawn(),
            codex: codex::spawn(),
            last_codex: None,
            notices: VecDeque::new(),
            notice_seq: 0,
            next_id,
            quit: false,
        }
    }

    /// a /shutdown was requested
    pub fn quitting(&self) -> bool {
        self.quit
    }

    /// pump the hosts, refresh building snapshots, autosave. returns whether
    /// anything changed since the last call (and clears the flag).
    pub fn tick(&mut self) -> bool {
        self.now_min = now_min();
        self.worker_pump();
        self.wasm_pump();
        if self.last_wasm_sync.elapsed() > Duration::from_secs(1) {
            self.wasm_sync();
        }
        let usage = self.codex.lock().unwrap().clone();
        if usage != self.last_codex {
            self.last_codex = usage;
            self.changed = true;
        }
        if self.dirty && self.last_save.elapsed() > Duration::from_secs(2) {
            self.save();
        }
        std::mem::take(&mut self.changed)
    }

    pub fn save(&mut self) {
        if let Err(e) = store::save(&self.space_path, &self.world, &self.rt, &self.prefs) {
            eprintln!("failed to save {}: {}", self.space_path, e);
        }
        self.dirty = false;
        self.last_save = Instant::now();
    }

    pub fn shutdown(&mut self) {
        if self.dirty {
            self.save();
        }
    }

    fn touch(&mut self) {
        self.dirty = true;
        self.changed = true;
    }

    pub fn snapshot(&self, version: u64) -> Snapshot {
        let mut world = self.world.clone();
        if world.events.len() > SNAPSHOT_EVENTS {
            world.events.drain(..world.events.len() - SNAPSHOT_EVENTS);
        }
        Snapshot {
            version,
            world,
            rt: self.rt.clone(),
            unseen: self.unseen.clone(),
            prefs: self.prefs.clone(),
            modules: self
                .mod_status
                .iter()
                .map(|((proj, module), status)| ModuleStatusRec { proj: proj.clone(), module: module.clone(), status: status.clone() })
                .collect(),
            running: self.workers.running_keys(),
            codex: self.last_codex.clone(),
            notices: self.notices.iter().cloned().collect(),
            space_path: self.space_path.clone(),
            archive_path: store::archive_path(&self.space_path),
        }
    }

    // ---------- time / geometry ----------
    fn clock(&self) -> String {
        let m = (self.now_min as i64).rem_euclid(1440);
        format!("{:02}:{:02}", m / 60, m % 60)
    }
    fn tier(&self, i: usize) -> Tier {
        Tier::from_age_min(self.now_min - self.rt[i].last_visit_min)
    }
    fn base_center(&self, i: usize) -> (f32, f32) {
        let (x, y) = self.world.projects[i].pos;
        (x + 180.0, y + 130.0)
    }
    /// world anchor of task ti's pylon: stored pos, or an auto slot on the arc under the base
    pub fn pylon_world_pos(&self, pi: usize, ti: usize) -> (f32, f32) {
        if let Some(p) = self.world.projects[pi].tasks.get(ti).and_then(|t| t.pos) {
            return p;
        }
        let c = self.base_center(pi);
        let ang = 1.05 + ti as f32 * 0.5;
        ((c.0 + ang.cos() * 300.0).clamp(0.0, WW), (c.1 + ang.sin() * 230.0).clamp(0.0, WH))
    }
    /// world anchor of question qi's sensor array: stored pos, or an auto slot on the arc above
    pub fn question_world_pos(&self, pi: usize, qi: usize) -> (f32, f32) {
        if let Some(p) = self.world.projects[pi].questions.get(qi).and_then(|q| q.pos) {
            return p;
        }
        let c = self.base_center(pi);
        let ang = -1.05 - qi as f32 * 0.5;
        ((c.0 + ang.cos() * 300.0).clamp(0.0, WW), (c.1 + ang.sin() * 230.0).clamp(0.0, WH))
    }

    // ---------- notices ----------
    fn notify(&mut self, n: Notice) {
        self.notices.push_back(n);
        while self.notices.len() > MAX_NOTICES {
            self.notices.pop_front();
        }
        self.changed = true;
    }
    fn toast(&mut self, head: &str, body: &str, sub: &str, ok: bool, proj: Option<usize>) {
        self.notice_seq += 1;
        self.notify(Notice {
            seq: self.notice_seq,
            proj,
            head: head.into(),
            body: body.into(),
            sub: sub.into(),
            ok,
            loud: true,
            ping: false,
            report: false,
        });
    }
    fn ping(&mut self, proj: usize) {
        self.notice_seq += 1;
        self.notify(Notice {
            seq: self.notice_seq,
            proj: Some(proj),
            head: String::new(),
            body: String::new(),
            sub: String::new(),
            ok: true,
            loud: false,
            ping: true,
            report: false,
        });
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
        self.world.events.push(Event { ts: ts.clone(), proj: Some(proj), agent: agent.map(String::from), text: text.into() });
        if let Some(aid) = agent {
            if let Some(ag) = self.world.projects[proj].agents.iter_mut().find(|a| a.id == aid) {
                ag.last_report = ts.clone();
            }
        }
        let who = agent.map(String::from).unwrap_or_else(|| self.world.projects[proj].name.clone());
        // the frontend that has this base in view consumes the delta at once
        // (it visits on sight); everyone else sees it accumulate
        let rt = &mut self.rt[proj];
        rt.delta.push(format!("{}: {} ({})", who, text, ts));
        if rt.delta.len() > MAX_DELTA {
            rt.delta.remove(0);
        }
        rt.unseen_events += 1;
        self.unseen.retain(|&p| p != proj);
        self.unseen.push(proj);
        self.notice_seq += 1;
        self.notify(Notice {
            seq: self.notice_seq,
            proj: Some(proj),
            head: format!("📡 {} · {}", who, ts),
            body: text.into(),
            sub: String::new(),
            ok: false,
            loud,
            ping: true,
            report: true,
        });
        self.touch();
    }

    fn proj_by_name(&self, name: &str) -> Option<usize> {
        self.world.projects.iter().position(|p| p.name == name)
    }

    // ---------- wasm module host ----------
    fn wasm_sync(&mut self) {
        let buildings = self
            .world
            .projects
            .iter()
            .map(|p| wasm::Building { name: p.name.clone(), state_json: serde_json::to_string(p).unwrap_or_default(), modules: p.modules.clone() })
            .collect();
        self.wasm.sync(buildings);
        self.last_wasm_sync = Instant::now();
    }

    fn wasm_pump(&mut self) {
        use wasm::Out;
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
                    self.changed = true;
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
                    self.changed = true;
                }
            }
        }
    }

    /// apply one reducer command from a module to building i's state
    fn apply_wasm_reduce(&mut self, i: usize, module: &str, cmd: &Value) {
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
                let effort = s("effort").as_deref().and_then(parse_effort);
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
                            if let Some(e) = effort {
                                t.effort = e;
                            }
                        }
                        None => p.tasks.push(Task { title, state, pos, notes: notes.unwrap_or_default(), effort: effort.flatten(), unread: false }),
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
                let effort = s("effort").as_deref().and_then(parse_effort);
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
                            if let Some(e) = effort {
                                q.effort = e;
                            }
                        }
                        None => p.questions.push(Question {
                            text,
                            resolved: resolved.unwrap_or(false),
                            pos,
                            notes: notes.unwrap_or_default(),
                            effort: effort.flatten(),
                        }),
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
        self.touch();
    }

    // ---------- world edits ----------

    fn event(&mut self, proj: Option<usize>, agent: Option<&str>, text: String) {
        let ts = self.clock();
        self.world.events.push(Event { ts, proj, agent: agent.map(String::from), text });
    }

    fn place(&mut self, cx: f32, cy: f32, name: String) -> usize {
        let idx = self.world.projects.len();
        let color = PROJ_COLORS[idx % PROJ_COLORS.len()];
        let pos = ((cx - 180.0).clamp(0.0, WW - BASE_W), (cy - 130.0).clamp(0.0, WH - 200.0));
        self.world.projects.push(Project {
            id: self.next_id,
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
            effort: None,
        });
        self.next_id += 1;
        self.rt.push(new_rt(self.now_min));
        let ts = self.clock();
        self.event(Some(idx), None, format!("base established: {}", name));
        self.toast(&format!("⌂ BASE ESTABLISHED · {}", ts), &name, "drag to reposition · L links it to another base", true, Some(idx));
        self.touch();
        idx
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
        self.event(Some(proj), None, format!("DECIDED: {} → {}", title, name));
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
            self.event(Some(proj), Some(aid), format!("Unblocked — resuming with {} strategy", name.to_lowercase()));
        }
        let sub = if released.is_empty() { String::new() } else { format!("{} released · minimap marker cleared", released.join(", ")) };
        self.toast(&format!("✓ ORDER COMMITTED · {}", ts), &format!("{} → {}", title, name), &sub, true, Some(proj));
        self.ping(proj);
        self.touch();
    }

    /// demolish base i: archive its record and history to disk, then remove it
    /// from the world, shifting every live project index above i down by one
    fn destroy_base(&mut self, i: usize) -> Result<(), String> {
        if i >= self.world.projects.len() {
            return Err("no such base".into());
        }
        let ts = self.clock();
        let name = self.world.projects[i].name.clone();

        // archive first — the base is only demolished once its record is safely on disk
        let rec = store::ArchivedBase {
            t: "archived_base",
            ts: ts.clone(),
            project: self.world.projects[i].clone(),
            last_visit_min: self.rt[i].last_visit_min,
            decisions: self.world.decisions.iter().filter(|d| d.proj == i).cloned().collect(),
            events: self.world.events.iter().filter(|e| e.proj == Some(i)).cloned().collect(),
        };
        let apath = store::archive_path(&self.space_path);
        if let Err(e) = store::append_archive(&apath, &rec) {
            let msg = format!("could not archive {}: {}", name, e);
            self.toast("💥 DESTROY ABORTED", &msg, "the base still stands", false, Some(i));
            return Err(msg);
        }

        // the unit processes of the base die with it
        for ag in self.world.projects[i].agents.clone() {
            self.workers.halt(&name, &ag.id);
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
        let shift = |v: usize| if v > i { v - 1 } else { v };
        self.unseen.retain(|&p| p != i);
        for p in &mut self.unseen {
            *p = shift(*p);
        }
        for n in &mut self.notices {
            n.proj = n.proj.and_then(|p| if p == i { None } else { Some(shift(p)) });
        }
        self.mod_status.retain(|(pn, _), _| *pn != name);

        self.event(None, None, format!("base destroyed: {} (record archived)", name));
        self.toast(&format!("💥 BASE DESTROYED · {}", ts), &name, &format!("record archived → {}", apath), true, None);
        self.touch();
        self.wasm_sync();
        Ok(())
    }

    fn set_pylon_state(&mut self, pi: usize, ti: usize, st: TaskState) -> bool {
        let Some(t) = self.world.projects.get_mut(pi).and_then(|p| p.tasks.get_mut(ti)) else { return false };
        if t.state != st {
            t.state = st;
            t.unread = false;
            let title = t.title.clone();
            self.event(Some(pi), None, format!("pylon {} → {}", title, st.label()));
            self.touch();
        }
        true
    }

    fn set_question_resolved(&mut self, pi: usize, qi: usize, r: bool) -> bool {
        let Some(q) = self.world.projects.get_mut(pi).and_then(|p| p.questions.get_mut(qi)) else { return false };
        if q.resolved != r {
            q.resolved = r;
            let text = q.text.clone();
            self.event(Some(pi), None, if r { format!("question resolved: {}", text) } else { format!("question reopened: {}", text) });
            self.touch();
        }
        true
    }

    fn set_effort(&mut self, pi: usize, at: Target, e: Option<String>) -> bool {
        let (what, title, slot) = match at {
            Target::Pylon(ti) => match self.world.projects.get_mut(pi).and_then(|p| p.tasks.get_mut(ti)) {
                Some(t) => ("pylon", t.title.clone(), &mut t.effort),
                None => return false,
            },
            Target::Question(qi) => match self.world.projects.get_mut(pi).and_then(|p| p.questions.get_mut(qi)) {
                Some(q) => ("sensor array", q.text.clone(), &mut q.effort),
                None => return false,
            },
        };
        if *slot != e {
            *slot = e.clone();
            self.event(Some(pi), None, format!("{} {} effort → {}", what, title, e.as_deref().unwrap_or("base default")));
            self.touch();
        }
        true
    }

    fn remove_structs(&mut self, list: Vec<(usize, Target)>) -> usize {
        // remove per project in descending index order so earlier removals don't shift later ones
        let mut tasks: Vec<(usize, usize)> = vec![];
        let mut quests: Vec<(usize, usize)> = vec![];
        for (pi, at) in list {
            match at {
                Target::Pylon(ti) => tasks.push((pi, ti)),
                Target::Question(qi) => quests.push((pi, qi)),
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
                    self.event(Some(pi), None, format!("pylon demolished: {}", t0.title));
                    n += 1;
                }
            }
        }
        for (pi, qi) in quests {
            if let Some(p) = self.world.projects.get_mut(pi) {
                if qi < p.questions.len() {
                    let q = p.questions.remove(qi);
                    self.event(Some(pi), None, format!("sensor array decommissioned: {}", q.text));
                    n += 1;
                }
            }
        }
        let ts = self.clock();
        self.toast(&format!("💥 DEMOLISHED · {}", ts), &format!("{} structure{} removed", n, if n == 1 { "" } else { "s" }), "logged in the event feed", true, None);
        self.touch();
        n
    }

    fn toggle_link(&mut self, from: usize, to: usize) -> Result<(), String> {
        if from == to {
            return Err("A base cannot link to itself.".into());
        }
        if from >= self.world.projects.len() || to >= self.world.projects.len() {
            return Err("no such base".into());
        }
        let existing = self.world.links.iter().position(|l| (l.a == from && l.b == to) || (l.a == to && l.b == from));
        let ts = self.clock();
        let names = format!("{} ⟷ {}", self.world.projects[from].name, self.world.projects[to].name);
        match existing {
            Some(k) => {
                self.world.links.remove(k);
                self.toast(&format!("⛓ LINK SEVERED · {}", ts), &names, "", true, None);
            }
            None => {
                self.world.links.push(Link { a: from, b: to });
                self.event(Some(from), None, format!("link established: {}", names));
                self.toast(&format!("⛓ LINK ESTABLISHED · {}", ts), &names, "click the ◆ midpoint node to sever", true, None);
            }
        }
        self.touch();
        Ok(())
    }

    fn visit(&mut self, i: usize) -> bool {
        let Some(rt) = self.rt.get_mut(i) else { return false };
        rt.delta.clear();
        rt.unseen_events = 0;
        rt.last_visit_min = self.now_min;
        self.unseen.retain(|&p| p != i);
        self.touch();
        true
    }

    // ---------- codex workers ----------

    /// send a unit to a structure of base `pi`: picks `agent` (or the first idle
    /// unit, or hires a new one) and starts a fresh codex thread. a pylon goes
    /// doing and the unit works it; a sensor array gets a scout that answers the
    /// question (read-only unless the base says otherwise) and resolves it on DONE
    /// `cont`: continuation — the unit that last held the structure is sent
    /// again on a fresh thread, with its last report in the prompt, so the
    /// next turn starts from the state it left rather than from the brief alone
    fn dispatch(&mut self, pi: usize, at: Target, agent: Option<String>, extra: Option<String>, cont: bool) -> Result<String, String> {
        let sensor = matches!(at, Target::Question(_));
        let proj = self.world.projects.get(pi).ok_or("no such base")?;
        let (title, notes, effort) = match at {
            Target::Pylon(ti) => proj.tasks.get(ti).map(|t| (t.title.clone(), t.notes.clone(), t.effort.clone())).ok_or("no such pylon")?,
            Target::Question(qi) => {
                proj.questions.get(qi).map(|q| (q.text.clone(), q.notes.clone(), q.effort.clone())).ok_or("no such sensor array")?
            }
        };
        // the structure's own effort wins; else the base's; else codex's config
        let effort = effort.or_else(|| proj.effort.clone());
        let what = if sensor { "sensor array" } else { "pylon" };
        let cwd = proj.cwd.clone().ok_or_else(|| format!("base {} has no repo (cwd) set", proj.name))?;
        let name = proj.name.clone();
        // a structure already being worked is not handed to a second unit
        if let Some(ag) = proj.agents.iter().find(|a| a.task == title && a.sensor == sensor && self.workers.running(&name, &a.id)) {
            return Err(format!("{} is already working this {}", ag.id, what));
        }
        // the report a continuation starts from: the last message of the unit that held this structure
        let prior = if cont {
            let ag = proj
                .agents
                .iter()
                .find(|a| a.task == title && a.sensor == sensor && !a.last_msg.trim().is_empty())
                .ok_or_else(|| format!("no report on this {} to continue from — dispatch it first", what))?;
            format!("(unit {} · {})\n{}", ag.id, ag.last_report, ag.last_msg.trim())
        } else {
            String::new()
        };
        let aid = match agent {
            Some(a) => a,
            // the unit that last held this structure keeps it (fresh thread); else an idle one; else hire
            None => match proj
                .agents
                .iter()
                .find(|a| a.task == title && a.sensor == sensor)
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
        let prompt = worker::prompt(&aid, &name, &proj.goal, &title, &notes, extra.as_deref().unwrap_or(""), &prior, sensor);
        // scouts answer questions; they don't get write access unless the base set a sandbox
        let default_sandbox = if sensor { "read-only" } else { "workspace-write" };
        let job = worker::Job {
            proj: name.clone(),
            agent: aid.clone(),
            cwd,
            sandbox: proj.sandbox.clone().unwrap_or_else(|| default_sandbox.into()),
            model: proj.model.clone(),
            effort: effort.clone(),
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
        ag.sensor = sensor;
        ag.blocked_on = None;
        ag.thread_id = None;
        ag.effort = effort;
        ag.last_msg.clear();
        ag.last_report = ts;
        match at {
            Target::Pylon(ti) => {
                p.tasks[ti].state = TaskState::Doing;
                p.tasks[ti].unread = false;
            }
            // re-scouting a resolved question reopens it: the array sweeps again
            Target::Question(qi) => p.questions[qi].resolved = false,
        }
        let verb = match (cont, sensor) {
            (true, _) => "continuing",
            (false, true) => "scouting",
            (false, false) => "dispatched",
        };
        self.report(pi, Some(&aid), &format!("{}{} → {}", if hired { "hired · " } else { "" }, verb, title));
        Ok(aid)
    }

    /// follow-up order for a unit: resumes its codex thread with `text`
    fn tell(&mut self, pi: usize, aid: &str, text: &str) -> Result<(), String> {
        let proj = self.world.projects.get(pi).ok_or("no such base")?;
        let ag = proj.agents.iter().find(|a| a.id == aid).ok_or("no such unit")?;
        let thread = ag.thread_id.clone().ok_or("unit has no codex thread yet — dispatch it first")?;
        let cwd = proj.cwd.clone().ok_or("base has no repo (cwd) set")?;
        let name = proj.name.clone();
        let job = worker::Job {
            proj: name,
            agent: aid.to_string(),
            cwd,
            sandbox: proj.sandbox.clone().unwrap_or_else(|| "workspace-write".into()),
            model: proj.model.clone(),
            effort: ag.effort.clone().or_else(|| proj.effort.clone()),
            prompt: format!(
                "Commander's follow-up order: {}\n\nSame closing rule as before: end with one DONE: / BLOCKED: / PARTIAL: line.",
                text
            ),
            resume: Some(thread),
        };
        self.workers.start(job)?;
        let ts = self.clock();
        let p = &mut self.world.projects[pi];
        let (task, sensor) = {
            let ag = p.agents.iter_mut().find(|a| a.id == aid).unwrap();
            ag.state = AgentState::Working;
            ag.blocked_on = None;
            ag.last_report = ts;
            (ag.task.clone(), ag.sensor)
        };
        if !sensor {
            if let Some(t) = p.tasks.iter_mut().find(|t| t.title == task) {
                if t.state == TaskState::Blocked {
                    t.state = TaskState::Doing;
                }
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
        use worker::{verdict, Out, Verdict};
        for out in self.workers.drain() {
            match out {
                Out::Started { proj, agent, thread_id } => {
                    if let Some((_, ag)) = self.agent_mut(&proj, &agent) {
                        ag.thread_id = Some(thread_id);
                        self.touch();
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
                        let names: Vec<String> = paths.iter().map(|p| p.rsplit('/').next().unwrap_or(p).to_string()).take(6).collect();
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
                        let (task, sensor) = (ag.task.clone(), ag.sensor);
                        if !sensor {
                            if let Some(t) = self.world.projects[pi].tasks.iter_mut().find(|t| t.title == task) {
                                t.state = TaskState::Blocked;
                            }
                        }
                        let short: String = text.chars().take(200).collect();
                        self.report(pi, Some(&agent), &format!("✗ {}", short));
                    }
                }
                Out::Turn { proj, agent, input_tokens, output_tokens } => {
                    if let Some((pi, ag)) = self.agent_mut(&proj, &agent) {
                        ag.turns += 1;
                        ag.tokens += input_tokens + output_tokens;
                        let (task, sensor, last_msg) = (ag.task.clone(), ag.sensor, ag.last_msg.clone());
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
                        if sensor {
                            // a scout's DONE answers the question: the array resolves and the
                            // report is filed into its brief so the answer outlives the unit
                            if matches!(v, Some((Verdict::Done, _))) {
                                let stamp = self.clock();
                                if let Some(q) = self.world.projects[pi].questions.iter_mut().find(|q| q.text == task) {
                                    q.resolved = true;
                                    let report = last_msg.trim();
                                    if !report.is_empty() && !q.notes.contains(report) {
                                        if !q.notes.trim().is_empty() {
                                            q.notes.push_str("\n\n");
                                        }
                                        q.notes.push_str(&format!("— {} · {} —\n{}", agent, stamp, report));
                                    }
                                }
                            }
                        } else if let Some(ts) = tstate {
                            if let Some(t) = self.world.projects[pi].tasks.iter_mut().find(|t| t.title == task) {
                                t.state = ts;
                                // a finished report pulses on the map until the commander opens
                                // the room (a frontend standing in it marks it read at once)
                                t.unread = ts == TaskState::Done;
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
                        self.touch();
                    }
                }
                Out::Eof { .. } => {}
            }
        }
    }

    // ---------- commands ----------

    fn err(e: impl std::fmt::Display) -> Value {
        json!({ "err": e.to_string() })
    }

    /// apply one command; the reply is what the HTTP caller gets back
    pub fn handle(&mut self, cmd: Cmd) -> Value {
        let ok = json!({ "ok": true });
        match cmd {
            Cmd::State => self.state_json(),
            Cmd::Save => {
                self.save();
                ok
            }
            Cmd::Shutdown => {
                self.quit = true;
                ok
            }
            Cmd::Place { x, y, name } => {
                let i = self.place(x, y, name);
                json!({ "ok": true, "proj": i })
            }
            Cmd::Destroy { i } => match self.destroy_base(i) {
                Ok(()) => ok,
                Err(e) => Self::err(e),
            },
            Cmd::Link { a, b } => match self.toggle_link(a, b) {
                Ok(()) => ok,
                Err(e) => Self::err(e),
            },
            Cmd::Unlink { li } => {
                if li >= self.world.links.len() {
                    return Self::err("no such link");
                }
                let l = self.world.links.remove(li);
                let names = format!(
                    "{} ⟷ {}",
                    self.world.projects.get(l.a).map(|p| p.name.as_str()).unwrap_or("?"),
                    self.world.projects.get(l.b).map(|p| p.name.as_str()).unwrap_or("?"),
                );
                let ts = self.clock();
                self.toast(&format!("⛓ LINK SEVERED · {}", ts), &names, "", true, None);
                self.touch();
                ok
            }
            Cmd::Decide { d, o } => {
                if d >= self.world.decisions.len() || o >= self.world.decisions[d].options.len() {
                    return Self::err("no such decision/option");
                }
                self.commit_decision(d, o);
                ok
            }
            Cmd::Capture { text, pos } => {
                let ts = self.clock();
                let n = self.world.captures.len() as f32;
                let (ax, ay) = pos.unwrap_or((WW / 2.0, WH / 2.0));
                let pos = (
                    (ax - 220.0 + (n % 3.0) * 150.0).clamp(0.0, WW - 200.0),
                    (ay + 120.0 + ((n / 3.0).floor() % 3.0) * 100.0).clamp(0.0, WH - 80.0),
                );
                self.world.captures.push(CaptureNote { text: text.clone(), ts: ts.clone(), pos });
                self.world.events.push(Event { ts: ts.clone(), proj: None, agent: None, text: text.clone() });
                self.toast(&format!("⚡ CAPTURED · {}", ts), &text, "drifting unsorted mid-map — file it whenever", true, None);
                self.touch();
                ok
            }
            Cmd::FileCapture { cap, proj } => {
                if cap >= self.world.captures.len() || proj >= self.world.projects.len() {
                    return Self::err("no such capture/base");
                }
                let c = self.world.captures.remove(cap);
                let ts = self.clock();
                let pname = self.world.projects[proj].name.clone();
                self.event(Some(proj), None, format!("filed capture: {}", c.text));
                let rt = &mut self.rt[proj];
                rt.delta.push(format!("you filed: {} ({})", c.text, ts));
                rt.unseen_events += 1;
                self.toast(&format!("⚡ FILED · {}", ts), &c.text, &format!("→ {}", pname), true, Some(proj));
                self.touch();
                ok
            }
            Cmd::DiscardCapture { cap } => {
                if cap >= self.world.captures.len() {
                    return Self::err("no such capture");
                }
                let c = self.world.captures.remove(cap);
                self.toast("🗑 DISCARDED", &c.text, "", true, None);
                self.touch();
                ok
            }
            Cmd::ModuleAdd { i, cfg } => {
                if i >= self.world.projects.len() {
                    return Self::err("no such base");
                }
                if self.world.projects[i].modules.iter().any(|m| m.name == cfg.name) {
                    return Self::err("module name already installed");
                }
                let name = cfg.name.clone();
                self.world.projects[i].modules.push(cfg);
                self.event(Some(i), None, format!("program installed: ⚙{}", name));
                self.touch();
                self.wasm_sync();
                ok
            }
            Cmd::ModuleRm { i, name } => {
                if i >= self.world.projects.len() {
                    return Self::err("no such base");
                }
                let before = self.world.projects[i].modules.len();
                self.world.projects[i].modules.retain(|m| m.name != name);
                if self.world.projects[i].modules.len() == before {
                    return Self::err("no such module");
                }
                let pname = self.world.projects[i].name.clone();
                self.mod_status.remove(&(pname, name.clone()));
                self.event(Some(i), None, format!("program uninstalled: ⚙{}", name));
                self.touch();
                self.wasm_sync();
                ok
            }
            Cmd::ModuleToggle { i, name } => match self.world.projects.get_mut(i).and_then(|p| p.modules.iter_mut().find(|m| m.name == name)) {
                Some(m) => {
                    m.enabled = !m.enabled;
                    let enabled = m.enabled;
                    self.touch();
                    self.wasm_sync();
                    json!({ "ok": true, "enabled": enabled })
                }
                None => Self::err("no such base/module"),
            },
            Cmd::Cfg { struct_scale, show_rail } => {
                if let Some(v) = struct_scale {
                    self.prefs.struct_scale = v.clamp(0.3, 5.0);
                    self.touch();
                }
                if let Some(v) = show_rail {
                    self.prefs.show_rail = v;
                    self.touch();
                }
                json!({ "ok": true, "struct_scale": self.prefs.struct_scale, "show_rail": self.prefs.show_rail })
            }
            Cmd::Pylon { i, title, pos, state, notes, effort } => {
                let effort = match effort.as_deref().map(parse_effort) {
                    Some(None) => return Self::err(format!("bad effort; use one of {}", EFFORTS.join("|"))),
                    e => e.flatten(),
                };
                if i >= self.world.projects.len() {
                    return Self::err("no such base");
                }
                let st = state.as_deref().and_then(TaskState::parse).unwrap_or(TaskState::Todo);
                let ts = self.clock();
                let mut created = false;
                match self.world.projects[i].tasks.iter_mut().find(|t| t.title == title) {
                    Some(t) => {
                        t.state = st;
                        if pos.is_some() {
                            t.pos = pos;
                        }
                        if let Some(n) = notes {
                            t.notes = n;
                        }
                        if let Some(e) = effort {
                            t.effort = e;
                        }
                    }
                    None => {
                        self.world.projects[i].tasks.push(Task {
                            title: title.clone(),
                            state: st,
                            pos,
                            notes: notes.unwrap_or_default(),
                            effort: effort.flatten(),
                            unread: false,
                        });
                        created = true;
                    }
                }
                if created {
                    self.event(Some(i), None, format!("pylon warped in: {}", title));
                    self.toast(&format!("◆ PYLON WARPED IN · {}", ts), &title, "enter it to set the brief · drag to reposition", true, Some(i));
                    self.ping(i);
                }
                self.touch();
                json!({ "ok": true, "created": created })
            }
            Cmd::Question { i, text, pos, resolved, notes, effort } => {
                let effort = match effort.as_deref().map(parse_effort) {
                    Some(None) => return Self::err(format!("bad effort; use one of {}", EFFORTS.join("|"))),
                    e => e.flatten(),
                };
                if i >= self.world.projects.len() {
                    return Self::err("no such base");
                }
                let ts = self.clock();
                let mut created = false;
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
                        if let Some(e) = effort {
                            q.effort = e;
                        }
                    }
                    None => {
                        self.world.projects[i].questions.push(Question {
                            text: text.clone(),
                            resolved: resolved.unwrap_or(false),
                            pos,
                            notes: notes.unwrap_or_default(),
                            effort: effort.flatten(),
                        });
                        created = true;
                    }
                }
                if created {
                    self.event(Some(i), None, format!("sensor array raised: {}", text));
                    self.toast(&format!("⌖ SENSOR ARRAY ONLINE · {}", ts), &text, "scanning — a scout's DONE resolves it", true, Some(i));
                    self.ping(i);
                }
                self.touch();
                json!({ "ok": true, "created": created })
            }
            Cmd::Struct { i, at, state, resolved, notes, effort, pos, read } => {
                let exists = match at {
                    Target::Pylon(ti) => self.world.projects.get(i).map_or(false, |p| ti < p.tasks.len()),
                    Target::Question(qi) => self.world.projects.get(i).map_or(false, |p| qi < p.questions.len()),
                };
                if !exists {
                    return Self::err("no such base/structure");
                }
                if let (Some(st), Target::Pylon(ti)) = (state, at) {
                    self.set_pylon_state(i, ti, st);
                }
                if let (Some(r), Target::Question(qi)) = (resolved, at) {
                    self.set_question_resolved(i, qi, r);
                }
                if let Some(e) = effort {
                    self.set_effort(i, at, e);
                }
                let p = &mut self.world.projects[i];
                match at {
                    Target::Pylon(ti) => {
                        let t = &mut p.tasks[ti];
                        if let Some(n) = notes {
                            if t.notes != n {
                                t.notes = n;
                                self.dirty = true;
                            }
                        }
                        if let Some((x, y)) = pos {
                            t.pos = Some((x.clamp(0.0, WW), y.clamp(0.0, WH)));
                            self.dirty = true;
                        }
                        if read && t.unread {
                            t.unread = false;
                            self.dirty = true;
                        }
                    }
                    Target::Question(qi) => {
                        let q = &mut p.questions[qi];
                        if let Some(n) = notes {
                            if q.notes != n {
                                q.notes = n;
                                self.dirty = true;
                            }
                        }
                        if let Some((x, y)) = pos {
                            q.pos = Some((x.clamp(0.0, WW), y.clamp(0.0, WH)));
                            self.dirty = true;
                        }
                    }
                }
                self.changed = true;
                ok
            }
            Cmd::RemoveStructs { list } => {
                let n = self.remove_structs(list);
                json!({ "ok": true, "removed": n })
            }
            Cmd::MoveBase { i, x, y } => match self.world.projects.get_mut(i) {
                None => Self::err("no such base"),
                Some(p) => {
                    p.pos = (x.clamp(0.0, WW - BASE_W), y.clamp(0.0, WH - 200.0));
                    self.touch();
                    ok
                }
            },
            Cmd::MoveCapture { ci, x, y } => match self.world.captures.get_mut(ci) {
                None => Self::err("no such capture"),
                Some(c) => {
                    c.pos = (x.clamp(0.0, WW - 200.0), y.clamp(0.0, WH - 80.0));
                    self.touch();
                    ok
                }
            },
            Cmd::Base { i, cwd, sandbox, model, effort } => {
                let effort = match effort.as_deref().map(parse_effort) {
                    Some(None) => return Self::err(format!("bad effort; use one of {}", EFFORTS.join("|"))),
                    e => e.flatten(),
                };
                let Some(p) = self.world.projects.get_mut(i) else { return Self::err("no such base") };
                if let Some(e) = effort {
                    p.effort = e;
                }
                if let Some(c) = cwd {
                    p.cwd = if c.is_empty() { None } else { Some(c) };
                }
                if let Some(sb) = sandbox {
                    p.sandbox = if sb.is_empty() { None } else { Some(sb) };
                }
                if let Some(m) = model {
                    p.model = if m.is_empty() { None } else { Some(m) };
                }
                self.touch();
                ok
            }
            Cmd::Dispatch { i, at, agent, prompt, cont } => {
                let at = match at {
                    DispatchTarget::At(t) => Some(t),
                    DispatchTarget::Title(title) => self.world.projects.get(i).and_then(|p| {
                        p.tasks
                            .iter()
                            .position(|t| t.title == title)
                            .map(Target::Pylon)
                            .or_else(|| p.questions.iter().position(|q| q.text == title).map(Target::Question))
                    }),
                };
                match at {
                    None => Self::err("no such base/pylon/sensor"),
                    Some(at) => match self.dispatch(i, at, agent, prompt, cont) {
                        Ok(aid) => json!({ "ok": true, "agent": aid }),
                        Err(e) => Self::err(e),
                    },
                }
            }
            Cmd::Tell { i, agent, text } => match self.tell(i, &agent, &text) {
                Ok(()) => ok,
                Err(e) => Self::err(e),
            },
            Cmd::Halt { i, agent } => {
                if self.halt(i, &agent) {
                    ok
                } else {
                    Self::err("unit is not working")
                }
            }
            Cmd::Fire { i, agent } => {
                let Some(p) = self.world.projects.get_mut(i) else { return Self::err("no such base") };
                let name = p.name.clone();
                let before = p.agents.len();
                p.agents.retain(|a| a.id != agent);
                if p.agents.len() == before {
                    return Self::err("no such unit");
                }
                self.workers.halt(&name, &agent);
                self.touch();
                ok
            }
            Cmd::Visit { i } => {
                if self.visit(i) {
                    ok
                } else {
                    Self::err("no such base")
                }
            }
        }
    }

    /// the `/state` document: the world as test scripts and tools read it
    pub fn state_json(&self) -> Value {
        let projects: Vec<Value> = self
            .world
            .projects
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let agents: Vec<Value> = p
                    .agents
                    .iter()
                    .map(|a| {
                        json!({
                            "id": a.id, "state": a.state.label(), "task": a.task, "sensor": a.sensor,
                            "blocked_on": a.blocked_on, "running": self.workers.running(&p.name, &a.id),
                            "thread_id": a.thread_id, "turns": a.turns, "tokens": a.tokens, "effort": a.effort,
                            "last_report": a.last_report, "last_msg": a.last_msg,
                        })
                    })
                    .collect();
                let tasks: Vec<Value> = p
                    .tasks
                    .iter()
                    .enumerate()
                    .map(|(ti, t)| {
                        let (x, y) = t.pos.unwrap_or_else(|| self.pylon_world_pos(i, ti));
                        json!({ "title": t.title, "state": t.state.label(), "unread": t.unread, "notes": t.notes, "effort": t.effort, "pos": [x, y] })
                    })
                    .collect();
                let questions: Vec<Value> = p
                    .questions
                    .iter()
                    .enumerate()
                    .map(|(qi, q)| {
                        let (x, y) = q.pos.unwrap_or_else(|| self.question_world_pos(i, qi));
                        json!({ "text": q.text, "resolved": q.resolved, "notes": q.notes, "effort": q.effort, "pos": [x, y] })
                    })
                    .collect();
                let modules: Vec<Value> = p
                    .modules
                    .iter()
                    .map(|m| {
                        let st = self.mod_status.get(&(p.name.clone(), m.name.clone()));
                        json!({
                            "name": m.name, "path": m.path, "enabled": m.enabled, "interval_sec": m.interval_sec,
                            "fuel_per_tick": m.fuel_per_tick, "max_http_per_tick": m.max_http_per_tick,
                            "ticks": st.map_or(0, |s| s.ticks), "fuel_used": st.map_or(0, |s| s.fuel_used),
                            "http_used": st.map_or(0, |s| s.http_used), "ms": st.map_or(0.0, |s| s.ms),
                            "error": st.and_then(|s| s.error.clone()),
                        })
                    })
                    .collect();
                json!({
                    "i": i, "id": p.id, "name": p.name, "status": p.status, "goal": p.goal, "pos": [p.pos.0, p.pos.1],
                    "tier": self.tier(i).label(), "unseen": self.rt[i].unseen_events,
                    "cwd": p.cwd, "sandbox": p.sandbox, "model": p.model, "effort": p.effort,
                    "agents": agents, "tasks": tasks, "questions": questions, "modules": modules,
                })
            })
            .collect();
        let links: Vec<Value> = self.world.links.iter().map(|l| json!([l.a, l.b])).collect();
        let decisions: Vec<Value> = self
            .world
            .decisions
            .iter()
            .enumerate()
            .map(|(di, d)| json!({ "i": di, "id": d.id, "proj": d.proj, "title": d.title, "due": d.due, "resolved": d.resolved, "chosen": d.chosen }))
            .collect();
        let captures: Vec<Value> = self.world.captures.iter().map(|c| json!({ "text": c.text, "ts": c.ts, "pos": [c.pos.0, c.pos.1] })).collect();
        let events: Vec<Value> = self
            .world
            .events
            .iter()
            .rev()
            .take(10)
            .map(|e| json!({ "ts": e.ts, "proj": e.proj, "agent": e.agent, "text": e.text }))
            .collect();
        json!({
            "clock": self.clock(),
            "space": self.space_path,
            "struct_scale": self.prefs.struct_scale,
            "show_rail": self.prefs.show_rail,
            "codex": self.last_codex.as_ref().map(|u| json!({ "pct_left": u.pct_left, "resets_at": u.resets_at, "eta": codex::eta(u.resets_at) })),
            "workers_running": self.workers.running_count(),
            "unseen": self.unseen,
            "projects": projects,
            "links": links,
            "decisions": decisions,
            "captures": captures,
            "events": events,
        })
    }
}
