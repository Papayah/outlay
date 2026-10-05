# outlay: keyboard-driven xrandr layout editor (plan + handoff)

> **Handoff.** This file is the full brief for three implementation sessions: A, B and C. Each one
> starts fresh in `/home/mc2/workspace/this/outlay` and delivers one PR (see "Sessions and PRs").
> The user creates the empty directory before session A. The planning session made no code
> changes. Read the whole file before running any command. This file stays the source of truth;
> Phase 0 copies it into the repo as `docs/PLAN.md`.

## Context

Today the user arranges monitors with arandr, a GUI that needs a mouse. They keep 19 arandr-style
scripts in `~/.screenlayout/`. They want a terminal tool that:
- arranges monitors with the keyboard only;
- draws the layout to scale in the terminal, so they can check it before applying;
- lets them stick one display to a chosen side of another;
- edits modes and refresh rates.

It must feel intuitive and satisfying, and ideally run on other Linux distros.

**Prior art.** Small xrandr TUIs exist:
- `vrandr` (ratatui, on the AUR);
- `tuirandr` (Python Textual);
- `hhhhhhhhhn/trandr` (Go; its README says "still buggy").

They are list- and menu-driven. None of them has any of the features that define outlay:
- a to-scale spatial canvas;
- persistent stick links that re-flow on changes;
- snap and swap movement;
- seam visualization;
- an auto-revert safety net.

## Fixed decisions (made with the user; do not re-ask)

| Topic | Decision |
|---|---|
| Name | **outlay**, used for the binary, the crate and the directory. It is a pun on xrandr "outputs" + layout. The name is free on crates.io, AUR, pacman and Debian. |
| Stack | **Rust 2024 + ratatui 0.30** (crossterm backend). One binary; its only runtime dependency is the `xrandr` program. |
| Sticking | **Persistent links.** A stuck display follows its target when the target's mode, rotation or position changes. Links are inferred from touching edges at load. |
| Direction keys | **Vim `h j k l`** plus the arrow keys. Plain key = focus, Shift = snap-move, Alt = nudge. This matches the pattern of the user's i3 bindings. The four letters can be changed in the config. |
| Hosting | **GitHub, private repo `Papayah/outlay`**, created with `gh` (logged in as Papayah over SSH). Licensed **GPL-3.0-or-later** (`LICENSE` holds the text from gnu.org; every dependency is MIT or Apache-2.0). |
| Git identity | **Author and committer: `Papayah <maciej.chmiest@gmail.com>`.** The global git identity is the work one and must never appear in this repo. See Session rules. |
| Delivery | **Three sessions, one PR each.** A = phases 0–2, B = phases 3–4, C = phases 5–6. The user merges each PR before starting the next session. |

## Environment facts (verified 2026-09-26 on the developer's laptop)

- **Session.** X11 with i3 (`$mod` = Super, so Alt is free for outlay). kitty terminal (`TERM=xterm-kitty`). xrandr 1.5.4, RandR 1.6, screen maximum 16384x16384.
- **GPUs.** Hybrid, two providers, 4 CRTCs each:
  - `modesetting`: eDP-1, DP-1..5, HDMI-1;
  - `NVIDIA-G0`: HDMI-1-0, DP-1-0..4.
- **Live outputs:**
  - `HDMI-1-0`: Philips TV (EDID name "Philips FTV"), 1920x1080@59.94 at 0,0;
  - `eDP-1`: AUO panel (EDID text "B156HAN12.H"), primary, 1920x1080@165.01 at 1920,0;
  - 13 other outputs are disconnected;
  - `DP-1-4` is disconnected but still lists modes.
- **Output names vary across setups.** The user's scripts use `eDP-1` and `eDP-2`, MST names (`DP-2.1`, `DP-1-2.2`) and `DVI-I-2-1`. Script traps to reuse as fixtures:
  - `mirror-work-setup.sh` mirrors two outputs by giving them identical positions;
  - `home-setup-rotr.sh` has a minimum x of 16;
  - `home-setup_save.sh` asks for 60.00 Hz where only 59.94 exists;
  - `monitors-only-dynamic.sh` repeats `--output eDP-1 --off`.
- **Tools.** cargo/rustc 1.91.1 with clippy and rustfmt; only the gnu target is installed. `gh` 2.101, `scrot`, `import` (ImageMagick), `xdotool`, `kitten`, arandr 0.1.11 and autorandr 1.15 are all present.
- **Crates** (checked today; all build with Rust 1.91):
  - ratatui 0.30.2 (MSRV 1.88). It has `Block::merge_borders(MergeStrategy::Fuzzy)` and dashed `BorderType`s (`LightDoubleDashed` …).
  - crossterm 0.29. Use the `ratatui::crossterm` re-export, not a separate dependency.
  - clap 4.6 (+ clap_complete), anyhow 1, thiserror 2, shell-words 1.1, serde 1, toml 1.1, dirs 7, signal-hook, base64.
  - Dev: insta 1.48, proptest 1.11.
- **Conventions from the sibling project `../split`:**
  - edition 2024, `rust-version = "1.91"`;
  - conventional commits (`feat:`, `fix:`, `docs:` …);
  - CI with `dtolnay/rust-toolchain@1.91.1` + `Swatinem/rust-cache@v2`, running `fmt --check` → `clippy -D warnings` → `test`;
  - English PR descriptions written as prose, one commit per plan phase.

## Product design

### CLI (clap derive)

```
outlay                      open the TUI (default)
outlay show                 print the to-scale diagram and output table, then exit
outlay list                 list outputs, then resolutions with their rates
outlay apply <profile>      apply ~/.screenlayout/<profile>.sh or a path (-n prints the command only)
outlay save <profile>       save the live layout as an arandr-compatible script
outlay keys                 print the keymap (generated from the keymap table)
outlay completions <shell>

global: --from-file <xrandr-verbose.txt>   read state from a capture; never touches X
        --demo                             built-in 4-output fixture; never touches X
        -n, --dry-run                      live state, but show commands instead of running them
        --layouts-dir <dir>  (~/.screenlayout)   --revert-timeout <s> (15; 0 = off)   --no-anim
```

- **`--demo` and `--from-file`** use `FixtureBackend`. It simulates an apply: it records the argv and updates its snapshot, so the whole apply → countdown → revert flow can be tried with no real displays.
- **`<profile>`** means `<layouts-dir>/<profile>.sh`. A value containing `/` or ending in `.sh` is used as a path.

### TUI screen

