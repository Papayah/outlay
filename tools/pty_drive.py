#!/usr/bin/env python3
"""Drive outlay inside a pseudo-terminal: no window on the user's display.

usage: tools/pty_drive.py SCENARIO -- outlay args...     (build with cargo build --release)

Scenarios: keep, timeout (pass --revert-timeout 2), sigterm, sighup, keys, hold (a held Alt-l;
PTY_DUMP=1 prints the screen), profiles (pass --layouts-dir with copies of tv-home.sh and
home-setup.sh; it saves desk.sh there). With --demo or -n nothing reaches the X server. Without them the
apply is real: ask the user first.
"""
import fcntl
import os
import pty
import re
import select
import signal
import struct
import sys
import termios
import time

BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "outlay")


def spawn(args, cols=110, rows=32, env=None):
    pid, fd = pty.fork()
    if pid == 0:
        os.execvpe(BIN, [BIN] + args, env or os.environ)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    os.kill(pid, signal.SIGWINCH)
    return pid, fd


class Screen:
    """Just enough VT100 for ratatui output: cursor moves, clears and text."""

    def __init__(self, cols=110, rows=32):
        self.cols, self.rows = cols, rows
        self.grid = [[" "] * cols for _ in range(rows)]
        self.x = self.y = 0

    def feed(self, text):
        i = 0
        while i < len(text):
            c = text[i]
            if c == "\x1b" and i + 1 < len(text) and text[i + 1] == "[":
                j = i + 2
                while j < len(text) and not ("@" <= text[j] <= "~"):
                    j += 1
                params, final = text[i + 2:j], text[j:j + 1]
                if final == "H":
                    parts = [p for p in params.split(";")] if params else ["1", "1"]
                    self.y = int(parts[0] or 1) - 1
                    self.x = int(parts[1] or 1) - 1 if len(parts) > 1 else 0
                elif final == "J" and params in ("2", ""):
                    self.grid = [[" "] * self.cols for _ in range(self.rows)]
                elif final == "K":
                    for x in range(self.x, self.cols):
                        self.grid[self.y][x] = " "
                i = j + 1
            elif c == "\x1b":
                # ESC ] ... BEL (OSC) or a two-byte escape
                if i + 1 < len(text) and text[i + 1] == "]":
                    j = text.find("\x07", i)
                    i = len(text) if j < 0 else j + 1
                else:
                    i += 2
            elif c == "\r":
                self.x = 0
                i += 1
            elif c == "\n":
                self.y = min(self.y + 1, self.rows - 1)
                i += 1
            elif c >= " ":
                if 0 <= self.y < self.rows and 0 <= self.x < self.cols:
                    self.grid[self.y][self.x] = c
                self.x += 1
                i += 1
            else:
                i += 1

    def text(self):
        return "\n".join("".join(r).rstrip() for r in self.grid)


SCREEN = Screen()


def read_for(fd, seconds):
    out = b""
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            out += data
    text = out.decode("utf-8", "replace")
    SCREEN.feed(text)
    return text


def send(fd, text, pause=0.3):
    os.write(fd, text.encode())
    return read_for(fd, pause)


def wait(pid, seconds=3.0):
    end = time.time() + seconds
    while time.time() < end:
        done, status = os.waitpid(pid, os.WNOHANG)
        if done:
            return os.waitstatus_to_exitcode(status)
        time.sleep(0.05)
    return None


