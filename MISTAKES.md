# Traps that cost time

Each entry: symptom → cause → the fix that worked. Append new ones; keep entries that are still true.

## Commits carry the work email

- **Symptom:** a commit shows the global (work) email as author or committer, and GitHub
  attributes it to the wrong account.
- **Cause:** the global git identity on this machine is the work one. A fresh clone or a new
  `git init` inherits it until a local identity is set.
- **Fix:** before the first commit in any clone, run
  `git config user.name "Papayah"` and `git config user.email "maciej.chmiest@gmail.com"`
  (local, never `--global`). After every commit, `git log -1 --format='%an <%ae> | %cn <%ce>'`
  must show Papayah and the gmail address twice. To repair the last commit:
  `git commit --amend --no-edit --reset-author`.

## Every commit prints a Docker "network is unreachable" error

- **Symptom:** `git commit` prints a Docker `Error response from daemon: … network is unreachable`
  and a Checkstyle banner.
- **Cause:** the global `core.hooksPath` (`~/.git-hooks`) runs a work Checkstyle pre-commit hook in
  every repository. Off the work network it cannot pull its image and falls back to a local one.
- **Fix:** nothing to fix. The hook finds no Java files and the commit succeeds; check
  `git log -1` instead of reading the noise as a failure.

## The live capture has 13 outputs, not 15

- **Symptom:** a test asserting 15 outputs and 13 disconnected fails on `laptop-edp-hdmi.txt`.
- **Cause:** the plan's environment notes say "13 other outputs are disconnected", but the providers
  report 7 + 6 = 13 outputs in total: eDP-1 and HDMI-1-0 connected, 11 disconnected.
- **Fix:** trust the capture (`xrandr --listproviders` shows the per-provider counts).

## `PROPTEST_CASES` does not raise the case count

- **Symptom:** `PROPTEST_CASES=20000 cargo test --test properties` still runs 400 cases.
- **Cause:** `tests/properties.rs` sets `cases` explicitly in `proptest_config`, which overrides the
  environment variable.
- **Fix:** for a long run, edit `cases: 400` temporarily and use `--release`
  (20 000 cases take about 4 s). Failing seeds land in `tests/properties.proptest-regressions`;
  keep that file committed, proptest replays it first.

## Golden command tests depend on the demo's XIDs

- **Symptom:** after editing `tests/fixtures/xrandr/demo.txt`, `tests/commands.rs` fails with
  different `--mode 0x…` values.
- **Cause:** the goldens hard-code the demo's mode XIDs (`0x1c3`, `0x1cb`, `0x1cf`, `0x1d6`). The
  synthetic fixtures were generated once by a throwaway script, and a mode's XID is shared by
  every output that lists the same timing.
- **Fix:** after changing the demo, read the new XIDs with `grep -n '(0x' tests/fixtures/xrandr/demo.txt`
  and update the goldens on purpose, never blindly.

## Clippy 1.91 and rustfmt defaults

- **Symptom:** clippy `-D warnings` fails on `n % 2 != 0` (`manual_is_multiple_of`) and on
  `Ok(expr.context(…)?)` (`needless_question_mark`); `cargo fmt --check` fails on lines over
  100 columns.
- **Cause:** Rust 1.91's clippy prefers `!n.is_multiple_of(2)` and returning the `Result` directly,
  and there is no `rustfmt.toml` (like `../split`), so rustfmt uses 100 columns.
- **Fix:** write those forms from the start, and run `cargo fmt --all` before clippy.

## A `cd` in one shell command moves every later command

- **Symptom:** a relative path such as `src/show.rs` is not found, or a command runs in
  `tests/fixtures/xrandr`.
- **Cause:** the working directory persists between tool shell calls, so an earlier `cd` sticks.
- **Fix:** use absolute paths, or `cd /home/mc2/workspace/this/outlay && …` in the same command.

## A display turns off in the screenshot window without any key sent

- **Symptom:** in a kitty window opened for screenshots, `--demo` shows DP-1-2 turned off and
  "3 pending" although the script sent no keys.
- **Cause:** the developer was typing on another i3 workspace; the new window took focus and got
  their keystrokes (a Space toggles the focused display).
- **Fix:** ask before opening any window on the developer's display, and close it right after
  the captures. Check rendering headlessly first: insta snapshots in `tests/tui.rs`, or
  `tools/pty_drive.py`. When a live window shows input you did not send, suspect typing first.

## kitty ignores `--listen-on` in the scratchpad

- **Symptom:** kitty shows "Invalid listen_on=unix:/tmp/claude-1000/…/outlay.sock, ignoring" and
  `kitten @` fails with `connect: invalid argument`.
- **Cause:** a unix socket path is limited to 108 bytes and the scratchpad path is longer.
- **Fix:** put the socket in `$XDG_RUNTIME_DIR` (`tools/shot.sh` uses
  `$XDG_RUNTIME_DIR/outlay-shot.sock`).

## `pkill -f` kills the tool call that runs it

- **Symptom:** a shell command exits with code 144 and nothing after `pkill` runs.
- **Cause:** `pkill -f "class outlay-shot"` matched the shell running the command, whose command
  line contains the same pattern.
