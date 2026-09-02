//! Codex workers — the units that do real work.
//!
//! A worker is a `codex exec --json` process bound to one unit in one base.
//! The app dispatches a pylon (task) to a unit; this host spawns codex inside
//! the base's repo, tails its JSONL event stream on a reader thread and hands
//! parsed events back to the app thread through a channel. A follow-up order
//! resumes the unit's codex thread (`codex exec resume <id>`) so the unit keeps
//! its context between turns.
//!
//! Process lifetime: one turn per process. When stdout closes the reader sends
//! `Eof`; `drain()` reaps the child and turns that into `Exited`. Each codex
//! runs in its own process group: `codex` on PATH is a node wrapper around the
//! real binary, so halting must signal the whole group, not just the child.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// one turn of work for a unit
pub struct Job {
    pub proj: String,
    pub agent: String,
    pub cwd: String,
    /// codex sandbox: read-only | workspace-write | danger-full-access
    pub sandbox: String,
    pub model: Option<String>,
    pub prompt: String,
    /// codex thread id to resume (None = fresh thread)
    pub resume: Option<String>,
}

pub enum Out {
    Started { proj: String, agent: String, thread_id: String },
    /// an agent message item completed (the unit talking)
    Msg { proj: String, agent: String, text: String },
    Cmd { proj: String, agent: String, command: String, exit_code: Option<i32>, ok: bool },
    Files { proj: String, agent: String, paths: Vec<String> },
    Error { proj: String, agent: String, text: String },
    /// turn.completed with token usage
    Turn { proj: String, agent: String, input_tokens: i64, output_tokens: i64 },
    /// process finished (after Eof + reap)
    Exited { proj: String, agent: String, code: Option<i32>, stderr_tail: String },
    Eof { proj: String, agent: String },
}

struct Proc {
    child: Child,
    stderr: Arc<Mutex<Vec<String>>>,
}

pub struct Host {
    tx: Sender<Out>,
    rx: Receiver<Out>,
    procs: HashMap<(String, String), Proc>,
    /// codex binary (COMMANDER_CODEX, default "codex")
    bin: String,
}

impl Host {
    pub fn spawn() -> Host {
        let (tx, rx) = channel();
        Host {
            tx,
            rx,
            procs: HashMap::new(),
            bin: std::env::var("COMMANDER_CODEX").unwrap_or_else(|_| "codex".into()),
        }
    }

    pub fn running(&self, proj: &str, agent: &str) -> bool {
        self.procs.contains_key(&(proj.to_string(), agent.to_string()))
    }

    pub fn running_count(&self) -> usize {
        self.procs.len()
    }

    /// launch one turn; errors if the unit is already busy or codex can't start
    pub fn start(&mut self, job: Job) -> Result<(), String> {
        let key = (job.proj.clone(), job.agent.clone());
        if self.procs.contains_key(&key) {
            return Err("unit is already working".into());
        }
        if !std::path::Path::new(&job.cwd).is_dir() {
            return Err(format!("cwd is not a directory: {}", job.cwd));
        }
        let mut cmd = Command::new(&self.bin);
        cmd.arg("exec").arg("--json").arg("--color").arg("never").arg("--skip-git-repo-check").arg("-C").arg(&job.cwd);
        match job.sandbox.as_str() {
            "danger" | "danger-full-access" | "yolo" => {
                cmd.arg("--dangerously-bypass-approvals-and-sandbox");
            }
            s => {
                cmd.arg("-s").arg(s);
            }
        }
        if let Some(m) = &job.model {
            cmd.arg("-m").arg(m);
        }
        match &job.resume {
            Some(id) => {
                cmd.arg("resume").arg(id).arg("-");
            }
            None => {
                cmd.arg("-");
            }
        }
        cmd.current_dir(&job.cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        // own process group so halt() can take the wrapper and the real binary together
        unsafe {
            cmd.pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            });
        }
        let mut child = cmd.spawn().map_err(|e| format!("spawn {}: {}", self.bin, e))?;
        // prompt goes in on stdin, then stdin closes so codex sees EOF
        if let Some(mut stdin) = child.stdin.take() {
            let prompt = job.prompt.clone();
            std::thread::spawn(move || {
                let _ = stdin.write_all(prompt.as_bytes());
            });
        }
        let stderr_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        if let Some(err) = child.stderr.take() {
            let tail = stderr_tail.clone();
            let tag = format!("{}/{}", job.proj, job.agent);
            std::thread::spawn(move || {
                for line in BufReader::new(err).lines().map_while(Result::ok) {
                    eprintln!("[worker {}] {}", tag, line);
                    let mut t = tail.lock().unwrap();
                    t.push(line);
                    if t.len() > 20 {
                        t.remove(0);
                    }
                }
            });
        }
        if let Some(out) = child.stdout.take() {
            let tx = self.tx.clone();
            let (proj, agent) = key.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Some(o) = parse_line(&proj, &agent, &line) {
                        if tx.send(o).is_err() {
                            return;
                        }
                    }
                }
                let _ = tx.send(Out::Eof { proj, agent });
            });
        }
        self.procs.insert(key, Proc { child, stderr: stderr_tail });
        Ok(())
    }

    /// kill a running turn (TERM to the process group, KILL 3s later if it
    /// lingers); the reader's Eof then produces Exited as usual
    pub fn halt(&mut self, proj: &str, agent: &str) -> bool {
        match self.procs.get(&(proj.to_string(), agent.to_string())) {
            Some(p) => {
                let pgid = p.child.id() as i32;
                unsafe {
                    libc::kill(-pgid, libc::SIGTERM);
                }
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    unsafe {
                        // group still exists? (signal 0 probes without sending)
                        if libc::kill(-pgid, 0) == 0 {
                            libc::kill(-pgid, libc::SIGKILL);
                        }
                    }
                });
                true
            }
            None => false,
        }
    }

    /// collect outputs; Eof is turned into Exited here (reaping the child)
    pub fn drain(&mut self) -> Vec<Out> {
        let mut out = vec![];
        while let Ok(o) = self.rx.try_recv() {
            match o {
                Out::Eof { proj, agent } => {
                    let key = (proj.clone(), agent.clone());
                    let (code, tail) = match self.procs.remove(&key) {
                        Some(mut p) => {
                            let code = p.child.wait().ok().and_then(|s| s.code());
                            let tail = p.stderr.lock().unwrap().join("\n");
                            (code, tail)
                        }
                        None => (None, String::new()),
                    };
                    out.push(Out::Exited { proj, agent, code, stderr_tail: tail });
                }
                o => out.push(o),
            }
        }
        out
    }
}