def main():
    scenario = sys.argv[1]
    args = sys.argv[sys.argv.index("--") + 1:]
    pid, fd = spawn(args)
    screen = read_for(fd, 2.6)
    print("started:", "layout" in screen and "outlay" in screen)
    if scenario == "keep":
        send(fd, "3\x1bl")          # focus eDP-1, Alt-l nudge
        out = send(fd, "a", 0.5)
        print("confirm popup:", "Apply" in out and "Changes" in out)
        send(fd, "\r", 0.3)
        print("countdown:", "Keep this layout?" in SCREEN.text())
        send(fd, "y", 0.3)          # ~0.6 s after xrandr returned: must be ignored
        print("y ignored during the block:", "Keep this layout?" in SCREEN.text())
        read_for(fd, 0.8)
        send(fd, "y", 0.5)
        print("kept:", "Kept the new layout." in SCREEN.text())
        send(fd, "q", 0.3)
        print("exit:", wait(pid))
    elif scenario == "timeout":
        send(fd, "3\x1bl")
        send(fd, "a", 0.5)
        send(fd, "\r", 0.5)
        out = read_for(fd, 3.5)     # run with --revert-timeout 2
        print("reverted:", "No answer in 2 s" in out)
        send(fd, "q", 0.5)
        print("asks to quit with pending edits:", "Discard 1 pending change and quit?" in SCREEN.text())
        send(fd, "\r", 0.3)
        print("exit:", wait(pid))
    elif scenario == "sigterm":
        send(fd, "3\x1bl")
        send(fd, "a", 0.5)
        out = send(fd, "\r", 1.0)
        print("countdown:", "Keep this layout?" in out)
        os.kill(pid, signal.SIGTERM)
        out = read_for(fd, 1.0)
        print("left the alternate screen:", "\x1b[?1049l" in out)
        print("exit:", wait(pid))
    elif scenario == "sighup":
        send(fd, "3\x1bl")
        send(fd, "a", 0.5)
        send(fd, "\r", 1.0)
        os.close(fd)                # the terminal goes away
        print("exit:", wait(pid))
    elif scenario == "keys":
        out = send(fd, "?", 0.5)
        print("help:", "Layout" in out and "Snap-move" in out)
        send(fd, "\x1b", 0.3)
        out = send(fd, "s", 0.5)
        print("stick hints:", "target" in out or "side" in out)
        send(fd, "\x1b", 0.3)
        out = send(fd, "\x1b", 0.3)
        print("Esc does not quit:", wait(pid, 0.3) is None)
        send(fd, "q", 0.3)
        print("exit:", wait(pid))
    elif scenario == "hold":
        # Alt-l every 40 ms for 1.2 s, as a key held with a 25 Hz autorepeat sends it.
        send(fd, "3", 0.3)
        for _ in range(30):
            os.write(fd, b"\x1bl")
            read_for(fd, 0.04)
        read_for(fd, 0.05)
        held = SCREEN.text()
        if os.environ.get("PTY_DUMP"):
            print(held)
        step = [l.strip() for l in held.splitlines() if "step 10px" in l]
        print("step indicator:", step[0] if step else None)
        pos = re.search(r"pos +([0-9,]+ → [0-9,]+)", held)
        print("moved:", pos.group(1) if pos else None)
        read_for(fd, 0.5)
        print("multiplier gone after release:", "×" not in SCREEN.text())
        send(fd, "u", 0.3)
        print("one undo restores it:", "no changes" in SCREEN.text())
        send(fd, "q", 0.3)
        print("exit:", wait(pid))
    elif scenario == "profiles":
        send(fd, "e", 0.5)
        print("picker:", "profiles · " in SCREEN.text())
        send(fd, "j\r", 0.5)
        print("opened:", "Opened tv-home" in SCREEN.text())
        send(fd, "w", 0.3)
        print("save prompt:", "save as tv-home" in SCREEN.text())
        send(fd, "\x7f" * 7 + "desk\r", 0.5)
        print("saved:", "Saved " in SCREEN.text())
        send(fd, ":e home-setup\r", 0.5)
        print("remap:", "Choose where each one goes" in SCREEN.text())
        send(fd, "\r", 0.5)
        print("opened after remap:", "Opened home-setup" in SCREEN.text())
        if os.environ.get("PTY_DUMP"):
            print(SCREEN.text())
        send(fd, "q", 0.3)
        send(fd, "\r", 0.3)
        print("exit:", wait(pid))
    else:
        print("unknown scenario")
        os.kill(pid, signal.SIGKILL)


main()