- **Fix:** anchor the pattern to the program (`pkill -f '^kitty -o allow_remote_control=…'`) or
  close the window with `kitten @ close-window`, as `tools/shot.sh stop` does.

## `import -window "$WID"` says "missing an image filename"

- **Symptom:** `WID=$(xdotool search --class outlay-shot | tail -1); import -window "$WID" x.png`
  fails, although `xdotool getwindowgeometry $WID` works in the same command.
- **Cause:** not found. The same `import` works with the literal window id, and with the id in a
  variable of a `sh` script.
- **Fix:** use `tools/shot.sh capture FILE`.

## outlay in a bare pty is slow to start and misreads early keys

- **Symptom:** driven through Python's `pty`, the first frame takes about 2 s, and keys sent before
  it act strangely.
- **Cause:** `supports_keyboard_enhancement()` waits for a reply that only a real terminal gives
  (kitty answers at once; a bare pty never does), up to crossterm's 2 s timeout.
- **Fix:** wait at least 2.6 s before the first key (`tools/pty_drive.py` does). Remember that the
  countdown's 1 s input block starts when xrandr returns, not when the popup is first read back.

## Text that is on screen is missing from the pty output

- **Symptom:** `"Kept the new layout." in output` is false although the status line shows it.
- **Cause:** ratatui sends only the cells that changed, so a string arrives in pieces between
  cursor moves.
- **Fix:** feed the output to the `Screen` model in `tools/pty_drive.py` and search
  `SCREEN.text()`.

## SIGHUP made outlay abort instead of reverting and exiting

- **Symptom:** closing the terminal during a countdown ends outlay with SIGABRT (exit -6).
- **Cause:** ratatui's `init()` panic hook and `Terminal::drop` both call `eprintln!`, which panics
  when the terminal is gone (EIO); a panic inside the panic hook aborts.
- **Fix (in place):** `tui::run` sets up raw mode and the alternate screen by hand, its panic hook
  never prints, and the `Terminal` is `mem::forget`-ed when `show_cursor` fails. Never switch back
  to `ratatui::init`/`restore`, and never `eprintln!` on the exit path (`main` uses `writeln!` and
  ignores the error).

## There is no `cargo insta`

- **Symptom:** `cargo insta review` is not a command.
- **Cause:** cargo-insta is not installed; only the insta crate is.
- **Fix:** run `cargo test`, read each `tests/snapshots/*.snap.new`, and when it is right, accept it
  with `for f in tests/snapshots/*.snap.new; do mv "$f" "${f%.new}"; done`.

## A scripted text replacement did nothing, and nothing said so

- **Symptom:** a feature is missing although the edit "ran": `+` did nothing, and no error came
  from the Python `str.replace` that should have added its handler.
- **Cause:** `cargo fmt` had re-wrapped the anchor line since it was read, so the old text no
  longer matched, and `str.replace` silently changes nothing.
- **Fix:** `assert old in s` before each scripted replacement, or use the Edit tool, which fails
  loudly on a missing match. Re-read a region after `cargo fmt` before anchoring on it.

## `--rate 60.00` gives different modes on the demo and on the live capture

- **Symptom:** a profile test expecting HDMI-1-0 at 59.94 fails on the demo with 60.00, and the
  one expecting 60.00 fails on `laptop-edp-hdmi.txt` with 59.94.
- **Cause:** the demo's HDMI-1-0 lists an exact 1920x1080 60.00 mode; the live capture has only
  59.94 (and slower). The plan's "home-setup_save asks for 60.00 where only 59.94 exists" is
  about the live capture.
- **Fix:** check `outlay --demo list` / `--from-file … list` before writing rate expectations.

## Inferred links start from the primary

- **Symptom:** a test expects `eDP-1` stuck below `HDMI-1-0` after loading `tv-home`, but
  `eDP-1` has no link.
- **Cause:** inference roots each component at the primary (else the largest display), so the
  primary is an anchor and its neighbours are stuck to it: `HDMI-1-0` is above `eDP-1`.
- **Fix:** write link expectations from the primary outwards.

## Reusing the proptest edits panics on Undo

- **Symptom:** a new property panics with "handled by the history" in `tests/properties.rs`.
- **Cause:** `apply()` there treats `Edit::Undo`/`Edit::Redo` as unreachable; only the main
  property routes them to `History`.
- **Fix:** filter them out: `edits.iter().filter(|e| !matches!(e, Edit::Undo | Edit::Redo))`.

## A script cannot round-trip where an off output was

- **Symptom:** the layout → script → layout round trip fails on an output that is off, with a
  different `pos` or `mode`.
- **Cause:** a script says only `--off`; the loaded layout keeps the live state of that output.
- **Fix:** compare off outputs by their on/off state only (copy the expected `OutputState` over
  the loaded one when both are off), as `tests/properties.rs` does.

## `outlay apply` with a piped `y` reverts

- **Symptom:** `echo y | outlay --demo apply tv-home` reverts after the countdown.
- **Cause:** lines that arrive in the first second after xrandr returns are ignored, like keys in
  the editor, and a pipe delivers the `y` at once.
- **Fix:** write the `y` after more than a second (`tests/cli.rs` sleeps 1.3 s), or pass
  `--revert-timeout 0`.

## CLI tests could reach `~/.screenlayout`

