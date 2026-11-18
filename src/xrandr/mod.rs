//! Talking to X11 through the `xrandr` program: [`XrandrCli`], the `xrandr --verbose` parser,
//! plans as xrandr arguments, and screenlayout scripts.

pub mod command;
pub mod events;
pub mod parse;
pub mod script;

use std::ffi::OsString;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::backend::fixture::simulate;
use crate::backend::{ApplyOutcome, Backend, Plan};
use crate::model::{Dpi, Snapshot, Transform};
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

    /// When `plan` shrinks the screen, a size (`WxH`) that holds both the live layout and the
    /// one `plan` makes; `None` when one call does no harm, or when that cannot be worked out.
    fn room_for(&self, plan: &Plan) -> Option<String> {
        let now = self.requery().ok()?;
        // The simulation does not follow panning.
        if now
            .outputs
            .iter()
            .any(|o| o.active.as_ref().is_some_and(|a| a.panning.is_some()))
        {
            return None;
        }
        let mut after = now.clone();
        if !simulate(&mut after, plan).success {
            return None;
        }
        let (now, after) = (now.screen?.current, after.screen?.current);
        (after.w < now.w || after.h < now.h)
            .then(|| format!("{}x{}", now.w.max(after.w), now.h.max(after.h)))
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
        // Before xrandr shrinks the screen, it turns off every display that stays on but does not
        // fit the smaller screen at its old place, and turns it on again after: a second of black
        // on a laptop panel. So first move the displays on a screen that holds both layouts; the
        // same call then only shrinks it.
        let Some(fb) = self.room_for(plan) else {
            return self.run(&args);
        };
        let first = self.run(&[&args[..], &["--fb", &fb]].concat())?;
        if !first.success {
            return Ok(first);
        }
        let mut out = self.run(&args)?;
        out.stderr = first.stderr + &out.stderr;
        Ok(out)
    }

    fn dump(&self) -> Result<String> {
        self.text(&["--verbose"])
    }

    fn is_live(&self) -> bool {
        true
    }
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

/// `--dpi`: a positive number, or else, as xrandr reads it, an output name.
pub(crate) fn parse_dpi_arg(value: &str) -> Option<Dpi> {
    match value.parse::<f64>() {
        Ok(dpi) if dpi.is_finite() && dpi > 0.0 => Some(Dpi::Value(dpi)),
        Ok(_) => None,
        Err(_) if value.is_empty() => None,
        Err(_) => Some(Dpi::Output(value.to_owned())),
    }
}

/// `--fbmm`: `WxH` in millimetres, both positive.
pub(crate) fn parse_fbmm_arg(value: &str) -> Option<(u32, u32)> {
    let (w, h) = value.split_once('x')?;
    let (w, h): (u32, u32) = (w.parse().ok()?, h.parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
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

    /// A fake `xrandr` in a temporary directory: it logs each call, and a re-query prints the
    /// demo capture.
    fn fake_xrandr(tag: &str) -> (XrandrCli, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("outlay-xrandr-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (demo, log) = (dir.join("demo.txt"), dir.join("calls"));
        std::fs::write(&demo, DEMO).unwrap();
        let program = dir.join("xrandr");
        let script = format!(
            "#!/bin/sh\necho \"$*\" >>'{}'\n[ \"$*\" = '--verbose --current' ] && cat '{}'\nexit 0\n",
            log.display(),
            demo.display()
        );
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cli = XrandrCli {
            program: program.into(),
        };
        (cli, dir)
    }

    #[test]
    fn a_shrinking_apply_moves_the_displays_before_the_screen_shrinks() {
        use crate::model::layout::Layout;
        let (cli, dir) = fake_xrandr("shrink");
        let demo = parse_verbose(DEMO).unwrap();
        let screen = demo.screen.as_ref().unwrap().current;
        let calls = || std::fs::read_to_string(dir.join("calls")).unwrap();

        // Only the panel stays on: the screen shrinks to its size.
        let mut alone = Layout::inferred(&demo);
        for name in ["HDMI-1-0", "DP-1-2"] {
            alone
                .turn_off(alone.names.iter().position(|n| n == name).unwrap())
                .unwrap();
        }
        let plan = Plan::pending(&alone, &demo);
        let args = command::argv(&plan).join(" ");
        assert!(cli.apply(&plan).unwrap().success);
        assert_eq!(
            calls(),
            format!(
                "--verbose --current\n{args} --fb {}x{}\n{args}\n",
                screen.w, screen.h
            )
        );

        // A layout that grows the screen needs one call.
        std::fs::remove_file(dir.join("calls")).unwrap();
        let mut grown = Layout::inferred(&demo);
        let lg = grown.names.iter().position(|n| n == "DP-1-3").unwrap();
        grown.turn_on(&demo, lg).unwrap();
        let plan = Plan::pending(&grown, &demo);
        assert!(cli.apply(&plan).unwrap().success);
        assert_eq!(
            calls(),
            format!("--verbose --current\n{}\n", command::argv(&plan).join(" "))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn screen_size_arguments() {
        assert_eq!(parse_dpi_arg("96"), Some(Dpi::Value(96.0)));
        assert_eq!(parse_dpi_arg("120.5"), Some(Dpi::Value(120.5)));
        assert_eq!(
            parse_dpi_arg("eDP-1"),
            Some(Dpi::Output("eDP-1".to_owned()))
        );
        for bad in ["0", "-96", "inf", "NaN", ""] {
            assert_eq!(parse_dpi_arg(bad), None, "{bad}");
        }
        assert_eq!(parse_fbmm_arg("1354x381"), Some((1354, 381)));
        for bad in ["1354", "0x381", "-1x2", "1354x381mm", "x"] {
            assert_eq!(parse_fbmm_arg(bad), None, "{bad}");
        }
    }
}
