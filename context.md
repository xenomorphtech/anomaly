● Round 2 — five RTS-informed hybrids. Try: press 1 2 3 4 to sweep, Space to jump to an alert, Esc to return, click the idle-agent button.

  › [file] index.html (10KB)
  › [file] 11-commander.html (76.2KB)
  › [file] 12-hud-operations.html (74.4KB)
  › [file] 13-patrol.html (63.8KB)
  › [file] 14-starmap.html (64.9KB)
  › [file] 15-warroom.html (69.4KB)

Round 2 delivered — five converged mocks (11–15), all verified (well-formed HTML, node --check clean, several exercised in headless Chromium with zero runtime errors), plus the updated gallery index with both rounds.

Unlike round 1's one-dimensional paradigms, every round-2 mock implements the same 7-point RTS contract, so what you're comparing now is how each arranges the dimensions, not whether they exist:

- A spatially-stable glance surface with event pings that fade in ~30s, plus persistent markers for blocked agents and pending decisions
- Constant-time navigation: 1–4 select projects (double-press expands), Space jumps to the latest unseen alert, Esc/0 is camera-back with a breadcrumb
- Non-modal alerts — the simulated report only pings, toasts, and badges; it never steals your view
- An SC2-style idle/blocked-agent button (starts at 2: market-bot idle, navigator blocked) that cycles through them with reasons and suggested next actions
- Fog of staleness in four tiers — homelab is warm, Albion cooling, the paper cold, the apd defrost when you visit