- **Symptom:** none yet; a `save` test without `--layouts-dir` would write into the real
  `~/.screenlayout`, because the default config points there.
- **Cause:** `tests/cli.rs` hides the config with `XDG_CONFIG_HOME`, but `layouts_dir` defaults
  to `~/.screenlayout`.
- **Fix:** the helper also sets `HOME=/nonexistent/outlay-test-home`, and every profile test
  passes `--layouts-dir` to a temporary directory.

## A held key in the pty moves further than the ramp says

- **Symptom:** 30 `Alt-l` sent 40 ms apart through `tools/pty_drive.py hold` move eDP-1 1240 px,
  not the 800 px the ramp predicts, and the indicator shows ×10.
- **Cause:** `read_for(fd, 0.04)` polls in 50 ms steps, so the presses are further apart and the
  hold lasts longer than 1.2 s.
- **Fix:** assert the exact ramp in `tests/tui.rs` with `tick`; in the pty, check only that the
  indicator shows a multiplier and that one `u` undoes the hold.

## A flag in a zsh variable reaches outlay as one argument

- **Symptom:** in a shell loop, `extra="--revert-timeout 2"; tools/pty_drive.py timeout -- --demo $extra`
  prints `started: False` and `exit: 2`.
- **Cause:** the tool shell is zsh, which does not split an unquoted `$extra` into words, so clap
  gets the single argument `--revert-timeout 2` and exits with a usage error.
- **Fix:** write the flags out, use an array (`extra=(--revert-timeout 2)` … `$extra`), or run the
  loop with `bash -c`.

## The fake backend in `tests/apply.rs` is simulated

- **Symptom:** a status assertion fails with an extra
  `Simulated: nothing was sent to the X server.` in front of the expected text.
- **Cause:** `Fake` keeps the default `touches_x() == false`, so a countdown on it always says so
  first. `Session` still runs hooks on it: only `tui::confine`, which the tests do not call,
  clears them for a simulated backend.
- **Fix:** expect the prefix (or use `ends_with`), and count hook runs with a hook that appends to
  a file in a unique `std::env::temp_dir()` directory (the `Counter` helper).

## `pty_drive.py sighup` prints `exit: None`

- **Symptom:** `tools/pty_drive.py sighup -- --demo` ends with `exit: None`, on `main` as well
  (seen 2026-09-26 at f7a81b7). The process is still there 10 s after the pty master closes, in
  state `R`, using the CPU, instead of reverting and exiting.
- **Cause:** not found. There is no `strace` or `perf` here, and `kernel.yama.ptrace_scope = 1`
  stops `gdb -p` from attaching to a process that is not its child.
- **Fix:** none yet. To look at it, start outlay under gdb from the driver, or add a debug log to
  the event loop. When you exec `pty_drive.py` from another script, set `BIN` again afterwards:
  it comes from `__file__`, and a wrong path makes the child exit 1 at once.
- **Clean-up:** every run leaves that `outlay --demo` spinning, and it ignores SIGTERM (the flag
  is set, but the loop never reads it). After running the scenarios, `pgrep -x outlay -a`, then
  `kill -9` the `--demo` PIDs (they never touch a display). Do not use `pkill -f outlay`: it
  matches the Bash tool's own shell, whose command line holds the pattern, and kills it (exit
  144). Still the same on 2026-10-05 at `4acff97`.

## Does `xrandr --current` see a hotplug without a probe? Yes, here

- **Symptom:** none; this was an open question before the hotplug watch was built. The watch
  polls `xrandr --verbose --current` (about 5 ms) instead of a full probe (70–77 ms, which can
  wake the NVIDIA GPU).
- **Cause:** checked live on 2026-09-26 with a read-only loop of
  `xrandr --current | grep -E ' (dis)?connected'` every 0.5 s. `--current` saw HDMI-1-0
  (NVIDIA-G0) unplugged and plugged back twice, and DP-1-2 (NVIDIA-G0, USB-C) plugged and
  unplugged; every change showed up in the loop's log. The MST dock (`DP-2.x`) was not tried.
- **Fix:** nothing to fix. If a machine turns up where only a full `xrandr` sees the plug, the
  choice is a slower full probe or the manual `R`; ask the user.

## A layout rebuilt from the snapshot gets the wrong display numbers

- **Symptom:** after a hotplug, a profile preview, a loaded profile or a kept layout labels a
  display differently from the canvas, or `load_profile` says "is the layout you have already"
  when it is not.
- **Cause:** numbers stay fixed for the session, and a display connected later gets the lowest
  free number. `Layout::inferred` (and so `from_snapshot`, `from_states` and `Profile::layout`)
  numbers by xrandr order with `snap.number_of`, which differs once a refresh has renumbered.
- **Fix:** every layout the editor builds from the snapshot copies `self.layout.numbers`
  (`kept`, `load_profile`, `open_profiles` do). A new place that builds one must do the same.

## `Layout::inferred` normalises; `state_from_output` does not

- **Symptom:** a property comparing an output that came back after `remapped` with
  `Layout::inferred(&snap).outputs[k]` fails with a different `pos`.
- **Cause:** a random desk can have negative positions; `inferred` shifts the whole layout to
  0,0, but an output new to the list starts from its raw live state.
- **Fix:** compare with the live `ActiveConfig` (enabled, `pos`, mode XID) instead.

