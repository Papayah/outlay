# outlay on Wayland (plan + handoff)

> **Handoff.** This file briefs three implementation sessions, D, E and F, plus two optional later
> sessions, G and H. Each session starts fresh in `/home/mc2/workspace/this/outlay` and delivers one
> PR.
>
> Session D's first commit copies this file into the repo as `docs/PLAN-wayland.md`. From then on
> that copy is the source of truth for Wayland work. `docs/PLAN.md` stays the source of truth for
> everything else; where the two differ, `docs/PLAN-wayland.md` wins.
>
> Read the whole file before you run any command. The planning session changed no code.

## Context

outlay is a keyboard-driven monitor layout editor. Today it runs only on X11, through the `xrandr`
program. On Wayland it refuses to start (`check_session`, `src/xrandr/mod.rs:151`), because there
xrandr would only reconfigure XWayland.

The goal is to make the same editor work on Wayland compositors:
- the same to-scale canvas, stick links, and snap and swap movement;
- the same apply → verify → countdown → keep or revert safety net;
- profiles that a Wayland tool re-applies on hotplug.

**Prior art on Wayland.**
- wdisplays: GTK, driven by the mouse.
- wlr-randr: a CLI.
- kanshi and shikane: profile daemons.
- `xwlm`: a ratatui list TUI for Hyprland, sway and river.

None of them has a to-scale canvas, persistent stick links or an auto-revert countdown.

**Where outlay stands (2026-10-02).**
- v0.1.0, six merged PRs, about 215 tests, static musl releases with `install.sh`.
- The developer stays on X11 with i3 and has no Wayland compositor installed.
- Wayland testing therefore runs against **headless sway**: in CI, and locally once the user agrees
  to install it.

## Fixed decisions (made with the user on 2026-10-02; do not re-ask)

| Topic | Decision |
|---|---|
| Scope | **The wlroots protocol family first.** A native backend for `zwlr_output_manager_v1` covers sway, Hyprland, niri, river, labwc, Wayfire, COSMIC and others. GNOME (Mutter D-Bus) and KDE (KWin protocols) come later, as optional sessions G and H behind the same trait. Until then, outlay refuses to run on GNOME and KDE with a clear message. |
| Profiles on Wayland | **kanshi config.** `w`/`e`, `outlay apply` and `outlay save` read and write profile blocks in `~/.config/kanshi/config`, keeping every other byte. kanshi re-applies them on hotplug. X11 keeps its `~/.screenlayout` scripts. |
| Scale editing | **Both backends.** A scale picker plus `:scale <f>`. On X11 it is `--scale`; on Wayland it is the compositor's own scale. Neighbours re-flow. |
| Live test target | **Headless sway only.** The developer stays on X11, so there is no everyday compositor to smoke-test against. |
| Talking to the compositor | **The protocol directly,** through the pure-Rust `wayland-client`. There is no run-time `wlr-randr`. Wayland sessions need no other program, and releases stay fully static. |
| Delivery | **Three sessions, one PR each:** D = phases W0–W3, E = W4–W6, F = W7–W9. The user merges each PR before the next session starts. |

## Verified facts (2026-10-02)

### wlr-output-management-unstable-v1

Sources: the spec on wayland.app, cross-checked against Hyprland's copy. Do not use the GitHub
mirror `swaywm/wlr-protocols`: it is stale and stops at v2.

**Versions.** v4 is the latest:
- v2 added head `make`, `model` and `serial_number`;
- v3 added `release` on heads and modes;
- v4 added `adaptive_sync`.

wlroots has served v4 since 0.17.

**Objects:**
- **Manager:** the `head` and `done(serial)` events, and `create_configuration(serial)`.
- **Head:**
  - fields: `name`, `description`, `physical_size`, `mode`s, `enabled`, `current_mode`, `position`,
    `transform`, `scale`, `make`/`model`/`serial_number` and `adaptive_sync`;
  - `current_mode`, `position` and `scale` are sent only while the head is enabled;
  - a head exists while its connector is connected, whether enabled or not.
- **Mode:** `size`, `refresh` in **mHz**, and `preferred`.
- **Configuration:** `enable_head` (returns a configuration head), `disable_head`, `test` and
  `apply`. It answers `succeeded`, `failed` or `cancelled`.
- **Configuration head:**
  - `set_mode` or `set_custom_mode(w, h, mHz)`, never both;
  - `set_position`, `set_transform`;
  - `set_scale(wl_fixed)`, which is 24.8 fixed point: 1.25 is exact, 4/3 is not;
  - `set_adaptive_sync`.

**Rules:**
- **Every head must be in a configuration,** enabled or disabled. The spec makes leaving one out a
  protocol error; wlroots does not enforce it.
  - wlroots' `enable_head` copies the head's current state, so a property you don't set keeps its
    value.
- **A stale serial gives `cancelled`** (wlroots, niri). Create a new configuration with the new
  serial and try again. Hyprland never checks the serial.
- **`test` is only advisory.** On Hyprland it is a stub that always succeeds; niri rejects only
  "all outputs off".
  - Even after `succeeded` the state may differ (for example a rounded scale), so outlay keeps its
    own verification.
  - After `succeeded`, the new head state arrives as head events and then `done`.
- **Positions are in logical (scaled) coordinates** in sway, Hyprland and niri. A 2560x1440 output at
  scale 2 takes 1280x720 of the layout.
- **No mirroring and no primary** in the protocol. Hyprland, Wayfire and COSMIC mirror only through
  their own config or extensions.

### Compositors

- **Implement it (v4):** sway/wlroots, Hyprland, niri (from 0.1.8), river, labwc, Wayfire, COSMIC,
  Jay, Cage, phoc, dwl, miracle-wm.
- **Do not:** Mutter (GNOME), KWin (KDE), Weston, Gamescope.
- **How long a change made through the protocol lasts:**
  - sway: until `swaymsg reload`. This is likely but was not tested at run time.
  - niri: until the `output` sections of its config file change.
  - **Hyprland: the override belongs to the client's binding.** Once that client disconnects,
    Hyprland drops it at the next reload or hotplug. outlay exits after an apply, so its layout
    lasts only until then. kanshi stays connected, so its layouts last.

### Crates

All are pure Rust with an MSRV of at most 1.91:

| Crate | Version | MSRV | Notes |
|---|---|---|---|
| `wayland-client` | 0.31.15 | 1.71 | Has no default features. Never enable `system`: it links libwayland. |
| `wayland-protocols-wlr` | 0.3.12 | 1.71 | Feature `client`; the module is `wayland_protocols_wlr::output_management::v1::client`. |
| `serde_json` | 1 | | Reads and writes the capture format. |
| `zbus` | 5.19 | 1.87 | Later session G only. |

