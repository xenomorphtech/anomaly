# Shared spec for context-management UI mocks

Every mock is one self-contained HTML file (vanilla HTML/CSS/JS, zero external deps, no fetch, no fonts from CDN — system font stacks only). Dark UI base, but each paradigm must have a **distinct visual identity** so it reads as a different product (accent palette, typography feel, density, chrome style).

## Required page structure

1. A slim top bar or corner badge with: paradigm name + one-sentence rationale.
2. A collapsible **"Try the workflows"** hint panel (small, out of the way, openable) listing the 3 demo flows and where to click.
3. A small fixed **strengths/weaknesses** note (e.g. behind an "ⓘ compare" toggle) — 2-3 bullets each, honest.
4. Link back to `index.html` (plain `<a href="index.html">`).

## The three workflows (ALL must actually work via JS local state)

1. **Capture** — a quick-capture affordance (button/hotkey/input always reachable). User types a thought; it lands *without forcing classification* (goes to an "unsorted" area, inline stream, floating card, etc. — whatever fits the paradigm). It must visibly appear.
2. **Agent report arrives** — a "▶ simulate agent report" control (and/or auto-fire ~8s after load, once). A new status event from `packet-tracer` arrives ("gate-probe run #4 complete: server pos unchanged after 3 moves — desync confirmed at handshake layer"). Show how the paradigm surfaces it: toast, stream entry, node pulse, pane update… It must NOT steal focus destructively.
3. **Orient for a decision** — the pending decision "Gate desync fix: restart-probe vs re-handshake" is visible somewhere. Clicking it opens the paradigm's **re-orientation mechanism**: the accumulated context (use `decisionContext` below) presented for fast re-reading (replay, resume card, thread scroll-back, node expansion, brief section…). Then two commit buttons (Choose restart-probe / Choose re-handshake); choosing visibly resolves the decision (status change, event appended).

## Shared dataset (inline this JS object in every file; may extend but not contradict)

