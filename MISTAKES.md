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