## A test clock taken from one `App` does not fit another

- **Symptom:** `app.tick(start + WATCH_INTERVAL)` returns no refresh for a second `App` in the
  same test.
- **Cause:** each `App` starts its watch at its own `now`, taken when it is created, a little
  later than the first one's `start`.
- **Fix:** take `let start = app.now;` again for every new `App`.

## The live editor in a pty is refused in auto mode

- **Symptom:** `tools/pty_drive.py SCENARIO --` (no `--demo`, the live X server) is denied by the
  auto-mode classifier, although a scenario that only presses `q` is read-only.
- **Cause:** the classifier judges it without explaining why.
- **Fix:** do not work around it. Test the event loop with `--demo`, and ask the user to run
  `cargo run --release` in their own terminal for the live check.

## A closure cannot return a reference into its parameter

- **Symptom:** `let find = |snap: &Snapshot, name: &str| snap.find(name).map(|i| &snap.outputs[i]);`
  fails with "lifetime may not live long enough".
- **Cause:** closure signatures do not get lifetime elision the way `fn` signatures do.
- **Fix:** use a nested `fn find<'a>(snap: &'a Snapshot, name: &str) -> Option<&'a Output>`.

## `git fetch` and `git push` hang: SSH to GitHub is blocked on the wired network

- **Symptom:** `git fetch origin` never returns (the tool call goes to the background after
  120 s); `ssh -T git@github.com` times out on port 22, and `ssh.github.com:443` times out
  during the banner exchange.
- **Cause:** the wired network behind the dock (default route `192.168.227.254`, on
  `enp0s13f0u1u1`) blocks SSH to GitHub, with or without the VPN; disconnecting the VPN did not
  help (2026-09-29). HTTPS works. Check the route with `ip route | head -2`.
- **Fix:** put a `timeout` on network commands, and go over HTTPS with gh as the credential
  helper, keeping `origin` as it is:
  `git -c url."https://github.com/".insteadOf=git@github.com: -c credential.helper= -c credential.helper='!gh auth git-credential' push -u origin BRANCH`.
  A push that changes a workflow file needs more (next entry). Over the user's mobile hotspot
  (default route on `wlan0`) SSH works: `ssh -T git@github.com` greets Papayah.

## An HTTPS push that touches `.github/workflows/` is rejected

- **Symptom:** `! [remote rejected] … (refusing to allow an OAuth App to create or update
  workflow .github/workflows/ci.yml without workflow scope)`.
- **Cause:** gh's token has the `repo` scope but not `workflow`, and GitHub requires `workflow`
  for any push that changes a workflow file. SSH pushes do not need it.
- **Fix:** push over SSH from a network that allows it (on 2026-09-29 the user switched to a
  mobile hotspot, and `git push` over SSH went through). Or the user runs
  `gh auth refresh -h github.com -s workflow` (a browser device flow, so it is theirs to run;
  suggest `! gh auth refresh -h github.com -s workflow`), and the HTTPS push above works.

## gnu.org is unreachable, and two local GPLv3 texts differ

- **Symptom:** `curl https://www.gnu.org/licenses/gpl-3.0.txt` times out. Local 674-line
  GPLv3 copies come in two versions.
- **Cause:** the work VPN blocks gnu.org; without the VPN it answers. The copies with
  `http://fsf.org/` (sha256 `8ceb4b9e…`) are the older revision; the current text uses `https://`
  and `licenses/why-not-lgpl.html`.
- **Fix:** `/usr/share/doc/bison/COPYING` (sha256 `3972dc97…`) is the current text. It is
  identical to `gh api licenses/gpl-3.0 --jq .body`, apart from the newline jq adds at the end,
  and `cmp` against gnu.org's `gpl-3.0.txt` (checked off the VPN) finds no difference.
  `LICENSE` in this repo is that file.

## shellcheck, actionlint, dash and busybox are not installed

- **Symptom:** none of them is on PATH, `/bin/sh` is bash, and installing packages needs sudo.
- **Fix:** fetch them into the scratchpad, without installing anything:
  - `gh release download -R koalaman/shellcheck --pattern '*linux.x86_64.tar.xz'` and
    `gh release download -R rhysd/actionlint --pattern '*linux_amd64.tar.gz'`, then unpack them.
    actionlint's flag is `-no-color` (`-color=never` is a parse error); pass
    `-shellcheck <path>` so it also checks the `run:` scripts.
  - dash and busybox: take the version from `pacman -Si dash busybox`, `curl -fsSLO` the
    `.pkg.tar.zst` from a server in `/etc/pacman.d/mirrorlist`
    (`$repo/os/x86_64/dash-<ver>-x86_64.pkg.tar.zst`; dash is in `core`, busybox in `extra`),
    and unpack it with `tar --zstd -xf`.
  - `OUTLAY_TEST_SH=<scratch>/usr/bin/dash cargo test --test install` runs the installer tests
    under that shell. For busybox, point it at a two-line wrapper that runs
    `exec …/busybox sh "$@"`.

## `kill -INT` of a background installer tests nothing

- **Symptom:** a SIGINT sent to `sh install.sh &` (even through `setsid`, to the process group)
  does not run the INT trap. The script goes on and fails on the interrupted download instead
  of exiting 130.
