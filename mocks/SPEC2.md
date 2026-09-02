# Round 2 spec — RTS-informed, multi-dimensional mocks

Read `/home/sdancer/anomaly/mocks/SPEC.md` FIRST: the WORLD dataset (inline verbatim + the extension below), page structure rules, quality bar, and self-containment rules (single file, vanilla JS, no external requests, file://-safe) all still apply. Round 2 mocks are numbered 11–15.

Round 1 built ten one-dimensional paradigms. Round 2 mocks are **converged products**: each one implements the full RTS core contract below, and fuses SEVERAL round-1 dimensions (listed per mock). These should feel like the real product, not a sketch — more depth than round 1, budget up to ~90KB per file.

## WORLD extension (add to the inlined dataset)

```js
WORLD.lastVisited = { albion: "13:05", homelab: "14:10", paper: "2026-08-21T18:00", apartment: "2026-08-19T09:30" };
// staleness tiers (vs now=14:30): warm <30m (homelab), cooling <2h (albion),
// cold <1d (paper), frozen >1d (apartment)
WORLD.sinceLastVisit = { // events accumulated while user was away, per project
  albion: 2, homelab: 1, paper: 1, apartment: 0
};
```

## RTS core contract — EVERY round-2 mock MUST implement all 7

1. **Glance surface ("minimap").** A fixed, always-visible, spatially-stable surface showing all 4 projects in the same positions at all times. Event pings appear ON it (at the project's position) and fade over ~30s; blocked agents and pending decisions show as persistent markers. Clicking a ping/marker jumps the main view there. Must be readable in a sub-second glance: color = project, shape/blink = severity.
2. **Constant-time navigation.** Keys `1`–`4` select/focus projects (visible rail with the numbers shown, also clickable). `Space` = jump to most recent unseen alert. `Esc` (or `0`) = return to previous view ("camera back" — keep a small breadcrumb showing where Space took you from). Double-press of the same number = expand/center that project. Show a compact key-hint bar.
3. **Non-modal alerts.** The simulated agent report (button + one-shot auto-fire ~8s) produces: ping on glance surface + small corner toast + unseen-badge increment. It must NEVER change or steal the main view. Space is the *offer* to jump.
4. **Idle/blocked-agent button.** A persistent button (SC2 idle-worker style) with a live count — initially 2 (market-bot idle, navigator blocked). Clicking cycles focus through those agents one by one, showing why each is idle/blocked and a suggested next action. Count updates when the decision commit unblocks navigator.
5. **Fog of staleness.** Every project visibly shows how long since you last visited it (use WORLD.lastVisited): warm = full color; cooling = slight desaturation + age label; cold/frozen = heavy desaturate/veil + prominent age. Visiting a project clears its fog (animate the clear) and its since-last-visit counter; leaving it starts accumulation again (simulated report to a non-focused project bumps that project's counter).
6. **Cheap micro-check-in loop.** Pressing `1 2 3 4` in sequence must be a genuinely useful ~10-second sweep: each stop leads with a **delta-first resume header** ("since you left 85m ago: 2 events — navigator blocked, probe #3 confirmed") before the full view. Warm projects = one-line delta; cold projects = offer the deep recovery (below).
7. **Recovery fallback for cold contexts + the decision flow.** Cold/frozen projects (paper, apartment) and the pending gate-desync decision open a deep re-read mechanism (memo / replay / briefing — per mock's style, using decisionContext for the decision). Decision commit still resolves it, unblocks navigator, decrements the idle/blocked count.

## Demo workflows (replace round 1's three — put these in the "Try the workflows" panel)

1. **Capture** — same as round 1: instant, unclassified, visible.
2. **Alert contract** — simulate report → watch ping/toast/badge only → `Space` jumps to it → `Esc` returns.
3. **The sweep** — press `1 2 3 4`: delta-first resume at each stop, fog clears as you visit.
4. **Idle button** — click through the 2 idle/blocked agents.
5. **Deep recovery + decision** — open the frozen Apartment or cold Paper project (deep re-read offer), and open the gate-desync decision → context re-read → commit.

## The five mocks and their fused dimensions

- `11-commander.html` — **Commander.** The literal RTS translation. Main viewport = pannable spatial map where each project is a "base" (buildings = tasks, units = agents with state animations, captures drift unattached); true minimap bottom-left with viewport rectangle; right-side "command card" for the selected base (tasks/agents/actions like an RTS unit panel); top resource bar reinterpreted (attention budget: # warm contexts, # pending decisions, # idle agents). Fuses: spatial canvas (03) + fleet board (01) + resume cards (10). Identity: dark strategy-game HUD, beveled panels, unit-selection green brackets.
- `12-hud-operations.html` — **Operations HUD.** The chrome never changes, only the center viewport does. Stable frame: left control-group rail (1–4 with fog + delta badges), top alert ticker, bottom strip = minimap + agent fleet bar + idle button, center = the selected project rendered in its best-fit view with view tabs: Brief / Thread / Board (all three implemented, switchable, per project, remembered per project). Fuses: living brief (08) + thread chat (05) + fluid board (07) + fleet (01) inside RTS chrome. Identity: broadcast/observer-mode esports HUD, sharp angles, thin accent lines.
- `13-patrol.html` — **Patrol.** Built around the macro cycle as a first-class object. `Tab` or `.` advances a rehearsed rotation: each stop is a full-screen resume card (delta-first, agents, next actions) with inline triage of that project's new items (assign/snooze/done) right on the card; unclassified captures and decisions queue at the end of the loop as the final stop; a patrol HUD shows cycle progress (○○●○) and per-project staleness meters that patrol visits reset. Minimap = the rotation ring itself, with pings on ring segments. Fuses: context stack (10) + inbox triage (04) + timeline replay (02, as the deep recovery for cold stops). Identity: focused single-card flow like round-1 #10 but faster, rhythm-forward, big staleness dials.
- `14-starmap.html` — **Starmap.** Semantic zoom — the minimap IS the interface. Fully zoomed out: the whole world on one screen, projects as star systems with orbiting agent bodies, pings flare in place; scroll/dblclick zooms INTO a system → its base view (tasks as planets/structures, agent detail, delta-first banner); zoom further onto the log → the project's event thread/timeline inline. `Esc` zooms back out one level; 1–4 fly between systems. One continuous space, zero navigation chrome. Fuses: spatial (03) + graph structure (06) + timeline (02). Identity: deep-space starmap, luminous, smooth camera easing (transform transitions).
- `15-warroom.html` — **War Room.** Interrupts on the left, truth in the middle, forces on the right. Left = triage queue where ALL arrivals land (reports/captures/decisions; never modal; triage actions inline); center = the selected project's maintained "state of the world" brief with append-only log (agent updates as diffs); right = production bar: every agent across projects as a queue slot with state/task/ETA + the idle button; bottom = thin minimap strip with pings. Dense keyboard layer: `j/k` move triage queue, `e` archive, hotkey hints everywhere. Fuses: inbox (04) + living briefs (08) + fleet (01) + TUI keyboard density (09). Identity: command-center, cool steel dark, tabular, small caps headers.

## Chrome (same as round 1, adjusted)

Top badge: paradigm name + one-line "which round-1 dimensions this fuses". Collapsible "Try the workflows" panel with the 5 flows above. ⓘ compare toggle (strengths/weaknesses). Link back to `index.html`.