- Avoid `smithay-client-toolkit`: its defaults pull in xkbcommon, a C library.
- **Reference to read:** cosmic-randr (pop-os; `wayland-client` 0.31 + `wayland-protocols-wlr` 0.3).
  It binds the manager at `version.min(4)` and rejects versions below 2.

### Headless sway

- **The command:**
  `WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_HEADLESS_OUTPUTS=2 WLR_LIBINPUT_NO_DEVICES=1 sway -c <cfg>`.
  - Run it with a private `XDG_RUNTIME_DIR` (mode 0700).
  - It needs no GPU, no seat and no window.
- **Outputs:**
  - they are named `HEADLESS-N` and start at 1280x720;
  - `swaymsg create_output` adds one (undocumented, but it works on headless);
  - `swaymsg output <name> unplug` removes one.
- **Modes.** Headless heads have no real modes, only custom ones.
  - **Probably** (to be checked in source and on the wire as the first step of W5): wlroots
    advertises a *virtual* mode object for a custom current mode, and a new one after every
    change.
  - `set_mode` with a virtual mode on a head that has real modes is a protocol error ("mode
    doesn't belong to head"), and it kills the connection.
- **CI runners:**
  - Ubuntu 24.04 has sway 1.9 with wlroots 0.17 (protocol v4).
  - **`ubuntu-latest` moves to 26.04 between 2026-10-19 and 2026-11-19,** so the Wayland job pins
    `runs-on: ubuntu-24.04`.
- **Danger:** with `DISPLAY` set and `WLR_BACKENDS` unset, sway opens an X11 window on the
  developer's display. The harness always sets `WLR_BACKENDS=headless` and removes `DISPLAY` and
  `WAYLAND_DISPLAY` from sway's environment.
- **Socket paths are limited to 108 bytes** (`MISTAKES.md`, "kitty ignores `--listen-on`"). Put the
  temporary runtime dir at `$XDG_RUNTIME_DIR/outlay-test-<pid>`, or `/tmp/outlay-test-<pid>` when
  `XDG_RUNTIME_DIR` is unset. Never under the scratchpad.

### kanshi 1.9.0

- **Config:** `$XDG_CONFIG_HOME/kanshi/config`; `kanshictl reload` reloads it.
- **Top level:** `profile [name] { … }`, global `output <criteria> <directives>` defaults, and
  `include <path>`.
- **Inside a profile:**
  - `output <criteria> <directives…>`, inline or as a `{ }` block;
  - `exec <cmd>`;
  - `...output`, which matches any number of outputs.
- **Criteria:**
  - a connector name;
  - `"Make Model Serial"`, where globs are allowed and a missing field is `Unknown`;
  - `$alias`;
  - `*`.
- **Directives:**
  - `enable` / `disable`;
  - `mode [--custom] WxH[@R[Hz]]` or `mode preferred`;
  - `position X,Y`;
  - `scale F`;
  - `transform normal|90|180|270|flipped|flipped-90|flipped-180|flipped-270`;
  - `adaptive_sync on|off`.
- **Matching:** a profile matches only when exactly its outputs are connected.

## Design

### Module layout after this work

```
src/backend/mod.rs     Backend trait, Caps, Kind, Verdict, ApplyOutcome, detect() (replaces check_session)
src/backend/plan.rs    Plan: the neutral, self-contained "what to set", built from a Layout or a Snapshot
src/backend/fixture.rs FixtureBackend (simulates a Plan, both kinds) and DryRun
src/model/edid.rs      EDID decoding, moved from src/xrandr/edid.rs (pure); Identity next to it
src/xrandr/            XrandrCli, parse.rs, command.rs (Plan → argv / text, argv → Plan helper), script.rs
src/wayland/mod.rs     WlrBackend: the zwlr_output_manager_v1 client
src/wayland/capture.rs JSON capture ↔ Snapshot (fixtures, `dump`, the Wayland revert.sh)
src/wayland/command.rs Plan → equivalent `wlr-randr` command line (popup, `y`, `apply -n`)
src/wayland/kanshi.rs  kanshi config parser, block writer, save that keeps other bytes
src/model/profile.rs   neutral profile spec (moved out of xrandr/script.rs) → Layout
src/profiles.rs        ProfileStore: Screenlayout(dir) | Kanshi(file)
```

### The neutral model (phase W1)

| Today (shaped by xrandr) | After |
|---|---|
| `Mode.xid: u32`, `ActiveConfig.xid`, `Pick::Mode(u32)`, `set_mode(snap, i, xid)` | `ModeId(u64)`. On X11 it is the XID (< 2^32). On Wayland it is `w << 48 \| h << 32 \| mHz` (≥ 2^32), which stays the same across re-queries and processes. Wayland modes are de-duplicated by (w, h, mHz) per head, keeping the current one. Messages show the mode summary, never the id. `Mode` gains `custom: bool`. Wayland modes get the synthetic name `WxH`, so `find_mode(name, rate)` also serves kanshi. |
| `Output.edid` with `crate::xrandr::edid::Edid` | `edid.rs` moves to `src/model/`. `Output` keeps `edid` (X11 only) and gains `identity: Option<Identity { make, model, serial, label }>` for both kinds. Panels, `label()` and `Plugs` (`src/tui/app.rs:1898`) use `identity`. |
| `ActiveConfig.transform` / `OutputState.transform: Transform` | `scaling: Scaling`, which is either `X11(Transform)` (the matrix and filter, as today) or `Logical(f64)` (the Wayland scale). |
| `effective_size(mode, rotation, &Transform)` (`src/model/geometry.rs:179`) | `effective_size(mode, rotation, &Scaling)`. X11 keeps `Transform::bounds`. `Logical` divides the rotated size by the scale and **truncates**, as wlroots' `wlr_output_effective_resolution` does; W5 confirms this. `links.rs` and `snap.rs` use only integer rects, so they need no change. |
| `Snapshot.screen: ScreenLimits` | `Option<ScreenLimits>`. Wayland has none, so the screen-maximum check (`src/model/validate.rs:67-80`) and the `show` summary skip it. A default `max` of 0x0 would fail every Wayland layout. |
| X11 abilities that are only implied | `Snapshot.caps: Caps { kind: Kind, primary: bool, mirror: bool, all_reflections: bool }`. X11 sets them all; Wayland sets none. **`Layout` gets a copy** (set in `inferred`, `from_snapshot`, `from_states` and `remapped`), because `infer_links` (`src/model/links.rs:276`), `set_reflection` (`src/model/layout.rs:501`) and `state_from_output` (`src/model/layout.rs:691`) need it without a snapshot. |

- **Orientation:**
  - The model keeps `Rotation × Reflection`. Wayland has only `Reflection::{Normal, X}`.
  - On Wayland, a parsed or typed `y` becomes `inverted` + `x`, and `xy` becomes `inverted`, with a
    status note.
  - Expected mapping: X11 `normal/right/inverted/left` ↔ Wayland `normal/90/180/270`, with `x`
    adding `flipped`.
  - W4 confirms **both** the direction of `90` and the flip order from sway-output(5) and the
    wlroots source: does (`left`, `x`) map to `flipped-90` or `flipped-270`? It cites the source in
    a comment next to a unit test.
- **X11-only data.** CRTCs, panning, stale outputs and `Connection::Unknown` stay in the model, and
  only X11 fills them. The checks that use them already look at the data (empty `crtcs`, no
  `panning`).
- **Display numbers on Wayland.** Heads are sorted with built-in panels first (`eDP`, `LVDS`,
  `DSI`), then by natural name order, so numbers do not depend on the order things were plugged in.
  X11 keeps xrandr order.
- **W1 sets only X11 values.** The Wayland branches arrive with the first Wayland snapshot in W4.

### Plans instead of argv (phase W2)

```rust
pub struct Plan {
    pub outputs: Vec<Planned>,   // only the outputs to set; others stay as they are
    pub primary: PrimaryRule,    // Keep | Clear (X11 `--noprimary`), computed with locked outputs counted
    pub form: PlanForm,          // Apply | Restore (Restore: explicit --reflect/--transform/--filter)
}
pub struct Planned { pub name: String, pub on: Option<On> }   // None = off
pub struct On {
    pub mode: Mode, pub pos: Point, pub rotation: Rotation, pub reflection: Reflection,
    pub scaling: Scaling,
    pub scaling_change: ScalingChange,   // Keep | Set | Reset: X11 writes --scale / --transform none only on a change
    pub primary: bool,
}
```

- **A plan is self-contained.** Everything the X11 renderer needs is computed when the plan is
  built, so rendering is a pure `render(&Plan, Form)` with no snapshot. `XrandrCli::apply(plan)`
  has none.
  - **The panning trap:** `tests/fixtures/xrandr/panning.txt`'s eDP-1 is primary and locked.
    `PrimaryRule` must come from `layout.primary()` (`src/model/layout.rs:271`), which counts
    locked outputs. Otherwise `--noprimary` appears and `tests/commands.rs:137` breaks.
- **`Plan::pending(&Layout, &Snapshot)`** selects outputs exactly as `pending_args` does
  (`src/xrandr/command.rs:90`: relevant or enabled, and not locked).
- **`Plan::restore(&Snapshot)`** selects exactly as `revert_args` does (`command.rs:117`: relevant
  and not panning; off outputs become `None`).
- **Golden acceptance: byte-identical,** in all of:
  - `tests/commands.rs`;
  - `tests/cli.rs:140` (`apply -n`);
  - `tests/apply.rs:269` and `:442` (revert argv and `revert.sh`);
  - the command line in `tests/snapshots/tui__snapshot_apply_confirmation.snap`.
- **The trait:**
  ```rust
  pub trait Backend {
      fn query(&self) -> Result<Snapshot>;                   // full probe (X11: --verbose)
      fn requery(&self) -> Result<Snapshot> { self.query() }  // X11: --current; Wayland: a roundtrip
      fn test(&self, _plan: &Plan) -> Result<Verdict> { Ok(Verdict::Untested) }
      fn apply(&self, plan: &Plan) -> Result<ApplyOutcome>;
      fn dump(&self) -> Result<String>;                      // the capture text `--from-file` reads
      fn is_live(&self) -> bool { false }                    // was touches_x()
  }
  ```
  - `ApplyOutcome` keeps `success` and the tool's own messages; X11 fills them from stdout and
    stderr.
  - `Verdict` is `Untested | Accepted | Rejected(String)`.
  - `FixtureBackend` keeps its source text, for `dump`.
- **The kind decides the text, not the backend.** These depend on `snap.caps.kind`:
  - the confirm popup's command;
  - the `y` copy and `apply -n`;
  - `revert.sh`;
  - the profile store;
  - the wording: "xrandr" or "the compositor".

  So `--from-file wayland.json` and `-n` on Wayland show Wayland text.
- **Carriers:**
  - `ApplyRequest { plan, layout }` replaces `{ argv, layout }` (`src/tui/app.rs:110`).
  - `app::ApplyPlan` (`src/tui/app.rs:128`) is renamed `ApplyPreview { command, plan }`, to avoid
    the name clash.
  - `Session::apply` and `revert_quietly` (`src/tui/session.rs:303-463`) and `profile::apply`
    (`src/profile.rs:37-113`) carry plans.
- **The simulator.** `FixtureBackend` simulates a `Plan`, replacing the argv simulator
  (`src/xrandr/mod.rs:285-497`). It keeps everything the old one did:
  - normalising an identity transform (`:433-435`);
  - CRTC assignment, only when CRTCs exist (`:455-475`);
  - the screen-maximum check and `screen.current`, only when limits exist (`:477-490`);
  - the warning "output X not found; ignoring" for an unknown planned name.

  Two more pieces go with it:
  - `applied()` returns plans, and a test helper `applied_argv()` renders them;
  - the old argv parser becomes a `#[doc(hidden)] pub fn argv_to_plan`, used for an **Xid-form**
    round trip (Plan → argv → Plan). NameRate's `{:.2}` is not reversible.
- **Wording becomes neutral.** For example "Simulated: nothing was sent to the displays." and "Copy
  the pending command". The Applying popup's title names the tool. Review each `.snap.new`.

### Scale editing on both backends (phase W3)

- **Keys:** `x` opens the scale picker; `<` and `>` step to the previous or next scale in place.
  - These go only in the keymap table (`src/tui/keys.rs`).
  - Update the test at `src/tui/keys.rs:751`, which asserts that `x` is unbound.
- **Picker rows:** 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2.5 and 3, plus the current value if it is not
  among them.
  - Each row shows the effect, because the two systems scale in opposite directions:
    - X11: `×1.5 → 2880x1620 desktop` (the desktop gets larger, things look smaller);
    - Wayland: `150% → 1280x720 logical` (things look larger).
  - A size that is not a whole number gets `~`.
  - All the listed values are exact in `wl_fixed`.
- **`:scale F`** accepts 0.25 to 8, and Wayland also accepts `150%`. It replaces today's `:scale 1`
  (`src/tui/cmdline.rs:65,148-151`).
- **`Layout::set_scale(i, s)`** goes through `commit()`, so links re-flow and the push pass runs. It
  makes an undo step and animates, like a mode change.
- **`Layout::diff` compares the scale value,** with a tolerance (`src/model/layout.rs:597` today
  checks only "identity or not"). Without that, a change from ×1.25 to ×1.5 would:
  - show "No changes" in the popup (`src/tui/app.rs:1050`);
  - quit without asking (`:1141`);
  - be overwritten by the watch's untouched branch (`:572-579`).
- **X11 arguments.**
  - The Xid and NameRate forms write `--scale SxS` **only when the target differs from the live
    scale** (`ScalingChange::Set`).
  - A reset to 1 keeps today's `--transform none`.
  - An unchanged scaled output writes nothing, so its `--filter` and any matrix that is not a plain
    scale survive.
  - Picking a scale on an arbitrary matrix replaces it, with a status note.
- **Badges.** The canvas badge is `×1.5` on X11 and `150%` on Wayland. The details panel shows the
  scale, plus the logical size on Wayland.

### Reading Wayland outputs (phase W4)

- **The capture format is the JSON of `wlr-randr --json`.**
  - It is an array of heads with `name`, `description`, `make`, `model`, `serial`,
    `physical_size`, `enabled`, `modes[]` (`width`, `height`, `refresh` in Hz, `preferred`,
    `current`), `position`, `transform`, `scale` and `adaptive_sync`.
  - Check the exact schema in wlr-randr's `main.c` first.
  - The parser ignores unknown keys and tolerates missing optional ones.
  - outlay may add optional keys of its own, such as `custom`.
  - A refresh in Hz becomes mHz as `round(Hz × 1000)`.
- **`--from-file`** reads a file as JSON (Wayland) when its first non-space character is `[` or
  `{`; anything else is `xrandr --verbose`. The help text and the value name become `CAPTURE`.
- **`--demo[=x11|wayland]`** uses `require_equals`, so `outlay --demo list` still works.
  `--demo=wayland` embeds `tests/fixtures/wayland/demo.json`:
  - `HDMI-A-1`: the TV;
  - `DP-3`: Dell 2560x1440@143.912;
  - `eDP-1`: 2880x1800 at 200 % (1440x900 logical), centred below;
  - `DP-4`: connected but off.
- **`outlay dump`** prints `backend.dump()`: raw `xrandr --verbose` on X11, JSON on Wayland. It is
  meant for bug reports and fixtures.
- **Fixtures** (`tests/fixtures/wayland/`): `demo.json`, `laptop-scaled.json` (1.25 and 1.5),
  `rotated.json` (90 and flipped-270), `disabled-head.json`, `custom-mode.json`, `no-serial.json`.
  W6 adds `headless-sway.json`.
- **Wayland snapshots in the editor need this phase's gating.**
  - **The keymap table:**
    - Rows get a `const fn needs(self, Cap) -> Binding` builder (`Cap::Primary`, `Cap::Mirror`).
      Do not add a sixth argument to the roughly 60 `bind(...)` rows (`src/tui/keys.rs:230-370`).
    - `lookup` (`:473`) also returns the binding's needs.
    - Dispatch answers an unsupported key with a status message ("Wayland compositors have no
      primary display.", "This compositor cannot mirror displays."). It adds no undo step.
    - Hints filter on caps; the hint line already takes a filter, `src/tui/panels.rs:250`.
    - `help`/`reference(Option<&Caps>)`: `None` (for `outlay keys`) adds an "(X11)" note.
  - **The command line:** `USAGE` and `WORDS` (`src/tui/cmdline.rs:49-80`) get a needs field, and
    `complete` (`:204`) takes caps, so `same-as`, `:primary` and `y`/`xy` are hidden on Wayland.
  - **Inference** makes no `Same` links when `!caps.mirror`, so identical rectangles show as an
    overlap warning.
  - **Profiles** (`w`, `e`, `outlay apply`, `outlay save`) are refused on Wayland until W8: "kanshi
    profiles arrive in the next version." `Profile::layout` would otherwise put an X11 `Scaling`
    into a Wayland layout (`src/xrandr/script.rs:534`).
  - **`list` and `show`** drop the "not connected" section, show the layout size instead of
    "screen … of max …", and use `150%` badges.
- **Validation on Wayland:**
  - Skipped: the screen maximum, CRTCs, stale outputs, panning, and "no primary".
  - A custom mode that is the head's current mode is not "missing from its list".
  - **New warning:** a fractional logical size, e.g. "eDP-1 at 175 % is 1097.1 px wide; the
    compositor rounds it, which can leave a 1 px gap or overlap."
  - **New status note:** when a head that has never been on is turned on, it starts at 100 %:
    "`x` picks a scale."

### Driving wlroots compositors (phase W5)

- **Connecting.**
  - `Connection::connect_to_env()` and `registry_queue_init`.
  - Bind `zwlr_output_manager_v1` at `min(advertised, 4)`. Below v2 (no make/model/serial),
    refuse: "this compositor's output protocol is too old".
  - From v3, `release` heads and modes.
  - The manager's `head` and a head's `mode` events create objects, so their dispatch needs
    `event_created_child!`.
- **State.**
  - There is one persistent connection behind a `Mutex`, the same pattern as `FixtureBackend`, and
    no thread.
  - `query` and `requery` do a roundtrip and build a Snapshot from the heads as of the last `done`.
  - The existing 2 s watch (`WATCH_INTERVAL`) sees hotplugs without changes: a new head, or a
    `finished` one.
  - A `finished` manager or a protocol error becomes "Lost the connection to the compositor."
- **Apply and test:**
  1. Roundtrip to get the latest serial. If a planned head is gone:
     - an apply fails with "The outputs changed while applying." (the text `mismatches` already
       uses);
     - a restore skips that head with a warning.
  2. `create_configuration(serial)`. Then, for **every** head:
     - **planned off:** `disable_head`;
     - **planned on:** `enable_head`. Send `set_mode` or `set_custom_mode` **only when the planned
       `ModeId` differs from the current one.** Re-sending a virtual mode is a fatal protocol error.
       Always send `set_position`, `set_transform` and `set_scale`;
     - **not in the plan:** `enable_head` with nothing else (it keeps its state) or `disable_head`,
       matching its current state.
  3. `apply` or `test`. Dispatch until `succeeded`, `failed` or `cancelled`, then destroy the
     configuration.
  4. **After `succeeded`,** wait for the manager's next `done` (up to 1 s; skip the wait when the plan
     equals the current state). This way `Session::apply`'s immediate re-query
     (`src/tui/session.rs:334`) sees the final state and does not revert by mistake.
  5. **`cancelled`:** roundtrip. If the head names are unchanged, retry once; otherwise fail as in
     step 1.
  6. **Never send `set_adaptive_sync`.** VRR stays as it is; the details panel shows it.
- **The test step.**
  - `Session::apply` calls `backend.test(plan)` before it writes `revert.sh`. `Rejected` means "The
    compositor rejects this layout.", and nothing changes.
  - The confirm popup also shows the verdict: `App::open_apply` emits `Effect::Test(Plan)`, and
    `App::tested(Verdict)` adds it to the popup's blocking errors.
  - `outlay apply` gets the check through `Session::apply`.
  - Backends that cannot test answer `Untested`. Caps carry no "can test" flag.
- **Verification** (`Layout::mismatches`, `src/model/layout.rs:633`) compares:
  - the enabled set;
  - the `ModeId`;
  - the rectangle;
  - primary, only when `caps.primary`;
  - on Wayland, the scale within 0.01. A rounded scale within that tolerance is adopted.
- **Choosing a backend.** `backend::detect(env)` replaces `check_session`, again as a pure choice
  function plus a connect step:
  1. **`WAYLAND_DISPLAY` is set:** connect.
     - If the compositor does not answer: "WAYLAND_DISPLAY is `wayland-1`, but no compositor
       answers. If this is not a Wayland session (a stale value in tmux, for example), unset it."
     - If it lacks `zwlr_output_manager_v1`, refuse with a message that depends on
       `XDG_CURRENT_DESKTOP`:
       - GNOME: "GNOME is not supported yet. Use Settings → Displays or `gdctl`."
       - KDE: "KDE Plasma is not supported yet. Use System Settings → Display or
         `kscreen-doctor`."
       - otherwise, a generic message.
  2. **`XDG_SESSION_TYPE=wayland` without `WAYLAND_DISPLAY`:** refuse, as today.
  3. **`DISPLAY` is set:** `XrandrCli`.
  4. **Neither:** today's message, now naming both.

  `--demo` and `--from-file` never call `detect` (`src/cli.rs:85-98`). `-n` wraps whatever `detect`
  returns.
- **Compositor label.**
  - `detect` gets the compositor name from `SWAYSOCK`, `HYPRLAND_INSTANCE_SIGNATURE`, `NIRI_SOCKET`
    or `XDG_CURRENT_DESKTOP`. It is passed to the header the way `Options.source` is
    (`src/tui/panels.rs:27`).
  - Fixtures get `None`, so insta snapshots never depend on the machine.
  - It is a label only.
- **Revert on Wayland.** `revert.sh` becomes:
  ```sh
  #!/bin/sh
  # Written by outlay 0.2.0 before an apply. It restores the layout from before it.
  exec '/abs/path/to/outlay' restore - <<'OUTLAY-CAPTURE'
  …the pre-apply snapshot as capture JSON…
  OUTLAY-CAPTURE
  ```
  - **The program path is `Settings.restore_program`.**
    - The CLI fills it from `current_exe()`.
    - If the path is missing or ends in ` (deleted)` (after `install.sh` replaced the binary), it
      falls back to `outlay` on `PATH`.
    - **Tests pass `env!("CARGO_BIN_EXE_outlay")`**, because `current_exe()` in a test is the test
      harness.
  - **`outlay restore FILE|-`** is a hidden subcommand.
    - It is dispatched **before `Config::load()`**, next to `Completions` (`src/main.rs:24-29`), so
      a broken config cannot block a revert at panic time.
    - It reads a capture, detects the live backend and applies `Plan::restore`.
    - It has no countdown, writes no revert file and runs no hooks.
    - It prints problems to stderr and exits 1 when there are any.
  - X11 `revert.sh` stays a plain xrandr script.
  - The panic hook (`src/tui/mod.rs:67`) is unchanged.
- **Commands on Wayland.** The confirm popup shows an **equivalent** `wlr-randr …` command:
  `--output N --on|--off --mode WxH@R.RRRHz|--custom-mode … --pos X,Y --transform T --scale S`.
  `y` and `apply -n` use the same text.

### Profiles: kanshi (phases W7 and W8)

- **W7: neutral spec and store.**
  - Move `Profile`, `Entry` and `Profile::layout(snap, remap)` from `src/xrandr/script.rs` into
    `src/model/profile.rs`.
  - `Entry.target` becomes `Name(String) | Description(glob) | Any`.
  - `Entry.mode` becomes a `ModeRequest`: a name with a rate, an XID, or preferred.
  - `Entry.scaling` becomes a `Scaling`.
  - `script.rs` and `kanshi.rs` both produce the spec.
  - `line_diff` and `write_atomic` move to a shared module.
  - `Settings` keeps both `layouts_dir` and `kanshi_config`. It is built before any query
    (`src/cli.rs:101-131`), so the Session picks the `ProfileStore` from `app.snap.caps.kind` each
    time it needs it (`src/tui/session.rs:199-281`, `src/profile.rs`).
- **W8: kanshi.**
  - **Location:** config key `kanshi_config` (default `~/.config/kanshi/config`) and flag
    `--kanshi-config FILE`. Add both to `Config` and its `Default` (`deny_unknown_fields`), to the
    config sample in `docs/PLAN.md`, and to the README.
  - **Parsing** is lenient: problems become warnings, as in `script.rs`.
    - It handles comments, quoted strings, braces, and inline or block outputs.
    - It follows `include` when reading.
    - `exec`, global `output` defaults, `alias` and `...output` stay in the text; the editor
      ignores them, with a note.
    - `mode WxH@R` picks the nearest rate. Without `@R`, do what kanshi's source does (check
      whether that is the highest rate).
  - **Matching criteria:**
    - names match exactly;
    - descriptions are built as kanshi builds them (`Make Model Serial`, with `Unknown` for a
      missing field) and matched as globs;
    - `*` takes any one head not yet matched;
    - an unmatched entry opens the existing remap dialog.
  - **Saving** (one block):
    ```
    # generated by outlay 0.2.0
    profile home {
        output eDP-1 enable mode 2880x1800@60.001Hz position 560,1440 scale 2 transform normal
        output "Dell Inc. DELL U2719D 7LQ2M43" enable mode 2560x1440@143.912Hz position 1920,0 scale 1 transform normal
        output HDMI-A-1 disable
    }
    ```
    - The block lists every connected head.
    - **Criteria:** an existing entry keeps the criteria the user wrote. A new entry uses the name
      for built-in panels and for heads without make, model and serial; otherwise the quoted
      description.
    - Only this profile's block (and its `# generated` comment) is replaced, or a new block is
      appended. Every other byte stays.
    - The write is atomic, after a diff and a confirmation, as for scripts. It goes into the file
      that holds the profile, even an `include`d one.
    - The status line adds "Run `kanshictl reload` so kanshi uses it." outlay never runs it.
  - **Commands.** `outlay apply <profile>` and `outlay save <profile>` use the kanshi config on
    Wayland, and `w`/`e` list its profiles with mini previews. This lifts W4's refusal.
  - **Check with the user's OK** (it needs `pacman -S kanshi`): run kanshi against headless sway
    with a matching profile. Does it re-apply its profile on every `done`? If so, it undoes
    outlay's apply during the countdown, and the README must say so. If it does not, record that
    fact in `MISTAKES.md`.