- **Cause:** a non-interactive shell starts background jobs with SIGINT ignored, and a signal
  that is ignored on entry cannot be trapped. SIGTERM is not affected.
- **Fix:** test SIGTERM with `kill -TERM -- -PGID`. For a real Ctrl-C, run the shell under
  Python's `pty.fork()` and write `\x03` to the master. The installer exited 130 and cleaned up
  under dash, bash and busybox.

## A zsh glob that matches nothing aborts the command

- **Symptom:** `rm -rf "$dir"/*` on an empty directory prints `no matches found` and removes
  nothing, and a test loop built on it measures the wrong thing.
- **Cause:** the tool shell is zsh, and an unmatched glob is an error there (see also the entry
  on `$extra`).
- **Fix:** write loops and test harnesses to a file and run them with `bash file`.

## Only the x86_64 musl binary is static-pie

- **Symptom:** none yet; a check that greps `file` output for `static-pie` would fail on
  aarch64.
- **Cause:** in the release run, `file` reported the x86_64 musl binary as `static-pie linked`,
  but the aarch64 one as `statically linked` (not PIE).
- **Fix:** match `static`, as `release.yml` does, and do not describe both as static-pie.

## `sleep` in a tool shell command prints "Too many arguments." and does not wait

- **Symptom:** a polling loop such as `for i in …; do gh pr checks …; sleep 30; done` finishes
  in seconds, with one `Too many arguments.` per iteration, while the checks are still pending.
- **Cause:** the tool harness blocks `sleep`, in a `run_in_background` command too: an
  `until …; do sleep 15; done` there spins and calls `gh` hundreds of times a minute.
- **Fix:** let a program do the waiting: `gh pr checks N --watch --interval 30` with
  `run_in_background`; the harness reports when it exits. `gh pr checks` exits 8 while checks
  are pending.

## A live pty run misses the countdown that a `--demo` run sees

- **Symptom:** `tools/pty_drive.py` on the live X server reports `countdown: False` and
  `reverted: False`, yet outlay exits 0 and the screens are back as they were.
- **Cause:** a live backend runs the `post_apply` hooks (feh here) after the apply, before the
  countdown starts, and again after the revert, before the status line changes. Each run takes
  about a second. `--demo` has no hooks, so the short sleeps that suit it are too short live.
- **Fix:** the `scale` scenario reads for 4 s after Enter and 12 s for a 10 s countdown, and
  checks the raw output as well as the screen. Run it with `PTY_DUMP=1` to see both screens.

## Proving a refactor keeps every xrandr command byte-identical

- **Symptom:** the goldens in `tests/commands.rs` cover a few layouts, and a refactor of the
  command path can change an untested case (stale output, panning, a profile with `--scale`).
- **Fix:** before the change, write a throwaway `tests/zz_golden_dump.rs`. For every fixture ×
  edit (toggle, rotate, reset scale, primary, rate, resolution, move, no primary) × screenlayout
  profile, it writes the apply, copy, script and revert text, the simulator outcome, the state
  afterwards, `mismatches` and the revert result to a file. Run it once on the old API and once
  on the new one, then `diff` the two files. In W2 this gave 236 cases, all identical. Delete the
  file before committing.
- **Note:** the Plan → argv → Plan round trip fails for the stale output in
  `active-disconnected.txt`, as it should: xrandr no longer lists the output's mode, and
  `argv_to_plan` refuses the mode, just as xrandr does.

## Binding a new key breaks the "unbound key" tests

- **Symptom:** after `x` became the scale picker, `another_key_ends_the_hold_and_the_run`
  (`tests/tui.rs`) failed, and `src/tui/keys.rs` asserted that `x` was unbound.
- **Cause:** both tests use a letter that happened to be free as "some unbound key".
- **Fix:** use another free key (`v` now) and update the keymap assertion. Before you bind a key,
  grep `tests/` and `keys.rs` for `char('<key>')` and `"<key>`.

## Growing a display that is centred below another moves it sideways

- **Symptom:** an expected rectangle after `set_scale` or a mode change is off by half the
  growth in x.
- **Cause:** in the demo, eDP-1 is stuck `below DP-1-2, centre`. When it grows from 1920 to
  2400 wide, it stays centred on DP-1-2 (centre x = 3200), so x goes from 2240 to 2000.
- **Fix:** work the expected position out from the link (`layout.link_text(i)`), not from the
  old x.

## An insta `.snap.new` changes `assertion_line`

- **Symptom:** accepting a `.snap.new` by renaming it also changes the header's
  `assertion_line`, which adds noise to a review whose point is a one-line change.
- **Cause:** insta writes the current line of the assertion. It ignores the header when it
  compares, so the old value still passes.
- **Fix:** diff the two files, then copy only the content lines into the old `.snap` (or set
  the header line back to the old value before renaming).

## Wayland's `90` is xrandr's `left`, and sway names it `270`

- **Symptom:** the plan expected X11 `right` ↔ Wayland `90`. A rotated head read back with the
  wrong turn; `swaymsg -t get_outputs` reports a head outlay set to `90` as `"270"`.
