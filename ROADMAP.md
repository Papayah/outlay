# Roadmap

Future work that has no session yet. An item gets its own plan, in `docs/`, before a session
implements it.

## Desktops

- **GNOME** (Mutter D-Bus). Outline: `docs/PLAN-wayland.md`, "Later session G: GNOME".
- **KDE Plasma** (KWin protocols). Outline: `docs/PLAN-wayland.md`, "Later session H: KDE Plasma".
- Profiles on GNOME and KDE.

## Mirroring on Wayland

wlr-output-management has no mirroring, so `=` on Wayland says "This compositor cannot mirror
displays." today.

- **Compositor extensions:**
  - COSMIC `zcosmic_output_manager_v1`, which also gives exact scales;
  - Hyprland `hl.monitor{ mirror = … }` through `hyprctl eval`.
- **[wl-mirror](https://github.com/Ferdi265/wl-mirror),** for the compositors with no mirror
  extension (sway, niri, river, labwc and the others):
  - `wl-mirror --fullscreen-output T F` shows F on T. The mirror side of `s` (`=`) starts it,
    instead of the message above, when `wl-mirror` is on `PATH`.
  - It captures through `wlr-screencopy`, `wlr-export-dmabuf` or `ext-image-copy-capture-v1`.
    Without one of them, the mirror side stays off.
  - The mirror is a client window, not output state, so the protocol cannot report it. outlay
    must own the process: start and stop it on apply, keep and revert, put it in `revert.sh`, and
    in kanshi profiles as an `exec` line.
  - Open: where T goes in the layout (T keeps its own mode, and wl-mirror scales F to fit), and
    how a mirror that outlay did not start shows after a requery.

## Wayland

- A VRR toggle (`set_adaptive_sync`).
- Custom modes from `:mode`.
- shikane profiles.

## X11

- **A screen of 0x0 mm.** On 2026-10-29 the X screen on the dock lost its physical size
  (`xdpyinfo`: `0x0 millimeters`). Then every xrandr call that changes the screen's pixel size
  fails with BadValue on `RRSetScreenSize`, unless it has `--dpi` or `--fbmm`. outlay keeps the
  profile's `--dpi`. It could also detect the state and add `--dpi 96`, or say what to do.
  `xrandr --verbose` does not show the millimetres: it needs `XDisplayWidthMM` (the X protocol)
  or `xdpyinfo`. The cause of the 0 mm state is not known.

## Automatic layouts

[Issue #23](https://github.com/Papayah/outlay/issues/23), from NikodemMarek: outlay applies the
screen layouts on its own, as autorandr does on X11.

- A systemd user service watches displays connect and disconnect.
- It works with the layouts already saved in configuration files (screenlayout scripts, the kanshi
  config).
- It finds the layout set earlier for the connected displays. It recognises the monitors
  themselves (EDID vendor, model and serial), not only the connectors they are on.
- It applies that layout. If the apply fails, it reverts to the layout from before the change.
- A change made by hand in outlay wins over the service's changes.
- **Fail-safe:** when the last external display disconnects, the service turns on the laptop's
  built-in display at its preferred mode, also when no saved layout matches. A panel that was off
  on the desk then does not stay dark, and nobody has to turn it on blind.
- Open: on Wayland kanshi already switches profiles, so the service must not fight it (only the
  fail-safe there, or nothing while kanshi runs). How long a change by hand wins: until the
  displays change again?

## Distribution

- An AUR package, planned in its own session. It waits for the AUR account.