### Distribution

- **Dependencies:** add `wayland-client = "0.31"` (no features), `wayland-protocols-wlr = {
  version = "0.3", features = ["client"] }` and `serde_json = "1"`. There is no cargo feature: the
  deps are pure Rust and small.
- **Static build.** The PR changes `Cargo.lock`, so `release.yml` runs on it: both musl builds,
  `file | grep -q static`, and the installer end to end. If a build stops being static, check
  `cargo tree -e features -i wayland-sys` for something that enables `wayland-client/system`.
- **`install.sh`:**
  - warn about a missing `xrandr` only outside a Wayland session (`[ -z "${WAYLAND_DISPLAY:-}" ]`);
  - the message for systems other than Linux says "X11 or a wlroots-based Wayland compositor";
  - it stays POSIX: `shellcheck -s sh`, and dash through `OUTLAY_TEST_SH`.
- **Metadata:** the Cargo `description`, `keywords` (`wayland`, `wlroots`, `kanshi`) and the
  `src/cli.rs` about text stop saying "xrandr only".

## Sessions and PRs

| Session | Phases | Branch | PR title | PR description file (in `~/workspace/claude-cage/pr-descriptions/`) |
|---|---|---|---|---|
| D | W0–W3 | `feat/backend-neutral` | `outlay: backend-neutral core and scale editing` | `pr-description-outlay-backend-neutral.md` |
| E | W4–W6 | `feat/wayland` | `outlay: Wayland support for wlroots compositors` | `pr-description-outlay-wayland.md` |
| F | W7–W9 | `feat/kanshi` | `outlay: kanshi profiles and Wayland docs` | `pr-description-outlay-kanshi.md` |
| G (later) | G1–G2 | `feat/gnome` | `outlay: GNOME support` | `pr-description-outlay-gnome.md` |
| H (later) | H1–H2 | `feat/kde` | `outlay: KDE Plasma support` | `pr-description-outlay-kde.md` |