- **Cause:** the protocol's transforms turn counter-clockwise, like RandR (`wayland.xml`;
  XWayland's `wl_transform_to_xrandr` maps `90` to `RR_Rotate_90`, which xrandr calls `left`).
  sway's own config and IPC turn clockwise and invert the name
  (`sway/commands/output/transform.c`, `sway/ipc-json.c`).
- **Fix:** the table is `TRANSFORMS` in `src/wayland/mod.rs`, with a pixel-for-pixel test.
  Check a head with `wlr-randr --json` or `outlay dump`, never with `swaymsg`, and remember that
  `swaymsg output X transform 90` sets the protocol's `270`.

## wlroots custom modes are one virtual mode object that changes size

- **Symptom:** a headless head lists one mode with refresh 0; after an apply the same mode object
  reports another size.
- **Cause:** for a custom current mode, wlroots sends each client one virtual
  `zwlr_output_mode_v1`, never `preferred`, with no `refresh` event while the rate is 0. A new
  custom mode re-sends `size` and `refresh` on that same object (checked on the wire with sway
  1.12 / wlroots 0.20.2, and in the source of 0.17 and master).
- **Fix:** keep mode state per object and let events overwrite it. outlay marks a mode custom
  when the capture says `"custom": true` or the rate is 0, sends a mode only when it changes, and
  sends `set_custom_mode` when no mode object matches.

## On sway the new state arrives before `succeeded`

- **Symptom:** waiting for a `done` after `succeeded` timed out (a full second) on every apply.
- **Cause:** sway sends the head events and `done` first, then `succeeded`.
- **Fix:** count `done`s before sending `apply`, and after `succeeded` wait only while no new one
  has come (`Inner::settle` in `src/wayland/client.rs`).

## 1280x720 cannot tell truncation from rounding

- **Symptom:** a test meant to pin the logical-size rounding passed with either rule.
- **Cause:** 1280 and 720 divided by 1.25, 1.5 or 1.75 never leave a fraction above one half.
- **Fix:** use a custom 1366x768 mode (`capture::mode(1366, 768, 60_000, false, true)`): at 1.5
  it is 910x512 in `swaymsg -t get_outputs`, so sway truncates (wlroots'
  `wlr_output_effective_resolution`).

## `outlay restore` loses the connection: the compositor aborted

- **Symptom:** "Lost the connection to the compositor: … Broken pipe" from `sh revert.sh` in a
  live test; `Sway::log()` shows `head_send_state: Assertion 'found' failed`.
- **Cause:** a wlroots bug up to 0.20.2 (fixed upstream by 40640950, 2026-09-30, unreleased): a
  client that bound while a custom-mode head was disabled enables it again while another client
  still holds that head's old virtual mode, and the compositor aborts.
- **Fix:** nothing outlay can do for other clients. Tests close their own `WlrBackend` before
  they run `revert.sh`. On a broken pipe, read sway's log before debugging outlay.

## Headless sway accepts every `test`

- **Symptom:** no configuration made `test` answer `failed`: 40000x40000, every head off, scale
  0.01 all succeed.
- **Fix:** cover a rejected test with the fake backend (`Fake::answering` in `tests/apply.rs`).
  On a real GPU the result may differ.

## Headless sway 1.9 and 1.12 place the outputs the other way round

- **Symptom:** a live test that passed here failed in CI: it expected HEADLESS-2 at 0,0, and
  `the_editor_applies_waits_for_the_answer_and_reverts` found HEADLESS-1 there.
- **Cause:** sway 1.12 (here) places HEADLESS-2 at 0,0 and HEADLESS-1 at 1280,0; sway 1.9
  (ubuntu-24.04, CI) places HEADLESS-1 at 0,0. Neither order is a contract. outlay sorts heads by
  name, so display 1 is HEADLESS-1 wherever it sits.
- **Fix:** pin the positions before the first query when a test depends on them:
  `sway.swaymsg(&["output", "HEADLESS-2", "pos", "0", "0"])` (and `"$swaymsg" output … pos` in
  `tools/pty_wayland.sh`). Otherwise read positions from `connect(&sway).query()` before you write
  an expectation. Only CI runs sway 1.9.

## Live Wayland tests share one process

- **Symptom:** connecting with `Connection::connect_to_env()` from a test reaches the wrong
  compositor, or the developer's.
- **Cause:** tests run in parallel threads, each with its own headless sway; the environment is
  global (and `set_var` is unsafe in edition 2024).
- **Fix:** `WlrBackend::connect(&sway.socket)` in-process, and `sway.command(program)` for
  children, which sets `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY` and `SWAYSOCK` and removes `DISPLAY`.

## A green `cargo test` does not mean the live tests ran

- **Symptom:** `cargo test` passes with the same count whether sway runs or not.
- **Cause:** without `OUTLAY_TEST_SWAY` each live test prints "skipped" and passes.
- **Fix:** `OUTLAY_TEST_SWAY=$(command -v sway) cargo test --test wayland_live -- --nocapture`
  and check that no "skipped" line appears.

## Tracing the Wayland protocol

- **Symptom:** an apply fails or the connection drops, and outlay's message says little.
- **Fix:** `WAYLAND_DEBUG=1` traces outlay too (wayland-backend's Rust implementation honours
  it), and `WAYLAND_DEBUG=1 wlr-randr …` traces the reference client. In a live test, the child
  processes inherit the variable from `cargo test`.