```
 outlay · 3 on · 1 off                                               ● 2 pending · a apply
┌ layout ─────────────────────────────────────────────────┐┌ 2 DP-1-2 ★ ─────────────────┐
│               ┏━━━━━━━━━━━━━━━━━━━━━━━━━━┓              ││ Dell U2719D                 │
│┌─────────────┲┛ 2 DP-1-2 ★  Dell U2719D  ┃              ││ mode  2560x1440 @ 143.91    │
││ 1 HDMI-1-0  ▸  2560x1440 @ 143.91       ┃              ││ pos   1920,0   rot normal   │
││ Philips FTV ┃                          ┃              ││ link  anchor                │
│└─────────────┺━━━━━┳━━━━━━━━━━┳━━━━━━━━━┛              ││ size  597x336 mm · 109 dpi  │
│                    │ 3 eDP-1 ▴│                        │├ off ────────────────────────┤
│                    └──────────┘                        ││ 4 DP-1-3   LG HDR 4K        │
└─────────────────────────────────────────────────────────┘└─────────────────────────────┘
 hjkl focus · HJKL move · s stick · m mode · r rate · o rotate · ␣ on/off · u undo · a apply · ? help
```

- **Canvas.** A custom widget writes into the `Buffer`. Do not use the braille `Canvas`, because text labels matter.
  - Each box covers cells `x0..=x1` and `y0..=y1`, with every edge rounded on its own (`round(px * s)`). Touching displays therefore share one border line.
  - Draw each box as a `Block` with `.merge_borders(MergeStrategy::Fuzzy)` so the joins become `┬ ┴ ┼`.
  - Cell aspect ratio: take it from `window_size()` pixels, re-read on every Resize event. Fall back to 2.0 (tmux reports 0), and let the config `cell_aspect` override it. Tests inject 2.0.
- **Viewport** (sticky, so the view does not jump around):
  - keep scale and origin between frames;
  - re-fit only when the bounding box leaves the view or fills less than 50% of it;
  - shift the origin by each normalisation delta, so displays that did not move stay still on screen;
  - `z` forces a re-fit.
- **Box contents.** Number, output name, ★ (primary), ● (pending), EDID model, `WxH@rate`, a rotation marker and a `×1.5` scale badge. Labels truncate. Below 3x3 cells, show the number only.
- **Mirrors.** A mirrored pair is drawn as one box labelled `2=4`.
- **Colours.** Each output gets a distinct named ANSI colour; respect `NO_COLOR`. Red, green and yellow are reserved: red = overlap, green = seam, yellow = warning.
- **Focus and structure:**
  - the focused box gets a thick border and is drawn last;
  - displays in its moving set (its descendants) are tinted in its colour;
  - the stick target gets a double border;
  - the ghost preview gets a `LightDoubleDashed` border.
- **Seams.** The shared-edge segment is drawn green, because that is where the mouse crosses. A stick link shows a glyph at the seam's midpoint, pointing from child to parent (`◂ ▸ ▴ ▾`).
- **Overlaps.** Overlapping cells get a red `░`.
- **Details panel** (`i` toggles it; it hides below 90 columns): EDID vendor, model and serial; mode `old → new`; position, rotation and scale; the link as text ("stuck right-of 1 HDMI-1-0, top") or "anchor"; physical size and DPI. Below it, a tray lists connected outputs that are off.
- **Status line.** The last message, coloured by severity, plus persistent validation warnings.
- **Hint line.** Context-sensitive and generated from the keymap table.
- **Small terminals.** Below 60x16, show a "terminal too small" screen.

### Keymap (one table in `tui/keys.rs` drives dispatch, hints, `?` help and `outlay keys`)

| Key | Action |
|---|---|
| `h j k l` / arrows | Focus the nearest enabled display in that direction. Candidates are displays whose centre lies in that half-plane. Score = main-axis distance + 2 × cross-axis offset. |
| `Tab` / `Shift-Tab`, `1`–`9` | Cycle focus / jump to display N. Numbers follow xrandr order at startup and stay fixed for the session; a display connected later gets the lowest free number. Displays in the off tray can be focused by number too. |
| `H J K L` / Shift-arrows | Snap-move (see Movement). |
| `Alt-h/j/k/l` / Alt-arrows | Nudge by the step. `+`/`-` cycle the step through 1/5/10/50/100 px (default 10); the current step appears in the status line. |
| `s` / `S` | Stick (see Stick flow) / unstick. |
| `m` / `[` `]` | Resolution picker / previous or next resolution in place. |
| `r` / `{` `}` | Rate picker / previous or next rate in place. |
| `x` / `<` `>` | Scale picker / previous or next scale in place (see `docs/PLAN-wayland.md`, "Scale editing"). |
| `o` / `O` | Rotate clockwise / counter-clockwise. |
| `p` | Make primary. |
| `Space` | Turn the output on or off. Refuses to turn off the last enabled display. |
| `u` / `Ctrl-r` | Undo / redo. A failed or no-op action adds no history step. |
| `a` | Apply (confirm popup). |
| `y` | Copy the pending command via OSC 52. |
| `R` | Refresh: re-probe the outputs and merge the result as the hotplug watch does (see Event loop), keeping pending edits and undo. No confirm. |
| `w` / `e` | Save a profile / open a profile (picker with mini previews). |
| `:` | Command line. |
| `z` | Re-fit the view. |
| `i` / `?` | Details panel / help. |
| `q` | Quit; confirms first if changes are pending. |

`Esc` only closes popups and cancels modes. It never quits.

Command line:
- `:pos X Y`, `:move DX DY`, `:mode WxH[@R]`, `:rate R`
- `:rotate normal|left|right|inverted`, `:reflect normal|x|y|xy`
- `:scale F` (0.25 to 8; `:scale 1` resets)
- `:stick A left-of|right-of|above|below|same-as B [start|center|end]`, `:unstick`
- `:primary`, `:on`, `:off`
- `:w [name]`, `:e name`, `:apply`, `:q`, `:q!`

An output can be given by number, name or a unique name prefix. Tab completes command words and
output names.

**Terminal input rules** (write tests for them):
- Handle `Press` and `Repeat`; ignore `Release`.
- Enable the alternate screen first, *then* push `DISAMBIGUATE_ESCAPE_CODES`, and only if `supports_keyboard_enhancement()` returns true.
- Wrap ratatui's panic hook so it pops the keyboard flags (and runs `revert.sh` if a countdown is active) before restoring the terminal.
- One normaliser handles modifiers:
  - `Char('h')+SHIFT` becomes `Char('H')`;
  - SHIFT is removed from non-letter chars;
  - `BackTab` and `Tab+SHIFT` are the same key.
- Never bind `Ctrl-h`, `Ctrl-j`, `Ctrl-i` or `Ctrl-m`: they collide with Backspace, Enter and Tab.

### Stick flow (`s`)

1. **Target.** Pick with a digit, `hjkl` (spatial) or `Tab`, then `Enter`.
   - The default target is the current parent, or else the nearest display.
   - With only 2 enabled displays, this step is skipped.