**Session start** (every session):
1. `git switch main && git pull --ff-only`. Check with `gh pr list --state merged` that the
   previous session's PR is merged. If it is not, stop and ask.
2. `git config user.email` must print the gmail address.
3. Read `CLAUDE.md`, `docs/PLAN-wayland.md` and the code your phases build on. Run `cargo test` for
   a green baseline. Read `MISTAKES.md` only when something fails, by grepping it for the error.
4. `git switch -c <branch>`.

**During a session.**
- Do only your phases, one commit per phase.
- Every commit passes `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings` and
  `cargo test`.
- Check the git identity after every commit.

## Implementation phases

### Session D: a backend-neutral core

Nothing changes on X11 except scale editing and the wording. D is the largest session by churn:
about 50 `xid`, 90 `transform` and 40 `edid` sites. Keep W1 mechanical.

**W0 `docs: wayland plan`.**
- Copy this file to `docs/PLAN-wayland.md`.
- In `docs/PLAN.md` → "Portability and environment checks", replace "A Wayland backend … is out of
  scope" with a pointer to the new file.
- Add one line to `CLAUDE.md`.

**W1 `refactor: a neutral output model`.**
- Add `Caps`/`Kind` (X11 values only), `ModeId`, `Identity` (move `edid.rs` into `src/model/`),
  `Scaling` with `effective_size`, `Option<ScreenLimits>` and `Layout.caps`.
