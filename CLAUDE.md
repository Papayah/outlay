# outlay — notes for coding sessions

Keyboard-driven monitor layout editor for X11 (xrandr) and wlroots Wayland compositors (Rust
2024, ratatui). `docs/PLAN.md` is the design brief and
the source of truth for behaviour; read `MISTAKES.md` before starting work.
`docs/PLAN-wayland.md` plans the Wayland work (neutral backend, wlroots, kanshi, scale editing)
phase by phase; where the two differ, it wins. `ROADMAP.md` lists the future work that has no
session yet.

## Commands

Every commit must pass all three, exactly as CI runs them:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Run modes

- `cargo run -- --demo <cmd>`: built-in four-output fixture (`tests/fixtures/xrandr/demo.txt`).
  Never touches X.
- `cargo run -- --demo=wayland <cmd>`: the same desk as a Wayland capture
  (`tests/fixtures/wayland/demo.json`). Never touches a display.
- `cargo run -- --from-file tests/fixtures/xrandr/<name>.txt <cmd>`: reads an
  `xrandr --verbose` capture. Never touches X.
- `cargo run -- --from-file tests/fixtures/wayland/<name>.json <cmd>`: reads a `wlr-randr --json`
  or `outlay dump` capture. Never touches a display.
- `cargo run -- list`, `cargo run -- show`, `cargo run -- dump`: query the live display server
  read-only. `dump` prints what `--from-file` reads (`xrandr --verbose`, or JSON on Wayland).
- `tools/pty_drive.py SCENARIO -- --demo`: runs the release build in a pseudo-terminal (no window)
  and checks the rendered screen; the way to test the event loop, signals and the apply flow.
- `OUTLAY_TEST_SWAY=$(command -v sway) cargo test --test wayland_live`: the Wayland backend
  against headless sway (no window, a private runtime dir). Without the variable these tests
  print "skipped" and pass. `OUTLAY_TEST_KANSHI=$(command -v kanshi)` adds the kanshi interplay
  test.
- `OUTLAY_BIN=$PWD/target/release/outlay tools/pty_wayland.sh`: the pty scenarios against
  headless sway, with temporary config and state dirs; it needs `OUTLAY_BIN` set to a built
  binary (default `target/release/outlay`).
- `tools/shot.sh`: the screenshot loop for `docs/screenshots/`. It opens a kitty window on the
  developer's display: ask first, every time.

## Rules

- **Live displays.** The developer's screens are live. Never run an xrandr command that changes
  state (`--output`, `--auto`, `--off`, a layout script, an outlay apply) without asking first.
  Always allowed: `xrandr --verbose`, `--query`, `--current`, `--listmonitors`, `--listproviders`.
  Tests never call the real `xrandr`; they use `FixtureBackend` or captured fixtures.
- **`~/.screenlayout` is read-only.** Copy scripts into `tests/fixtures/screenlayout/`, and point
  test saves at temporary directories.
- **Never read or write `~/.config/kanshi/`.** kanshi configs for tests live in
  `tests/fixtures/kanshi/`; tests copy them into temporary directories and always pass
  `--kanshi-config` (or `Settings.kanshi_config`) with a temporary path. A live kanshi runs only
  through the headless sway harness, with `-c` and a temporary config.
- **Headless sway** may run without asking, but only the way `tests/common/sway.rs` and
  `tools/pty_wayland.sh` run it: `WLR_BACKENDS=headless`, without `DISPLAY` or `WAYLAND_DISPLAY`,
  in a short private runtime dir. Nested sway (`WLR_BACKENDS=x11`) opens a window: ask first.
- **Git identity.** Author and committer must be `Papayah <maciej.chmiest@gmail.com>`. The global
  git identity is a work address that must never appear in this repo. The local config is set in
  `.git/config`; check `git config user.email` before committing and
  `git log -1 --format='%an <%ae> | %cn <%ce>'` after every commit.
- **Keys.** The keymap table in `src/tui/keys.rs` is the only source of keys. Dispatch, the hint
  line, `?` help and `outlay keys` are all generated from it; never hard-code a key elsewhere.
- **Commits.** Conventional commits (`feat:`, `fix:`, `docs:` …), one commit per plan phase.
- **Releases.** Bump `version` in `Cargo.toml` and run `cargo check` to update `Cargo.lock`, commit
  `chore: release X.Y.Z`, then `git tag -a vX.Y.Z -m "outlay X.Y.Z"`. Push the tag **only on the
  user's go-ahead**: it publishes a GitHub release that every `curl | sh` install then picks up.
  The asset names are a contract with `install.sh` (`docs/PLAN.md`, "Distribution").
