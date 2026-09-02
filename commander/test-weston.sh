#!/bin/sh
# Run commander inside a persistent weston (kiosk-shell, GL renderer).
# Weston is started once and left running so its X11 window stays stable
# under xmonad between test runs.
SOCK=${SOCK:-commander-test}
RUNDIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}

if ! { [ -S "$RUNDIR/$SOCK" ] && pgrep -f "weston.*$SOCK" >/dev/null; }; then
    echo "starting weston on $SOCK..."
    # --debug authorizes weston-screenshooter (wayland-native captures)
    nohup weston --backend=x11 --shell=kiosk-shell.so --renderer=gl --debug \
        --socket="$SOCK" --width=1500 --height=900 --idle-time=0 >/tmp/weston-$SOCK.log 2>&1 &
    sleep 2
fi

cd "$(dirname "$0")"
cargo build || exit 1
exec env -u DISPLAY WAYLAND_DISPLAY="$SOCK" ./target/debug/commander "$@"
