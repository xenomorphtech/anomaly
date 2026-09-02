#!/bin/sh
# Smoke-test commander through its HTTP control API (no real input needed).
# Prereq: app running (see test-weston.sh); API on COMMANDER_HTTP, default:
A=${A:-http://127.0.0.1:7700}
set -e

echo "== reset check (state):"
curl -sf $A/state | head -c 200; echo

echo "== place two bases and link them:"
curl -sf "$A/place?x=900&y=600&name=albion"; echo
curl -sf "$A/place?x=1500&y=950&name=homelab"; echo
curl -sf "$A/link?a=0&b=1"; echo

echo "== prompt path (dblclick ground -> type -> enter):"
curl -sf "$A/click?x=700&y=1200&double=1&world=1"; echo
sleep 0.3
curl -sf "$A/text?s=paper"; echo
sleep 0.3
curl -sf "$A/key?k=Enter"; echo
sleep 0.5

echo "== assert links present:"
curl -sf $A/state | grep -o '"links":\[[^]]*\]'

echo "== wayland-native screenshot (weston must run with --debug):"
env -u DISPLAY WAYLAND_DISPLAY=${SOCK:-commander-test} weston-screenshooter || true