- **`install.sh` stays POSIX sh.** No `local`, no arrays, no GNU-only flags; `shellcheck -s sh`
  must pass (CI runs it). Here `/bin/sh` is bash, so `OUTLAY_TEST_SH=/path/to/dash cargo test
  --test install` runs the installer tests under another shell.

## Layout

- `src/backend/`: the `Backend` trait (`query`, `requery`, `test`, `apply`, `dump`, `is_live`),
  `Verdict`, `ApplyOutcome`, and the command text per kind; `plan.rs` the neutral `Plan` an
  apply carries out;
  `fixture.rs` `FixtureBackend` (simulates a plan on a capture, both kinds) and `DryRun`;
  `detect.rs` picks the live backend from the environment (Wayland, X11, or why neither).
- `src/xrandr/`: `XrandrCli`, the `xrandr --verbose` parser, command generation (`Plan` → argv /
  text), and `script.rs`: screenlayout scripts (lenient parser into the neutral profile spec,
  save that keeps other lines).
- `src/wayland/`: `capture.rs` (the `wlr-randr --json` capture ↔ `Snapshot`, `dump`, the
  Wayland `revert.sh`), `client.rs` (`WlrBackend`, the `zwlr_output_manager_v1` client, and
  `hang_up` for the panic hook), `command.rs` (`Plan` → the equivalent `wlr-randr` command),
  `kanshi.rs` (kanshi config reader and block writer that keeps every other byte).
- `src/model/`: snapshot types (with `Caps` and `Kind`, X11 or Wayland), geometry, the layout
  and its stick links, movement, validation, undo history, `profile.rs` (the neutral profile spec: targets by name, description glob or
  `*`, matched onto a snapshot, → layout with remap) and `orientation.rs` (rotation and
  reflection ↔ Wayland transform names). Everything here is pure and tested without a terminal
  or a display server.
- `src/profiles.rs`: `ProfileStore`, screenlayout scripts on X11 or the kanshi config on
  Wayland, picked from the snapshot's kind. `src/files.rs`: atomic writes and line diffs.
- `src/profile.rs`: `outlay apply` and `outlay save`; the apply reuses the editor's `Session`.
  `src/restore.rs`: the hidden `outlay restore` a Wayland `revert.sh` runs.
- `src/tui/`: the editor. `app.rs` holds the state and `handle_key` (pure: it returns effects),
  `session.rs` carries the effects out (test → apply → verify → countdown → keep/revert, hooks,
  OSC 52, profiles) behind the `Backend` and `Input` traits, `mod.rs` owns the terminal, signals
  and panic hook, `keys.rs` the keymap table, `canvas.rs` the to-scale drawing and sticky
  viewport (also used by `outlay show`), `ui.rs` the screen composition.
- `install.sh`: the `curl | sh` installer and updater. `.github/workflows/release.yml` builds the
  static musl release binaries on `v*` tags, checks the installer on them end to end, and
  publishes them with `install.sh` and `SHA256SUMS`.
- Tests: `tests/common/mod.rs` builds snapshots from a few lines (`on("A", 1920, 1080, 0, 0)`);
  `tests/scenarios.rs` holds golden desk layouts, `tests/properties.rs` the proptest invariants,
  `tests/commands.rs` the golden apply/revert/script commands, `tests/tui.rs` key sequences
  and insta screen snapshots (`tests/snapshots/`; review `.snap.new` files before renaming them),
  `tests/apply.rs` the apply flow with a fake backend, clock and signal flag,
  `tests/profiles.rs` the user's scripts (copied into `tests/fixtures/screenlayout/`) on the
  fixtures, the remap, the round trip, and `w`/`e` through the session with temporary dirs,
  `tests/install.rs` `install.sh` piped into `sh -s --` against a `file://` fake release, with a
  cleared environment and a sandboxed home, `tests/kanshi.rs` the kanshi configs in
  `tests/fixtures/kanshi/` (parse, criteria, remap, byte-keeping saves, the session),
  `tests/wayland_fixtures.rs` the captures in `tests/fixtures/wayland/`, and
  `tests/wayland_live.rs` the backend against headless sway (`tests/common/sway.rs`).
