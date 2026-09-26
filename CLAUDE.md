# outlay — notes for coding sessions

Keyboard-driven xrandr layout editor (Rust 2024, ratatui). `docs/PLAN.md` is the design brief and
the source of truth for behaviour; read `MISTAKES.md` before starting work.

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
- `cargo run -- --from-file tests/fixtures/xrandr/<name>.txt <cmd>`: reads an
  `xrandr --verbose` capture. Never touches X.
- `cargo run -- list`, `cargo run -- show`: query the live X server read-only.
- `tools/pty_drive.py SCENARIO -- --demo`: runs the release build in a pseudo-terminal (no window)
  and checks the rendered screen; the way to test the event loop, signals and the apply flow.
- `tools/shot.sh`: the screenshot loop for `docs/screenshots/`. It opens a kitty window on the
  developer's display: ask first, every time.

## Rules

- **Live displays.** The developer's screens are live. Never run an xrandr command that changes
  state (`--output`, `--auto`, `--off`, a layout script, an outlay apply) without asking first.
  Always allowed: `xrandr --verbose`, `--query`, `--current`, `--listmonitors`, `--listproviders`.
  Tests never call the real `xrandr`; they use `FixtureBackend` or captured fixtures.
- **`~/.screenlayout` is read-only.** Copy scripts into `tests/fixtures/screenlayout/`, and point
  test saves at temporary directories.
- **Git identity.** Author and committer must be `Papayah <maciej.chmiest@gmail.com>`. The global
  git identity is a work address that must never appear in this repo. The local config is set in
  `.git/config`; check `git config user.email` before committing and
  `git log -1 --format='%an <%ae> | %cn <%ce>'` after every commit.
- **Keys.** The keymap table in `src/tui/keys.rs` is the only source of keys. Dispatch, the hint
  line, `?` help and `outlay keys` are all generated from it; never hard-code a key elsewhere.
- **Commits.** Conventional commits (`feat:`, `fix:`, `docs:` …), one commit per plan phase.

## Layout

- `src/xrandr/`: the `Backend` trait (`XrandrCli`, `FixtureBackend`), the `xrandr --verbose`
  parser, EDID decoding, and command generation.
- `src/model/`: snapshot types, geometry, the layout and its stick links, movement, validation,
  undo history. Everything here is pure and tested without a terminal or an X server.
- `src/tui/`: the editor. `app.rs` holds the state and `handle_key` (pure: it returns effects),
  `session.rs` carries the effects out (apply → verify → countdown → keep/revert, hooks, OSC 52)
  behind the `Backend` and `Input` traits, `mod.rs` owns the terminal, signals and panic hook,
  `keys.rs` the keymap table, `canvas.rs` the to-scale drawing and sticky viewport (also used by
  `outlay show`), `ui.rs` the screen composition.
- Tests: `tests/common/mod.rs` builds snapshots from a few lines (`on("A", 1920, 1080, 0, 0)`);
  `tests/scenarios.rs` holds golden desk layouts, `tests/properties.rs` the proptest invariants,
  `tests/commands.rs` the golden apply/revert/script commands, `tests/tui.rs` key sequences
  and insta screen snapshots (`tests/snapshots/`; review `.snap.new` files before renaming them),
  `tests/apply.rs` the apply flow with a fake backend, clock and signal flag.
