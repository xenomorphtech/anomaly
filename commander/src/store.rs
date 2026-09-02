//! JSONL persistence for the space: one record per line.
//!
//! The file (COMMANDER_SPACE, default "space.jsonl") is loaded at startup and
//! rewritten whole on autosave — small enough that a full rewrite stays cheap.

use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ProjectRec {
    #[serde(flatten)]
    pub project: Project,
    /// absolute minute of the commander's last visit (0 = unknown → treated as now)
    #[serde(default)]
    pub last_visit_min: f64,
}

/// world-level display preferences, persisted with the space
#[derive(Clone, Serialize, Deserialize)]
pub struct Prefs {
    /// size ratio applied to substructures (pylons, sensor arrays)
    #[serde(default = "d_struct_scale")]
    pub struct_scale: f32,
    /// left-side base enumeration rail (hidden by default)
    #[serde(default)]
    pub show_rail: bool,
}

fn d_struct_scale() -> f32 {
    1.5
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs { struct_scale: d_struct_scale(), show_rail: false }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Rec {
    Project(ProjectRec),
    Link(Link),
    Decision(Decision),
    Capture(CaptureNote),
    Event(Event),
    Prefs(Prefs),
}

/// self-contained snapshot of a destroyed base, appended to the archive file
#[derive(Serialize)]
pub struct ArchivedBase {
    pub t: &'static str, // "archived_base"
    pub ts: String,
    pub project: Project,
    pub last_visit_min: f64,
    pub decisions: Vec<Decision>,
    pub events: Vec<Event>,
}

pub fn path() -> String {
    std::env::var("COMMANDER_SPACE").unwrap_or_else(|_| "space.jsonl".into())
}

pub fn archive_path(space_path: &str) -> String {
    match space_path.strip_suffix(".jsonl") {
        Some(stem) => format!("{}.archive.jsonl", stem),
        None => format!("{}.archive", space_path),
    }
}

pub fn append_archive(path: &str, rec: &ArchivedBase) -> std::io::Result<()> {
    use std::io::Write;
    let mut line = serde_json::to_string(rec)?;
    line.push('\n');
    std::fs::OpenOptions::new().create(true).append(true).open(path)?.write_all(line.as_bytes())
}

pub fn load(path: &str) -> Option<(World, Vec<f64>, Prefs)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut world = initial_world();
    let mut visits = vec![];
    let mut prefs = Prefs::default();
    for (ln, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Rec>(line) {
            Ok(Rec::Project(pr)) => {
                visits.push(pr.last_visit_min);
                world.projects.push(pr.project);
            }
            Ok(Rec::Link(l)) => world.links.push(l),
            Ok(Rec::Decision(d)) => world.decisions.push(d),
            Ok(Rec::Capture(c)) => world.captures.push(c),
            Ok(Rec::Event(e)) => world.events.push(e),
            Ok(Rec::Prefs(p)) => prefs = p,
            Err(e) => eprintln!("{}:{}: skipping bad record: {}", path, ln + 1, e),
        }
    }
    Some((world, visits, prefs))
}

pub fn save(path: &str, world: &World, rt: &[Rt], prefs: &Prefs) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str(&serde_json::to_string(&Rec::Prefs(prefs.clone()))?);
    out.push('\n');
    for (i, p) in world.projects.iter().enumerate() {
        let rec = Rec::Project(ProjectRec {
            project: p.clone(),
            last_visit_min: rt.get(i).map(|r| r.last_visit_min).unwrap_or(0.0),
        });
        out.push_str(&serde_json::to_string(&rec)?);
        out.push('\n');
    }
    for l in &world.links {
        out.push_str(&serde_json::to_string(&Rec::Link(l.clone()))?);
        out.push('\n');
    }
    for d in &world.decisions {
        out.push_str(&serde_json::to_string(&Rec::Decision(d.clone()))?);
        out.push('\n');
    }
    for c in &world.captures {
        out.push_str(&serde_json::to_string(&Rec::Capture(c.clone()))?);
        out.push('\n');
    }
    for e in &world.events {
        out.push_str(&serde_json::to_string(&Rec::Event(e.clone()))?);
        out.push('\n');
    }
    // write-then-rename so a crash mid-write can't corrupt the space file
    let tmp = format!("{}.tmp", path);
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, path)
}
