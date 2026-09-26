# outlay

A keyboard-driven xrandr layout editor for the terminal. outlay draws your monitors to scale,
lets you arrange them with `h j k l`, sticks a display to a chosen side of another so it follows
when that one changes, and applies the result with an automatic revert if you do not confirm it.

> **Status: work in progress.** The editor works end to end: arrange, stick, change modes, and
> apply with an automatic revert (`outlay --demo` tries it on a built-in fixture, `outlay -n` on
> your live state without touching it). Profile support and polish follow. See
> [docs/PLAN.md](docs/PLAN.md).

## Why

arandr needs a mouse. The existing xrandr TUIs are list- and menu-driven. outlay is built around a
spatial canvas instead:

- a to-scale drawing of the layout, so you can check it before applying;
- persistent stick links that re-flow when a mode, rotation or position changes;
- snap and swap movement, seam visualisation, and an auto-revert safety net.

## Requirements

- Linux with an X11 session (Wayland compositors need their own tools, such as wlr-randr or kanshi)
- the `xrandr` program at run time
- Rust 1.91 or newer to build

## Build

```sh
cargo build --release
./target/release/outlay --help
```

## Usage

```
outlay                      open the editor (default)
outlay show                 print the to-scale diagram and output table, then exit
outlay list                 list outputs, then resolutions with their rates
outlay apply <profile>      apply ~/.screenlayout/<profile>.sh or a path (-n prints the command only)
outlay save <profile>       save the live layout as an arandr-compatible script
outlay keys                 print the keymap
```

`--demo` uses a built-in four-output fixture and `--from-file <capture>` reads an
`xrandr --verbose` capture; neither touches your displays. `-n` reads the live state but only
simulates applies.

## Applying safely

`a` shows the per-output changes, any errors that block the apply, and the exact xrandr command.
After xrandr returns, outlay reads the state back and checks it against the request (xrandr
exits 0 even when it ignores an output). Then it asks "Keep this layout?" for 15 seconds
(`--revert-timeout`, `0` turns it off): only `y` keeps it. A timeout, `n`, `Esc`, Ctrl-C,
SIGHUP or SIGTERM revert to the previous layout. Keys pressed in the first second after xrandr
returns are ignored, so one pressed while the screens were dark cannot answer. Before each
apply, outlay writes the revert command to `$XDG_STATE_HOME/outlay/revert.sh`, so you can run it
by hand if anything goes wrong.

## Profiles

Profiles are arandr-style scripts in `~/.screenlayout` (`--layouts-dir` or `layouts_dir` in the
config point elsewhere), so arandr and outlay read each other's files. In the editor, `e` opens a
picker that draws each profile to scale; opening one makes its layout the pending one (one `u`
undoes it), and `a` applies it as usual. `w` saves the pending layout: an existing script keeps
every line that is not an xrandr call, and outlay shows a diff and asks before overwriting it.
When a profile names an output that is not connected (`eDP-2` on a laptop that now calls its
panel `eDP-1`), a dialog asks where it goes, starting from a free output of the same kind.

From the shell, `outlay apply home` applies `~/.screenlayout/home.sh` with the same verification
and countdown (type `y` and Enter to keep it; `--revert-timeout 0` keeps it without asking, for
key bindings), and `outlay save home` saves the live layout (`-f` overwrites a different file
without asking). With `-n`, both only print what they would run or write.

## Configuration

`$XDG_CONFIG_HOME/outlay/config.toml` is optional; every key has a default:

```toml
revert_seconds = 15
nudge_step = 10
layouts_dir = "~/.screenlayout"
animations = true
directions = "hjkl"        # focus letters: left, down, up, right
# cell_aspect = 2.0        # detected from the terminal when omitted
double_borders = false     # true: double border on the stick target and the focused display's parent
post_apply = ["feh --bg-fill ~/Pictures/wallpapers/current-wallpaper/*"]
```

## License

Not chosen yet.
