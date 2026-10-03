//! Which display server outlay talks to: a Wayland compositor through `zwlr_output_manager_v1`,
//! or X11 through xrandr. The choice is a pure function of the environment; [`detect`] then
//! connects.

use std::path::PathBuf;

use anyhow::{Result, bail};

use super::Backend;
use crate::wayland::WlrBackend;
use crate::wayland::client::ConnectError;
use crate::xrandr::XrandrCli;

const STILL_WORK: &str = "(--demo and --from-file still work.)";

/// What the environment points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// The compositor whose socket `WAYLAND_DISPLAY` names.
    Wayland {
        display: String,
        socket: PathBuf,
    },
    X11,
}

/// The display server the environment points at, or why there is none. `env` looks up a
/// variable:
/// 1. `WAYLAND_DISPLAY` set: the compositor on that socket (under `XDG_RUNTIME_DIR` unless the
///    name is a path);
/// 2. `XDG_SESSION_TYPE=wayland` without it: none;
/// 3. `DISPLAY` set: X11;
/// 4. neither: none.
pub fn choose(env: &dyn Fn(&str) -> Option<String>) -> Result<Choice, String> {
    let set = |name: &str| env(name).filter(|v| !v.is_empty());
    if let Some(display) = set("WAYLAND_DISPLAY") {
        let socket = if display.starts_with('/') {
            PathBuf::from(&display)
        } else {
            let Some(runtime) = set("XDG_RUNTIME_DIR") else {
                return Err(format!(
                    "WAYLAND_DISPLAY is `{display}`, but XDG_RUNTIME_DIR is not set, so there is \
                     no socket to find. {STILL_WORK}"
                ));
            };
            PathBuf::from(runtime).join(&display)
        };
        return Ok(Choice::Wayland { display, socket });
    }
    if env("XDG_SESSION_TYPE").as_deref() == Some("wayland") {
        return Err(format!(
            "this is a Wayland session, but WAYLAND_DISPLAY is not set, so there is no \
             compositor to talk to. {STILL_WORK}"
        ));
    }
    if set("DISPLAY").is_some() {
        return Ok(Choice::X11);
    }
    Err(format!(
        "neither WAYLAND_DISPLAY nor DISPLAY is set, so there is no display server to talk to. \
         {STILL_WORK}"
    ))
}