## `wayland-sys` in `cargo tree` links nothing

- **Symptom:** `cargo tree -e features -i wayland-sys` lists it under `wayland-backend`.
- **Cause:** it is built with no features; only `client_system`/`dlopen` would link libwayland.
- **Fix:** check `ldd target/debug/outlay | grep wayland` (empty) and that no crate enables
  `wayland-client/system`; `release.yml` checks the static musl build.

## "HEADLESS-2 has a mode of 0x0" in CI only

- **Symptom:** `revert_sh_restores_the_layout_from_before` failed on ubuntu-24.04 (sway 1.9,
  wlroots 0.17.1) with "HEADLESS-2 has a mode of 0x0"; it passes with wlroots 0.20.2.
- **Cause:** wlroots 0.17 gives a client that binds while a custom-mode head is off a virtual mode
  and never sends its size (fixed upstream by wlroots 2c305337).
- **Fix:** `capture::output` in `src/wayland/capture.rs` leaves out a mode of no size; one zero
  side alone is still an error. A capture from such a client lists the off head with no modes.

## `wlr-randr` on ubuntu-24.04 has no `--json` and no `--version`

- **Symptom:** "wlr-randr: unrecognized option '--json'" in the CI log, and `wlr-randr --version`
  prints "failed to connect to display".
- **Cause:** noble ships wlr-randr 0.3.0, which has neither option and treats an unknown one as a
  request to connect.
- **Fix:** nothing to fix: `wlr_randr_and_outlay_dump_read_the_same` prints
  "skipped: wlr-randr --json" there and passes, so it runs only where wlr-randr is newer (here).
  The CI step `sway --version && wlr-randr --version || true` shows only the sway version; read the
  wlr-randr version from the apt "Setting up wlr-randr (…)" line.

## `cargo test` stops at the first test binary that fails

- **Symptom:** after a change that breaks two test files, `cargo test` reports only the first;
  the second shows up only after the first is fixed.
- **Fix:** `cargo test --no-fail-fast`, then filter the `test result` and `panicked` lines.

## `echo ===` fails in the tool shell

- **Symptom:** `(eval):1: == not found`, and the rest of the command does not run.
- **Cause:** zsh expands a word that starts with `=` (`=cmd` is the path of `cmd`).
- **Fix:** quote separators: `echo '---'`.

## Checking that the screenshot window closed

- **Symptom:** `pgrep -fa outlay-shot` after `tools/shot.sh stop` prints a line, as if the window
  were still open.
- **Cause:** pgrep matches the tool shell's own command line, which contains the pattern.
- **Fix:** `xdotool search --class outlay-shot | wc -l` (0 when closed).

## A temporary path that contains the word a test asserts is absent

- **Symptom:** `assert!(!stderr.contains("xrandr"))` failed although the warning was gone.
- **Cause:** the sandbox was named `outlay-install-no-xrandr-…`, and the installer prints its
  paths.
- **Fix:** assert on the whole message (`"on X11, outlay needs the xrandr program"`), not on a
  word.

## The details panel is open in a TUI test

- **Symptom:** a test pressed `i` to see the details panel, and the panel was gone.
- **Cause:** the panel is open by default when the screen is wide enough (120x40 is); `i` toggles
  it off.
- **Fix:** render without `i`. Keep panel lines under 30 columns: a longer one wraps (a Wayland
  transform label such as `flipped-90 (left, reflect x)` did, so the label became
  `flipped-90 (left)`).

## Command-line errors are lowercase

- **Symptom:** a test expected `Usage: :rotate …` in the status line and got `usage: :rotate …`.
- **Fix:** errors from `cmdline::parse` are shown as they are; only some app messages are
  capitalised. Read the string from `cmdline.rs` before writing an expectation.

## A new kanshi profile asks before it is written

- **Symptom:** after `wdesk<Enter>` on Wayland the config was unchanged; the mode was
  `UiMode::Overwrite`.
- **Cause:** appending a block changes an existing file, and every change to an existing file is
  shown as a diff first (a screenlayout script is a new file, so it is written at once).
- **Fix:** press `y` in session tests; from the CLI, `--force` without a terminal. A config that
  does not exist yet is written at once.

## kanshi's rules differ from xrandr's in small ways

- **Symptom:** a kanshi profile loaded with other modes or matches than expected.
- **Cause (kanshi 1.9.0, `main.c`):** `mode WxH` without `@R` takes the highest rate at that size,
  not the first listed one; a criteria is compared to the name with `strcmp` and to
  `Make Model Serial` with `fnmatch`, so `output DP-*` never matches DP-1 by name; matching is
  greedy, with the criteria that hold a `*` moved last; an output with neither `enable` nor
  `disable` keeps the head as it is.
- **Fix:** check against the source before changing `model/profile.rs` or `wayland/kanshi.rs`:
  `git clone --depth 1 --branch v1.9.0 https://gitlab.freedesktop.org/emersion/kanshi.git` and
  `git clone --depth 1 https://git.sr.ht/~emersion/libscfg` (the config syntax: `#` starts a
  comment only where a directive starts).

## Does kanshi undo an outlay apply? Not while the same heads stay connected

