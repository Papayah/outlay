#!/bin/sh
# The pty scenarios against headless sway: outlay runs in a pseudo-terminal, keeps one apply and
# lets another revert, and swaymsg checks the outputs after each.
#
# Nothing opens a window: sway runs headless, without DISPLAY or WAYLAND_DISPLAY, in a private
# runtime directory. outlay gets temporary config and state directories, so no post_apply hook
# runs and the real ~/.local/state/outlay/revert.sh is left alone.
#
# usage: tools/pty_wayland.sh
#   OUTLAY_TEST_SWAY  the sway binary (default: sway on PATH)
#   OUTLAY_BIN        the outlay binary (default: target/release/outlay)
set -eu

here=$(cd "$(dirname "$0")" && pwd)
sway=${OUTLAY_TEST_SWAY:-$(command -v sway)}
swaymsg=$(dirname "$sway")/swaymsg
rt="${XDG_RUNTIME_DIR:-/tmp}/outlay-pty-$$"
mkdir -m 0700 "$rt"
pid=
cleanup() {
    [ -n "$pid" ] && kill "$pid" 2>/dev/null
    rm -rf "$rt"
}
trap cleanup EXIT

: > "$rt/config"
(
    unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET SWAYSOCK
    export XDG_RUNTIME_DIR="$rt" WLR_BACKENDS=headless WLR_RENDERER=pixman \
        WLR_HEADLESS_OUTPUTS=2 WLR_LIBINPUT_NO_DEVICES=1
    exec "$sway" -c "$rt/config"
) > "$rt/sway.log" 2>&1 &
pid=$!

tries=0
while :; do
    display=$(cd "$rt" && ls -d wayland-* 2>/dev/null | grep -v '\.lock$' | head -n 1 || true)
    ipc=$(cd "$rt" && ls -d sway-ipc.*.sock 2>/dev/null | head -n 1 || true)
    if [ -n "$display" ] && [ -n "$ipc" ] &&
        SWAYSOCK="$rt/$ipc" "$swaymsg" -t get_version > /dev/null 2>&1; then
        break
    fi
    tries=$((tries + 1))
    if [ "$tries" -gt 100 ]; then
        echo "sway did not start:" >&2
        cat "$rt/sway.log" >&2
        exit 1
    fi
    sleep 0.05
done

unset DISPLAY
export XDG_RUNTIME_DIR="$rt" WAYLAND_DISPLAY="$display" SWAYSOCK="$rt/$ipc"
export XDG_CONFIG_HOME="$rt/home/config" XDG_STATE_HOME="$rt/home/state"
export OUTLAY_BIN="${OUTLAY_BIN:-$here/../target/release/outlay}"

# The x of an output's logical rectangle, as sway reports it.
x_of() {
    "$swaymsg" -t get_outputs -r |
        python3 -c 'import json, sys
print([o["rect"]["x"] for o in json.load(sys.stdin) if o["name"] == sys.argv[1]][0])' "$1"
}

failed=0
expect() {
    if [ "$2" = "$3" ]; then
        echo "$1: True"
    else
        echo "$1: False (got $2, expected $3)"
        failed=1
    fi
}

# Display 1 is HEADLESS-1, right of HEADLESS-2; Alt-l nudges it 10 px to the right.
expect "HEADLESS-1 starts at" "$(x_of HEADLESS-1)" 1280
python3 "$here/pty_drive.py" keep --display 1 -- || failed=1
expect "kept at" "$(x_of HEADLESS-1)" 1290
expect "revert.sh hands a capture to outlay restore" \
    "$(grep -c "restore - <<'OUTLAY-CAPTURE'" "$XDG_STATE_HOME/outlay/revert.sh")" 1
python3 "$here/pty_drive.py" timeout --display 1 -- --revert-timeout 2 || failed=1
expect "reverted to" "$(x_of HEADLESS-1)" 1290

if [ "$failed" -ne 0 ]; then
    echo "sway's log:" >&2
    cat "$rt/sway.log" >&2
fi
exit "$failed"