/// Why a compositor that answers cannot be driven, from `XDG_CURRENT_DESKTOP`.
pub fn unsupported(env: &dyn Fn(&str) -> Option<String>) -> String {
    let desktop = env("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let is = |name: &str| desktop.split(':').any(|d| d.eq_ignore_ascii_case(name));
    if is("GNOME") {
        "GNOME is not supported yet. Use Settings → Displays or `gdctl`.".to_owned()
    } else if is("KDE") {
        "KDE Plasma is not supported yet. Use System Settings → Display or `kscreen-doctor`."
            .to_owned()
    } else {
        format!(
            "this compositor does not offer wlr-output-management (zwlr_output_manager_v1), \
             which outlay needs. {STILL_WORK}"
        )
    }
}

/// The compositor's name for the title bar: from the variables sway, Hyprland and niri set,
/// else `XDG_CURRENT_DESKTOP`. A label only; `None` outside a Wayland session.
pub fn compositor_label(env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let set = |name: &str| env(name).filter(|v| !v.is_empty());
    set("WAYLAND_DISPLAY")?;
    if set("SWAYSOCK").is_some() {
        return Some("sway".to_owned());
    }
    if set("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return Some("Hyprland".to_owned());
    }
    if set("NIRI_SOCKET").is_some() {
        return Some("niri".to_owned());
    }
    set("XDG_CURRENT_DESKTOP").and_then(|d| d.split(':').next().map(str::to_owned))
}

/// The live backend the environment points at.
pub fn detect(env: &dyn Fn(&str) -> Option<String>) -> Result<Box<dyn Backend>> {
    match choose(env).map_err(anyhow::Error::msg)? {
        Choice::X11 => Ok(Box::new(XrandrCli::new())),
        Choice::Wayland { display, socket } => match WlrBackend::connect(&socket) {
            Ok(backend) => Ok(Box::new(backend)),
            Err(ConnectError::NoCompositor(_)) => bail!(
                "WAYLAND_DISPLAY is `{display}`, but no compositor answers. If this is not a \
                 Wayland session (a stale value in tmux, for example), unset it."
            ),
            Err(ConnectError::NoOutputManagement) => bail!("{}", unsupported(env)),
            Err(ConnectError::TooOld(version)) => bail!(
                "this compositor's output protocol is too old: it offers zwlr_output_manager_v1 \
                 version {version}, and outlay needs version 2."
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k: &str| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn the_environment_chooses_the_display_server() {
        let wayland = env(&[
            ("WAYLAND_DISPLAY", "wayland-1"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("DISPLAY", ":0"),
        ]);
        assert_eq!(
            choose(&wayland),
            Ok(Choice::Wayland {
                display: "wayland-1".to_owned(),
                socket: PathBuf::from("/run/user/1000/wayland-1"),
            }),
            "Wayland wins over XWayland's DISPLAY"
        );
        let absolute = env(&[("WAYLAND_DISPLAY", "/tmp/w/wayland-9")]);
        assert!(matches!(
            choose(&absolute),
            Ok(Choice::Wayland { socket, .. }) if socket == std::path::Path::new("/tmp/w/wayland-9")
        ));
        let lost = choose(&env(&[("WAYLAND_DISPLAY", "wayland-1")])).unwrap_err();
        assert!(lost.contains("XDG_RUNTIME_DIR is not set"), "{lost}");

        let no_socket = choose(&env(&[("XDG_SESSION_TYPE", "wayland"), ("DISPLAY", ":0")]));
        assert!(
            no_socket
                .unwrap_err()
                .starts_with("this is a Wayland session, but WAYLAND_DISPLAY is not set")
        );
        let x11 = env(&[("DISPLAY", ":0"), ("XDG_SESSION_TYPE", "x11")]);
        assert_eq!(choose(&x11), Ok(Choice::X11));
        assert!(
            choose(&env(&[("WAYLAND_DISPLAY", ""), ("DISPLAY", ":1")])) == Ok(Choice::X11),
            "an empty WAYLAND_DISPLAY is unset"
        );
        let none = choose(&env(&[])).unwrap_err();
        assert!(
            none.starts_with("neither WAYLAND_DISPLAY nor DISPLAY is set"),
            "{none}"
        );
        assert!(none.ends_with("(--demo and --from-file still work.)"));
    }

    #[test]
    fn gnome_and_kde_get_their_own_advice() {
        let gnome = unsupported(&env(&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")]));
        assert_eq!(
            gnome,
            "GNOME is not supported yet. Use Settings → Displays or `gdctl`."
        );
        let kde = unsupported(&env(&[("XDG_CURRENT_DESKTOP", "KDE")]));
        assert!(kde.starts_with("KDE Plasma is not supported yet."), "{kde}");
        let other = unsupported(&env(&[("XDG_CURRENT_DESKTOP", "Weston")]));
        assert!(other.contains("zwlr_output_manager_v1"), "{other}");
    }

    #[test]
    fn a_wayland_display_where_nothing_answers() {
        let dir = std::env::temp_dir().join(format!("outlay-detect-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let runtime: &'static str = Box::leak(dir.to_string_lossy().into_owned().into_boxed_str());
        let vars: &'static [(&'static str, &'static str)] = Box::leak(Box::new([
            ("WAYLAND_DISPLAY", "wayland-77"),
            ("XDG_RUNTIME_DIR", runtime),
        ]));
        let err = detect(&env(vars)).err().unwrap().to_string();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            err,
            "WAYLAND_DISPLAY is `wayland-77`, but no compositor answers. If this is not a \
             Wayland session (a stale value in tmux, for example), unset it."
        );
    }

    #[test]
    fn the_compositor_label() {
        let label = |vars| compositor_label(&env(vars));
        assert_eq!(label(&[("DISPLAY", ":0"), ("SWAYSOCK", "/x")]), None);
        assert_eq!(
            label(&[("WAYLAND_DISPLAY", "wayland-1"), ("SWAYSOCK", "/x")]).as_deref(),
            Some("sway")
        );
        assert_eq!(
            label(&[
                ("WAYLAND_DISPLAY", "wayland-1"),
                ("HYPRLAND_INSTANCE_SIGNATURE", "abc")
            ])
            .as_deref(),
            Some("Hyprland")
        );
        assert_eq!(
            label(&[("WAYLAND_DISPLAY", "wayland-1"), ("NIRI_SOCKET", "/n")]).as_deref(),
            Some("niri")
        );
        assert_eq!(
            label(&[
                ("WAYLAND_DISPLAY", "wayland-1"),
                ("XDG_CURRENT_DESKTOP", "river:wlroots")
            ])
            .as_deref(),
            Some("river")
        );
        assert_eq!(label(&[("WAYLAND_DISPLAY", "wayland-1")]), None);
    }
}