- No behaviour change: every existing test and snapshot passes unchanged.

**W2 `refactor: apply plans instead of xrandr argv`.**
- Add `src/backend/` with the trait (`is_live` replaces `touches_x`; it is used by `tui::confine`,
  `src/tui/mod.rs:116`), `Plan` with `PrimaryRule`/`PlanForm`/`ScalingChange`, `Verdict`,
  `ApplyOutcome` and `dump`.
- Move `FixtureBackend` and `DryRun` there, with the Plan simulator.
- `xrandr::command` renders argv from a plan, and `argv_to_plan` is added.
- Update the carriers and make the wording neutral.
- **Tests:**
  - every golden listed in "Plans instead of argv" stays byte-identical;
  - the Xid-form round trip, Plan → argv → Plan;
  - `tests/apply.rs`: `Fake` (`:36-123`) scripts outcomes on plans. `Skips(name)` drops a planned
    output, and verification still catches it. Asserts compare plans, plus `applied_argv()` where
    the X11 text matters;
  - only the wording changes in `.snap` files.

**W3 `feat: edit the scale`.**
- The picker, `<`/`>`, `:scale F`, `Layout::set_scale`, the `diff` fix, the X11 `--scale` rule,
  badges and panel rows.
- **Tests:**
  - key sequences (`x` ↓ ⏎, `> > <`);
  - a display stuck right-of a scaled one follows it;
  - the push pass on growth;
  - golden argv: `--scale 1.5x1.5` only for the changed output; a reset gives `--transform none`;
    an unchanged scaled output writes nothing;
  - `diff` reports ×1.25 → ×1.5;
  - an insta snapshot of the picker.