/// one JSONL line from `codex exec --json` → app-level event (None = ignored)
fn parse_line(proj: &str, agent: &str, line: &str) -> Option<Out> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let p = || proj.to_string();
    let a = || agent.to_string();
    let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    match v.get("type").and_then(|t| t.as_str())? {
        "thread.started" => Some(Out::Started { proj: p(), agent: a(), thread_id: s(&v, "thread_id") }),
        "turn.completed" => {
            let u = &v["usage"];
            Some(Out::Turn {
                proj: p(),
                agent: a(),
                input_tokens: u.get("input_tokens").and_then(|x| x.as_i64()).unwrap_or(0),
                output_tokens: u.get("output_tokens").and_then(|x| x.as_i64()).unwrap_or(0),
            })
        }
        "turn.failed" => Some(Out::Error { proj: p(), agent: a(), text: s(&v["error"], "message") }),
        "error" => Some(Out::Error { proj: p(), agent: a(), text: s(&v, "message") }),
        "item.completed" => {
            let item = &v["item"];
            match item.get("type").and_then(|t| t.as_str())? {
                "agent_message" => Some(Out::Msg { proj: p(), agent: a(), text: s(item, "text") }),
                "command_execution" => Some(Out::Cmd {
                    proj: p(),
                    agent: a(),
                    command: s(item, "command"),
                    exit_code: item.get("exit_code").and_then(|x| x.as_i64()).map(|x| x as i32),
                    ok: s(item, "status") == "completed",
                }),
                "file_change" => Some(Out::Files {
                    proj: p(),
                    agent: a(),
                    paths: item["changes"]
                        .as_array()
                        .map(|c| c.iter().map(|ch| s(ch, "path")).collect())
                        .unwrap_or_default(),
                }),
                "error" => Some(Out::Error { proj: p(), agent: a(), text: s(item, "message") }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// the unit's closing status line: DONE / BLOCKED / PARTIAL + summary
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    Done,
    Blocked,
    Partial,
}

pub fn verdict(msg: &str) -> Option<(Verdict, String)> {
    for line in msg.lines().rev() {
        let l = line.trim().trim_start_matches(|c: char| !c.is_ascii_alphabetic());
        for (tag, v) in [("DONE:", Verdict::Done), ("BLOCKED:", Verdict::Blocked), ("PARTIAL:", Verdict::Partial)] {
            if let Some(rest) = l.strip_prefix(tag) {
                return Some((v, rest.trim_matches(|c: char| c == '*' || c == ' ' || c == '`').to_string()));
            }
        }
    }
    None
}

/// the standing orders every dispatched unit receives around its assignment
pub fn prompt(agent: &str, base: &str, goal: &str, title: &str, notes: &str, extra: &str) -> String {
    let mut s = format!(
        "You are unit {agent}, garrisoned at base \"{base}\" of the Commander HQ.\n\
         Base goal: {goal}\n\
         Assignment (pylon): {title}\n"
    );
    if !notes.trim().is_empty() {
        // the pylon's room text: the title is only its name, this is the actual brief
        s.push_str("Assignment brief:\n");
        s.push_str(notes.trim());
        s.push_str("\n");
    }
    if !extra.trim().is_empty() {
        s.push_str("Commander's notes: ");
        s.push_str(extra.trim());
        s.push('\n');
    }
    s.push_str(
        "\nWork directly in this repository (your cwd). Keep going until the assignment is done \
         or you hit a real blocker; do not ask questions you can answer by reading the code.\n\
         End your final message with exactly one status line, alone on the last line:\n\
         DONE: <one-line summary>      (assignment complete)\n\
         BLOCKED: <what you need>      (you need a decision or input from the commander)\n\
         PARTIAL: <what remains>       (progress made, more turns needed)\n",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_parses_last_status_line() {
        let m = "did stuff\nPARTIAL: tests still red\nDONE: all good";
        assert_eq!(verdict(m), Some((Verdict::Done, "all good".into())));
        assert_eq!(verdict("nothing here"), None);
        assert_eq!(verdict("- **BLOCKED:** need key"), Some((Verdict::Blocked, "need key".into())));
    }

    #[test]
    fn parses_exec_events() {
        let l = r#"{"type":"item.completed","item":{"id":"1","type":"command_execution","command":"ls","aggregated_output":"","exit_code":0,"status":"completed"}}"#;
        match parse_line("p", "a", l) {
            Some(Out::Cmd { command, ok, .. }) => {
                assert_eq!(command, "ls");
                assert!(ok);
            }
            _ => panic!("cmd"),
        }
        let l = r#"{"type":"thread.started","thread_id":"abc"}"#;
        assert!(matches!(parse_line("p", "a", l), Some(Out::Started { thread_id, .. }) if thread_id == "abc"));
        assert!(parse_line("p", "a", "not json").is_none());
    }
}