2. **Side.**
   - `h`/`j`/`k`/`l` = left-of / below / above / right-of. The ghost preview updates live.
   - `=` = mirror (same-as).
   - `Tab`/`Shift-Tab` cycle the alignment: start → center → end. The labels read top/middle/bottom for sides and left/centre/right for above and below.
   - The default alignment follows the tie rule below: `Start` for sides (xrandr's `--right-of` behaviour) and `Center` for above and below.
   - `Enter` commits, `Backspace` returns to step 1 and `Esc` cancels.

Example: `s 2 h ⏎` sticks the focused display to the left of display 2.

### Sticking model (the core; `model/links.rs`)

**Data.** A directed forest of links, `links[child] = Some(Link { parent, side, align, offset })`.
- `side` is `LeftOf`, `RightOf`, `Above`, `Below`, or `Same` (a mirror: the child has the parent's position, and align and offset are unused).
- `align` is `Start`, `Center` or `End`, measured along the shared edge.
- A display with no link is an **anchor**. Its stored position is authoritative.

**Placement.** For a child of size `w×h` and a parent rectangle `P`:
- `RightOf`: `x = P.x+P.w`. `LeftOf`: `x = P.x−w`. For both, `y = anchor(P.y, P.h, h, align) + offset`.
- `Below`: `y = P.y+P.h`. `Above`: `y = P.y−h`. For both, `x = anchor(P.x, P.w, w, align) + offset`.
- `anchor`: `Start` = s, `Center` = `s + (plen−clen).div_euclid(2)`, `End` = `s + plen − clen`.
- Offsets are clamped so the shared edge stays ≥ 1 px. A link never shrinks to a corner touch.

**Alignment rule** (used by inference, relink, orphan attach and the stick default):
- Choose the alignment with the smallest |offset|.
- Ties go to `Center` for `Above`/`Below` and to `Start` for `LeftOf`/`RightOf`.
- Why: a laptop nudged 10 px off-centre under a monitor becomes `Center+10` and stays centred when the monitor's mode changes.

**Edit pipeline** (`Layout::commit(before)`; every edit goes through it):
1. Apply the edit.
2. `relink()` if the edit set positions directly (move, swap, nudge, `:pos`).
3. `resolve()`: walk breadth-first from each anchor, placing each child from its parent.
4. **Push pass**: handles overlaps created by a resize.
   - For each pair of displays that touched in `before` and now overlap, where only one of the two moved or grew, push the other one's moving set outward along the old seam by the overlap depth.
   - Each display is pushed at most once per edit.
   - If a push creates a new overlap, undo that push and flag it.
   - Every other overlap is flagged; overlaps never block editing.
   - Example: in a 2x2 grid, B grows to 2560x1440 and D is pushed down 360 px.
5. If anything was pushed, run `relink()` and `resolve()` again.
6. **Normalise**: translate so the minimum x and y are 0, since xrandr positions cannot be negative.
   - On load, normalise only if the minimum is below 0, so `home-setup-rotr.sh` does not start with phantom pending changes.

**`relink()`:**
- (a) A link whose child still touches its parent on the recorded side (or is still identical, for `Same`) keeps its side and recomputes align and offset using the alignment rule.
- (b) Every other link is detached.
- (c) Displays detached in (b) attach, in index order, to the touching display with the longest shared edge that is outside their own subtree. Repeat until a pass attaches nothing. No cycle can form.
- (d) A display that was an anchor before the edit stays an anchor. The user's unstick is respected.

**Inference** (on load, and after loading a profile):
1. Displays with identical rectangles become `Same` children of the lower-index one, or of the primary if one of them is primary.
2. Two displays are adjacent when their edges touch exactly and share a length > 0.
3. The root of each component is the primary; otherwise the largest area, then the lowest index.
4. Walk breadth-first, visiting by longest shared edge, then index. Each display becomes a child of the display it was reached from, with alignment chosen by the rule above.

**Stick F → T** (side S):
1. If T is in F's subtree, detach F's child on the path to T. That child becomes an anchor.
2. Re-root T's tree at T: reverse each link on the path from T to its root and recompute align and offset from positions. T becomes the anchor.
3. Set `links[F] = (T, S, default align, 0)`.
4. Insert loop. Start with `cur = F`, `par = T`. While some display X ≠ cur is linked to `par` on side S, overlaps `cur`, and has not been moved in this operation:
   - relink X to `(cur, S, X.align, X.offset)`;
   - set `par = cur` and `cur = X`.
   
   Example: A|B then `stick F right-of A` gives A|F|B, whichever display was the root.
5. Commit.

For `Same` (mirror), switch F to T's resolution if F supports it (nearest rate). Otherwise keep F's resolution and warn that the smaller display shows the top-left part. There is no insert loop.

**Unstick (`S`).** Remove F's link. F stays an anchor at its position until it is stuck again. Growing neighbours still push it (push pass).

**Turning a display D off:**
- The heir H is:
  - D's `Same` child if there is one; it inherits D's link and D's children unchanged;
  - otherwise D's parent;
  - otherwise D's largest child, which becomes an anchor in place.
- Every other child X links to `(H, X.side, X.align, X.offset)` if that placement overlaps nothing. Otherwise X becomes an anchor in place.
- Save a restore record: H, D's offset from H, D's link, its former children and their links, and whether it was primary.
- If D was primary, primary moves to the largest remaining display, with a status message.

**Turning D back on:**
- If the restore record's H is enabled:
  1. place D at H + offset;
  2. restore D's link if its parent is enabled;
  3. re-parent former children whose links are still the ones set when D was turned off;
  4. restore primary.
- Otherwise, place D right-of the rightmost display, top-aligned, with its preferred mode.

### Movement (`model/snap.rs`)

**Moving set G(F)** = F plus its descendants (stuck things come along) plus any `Same` mirrors. If G would contain every enabled display, F moves alone together with its `Same` mirror. `relink()` then re-attaches the children it leaves behind.

**Snap-move `HJKL`.** Try these in order:
1. **Swap.**
   - Applies when F touches a display outside G on the move side. Take N = the touching display with the longest shared edge.
   - Swap the two along the move axis. Moving right: `N.x′ = F.x` and `F.x′ = F.x + N.w`. Mirror this for the other directions. Cross-axis coordinates stay the same.
   - Only F and N move, each with its descendants attached on the cross-axis sides and its `Same` mirrors.
   - If the swap would create an overlap, skip to the alignment stops.
   - This is how a row gets reordered: A|B|C with Shift-l on A gives B|A|C.
2. **Alignment stops.**
   - Candidate positions for F come from each display O outside G: F's start or end aligned with O's start or end, or F's centre aligned with O's centre.
   - Keep candidates strictly in the move direction and sort them by distance.
   - Take the first where (a) no member of G overlaps a display outside G, and (b) F touches some display outside G along an edge longer than 0.
3. **Nothing qualified.** Show a hint such as "No snap spot further right. Alt-l nudges freely." Never fail silently, and add no undo step.

**Nudge `Alt-hjkl`** moves G by exactly the step, with no filters. Gaps and overlaps are allowed and flagged. **`:pos`/`:move`** work the same way with exact values. All moves then run `commit()`, which re-links F.

### Mode, rate and rotation

- **Pickers.**
  - Resolutions sorted by area, then width, each with its highest rate; preferred (+) and current (•) marked.
  - Rates for the chosen resolution, with DoubleScan shown as "dbl" and interlaced as "i".
- **Quick cycling.** `[`/`]` skip interlaced and DoubleScan modes. `{`/`}` go through all rates of the current resolution.
- **Rate after a resolution change:**
  1. keep the previous rate if one exists within 0.5 Hz;
  2. otherwise use the preferred rate, if this resolution is the preferred one;
  3. otherwise use the highest progressive rate.
- **Effective size** = mode size × scale, with width and height swapped for `left`/`right`. The edit pipeline re-flows neighbours.

### Apply and safety

1. **Confirm popup.** It shows, in order:
   - a readable per-output diff (`DP-1-2  rate 59.95 → 143.91`, `DP-1-3  off → 3840x2160@60`);
   - errors that block the apply, then warnings;
   - the exact command.
   
   `Enter` applies and `Esc` cancels.
2. **Revert command.**
   - Build it from the live snapshot taken just before applying. Cover every connected-or-active output:
     - `--mode 0xXID --pos --rotate` and an explicit `--reflect`;
     - `--transform a,…,i --filter F`, or `--transform none`;
     - `--off` for outputs that were off, including ones the apply turns on;
     - `--primary`, or `--noprimary` if nothing was primary.
   - Write it to `$XDG_STATE_HOME/outlay/revert.sh` (mode 0755) before applying.
3. **Execute.**
   - Run xrandr with `std::process::Command`: stdin set to null, output captured.
   - Live apply and revert always pass `--mode 0xXID`. Scripts and `y` use name + `--rate`.
4. **Verify.**
   - xrandr exits 0 even for `warning: output X not found; ignoring`, so the exit code is not enough.
   - Re-query with `xrandr --verbose --current` and compare with the request: enabled set, XIDs, rectangles, orientation (rotation and reflection, as the same picture), primary. A turn by 180° or a reflection keeps the rectangle, so only the orientation shows that it was ignored.
   - A mismatch or a non-zero exit is a failure: show stderr and the diff, and revert if the live state changed.
5. **Countdown.**
   - A popup with a shrinking `LineGauge`: "Keep this layout? y keep · n/Esc revert · 15s".
   - After xrandr returns, **drain queued input and ignore keys for 1 s**, so a key pressed while the screens were black cannot confirm.
   - Only `y` keeps; `Enter` does not.
   - Revert on timeout, `n`/`Esc`, or Ctrl-C (which also quits).
   - On SIGHUP or SIGTERM, revert first, then restore the terminal and ignore EIO.
6. **Afterwards.**
   - **The revert is verified too.** Re-query after it and compare each output `Plan::restore` covers with the state from before the apply (`restore_mismatches`): on/off, XID, rectangle, orientation, primary. Positions are compared as read, not normalised. Neither the exit status nor a failed read-back counts as a restore. An output that was on and is now gone, or disconnected and off, was unplugged meanwhile (`unplugged`): no revert can bring it back, so it is a note ("eDP-1 was unplugged, so it is not back."), not a failure.
   - A primary with panning is left out of the restore like any panned output, but the restore makes it primary again (`--output eDP-1 --primary`, `PrimaryRule::Output`).
   - A revert that failed, did not restore everything, or cannot be read back is a "Revert failed" report: what is wrong, then "Run …/revert.sh to restore it." The editor takes in the state it read back, if any, and the panic hook stays armed. After the automatic revert of a failed apply, the same lines go into the "Apply failed" report.
   - `outlay apply` prints the report after the countdown. The editor prints a report the screen never showed (a revert on Ctrl-C or a signal), since the alternate screen takes it along, and exits 1, on a signal too; a report already on screen is not printed again.
   - After a revert, the user's edits stay pending.
   - After keep, keep the in-memory links if the re-queried geometry matches; otherwise re-infer.
   - **Hooks.** Run each `post_apply` hook with `sh -c` after every change outlay makes to the screens:
     - once an apply passes verification, **before** the countdown; the input drain, the 1 s block and the countdown start when the hooks are done;
     - after **every** revert: timeout, `n`/`Esc`, Ctrl-C, a signal, the revert on exit, and the automatic revert after a failed apply (even when the revert itself fails);
     - **not** on keep (nothing changes), and **not** after a failed apply that changed nothing.
   - Each hook runs with no terminal, stdin and stdout discarded, and a `post_apply_timeout` limit (10 s by default). Do not wait for background programs a hook leaves holding stderr: after the hook exits, give its stderr 200 ms, then move on.
   - Report failures (the first stderr line) after the status line's text, or as extra lines in an "Apply failed"/"Revert failed" report.
   - A simulated backend (`--demo`, `--from-file`, `-n`) runs no hooks and writes no `revert.sh`: `tui::confine` clears both.
7. **Event loop.**
   - Always poll with a timeout: at most 250 ms when idle and 16 ms while animating. The loop must never block, because `signal_hook::flag::register` replaces the default SIGTERM action.
   - **Hotplug.** Every 2 s in normal mode, `App::tick` asks for `Effect::Refresh { probe: false }`, a `--current` re-query (about 5 ms; it does not probe, so it never wakes the NVIDIA GPU). Checked live: `--current` sees HDMI and USB-C plugs on both providers.
     - Never during the countdown, while applying or with a popup open (they hold output indices), never in `outlay apply`, and never for a simulated backend (`tui::confine`).
     - An identical reading does nothing. Otherwise `Layout::remapped` moves the pending layout and every undo step onto the new outputs by name: edits stay, links to outputs that are gone are dropped, and a newly relevant output gets the lowest free number.
     - A newly connected display takes the focus, so `Space` turns it on. One unplugged while on is turned off in the pending layout as an undo step, if another display stays on. Nothing is applied.
     - When nothing was pending and the layout changed outside outlay, the pending layout follows the live one.
   - Tests inject the signal `AtomicBool` instead of sending real signals.

**Validation** (`model/validate.rs`), also shown live:
- **Errors** (block the apply): nothing enabled; a mode not in the output's list; a bounding box larger than the screen maximum.
- **Warnings:**
  - overlap between displays that are not mirrors;
  - a floating display (touches nothing, with 2 or more enabled);
  - more enabled outputs than a CRTC set can drive (heuristic based on `CRTCs:`);
  - a disconnected output that is still active;
  - panning is set: that output is read-only in v0.1;
  - no primary (info level).

**Stale outputs.** Disconnected outputs that are still active, typically left behind after undocking, appear as ghost boxes. The initial pending layout turns them off and a status message says so.

### Profiles (arandr interop, `xrandr/script.rs`)

- **Format.** Plain `#!/bin/sh` scripts in `~/.screenlayout`, in arandr's format. Links are not stored; inference rebuilds them.
- **Save.**
  - The file starts with `# generated by outlay <ver>`.
  - One `--output` per line, joined with ` \` continuations.
  - Every other known output gets `--off`.
  - Mirrors get identical `--pos` values, as arandr writes them.
  - Write atomically (temp file, then rename). Show a diff and ask before overwriting.
  - Preserve non-xrandr lines byte-for-byte.
- **Parse.**
  1. Strip comments and join continuations.
  2. Tokenise each `xrandr` invocation with `shell-words`.
  3. Understand: `--output --mode --rate/--refresh --pos --rotate --reflect --primary --off --auto --scale --same-as --left-of --right-of --above --below`. The relative options become links, and `--same-as` becomes `Same`.
  4. If an output repeats, the later options win.
  5. `--rate` picks the nearest available rate for that mode name, as xrandr does (60.00 → 59.94). Without `--rate`, take the first listed mode with that name.
  6. Unknown options produce a warning.
- **Remap on load.** Profile outputs that are not connected (e.g. `eDP-2`) open a remap dialog: pick a free connected output or skip. The pre-selection is a free output with the same connector prefix.

### Portability and environment checks

- **Refuse to use live X** when `WAYLAND_DISPLAY` is set or `XDG_SESSION_TYPE=wayland`.
  - The message: xrandr would only reconfigure XWayland; use wlr-randr, kanshi, hyprctl or the desktop's display settings.
  - Also refuse when `DISPLAY` is unset.
  - `--demo` and `--from-file` still work in both cases.
- **Missing `xrandr`.** Print the install command for the distro in `/etc/os-release`:
  - Arch: `xorg-xrandr`
  - Debian/Ubuntu: `x11-xserver-utils`
  - Fedora, openSUSE, Void and Alpine: `xrandr`
- **Backend trait.** `trait Backend { fn query(&self) -> Result<Snapshot>; fn apply(&self, argv: &[String]) -> Result<ApplyOutcome>; }`, implemented by `XrandrCli`, `FixtureBackend` and `DryRun`. Wayland support (a wlroots backend behind this trait, kanshi profiles, scale editing) is planned in `docs/PLAN-wayland.md`, which wins where the two differ.
- **Config.** `$XDG_CONFIG_HOME/outlay/config.toml` is optional, and every key has a default:
  ```toml
  revert_seconds = 15
  nudge_step = 10
  layouts_dir = "~/.screenlayout"
  # kanshi_config = "~/.config/kanshi/config"   # Wayland profiles; default: the file kanshi reads
  animations = true
  directions = "hjkl"        # focus letters: left, down, up, right
  # cell_aspect = 2.0        # auto-detected when omitted
  double_borders = false     # true: double border on the stick target and the focused display's parent
  post_apply = []            # e.g. ["feh --bg-fill ~/Pictures/wallpapers/*"]
  post_apply_timeout = 10    # seconds each post_apply command may run
  ```
- **Release profile:** `lto = true`, `codegen-units = 1`, `strip = true`. The README documents a static musl build (`rustup target add x86_64-unknown-linux-musl`).

### Distribution

- **One command.** `curl -fsSL https://github.com/Papayah/outlay/releases/latest/download/install.sh | sh` installs outlay; the same command updates it. No Rust toolchain is needed.
- **Releases.** A `v*` tag runs `.github/workflows/release.yml`: static musl builds on native runners (`x86_64-unknown-linux-musl` on `ubuntu-latest`, `aarch64-unknown-linux-musl` on `ubuntu-24.04-arm`), a check that the tag is `v` + the Cargo.toml version, a smoke test, an end-to-end run of the installer on the real tarballs, then `gh release create`. A tag with a `-` is a prerelease and never becomes "latest". Pull requests that touch the workflow, `install.sh` or the manifest run everything except the publish.
- **Asset contract.** `install.sh` depends on these names; change them only together with it. Each release has exactly:
  - `outlay-<version>-<target>.tar.gz` for both targets, holding the directory `outlay-<version>-<target>/` with `outlay`, `README.md` and `LICENSE` (`<version>` without the `v`);
  - `install.sh`;
  - `SHA256SUMS`, in `sha256sum` format, covering the tarballs and `install.sh`.
- **Latest.** `releases/latest/download/` works only for names without a version, so the installer reads `SHA256SUMS` there and takes the version from the tarball name in it.
- **Installer** (`install.sh`, POSIX sh; shellcheck `-s sh`, dash and busybox ash):
  - Flags: `--version X.Y.Z` (with or without the `v`), `--to DIR` (default `~/.local/bin`; a relative one is taken from the working directory), `--force`, `--no-completions`, `--uninstall`, `--help`. The only environment variable is `OUTLAY_RELEASES_URL` (default `https://github.com/Papayah/outlay/releases`).
  - It needs Linux, `curl`, `tar`, `sha256sum` and `mktemp`. An unknown architecture, or a release without a build for it, gets an error that points to `cargo install --git https://github.com/Papayah/outlay --locked`.
  - When the installed `outlay --version` equals the chosen release, it prints `outlay X.Y.Z is up to date` and downloads nothing. Otherwise it installs: a pinned older version downgrades.
  - The tarball is checked against `SHA256SUMS`, and the binary is staged as `<dir>/.outlay.new.<pid>`, run with `--version` there, and renamed over `outlay`. On any failure the old binary stays as it was.
  - Completions are generated by the new binary for the shells present (bash, zsh, fish) into the XDG locations, only for the default directory.
  - It warns when the directory is not on `PATH`, when another `outlay` comes first on `PATH`, and when `xrandr` is missing. It never edits shell rc files.
  - `--uninstall` removes the binary and the completions, and keeps `$XDG_CONFIG_HOME/outlay` and `$XDG_STATE_HOME/outlay`.
- **License:** GPL-3.0-or-later.
- **Later:** an AUR package, planned in its own session. It waits for the AUR account.

## Architecture

```
outlay/
  Cargo.toml  README.md  CLAUDE.md  MISTAKES.md  LICENSE  docs/PLAN.md  docs/screenshots/
  install.sh           curl | sh installer and updater (POSIX sh)
  .github/workflows/ci.yml  .github/workflows/release.yml
  src/
    main.rs  cli.rs  config.rs
    show.rs              `show` / `list`: canvas rendered into a Buffer, printed as text (colour only on a TTY)
    xrandr/mod.rs        Backend trait + XrandrCli / FixtureBackend / DryRun
    xrandr/parse.rs      `xrandr --verbose` → Snapshot
    xrandr/edid.rs       EDID → vendor / model / serial / mm
    xrandr/command.rs    Layout → apply argv, revert argv, script text   (pure)
    xrandr/script.rs     screenlayout script → layout spec
    model/mod.rs         Snapshot, Output, Mode, Rotation, Reflection, ScreenLimits
    model/layout.rs      Layout, OutputState, from_snapshot, commit() pipeline, diff against live
    model/geometry.rs    Rect, effective size, shared_edge, overlap, bbox, normalise
    model/links.rs       Link, Side, Align, infer, relink, resolve, push pass, stick, unstick, on/off
    model/snap.rs        swap, alignment stops, nudge, spatial focus
    model/validate.rs    errors and warnings
    model/history.rs     undo/redo (Layout clones, capped at 200)
    tui/mod.rs           init/restore, event loop, ticks, signals, keyboard flags, panic hook
    tui/app.rs           App + UiMode; handle_key(KeyEvent) -> Vec<Effect>   (pure)
    tui/keys.rs          keymap table + normaliser
    tui/canvas.rs        layout widget + sticky viewport
    tui/panels.rs        details, off tray, status line, hint line
    tui/popups.rs        pickers, confirm, countdown, help, command line, profile picker, remap
    tui/theme.rs
  tests/fixtures/xrandr/*.txt   tests/fixtures/screenlayout/*.sh   tests/scenarios.rs   tests/snapshots/
  tests/install.rs     install.sh against a file:// fake release
```

- **`UiMode`**: `Normal`, `Stick{step, target, side, align}`, `ModePicker`, `RatePicker`, `Command(String)`, `ConfirmApply`, `Countdown{deadline, input_blocked_until}`, `Help`, `ProfilePicker`, `Remap`, `SavePrompt`, `ConfirmQuit`.
- **Effects** such as `Apply`, `Revert`, `Refresh`, `SaveFile`, `Copy` and `Quit` are carried out by the loop in `tui/mod.rs`. Because `handle_key` is pure, tests can drive the whole app with no terminal.
- **Types:**
  - `Mode { xid, name, width, height, refresh: f64, interlaced, double_scan, preferred }`. A mode reference is (output, xid), because XIDs are shared across outputs.
  - `Output { name, connection, primary, modes, edid, physical_mm, crtcs, active: Option<ActiveConfig{xid, pos, rotation, reflection, transform, panning}> }`.

### xrandr parsing traps

- **Queries.** Use `xrandr --verbose`: it gives XIDs, exact refresh rates and EDID. Startup and `R` re-probe; re-queries after an apply and the hotplug watch use `--current`.
- **Screen line:** `Screen 0: minimum W x H, current W x H, maximum W x H`.
- **Output header:** `NAME (connected|disconnected|unknown connection)[ primary][ WxH+X+Y (0xID)][ rotation][ X axis|Y axis|X and Y axis] (normal left inverted right x axis y axis)[ Wmm x Hmm]`.
  - The name is the first token and may contain `-` and `.`.
  - The header size is the *effective* size (rotated and scaled).
  - A disconnected output can still be active.
  - `Wmm x Hmm` appears only for active outputs. For the others, read the size from the EDID detailed timing descriptor (bytes 54+12..14).
- **Property lines.**
  - A key line is one tab followed by a non-space character. Anything else continues the previous key.
  - `Transform:` continuation lines and `filter:` start with a tab plus spaces.
  - NVIDIA prints `PRIME Synchronization: \t\tsupported: ?, ?` on one line.
  - Parse `EDID:`, `CRTC:`, `CRTCs:`, `Transform:` (3 rows; `filter:` follows) and `Panning:`. Ignore everything else.
- **Mode lines:** `  NAME (0xID) CLOCKMHz flags… [*current] [+preferred]`, then `h:` and `v:` lines.
  - Width and height come from `h: width` and `v: height`. Names are not sizes: `1024x768i`, `1920x1080_60.00`.
  - The refresh rate is `v: … clock NN.NNHz`. If that is missing, use `clock/(htotal·vtotal)`, ×2 for interlaced and ÷2 for DoubleScan.
  - A disconnected output's modes (e.g. `DP-1-4`) belong to the header they follow.
- **EDID:**
  - manufacturer = bytes 8–9, as three 5-bit letters (`0x06af` → AUO, `0x410c` → PHL);
  - product = bytes 10–11 (little-endian); serial = bytes 12–15;
  - descriptors are at 54/72/90/108. A descriptor is text **only if bytes 0–2 are 0**; the tag at +3 is 0xFC (name), 0xFF (serial) or 0xFE (text). Text ends at 0x0A; trim the space padding.
  - Label = the 0xFC name, else the 0xFE text, else PNP + product hex.
  - Add a short PNP → vendor table: DEL, SAM, GSM, PHL, AUO, BOE, LGD, SHP, SDC, CMN, ACR, AUS/ASU, BNQ, HWP, LEN, AOC, ENC, VSC, MSI.
- **Apply command** (`command.rs`):
  - Include outputs in snapshot order that are connected or active, in one invocation.
  - Enabled: `--output N [--primary] --mode 0xXID --pos XxY --rotate R --reflect F`, plus `--scale SxS` when the scale changes and `--transform none` when it is reset to 1.
  - Disabled: `--output N --off`.
  - Script and `y` form: `--mode NAME --rate {:.2}`.

## Sessions and PRs

| Session | Phases | Branch | PR title | PR description file (in `~/workspace/claude-cage/pr-descriptions/`) |
|---|---|---|---|---|
| A | 0–2 | `feat/engine` | `outlay: xrandr reader and layout engine` | `pr-description-outlay-engine.md` |
| B | 3–4 | `feat/tui` | `outlay: interactive editor with auto-revert apply` | `pr-description-outlay-tui.md` |
| C | 5–6 | `feat/profiles-polish` | `outlay: screenlayout profiles and polish` | `pr-description-outlay-profiles-polish.md` |

**Session start (B and C; session A starts at Phase 0 because no repo exists yet):**
1. Run `git switch main && git pull --ff-only`. Check that the previous session's PR is merged with `gh pr list --state merged`. If it is not merged, stop and ask the user.
2. Check that `git config user.email` prints the gmail address. The local config stays in the repo, but set it again if it is missing.
3. Read `CLAUDE.md`, `MISTAKES.md` and the code your phases build on. Run `cargo test` to confirm a green baseline.
4. Create the branch from the table with `git switch -c <branch>`.
5. If `docs/PLAN.md` differs from this file, copy this file over it and commit `docs: sync plan` as the first commit on the branch. A section called "Changes after review", if one exists, overrides the earlier sections.

**During a session.** Implement only your session's phases, with one commit per phase. Every commit must pass `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`. When your phases are done, follow Finishing and stop. Do not start the next session's phases, even if time remains.

## Implementation phases

0. **Bootstrap (on `main`).**
   - `git init -b main`, then set the local identity **before anything else** (see Session rules).
   - `cargo init --name outlay`. Set edition 2024, `rust-version = "1.91"` and the release profile.
   - Add `.gitignore`, `.github/workflows/ci.yml` (mirror `../split`: fmt, clippy, test on `pull_request` and pushes to `main`), and a README skeleton.
   - Write `CLAUDE.md`: commands, run modes, the live-xrandr rule, the git-identity rule, "the keymap table is the only source of keys".
   - Add `MISTAKES.md`, seeded with the git-identity trap, and `docs/PLAN.md` (a copy of this file).
   - Commit `build: bootstrap outlay crate`.
   - `gh repo create Papayah/outlay --private --source . --remote origin --push`. `main` then holds only the bootstrap commit; everything else arrives through PRs.
   - `git switch -c feat/engine` for phases 1–2.
1. **`feat: read xrandr state`.**
   - Model types, `parse.rs`, `edid.rs`, the `Backend` trait with `XrandrCli` and `FixtureBackend`.
   - Fixtures:
     - `laptop-edp-hdmi.txt`, from a live `xrandr --verbose` (read-only);
     - synthetic variants: `dock-mst-3.txt`, `rotated-left.txt`, `active-disconnected.txt`, `no-edid.txt`, `scaled.txt`, `panning.txt`, `mirror.txt`, `unknown-connection.txt`;
     - `demo.txt`: HDMI-1-0 1920x1080 left of DP-1-2 2560x1440@143.91 (primary), eDP-1 1920x1080@165 centred below, and DP-1-3 connected but off. Embed it with `include_str!`.
   - `outlay list` and the table part of `show`.
   - Tests assert the real values: AUO "B156HAN12.H", "Philips FTV", 165.01 at 1920,0, maximum 16384.
2. **`feat: layout engine with persistent stick links`.**
   - `geometry`, `links` (the full pipeline), `snap`, `validate`, `history`, the mode/rate/rotation operations, and `command.rs`, including revert generation.
   - `tests/scenarios.rs`: golden desk layouts. Each scenario is a start layout plus key actions, checked against the expected rectangles and links:
     - a laptop centred under a monitor, then a nudge, then a monitor mode change: the laptop stays centred;
     - A|B|C reordered by swapping;
     - moving the root display;
     - rotating a portrait side monitor;
     - a resize in a 2x2 grid causing a push;
     - `stick F right-of A` inserting into A|B;
     - a mirror pair;
     - off/on restore.
   - proptest over random edit sequences:
     - coordinates are ≥ 0 and the minimum is 0;
     - every link is flush with a shared edge ≥ 1 px;
     - there are no cycles;
     - `resolve` is idempotent;
     - undo then redo gives an identical layout;
     - off then on gives an identical layout;
     - a snap never creates an overlap;
     - a failed snap adds no undo step.
   - Golden tests for the apply, revert and script commands.
3. **`feat: interactive layout editor`.**
   - Canvas (merged borders, seams, glyphs, overlaps, ghost, sticky viewport), panels, keymap table and normaliser, stick flow, pickers, command line, help. The `show` diagram shares the canvas.
   - insta snapshots through `TestBackend`: the demo overview, the stick ghost, the mode picker, 60x20, an overlap, help.
   - Key-sequence tests through `handle_key` (e.g. `s 2 h Enter`).
4. **`feat: apply with automatic revert`.**
   - Confirm popup with the diff, execute, verify, countdown (input drain and 1 s block), revert triggers, `revert.sh`, the panic hook, signals, `post_apply`, `config.rs`, environment checks, `y` (OSC 52, written between draws).
   - Tests use a fake Backend that records argv, can fail, and can return mismatching state. Cover: keep, timeout, error revert, a mismatch caught by verification, an injected SIGHUP flag, and a queued Enter that is ignored.
5. **`feat: screenlayout profiles`.**
   - Script parser and writer, `w`/`e` with mini previews, the remap dialog, the `apply`/`save` subcommands.
   - Fixtures copied from `~/.screenlayout/`: `home-setup`, `tv-home`, `monitors-only-dynamic`, `work-setup-2`, `mirror-work-setup`, `home-setup-rotr` and `home-setup_save`.
   - A round-trip test: Layout → script → parse gives the same Layout.
6. **`feat: polish`.**
   - Animations: about 120 ms ease-out when positions change after a discrete action; `--no-anim` and the config switch turn them off.
   - `+`/`-` step sizes, `outlay keys`, completions, Tab completion on the command line.
   - README: pitch, install, keymap, screenshots, how outlay differs from arandr.

## Verification

- **`cargo test`.** Before accepting insta snapshots, review them with `cargo insta review` or by reading the `.snap.new` files.
- **Manual runs** (no live displays touched), each once its phase exists:
  - session A: `cargo run -- list` and `cargo run -- show`, both read-only;
  - sessions B and C: `cargo run -- --demo`, `cargo run -- --from-file tests/fixtures/xrandr/dock-mst-3.txt` and `cargo run -- -n`. In `--demo`, try the full apply → countdown → revert flow.
- **Screenshot loop** (sessions B and C). Use it to check rendering yourself (open the PNG with the Read tool) and to capture PR images:
  ```sh
  S=<scratchpad>/outlay.sock
  kitty -o allow_remote_control=socket-only --listen-on unix:$S --class outlay-shot \
        -e ./target/release/outlay --demo &
  sleep 1.5; WID=$(xdotool search --class outlay-shot | tail -1)
  import -window "$WID" docs/screenshots/overview.png
  kitten @ --to unix:$S send-text 's2h'   # stick flow → docs/screenshots/stick.png, etc.
  ```
- **Live apply (session B, after phase 4 is committed), only with the user's explicit consent in that session.** Ask first. Then do a no-op apply (the unchanged current layout), followed by a reversible change such as the HDMI-1-0 rate. Let the countdown revert it and confirm the screens come back. If the user declines, say so in the PR under "how it was tested".

## Session rules (implementation session)

- **Git identity, before the first commit:**
  - Run `git config user.name "Papayah"` and `git config user.email "maciej.chmiest@gmail.com"` (local, never `--global`).
  - Check the result with `git config user.email`.
  - After **every** commit, `git log -1 --format='%an <%ae> | %cn <%ce>'` must show Papayah and the gmail address in both places.
  - Never commit with the work identity.
  - If GitHub rejects a push for email privacy (GH007), stop and ask the user. Do not switch to a noreply address on your own.
- **Displays.** Never run an xrandr command that changes state (`--output`, `--auto`, `--off`, a script) without asking first; the user's screens are live. These are read-only and always allowed: `xrandr --verbose`, `--query`, `--current`, `--listmonitors`, `--listproviders`.
- **Leave `~/.screenlayout` unedited.** Copy files into fixtures, and point test saves at temporary directories.
- **Finishing, every session** (the user's global CLAUDE.md, plus a GitHub exception the user decided on 2026-09-26). Take the branch, title and file name from the "Sessions and PRs" table.
  1. Append every trap that cost real time to `MISTAKES.md`, as symptom → cause → working fix. Commit it on the branch (`docs: record traps`) so it is part of the PR.
  2. Write the PR description to `/home/mc2/workspace/claude-cage/pr-descriptions/<file>`. This is a GitHub repo, so:
     - do **not** use `mr-descriptions/`, which is for GitLab MRs only;
     - do **not** add the GitLab quick-action block (`/assign`, `/request_review`, `/label`); it does nothing on GitHub.
     
     Content: English prose in the style of `../split` PR #7, covering what this PR adds, a reading guide (one commit per phase), the changes, and how it was tested.
     - Session A has no GUI yet: include a short `outlay show` sample as a text block.
     - Sessions B and C: add a `## Screen z widoczną zmianą w GUI` section with the committed `docs/screenshots/*.png`, linked as `https://github.com/Papayah/outlay/blob/<branch>/docs/screenshots/<f>.png?raw=true`.
  3. Push the branch and open the PR with the file as its body: `gh pr create --base main --title "<title>" --body-file <file>`. Never merge it.
  4. Report the PR URL, the phases completed, and anything the user must decide or test before the next session.

## Changes after review

> Added 2026-09-26 after the user reviewed the editor from sessions A and B (PRs #1 and #2, both
> merged). Where this section and the earlier sections differ, this section wins. Session C
> implements it first, as phases 4b and 4c, before phase 5. Its first commit on
> `feat/profiles-polish` is this file (`docs: sync plan`).

### R1. The focused display stands out (phase 4b)

**Problem.** The focused box differs from the others only by its thick border (`┏━┓`) and a
bold title. In kitty, thick box lines are hardly thicker than light ones. Displays in the moving
set take the focus colour too, so a focused display and its stuck child look alike.

**Change** (replaces "the focused box gets a thick border" in Product design → Focus and
structure):
- The focused box's title line is a chip: the title with one space on each side, drawn in its
  colour with `Modifier::REVERSED`, so the colour becomes the background. If the chip does not
  fit, the reversed title fills the inner width and is truncated with `…`, like other labels.
  With `NO_COLOR` set, reverse video alone still marks it. The off tray already marks the
  focused off display with reverse video, so reverse video means focus everywhere.
- The thick border stays, and the focused box is still drawn last.
- Displays in the moving set keep the focus colour, without the chip.
- A focused box below 3x3 cells shows its number in reverse video.

### R2. No double borders by default (phase 4b)

**Problem.** In Normal mode, `draw_canvas` (`src/tui/ui.rs`) passes the focused display's link
parent to the canvas as the stick target, so the parent gets `BorderType::Double`. kitty draws
`═ ║` as two thin lines per cell, so the parent looks fatter than the focused box and seems to
be the focused one. Double lines also cannot join cleanly with thick or dashed lines; the stick
ghost snapshot shows broken corners such as `┐════` and `╚══┳━━━`.

**Change** (replaces "the stick target gets a double border"):
- Normal mode marks no target. The seam glyph (`◂ ▸ ▴ ▾`) and the details panel's `link` row
  already show the link.
- In the stick flow, the target gets a thick border and a bold, underlined title. Draw order:
  the other boxes, then the target, then the focused box. Thick lines join thick and plain
  lines cleanly (`┳ ┨ ┯`).
- The old look stays available behind a config key, off by default:
  `double_borders = false`. When it is true, the Normal-mode parent and the stick target get the
  double border exactly as before. Pass it `Config` → `Options` → `App` → `Scene`, the same way
  as `nudge_step`. `Config` uses `deny_unknown_fields`, so add the key to `Config` and its
  `Default`, to the config sample in Portability and environment checks, and to the README.

### R3. Held nudges speed up (phase 4c)

**Problem.** A nudge moves one step per key event. At the default 10 px and a typical 25 Hz
key repeat, holding `Alt-l` moves 250 px/s, so crossing a 1920 px display takes about 8 s. Also,
every repeat is an undo step, so a long hold pushes the whole history (cap 200) out.

**Change:**
- **Detecting a hold uses time only.** outlay pushes only `DISAMBIGUATE_ESCAPE_CODES`, so a
  terminal reports a held key as repeated presses; no `Repeat` kind arrives. A nudge continues
  the current burst when all three are true:
  - it is for the same display as the previous nudge;
  - it is in the same direction;
  - it arrives at most 150 ms after the previous nudge.

  Any other key event (bound or not), or a longer gap, ends the burst. The autorepeat delay
  (usually 250–660 ms) ends the burst after the first press. So a single press always moves
  exactly one step, and the speed-up starts with the first repeat.
- **Ramp**, by time since the burst started: under 0.4 s ×1, 0.4–0.8 s ×2, 0.8–1.2 s ×5, after
  that ×10. The multiplier applies to the current step. At 10 px and 25 Hz this gives
  250 → 500 → 1250 → 2500 px/s, and 1920 px take about 1.7 s. Keep the 150 ms gap and the ramp
  table as `const`s in `src/tui/app.rs`.
- **Clock.** Use `App::now`. `Session::handle_event` already calls `app.tick(now)` before
  `handle_key`, so `handle_key` stays pure, and tests set the time with `tick`.
- **Feedback.** While the multiplier is above ×1, the step indicator shows it:
  `step 10px ×5`.
- **Undo.** A run of successful nudges of the same display in the same direction, with no other
  action between them, is one undo step, for taps and holds alike. Record a history step only
  for the first nudge of a run; the top of the undo stack then holds the layout from before the
  run. A failed or no-op nudge does not start a run.
- `+`/`-` (phase 6) change the base step, and the multiplier scales it. Phase 6 animations skip
  nudges, so a held key never lags behind an animation.
- No Ctrl+Alt binding.

### Tests and checks for R1–R3

- R1 and R2: canvas tests on cell styles and symbols, because insta snapshots store only text.
  They check that:
  - the focused title cells are `REVERSED` and no other cell in any box is;
  - with the default config, Normal mode and the stick flow draw no `═ ║ ╔ ╗ ╚ ╝`;
  - with `double_borders = true`, the stick target has a double border again.

  Add a `Config::parse` test for the new key. Review the changed `.snap.new` files (stick
  ghost, overview) before accepting them.
- R3: key-sequence tests in `tests/tui.rs` that move the time with `tick`:
  - `Alt-l` every 40 ms follows the ramp;
  - `Alt-l` every 200 ms never speeds up;
  - a change of direction starts again at ×1;
  - one `u` undoes a whole run;
  - a focus key between two nudges gives two undo steps.
- Refresh `docs/screenshots/overview.png` and `stick.png` with `tools/shot.sh`. That script opens
  a window on the developer's display, so ask the user first.