- **Live X11 check, only after the user says yes in that session.** Apply `×1.25` on one output
  with a 10 s countdown and let it revert. If they decline, say so in the PR.

### Session E: the wlroots backend

E is the largest session by risk. It sets up the harness first, so the backend is written against a
real compositor.

**W4 `feat: read wayland outputs`.**
- The capture parser and writer, `--from-file` detection, `--demo=wayland`, `outlay dump`, the
  fixtures and `src/wayland/command.rs`.
- The Wayland branches of the model: orientation rule, sort order, `Logical` scaling. Confirm the
  transform mapping and cite the source.
- The gating (keymap `needs`, command line, inference, the profile refusal) and Wayland validation.
- `list` and `show` on Wayland captures.
- Test helpers: `tests/common` gets `desk_wl(&[Out])`, a Wayland-kind snapshot with logical scaling,
  no CRTCs, no screen limits and no primary.
- **Tests:**
  - parsing the fixtures, checking real values;
  - capture → Snapshot → capture round trip;
  - `--from-file *.json` CLI goldens;
  - `p` and `=` on Wayland: a message, no undo step, absent from help;
  - no `Same` link on Wayland;
  - no false screen-maximum error;
  - the scenarios in `tests/scenarios.rs` that do not depend on primary, mirroring or stale outputs
    (choose them one by one), run again on `desk_wl`;
  - the proptest invariants with a Wayland variant that places displays by logical size at scales
    1, 1.25, 1.5 and 2, and expects overlaps instead of `Same` links;
  - the full `--demo=wayland` editor flow in `tests/tui.rs`: stick, scale, apply → countdown →
    revert through `FixtureBackend`;
  - a snapshot of the Wayland demo overview.