- **Symptom:** the question from the plan: kanshi re-applying its profile during the countdown.
- **Cause:** `match_and_apply` keeps the current profile while it still matches the connected
  heads, so another client's change on `done` does not trigger it. A hotplug that changes the set
  of heads, or `kanshictl reload`, applies the profile again.
- **Fix:** none needed. Verified live on 2026-10-03 (kanshi 1.9.0, sway 1.12, headless):
  `kanshi_lets_an_apply_stand_until_a_hotplug_or_a_reload` passes, and kanshi logs no line at all
  for outlay's apply. Run it with `OUTLAY_TEST_SWAY=$(command -v sway)
  OUTLAY_TEST_KANSHI=$(command -v kanshi) cargo test --test wayland_live -- kanshi --nocapture`.
- **Note:** kanshi applies its profile once at start even when the heads already match it, so the
  test's count of `' applied` lines starts at 1. The log lives in sway's runtime dir, which is
  deleted when `Sway` drops: to read it after a passing run, add a temporary
  `eprintln!("{}", kanshi.log())` at the end of the test.

## The wlroots abort, from the panic hook

- **Symptom:** see "`outlay restore` loses the connection": the same abort, but from a panic
  during the countdown, whose `revert.sh` runs `outlay restore` while the editor is still
  connected.
- **Fix:** since session F the panic hook calls `wayland::client::hang_up()` (a socket shutdown,
  never a close) before `revert.sh`. `hanging_up_first_lets_revert_sh_turn_a_custom_mode_head_back_on`
  in `tests/wayland_live.rs` reproduces the abort on sway 1.12 if you remove its `hang_up()` line.

## The kanshi live test cannot run in CI on Ubuntu 24.04

- **Symptom:** adding `kanshi` to the `wayland` job's `apt-get install` and setting
  `OUTLAY_TEST_KANSHI` makes `kanshi_lets_an_apply_stand_until_a_hotplug_or_a_reload` fail at
  `kanshictl reload`, instead of running or skipping.
- **Cause:** noble ships kanshi 1.5.1 (universe), built without IPC: the package holds only
  `/usr/bin/kanshi`, no `kanshictl`
  (https://packages.ubuntu.com/noble/amd64/kanshi/filelist). The test needs `kanshictl`, and
  it was verified on kanshi 1.9.0.
- **Fix:** none. On 2026-10-03 the user chose to keep kanshi out of CI, so the test runs locally
  only. Check a newer runner image's file list before you try again.

## A release tag waits for the merge

- **Symptom:** after `chore: release X.Y.Z` on a PR branch, `git tag -a vX.Y.Z` there tags a
  commit that never reaches `main`.
- **Cause:** PRs are rebase-merged, so GitHub writes new commits on `main` (PR #9: `ae36cc6` on
  the branch became `67fd7a5` on `main`). `v0.1.0` sits on `main`'s commit, not the branch's.
- **Fix:** open the release PR without a tag. After the merge, run `git switch main && git pull
  --ff-only`, check that `Cargo.toml` says `version = "X.Y.Z"` and that CI on `main` passed,
  then tag `main`'s head. The head need not be `chore: release X.Y.Z`: in 0.2.0 a
  `docs: record traps` commit came after it, and `v0.2.0` sits on that. `release.yml` checks the
  tag against `Cargo.toml` anyway. Push the tag only on the user's go-ahead (`CLAUDE.md`,
  "Releases"). Before 0.2.0, the bump also broke `script_of_the_demo`, which had the version
  written out. Goldens take it from `CARGO_PKG_VERSION` now; keep new ones that way.

## `sleep` suspends the machine

- **Symptom:** `sleep 20` in a Bash tool call prints `Too many arguments.` and does not wait.
- **Cause:** the shell is set up from the user's profile, and `~/.zsh_aliases` has
  `alias sleep='systemctl suspend'`. With an argument, `systemctl` refuses; a bare `sleep`
  would suspend the developer's machine.
- **Fix:** never type plain `sleep`. Use `/usr/bin/sleep 20` (or `command sleep 20`), and check
  `type <cmd>` when a common command behaves oddly.

## A Wayland capture's outputs are not in the JSON order

- **Symptom:** a test that expects mismatches for `tests/fixtures/wayland/demo.json` in file
  order (`HDMI-A-1`, `DP-3`, `eDP-1`) fails: they come out `eDP-1`, `DP-3`, `HDMI-A-1`.
- **Cause:** `wayland::capture::parse` sorts the outputs (`head_order`), and everything that walks
  `snap.outputs` (`mismatches`, `restore_mismatches`) follows that order.
- **Fix:** look the order up with `snap.outputs.iter().map(|o| &o.name)`, or run the test once and
  copy it, instead of reading it off the JSON.

## A `Fake` stderr already starting with `xrandr:` is printed twice

- **Symptom:** an "Apply failed" report reads `xrandr: xrandr: Configure crtc 2 failed`.
- **Cause:** `Session::apply` prefixes every X11 stderr line with `xrandr: `. The revert's
  "The revert failed: …" line does not.
- **Fix:** in an apply's `Next::Fails`, give the stderr without the prefix
  (`"Configure crtc 2 failed\n"`) when the test compares the whole report;
  `a_failed_apply_that_changed_the_screens_is_reverted` gets away with it because it uses
  `contains`.
