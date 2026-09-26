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
