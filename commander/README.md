# Commander

An RTS-style context HQ: map buildings are projects, pylons are goals, sensor
arrays are open questions, and units are real `codex exec` processes working
in a repository. Buildings can also run wasm programs on a budget.

Single user, multi instance: one **daemon** owns a space, any number of
**frontends** look at it.

```
commander/
  core/     commander-core   model, persistence, wire protocol; with the `host`
                             feature also the engine (wasm host, codex units)
  daemon/   commanderd       the backend: owns space.jsonl, runs the engine,
                             serves HTTP on COMMANDER_HTTP (127.0.0.1:7700)
  ui/       commander        the egui frontend: mirrors the daemon's world,
                             keeps camera/selection/rooms/toasts to itself
  mods/                      sample wasm building programs (.wat)
```

## Running

```sh
cargo build
./target/debug/commander          # attaches to http://127.0.0.1:7700, or starts
                                  # commanderd detached (same cwd) if nothing answers
./target/debug/commanderd         # or run the daemon yourself, e.g. under a supervisor
```

Env:

| var                  | who      | default                 | meaning                                  |
|----------------------|----------|-------------------------|------------------------------------------|
| `COMMANDER_SPACE`    | daemon   | `space.jsonl`           | the space file (relative to the cwd)     |
| `COMMANDER_HTTP`     | daemon   | `127.0.0.1:7700`        | daemon listen address                    |
| `COMMANDER_CODEX`    | daemon   | `codex`                 | codex binary for units                   |
| `COMMANDER_API`      | frontend | `http://127.0.0.1:7700` | daemon to attach to                      |
| `COMMANDER_UI_HTTP`  | frontend | `127.0.0.1:7701`        | this frontend's control api              |
| `COMMANDER_SPAWN`    | frontend | `1`                     | `0`: never start a daemon                |

Wasm module paths in a space are relative to the daemon's working directory.
Closing a frontend never stops the daemon: units keep working, modules keep
ticking, the space keeps saving. `/shutdown` (or SIGTERM) saves and stops it.

Two spaces at once: two daemons on two ports, a frontend on each
(`COMMANDER_API=http://127.0.0.1:7790 COMMANDER_UI_HTTP=127.0.0.1:7791`).

## How the halves talk

The daemon publishes a versioned `Snapshot` (world, staleness, module status,
running units, codex usage, notices) whenever anything changes; frontends
long-poll `GET /snapshot?since=N`. Everything that changes the world is one
HTTP command (`core/src/proto.rs` lists them all — `/place`, `/pylon`,
`/struct`, `/dispatch`, `/tell`, …). Replies carry `"v"`, the version of the
snapshot that includes the change, so a frontend knows when its optimistic
local edit (a drag, a brief being typed) has landed and can stop overlaying it.

Toasts and map pings for anything that touched the world come from the
daemon as *notices*, so every attached frontend sees them; only view-local
feedback ("select a base first", the destroy confirmation) stays in the
frontend.

The frontend's control api answers `/key /text /click /band /state` itself
(`/state` merges its view — selection, camera, open room — into the daemon's
document) and forwards every other path to the daemon, so `test-api.sh` drives
both through one port.

## Testing headless

```sh
./test-weston.sh                       # weston kiosk + frontend (+ daemon if needed)
./test-api.sh                          # drive it through 127.0.0.1:7701
env -u DISPLAY WAYLAND_DISPLAY=commander-test weston-screenshooter
```

Keep the real space untouched by pointing a test daemon elsewhere:
`COMMANDER_SPACE=/tmp/x/space.jsonl COMMANDER_HTTP=127.0.0.1:7790 commanderd`.
