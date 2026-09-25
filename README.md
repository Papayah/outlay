# outlay

A keyboard-driven xrandr layout editor for the terminal. outlay draws your monitors to scale,
lets you arrange them with `h j k l`, sticks a display to a chosen side of another so it follows
when that one changes, and applies the result with an automatic revert if you do not confirm it.

> **Status: work in progress.** The xrandr reader and the layout engine are being built first;
> the interactive editor, the apply flow and profile support follow. See [docs/PLAN.md](docs/PLAN.md).

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
`xrandr --verbose` capture; neither touches your displays.

## License

Not chosen yet.