```js
const WORLD = {
  now: "2026-08-22T14:30:00",
  projects: [
    { id: "albion", name: "Albion Automation", color: "#e0a458",
      status: "active", goal: "Fully autonomous gathering + market loop",
      agents: [
        { id: "packet-tracer", name: "packet-tracer", state: "working",
          task: "Instrument gate transition packets", lastReport: "12:41" },
        { id: "navigator", name: "navigator", state: "blocked",
          task: "Cross-map routing through gates", lastReport: "11:58",
          blockedOn: "decision-gate-desync" },
        { id: "market-bot", name: "market-bot", state: "idle",
          task: "Waiting on price feed refactor", lastReport: "09:12" }
      ],
      tasks: [
        { t: "Fix gate desync", state: "blocked" },
        { t: "Price feed refactor", state: "todo" },
        { t: "Resource route optimizer", state: "done" }
      ]},
    { id: "homelab", name: "Homelab Migration", color: "#6fb3d2",
      status: "active", goal: "Move all services from old NUC to Proxmox cluster",
      agents: [
        { id: "infra-agent", name: "infra-agent", state: "working",
          task: "Migrating Postgres volumes to new ZFS pool", lastReport: "14:07" }
      ],
      tasks: [
        { t: "Migrate Postgres volumes", state: "doing" },
        { t: "Move reverse proxy + certs", state: "todo" },
        { t: "Decommission old NUC", state: "todo" }
      ]},
    { id: "paper", name: "Anomaly Detection Paper", color: "#b58ee0",
      status: "deadline", goal: "Submit to conference — deadline Sep 5",
      agents: [],
      tasks: [
        { t: "Rerun eval on v2 dataset", state: "doing" },
        { t: "Write related-work section", state: "todo" },
        { t: "Address advisor comments", state: "blocked" }
      ]},
    { id: "apartment", name: "Apartment Hunt", color: "#7fc98a",
      status: "background", goal: "Find 2br under budget before Nov",
      agents: [],
      tasks: [
        { t: "Visit Riverside listing", state: "todo" },
        { t: "Compare commute times", state: "todo" }
      ]}
  ],
  captures: [ // unclassified quick captures
    { t: "idea: agents should emit confidence with every status report", ts: "13:22" },
    { t: "check if ZFS snapshot schedule survives migration", ts: "10:45" },
    { t: "landlord mentioned garage option?? follow up", ts: "yesterday" }
  ],
  events: [ // newest last
    { ts: "09:12", proj: "albion", agent: "market-bot", kind: "status", t: "Paused: price feed schema changed upstream, refactor needed" },
    { ts: "10:03", proj: "paper", kind: "note", t: "Advisor replied: eval section needs v2 dataset numbers" },
    { ts: "10:45", proj: null, kind: "capture", t: "check if ZFS snapshot schedule survives migration" },
    { ts: "11:20", proj: "albion", agent: "packet-tracer", kind: "status", t: "Gate-probe run #2: emitted 1 move, restarted client — server position UNCHANGED" },
    { ts: "11:58", proj: "albion", agent: "navigator", kind: "blocked", t: "Blocked: cannot route through gates until desync strategy chosen" },
    { ts: "12:41", proj: "albion", agent: "packet-tracer", kind: "status", t: "Gate-probe run #3: same result. Server never applied any move after gate transition" },
    { ts: "13:22", proj: null, kind: "capture", t: "idea: agents should emit confidence with every status report" },
    { ts: "14:07", proj: "homelab", agent: "infra-agent", kind: "status", t: "Postgres volume 2/5 migrated, ETA 40m for remainder" }
  ],
  decisions: [
    { id: "decision-gate-desync", proj: "albion", state: "pending",
      title: "Gate desync fix: restart-probe vs re-handshake",
      options: ["Restart-probe: auto-restart client after each gate to resync",
                "Re-handshake: reverse-engineer and replay the gate handshake packets"],
      due: "today" },
    { id: "decision-eval-scope", proj: "paper", state: "pending",
      title: "Eval scope: rerun full grid or only v2 delta",
      options: ["Full grid (2 days compute)", "v2 delta only (4h, weaker claim)"],
      due: "Aug 25" }
  ],
  decisionContext: [ // accumulated context behind the gate-desync decision, oldest first
    { ts: "Aug 19 16:10", t: "navigator first reported chars 'rubber-banding' to gate entrance after transitions" },
    { ts: "Aug 19 18:34", t: "hypothesis 1: movement-lock flag not cleared — DISPROVEN (flag clears in packet trace)" },
    { ts: "Aug 20 11:02", t: "hypothesis 2: reliable-seq collision after zone load — DISPROVEN (seqs contiguous)" },
    { ts: "Aug 20 15:47", t: "key insight: all movement after gate is client-side prediction; server acks contain no position echo" },
    { ts: "Aug 21 12:30", t: "restart experiment: emit 1 move, restart client, read authoritative resume position → server never moved the char" },
    { ts: "Aug 21 12:55", t: "conclusion: server drops move stream after gate until a fresh handshake occurs. Two viable fixes identified" },
    { ts: "Aug 22 12:41", t: "packet-tracer run #3 confirms: reproducible 3/3 times" }
  ],
  incomingReport: { ts: "14:31", proj: "albion", agent: "packet-tracer", kind: "status",
    t: "Gate-probe run #4 complete: server pos unchanged after 3 moves — desync confirmed at handshake layer" }
};
```

## Quality bar

- Looks like a real product screenshot, not a wireframe: realistic density, believable microcopy, hover states, smooth small transitions.
- No dead-looking placeholder lorem. Everything from WORLD.
- Keyboard shortcut for capture where it fits (e.g. `c`), but always also a visible button.
- Single file ≤ ~60KB. Works from `file://` (no modules, no fetch).
