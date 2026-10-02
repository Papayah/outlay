//! Talking to X11 through the `xrandr` program: [`XrandrCli`], the `xrandr --verbose` parser,
//! plans as xrandr arguments, and screenlayout scripts.

pub mod command;
pub mod parse;
pub mod script;

use std::ffi::OsString;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::backend::{ApplyOutcome, Backend, Plan};
use crate::model::{Snapshot, Transform};
pub use parse::{ParseError, parse_verbose};

/// The built-in fixture behind `--demo`.
pub const DEMO: &str = include_str!("../../tests/fixtures/xrandr/demo.txt");

/// The real `xrandr` program.
pub struct XrandrCli {
    program: OsString,
}

impl Default for XrandrCli {
    fn default() -> Self {
        Self {
            program: "xrandr".into(),
        }
    }
}

impl XrandrCli {
    pub fn new() -> Self {
        Self::default()
    }

    fn run(&self, args: &[&str]) -> Result<ApplyOutcome> {
        let program = self.program.to_string_lossy();
        let out = match Command::new(&self.program)
            .args(args)
            .stdin(Stdio::null())
            .output()
        {
            Ok(out) => out,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
                bail!(
                    "`{program}` is not installed. {}",
                    install_hint(&os_release)
                );
            }
            Err(err) => return Err(err).with_context(|| format!("could not run `{program}`")),
        };
        Ok(ApplyOutcome {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// What a read-only call printed.
    fn text(&self, args: &[&str]) -> Result<String> {
        let out = self.run(args)?;
        if !out.success {
            bail!("`xrandr {}` failed: {}", args.join(" "), out.stderr.trim());
        }
        Ok(out.stdout)
    }

    fn read(&self, args: &[&str]) -> Result<Snapshot> {
        parse_verbose(&self.text(args)?).context("could not read the xrandr output")
    }
}

impl Backend for XrandrCli {
    fn query(&self) -> Result<Snapshot> {
        self.read(&["--verbose"])
    }

    fn requery(&self) -> Result<Snapshot> {
        self.read(&["--verbose", "--current"])
    }

    fn apply(&self, plan: &Plan) -> Result<ApplyOutcome> {
        let argv = command::argv(plan);
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.run(&args)
    }

    fn dump(&self) -> Result<String> {
        self.text(&["--verbose"])
    }

    fn is_live(&self) -> bool {
        true
    }
}

/// Refuses to drive the live X server where xrandr cannot work: under Wayland it would only
/// reconfigure XWayland, and without `DISPLAY` there is no server to talk to. `env` looks up an
/// environment variable.
pub fn check_session(env: impl Fn(&str) -> Option<String>) -> Result<(), String> {
    let set = |name: &str| env(name).is_some_and(|v| !v.is_empty());
    if set("WAYLAND_DISPLAY") || env("XDG_SESSION_TYPE").as_deref() == Some("wayland") {
        return Err(
            "this is a Wayland session, where xrandr would only reconfigure XWayland. \
                    Use wlr-randr, kanshi, hyprctl or the desktop's display settings instead. \
                    (--demo and --from-file still work.)"
                .to_owned(),
        );
    }
    if !set("DISPLAY") {
        return Err("DISPLAY is not set, so there is no X server to talk to. \
             (--demo and --from-file still work.)"
            .to_owned());
    }
    Ok(())
}

/// How to install xrandr on the distribution `/etc/os-release` describes.
pub fn install_hint(os_release: &str) -> String {
    let field = |key: &str| {
        os_release
            .lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim_matches('"').to_lowercase())
            .unwrap_or_default()
    };
    let ids = format!("{} {}", field("ID"), field("ID_LIKE"));
    let has = |id: &str| ids.split_whitespace().any(|w| w == id);
    let command = if has("arch") {
        "sudo pacman -S xorg-xrandr"
    } else if has("debian") || has("ubuntu") {
        "sudo apt install x11-xserver-utils"
    } else if has("fedora") || has("rhel") {
        "sudo dnf install xrandr"
    } else if has("opensuse") || has("suse") {
        "sudo zypper install xrandr"
    } else if has("void") {
        "sudo xbps-install xrandr"
    } else if has("alpine") {
        "sudo apk add xrandr"
    } else {
        return "Install your distribution's xrandr package.".to_owned();
    };
    format!("Install it with: {command}")
}

/// `none`, or nine comma-separated numbers.
pub(crate) fn parse_transform_arg(value: &str) -> Option<Transform> {
    if value == "none" {
        return Some(Transform::identity());
    }
    let values: Vec<f64> = value
        .split(',')
        .map(|v| v.trim().parse().ok())
        .collect::<Option<_>>()?;
    if values.len() != 9 {
        return None;
    }
    let mut matrix = [[0.0; 3]; 3];
    for (k, v) in values.into_iter().enumerate() {
        matrix[k / 3][k % 3] = v;
    }
    Some(Transform {
        matrix,
        filter: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_x_needs_an_x_session() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                vars.iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        assert!(check_session(env(&[("DISPLAY", ":0"), ("XDG_SESSION_TYPE", "x11")])).is_ok());
        let wayland = check_session(env(&[("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-0")]));
        assert!(wayland.unwrap_err().contains("wlr-randr"));
        let wayland = check_session(env(&[("DISPLAY", ":0"), ("XDG_SESSION_TYPE", "wayland")]));
        assert!(wayland.is_err());
        assert!(
            check_session(env(&[]))
                .unwrap_err()
                .starts_with("DISPLAY is not set")
        );
    }

    #[test]
    fn install_hints_follow_os_release() {
        let hint = |text: &str| install_hint(text);
        assert!(
            hint("NAME=\"EndeavourOS\"\nID=\"endeavouros\"\nID_LIKE=\"arch\"\n")
                .ends_with("pacman -S xorg-xrandr")
        );
        assert!(hint("ID=ubuntu\nID_LIKE=debian\n").ends_with("apt install x11-xserver-utils"));
        assert!(hint("ID=fedora\n").ends_with("dnf install xrandr"));
        assert!(
            hint("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n")
                .ends_with("zypper install xrandr")
        );
        assert!(hint("ID=void\n").ends_with("xbps-install xrandr"));
        assert!(hint("ID=alpine\n").ends_with("apk add xrandr"));
        assert_eq!(hint(""), "Install your distribution's xrandr package.");
    }
}
