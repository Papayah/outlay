#!/bin/sh
# Screenshot loop for outlay. It opens a window on the user's display: ask the user first, and
# keep the window open only as long as the captures take (see MISTAKES.md). Usage:
#   shot.sh start COLS ROWS -- outlay args...   launch a floating kitty running outlay
#   shot.sh keys TEXT                          send text (kitten send-text escapes: \r, \x1b)
#   shot.sh capture FILE                       capture the window as PNG
#   shot.sh stop
S="$XDG_RUNTIME_DIR/outlay-shot.sock"
BIN="$(cd "$(dirname "$0")/.." && pwd)/target/release/outlay"
nap() { python3 -c "import time; time.sleep($1)"; }
win() { xdotool search --class outlay-shot 2>/dev/null | tail -1; }
case "$1" in
start)
  cols=$2; rows=$3; shift 4
  rm -f "$S"
  kitty -o allow_remote_control=socket-only --listen-on "unix:$S" --class outlay-shot \
    -o font_size=11 -o remember_window_size=no -o confirm_os_window_close=0 \
    -e "$BIN" "$@" >/dev/null 2>&1 &
  for i in $(seq 1 50); do [ -S "$S" ] && [ -n "$(win)" ] && break; nap 0.1; done
  nap 0.3
  i3-msg '[class="outlay-shot"] floating enable, move position 40 40' >/dev/null
  nap 0.3
  kitten @ --to "unix:$S" resize-os-window --action resize --unit cells --width "$cols" --height "$rows"
  nap 1.0
  ;;
keys) kitten @ --to "unix:$S" send-text "$2"; nap 0.4 ;;
capture) w=$(win); import -window "$w" "$2" ;;
stop)
  kitten @ --to "unix:$S" close-window >/dev/null 2>&1
  nap 0.3
  pkill -f '^kitty -o allow_remote_control=socket-only --listen-on unix:/run/user/1000/outlay-shot' 2>/dev/null
  rm -f "$S"
  ;;
esac