**W5 `feat: drive wlroots compositors`.**
- **First, the harness** (`tests/common/sway.rs`):
  - start sway as described in "Headless sway", with an empty config and the short runtime dir;
  - wait for the socket for up to 5 s;
  - find `SWAYSOCK` by glob;
  - kill sway on drop;
  - run only when `OUTLAY_TEST_SWAY=/path/to/sway` is set; otherwise print "skipped" and pass, as
    `OUTLAY_TEST_SH` does.
- **Locally** sway is not installed. Ask before `sudo pacman -S sway wlr-randr`. If the user
  declines, iterate through CI.
  - An HTTPS push that changes `ci.yml` needs the `workflow` scope (`MISTAKES.md`, "An HTTPS push
    that touches `.github/workflows/`"). Push over SSH, or suggest
    `! gh auth refresh -h github.com -s workflow`.
- **Then dump the real event sequence** of headless sway (heads, modes, `done`) before an apply and
  after it. This settles the virtual-mode question, and `MISTAKES.md` records the answer.
- **Then the code:** `WlrBackend`, `backend::detect` (update `live_x_needs_an_x_session`,
  `src/xrandr/mod.rs:530-548`), the test step and `Effect::Test`, waiting for `done`, `cancelled`
  with one retry, `restore_program`, the hidden `outlay restore`, the Wayland `revert.sh`, and the
  compositor label. Remove the Wayland refusal from the README.
- **Tests** (`tests/wayland_live.rs`, plus unit tests):
  - the `detect` choice table: Wayland, Wayland without a socket, GNOME, KDE, X11, none;
  - a golden for `revert.sh` (with `CARGO_BIN_EXE_outlay`);
  - `restore` through the CLI against `FixtureBackend`;
  - `tests/apply.rs` on a Wayland snapshot: keep, timeout, `cancelled` then a retry, a rejected
    test (the popup blocks, `outlay apply` refuses), a verification mismatch;
  - live: the query sees `HEADLESS-1/2`;
  - live: an apply that moves, rotates (90) and scales (1.25, 1.5, 1.75), and a re-query that
    matches;
  - live: `swaymsg -t get_outputs` `rect` equals outlay's logical rect, which pins the truncation
    rule;
  - live: off and on again;
  - live: `swaymsg create_output` / `output HEADLESS-3 unplug` seen through `requery`;
  - live: a stale serial gives `cancelled` and one retry, through a `#[doc(hidden)]` entry point
    that skips the fresh roundtrip;
  - live: `sh revert.sh` restores;
  - live: with `wlr-randr` installed, `wlr-randr --json` and `outlay dump` parse to equal
    snapshots.
- **Static build:** install the musl target only with the user's OK; otherwise trust
  `release.yml` on the PR.

**W6 `test: wayland end to end in CI`.**
- **CI** (`.github/workflows/ci.yml`): a `wayland` job on `runs-on: ubuntu-24.04`, with a
  10-minute timeout:
  ```sh
  sudo apt-get install -y --no-install-recommends sway wlr-randr
  OUTLAY_TEST_SWAY=/usr/bin/sway cargo test --test wayland_live
  ```
- **The binary in a pty.** Run `tools/pty_drive.py` `keep` and `timeout` with `WAYLAND_DISPLAY`
  pointing at headless sway.
  - **Isolate it.** Set `XDG_CONFIG_HOME` and `XDG_STATE_HOME` to temporary dirs. Otherwise the
    real `post_apply` hooks (feh, nitrogen) hit the live X display, and the real
    `~/.local/state/outlay/revert.sh` is overwritten.
  - Make the display number in `keep` a parameter (`tools/pty_drive.py:129` hard-codes `3`;
    headless has two heads).
  - Check the results with `swaymsg`.
- **Fixture:** capture `tests/fixtures/wayland/headless-sway.json` with `outlay dump`.

### Session F: kanshi profiles and docs

**W7 `refactor: a neutral profile spec and store`.**
- As in "Profiles: kanshi". No behaviour change on X11: `tests/profiles.rs` and the script round
  trip (`tests/properties.rs:290-300`) pass unchanged.

**W8 `feat: kanshi profiles`.**
- `src/wayland/kanshi.rs`, the store, the config key and flag, `w`/`e`, `apply`/`save` on Wayland,
  the remap by name or description, and the kanshi interplay check.
- **Fixtures:** synthetic configs in `tests/fixtures/kanshi/`, built from the kanshi(5) examples.
  They cover several profiles, `include`, `exec`, comments, block and inline outputs, description
  criteria, `*` and `...output`.
- **Tests:**
  - parsing each fixture;
  - saving keeps every other byte (golden diff);
  - Layout → block → parse round trip (positions, modes, scale, transform);
  - the criteria rules;
  - the remap of an unknown description;
  - `w`/`e` through the Session with temporary files;
  - CLI `apply -n` and `save` with `--kanshi-config`.

**W9 `docs: wayland`.**
- **README.** A "Wayland" section:
  - which compositors work and which don't yet;
  - what differs: no primary, no mirroring, the scale direction and `150%`;
  - kanshi profiles;
  - how long a change lasts on each compositor, including the Hyprland caveat and the kanshi
    interplay found in W8;
  - `revert.sh` and `restore`.

  Refresh the comparison table and the `--from-file` and `--demo=wayland` docs.
- **CLAUDE.md:** the run modes (`--demo=wayland`, `--from-file *.json`, the headless sway tests),
  and the rule "never touch `~/.config/kanshi`".
- `install.sh` messages and the Cargo metadata.
- **Screenshots:** the Wayland demo overview and the scale picker, taken with `tools/shot.sh` and
  `--demo=wayland`. It opens a kitty window on the X display, so **ask first**.
- **After the merge,** offer a 0.2.0 release (the `CLAUDE.md` release steps). Push the tag only on
  the user's go-ahead.

### Later session G: GNOME (outline)

- **API:** `org.gnome.Mutter.DisplayConfig`, through `zbus` 5's blocking API (pure Rust).
- **Reading:** `GetCurrentState` gives:
  - the serial;
  - monitors: connector, vendor, product, serial, and modes with string ids and
    `supported_scales`;
  - logical monitors: x, y, scale, transform, primary, member monitors;
  - `layout-mode`. Read it every time: 1 means logical, 2 means physical.
- **Model:**
  - `ModeId` interns Mutter's id strings.
  - Caps: `primary`, and `mirror` (several monitors in one logical monitor).
  - The scale picker offers only `supported_scales`.
- **Writing:** `ApplyMonitorsConfig(serial, method, …)`.
  - Method 0 (verify) is `Backend::test`.
  - Monitors that are not listed are disabled, so always list all of them.
  - A stale serial gives `AccessDenied` ("stale information"): refresh and retry once.
- **Spike first: how to keep a change.** The proposal: apply with method 1 (temporary: no GNOME
  dialog, not saved) for outlay's countdown, and re-apply with method 2 (persistent) on keep. But
  method 2 makes gnome-shell show its own 20 s "keep changes?" dialog, which must be confirmed as
  well.
- **Hotplug:** the `MonitorsChanged` signal, or the 2 s requery.
- **Profiles:** none. kanshi does not run on GNOME, and GNOME keeps `monitors.xml` itself.
- **Testing:** `dbus-run-session mutter --headless --wayland --no-x11 --virtual-monitor 1920x1080`.
  Bare mutter confirms persistent changes automatically.
- **Caveat:** the API is "semi-private"; `gdctl` (GNOME 48+) uses it.

### Later session H: KDE Plasma (outline)

- **Protocols:** `kde_output_device_v2` and `kde_output_management_v2`.
- **Bindings:** `wayland-protocols-plasma` 0.3.12 is too old for mirroring (device v11 and
  management v12; mirroring needs management v13). Generate bindings from the current
  plasma-wayland-protocols XML with `wayland-scanner`, vendored in `protocols/`.
- **Applying:**
  - There is no serial and no test.
  - `apply` answers `applied`, or `failure_reason` followed by `failed`.
  - KWin wants no gaps and no overlaps, so "floating" and "overlap" become blocking errors on KDE.
- **Persistence:** KWin keeps the config itself (`kwinoutputconfig.json`). No profiles at first.
- **Testing:** `kwin_wayland --virtual --width W --height H --output-count N`.

### Later, unscheduled

- Mirroring through compositor extensions:
  - COSMIC `zcosmic_output_manager_v1`, which also gives exact scales;
  - Hyprland `hl.monitor{ mirror = … }` through `hyprctl eval`.
- A VRR toggle (`set_adaptive_sync`).
- Custom modes from `:mode` on Wayland.
- shikane profiles.
- Profiles on GNOME and KDE.

## Verification

- **Every commit:** the three CI commands. Review every `.snap.new` before renaming it; there is no
  `cargo insta`.
- **Manual runs that touch no live display:**
  - `cargo run -- --demo=wayland`;
  - `--from-file tests/fixtures/wayland/laptop-scaled.json` with `show`, `list` and `dump`;
  - the full apply → countdown → revert flow in `--demo=wayland`;
  - `cargo run -- --demo`, which must look as before, apart from the wording.
- **A real compositor (headless sway):**
  - `OUTLAY_TEST_SWAY=$(command -v sway) cargo test --test wayland_live`;
  - the isolated pty scenarios;
  - the same job in CI on `ubuntu-24.04`.
- **Static release:** `release.yml` on the PR (both musl targets, `file | grep static`, the
  installer end to end).
- **Live X11:** only the W3 scale check, and only after the user says yes in that session.

## Session rules (additions to `docs/PLAN.md` → "Session rules")

- **The live X display** is unchanged: no state-changing `xrandr`, layout script or outlay apply
  without asking.
- **Headless sway** may run without asking once it is installed, but only:
  - with `WLR_BACKENDS=headless`;
  - with `DISPLAY` and `WAYLAND_DISPLAY` removed;
  - with a short temporary runtime dir;
  - for pty runs, with temporary `XDG_CONFIG_HOME` and `XDG_STATE_HOME`.

  Installing packages (`pacman -S sway wlr-randr kanshi`, `rustup target add`) needs the user's
  yes.
- **Never read or write `~/.config/kanshi/`.** Copy configs into `tests/fixtures/kanshi/`, and
  point tests at temporary files. `~/.screenlayout` stays read-only.
- **Windows:** `tools/shot.sh` and nested sway (`WLR_BACKENDS=x11`) open windows on the developer's
  display, so ask first, every time.
- **Finishing every session:**
  1. Add the traps that cost time to `MISTAKES.md` (`docs: record traps`).
  2. Write the PR description. This is a GitHub repo: English prose and English headings, **no**
     `## Screen z widoczną zmianą w GUI` heading, and **no** GitLab quick actions. Put screens or
     text samples under English headings.
  3. `gh pr create --base main --title … --body-file …`, with the session link line the harness
     provides at the end.
  4. Never merge.

## Risks and early checks

| Risk | Checked in | Fallback |
|---|---|---|
| The direction of Wayland `90` and the flip order, against X11 | W4 (sway-output(5), wlroots source, cited in a comment) | Fix the mapping table; W5 checks the axes live |
| wlroots' virtual modes make `set_mode` fatal | Start of W5 (dump the event sequence) | Already designed around: set the mode only on a change |
| The logical size rounds instead of truncating | W5 (`swaymsg` rect at 1.25, 1.5, 1.75) | A rounding field in `Caps` that `detect` sets per compositor |
| State arrives after `succeeded`, and verification reverts by mistake | W5 (live apply test) | Already designed around: wait for `done` |
| Headless sway does not start in CI | Start of W5/W6 | labwc headless; or rely on the fake backend plus capture fixtures |
| `wayland-client` links libwayland, so the build is not static | W5 (`release.yml` on the PR) | `cargo tree -e features -i wayland-sys` and turn off the feature responsible |
| W2 changes an X11 golden | W2 | Fix the renderer, never the golden |
| kanshi re-applies its profile during outlay's countdown | W8 (headless sway, with the user's OK to install it) | Document it; suggest pausing kanshi while editing |
| Hyprland: `test` always succeeds, and changes are dropped after outlay exits | Documented in W9 | Recommend kanshi or `hyprland.lua` |

## After approval

The planning session implements nothing. It writes one self-contained handoff prompt per session
next to this file: `we-have-a-working-purring-eagle-handoff-{d,e,f}.md`. Each prompt carries the
session's phases, its start and finish steps, and the file:line facts it needs.

Suggested effort per session:
- **D: xhigh.** The churn is wide, and the goldens must stay byte-identical.
- **E: xhigh.** Protocol risk, and CI iteration without a local sway.
- **F: high.**
