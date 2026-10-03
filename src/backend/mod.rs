//! The neutral side of talking to a display server: the [`Backend`] trait, the [`Plan`] an apply
//! carries out, and the in-memory backends behind `--demo`, `--from-file` and `-n`.

pub mod detect;
pub mod fixture;
pub mod plan;

use anyhow::Result;

use crate::model::{Kind, Snapshot};
use crate::{wayland, xrandr};
pub use detect::{compositor_label, detect};
pub use fixture::{DryRun, FixtureBackend, parse_capture};
pub use plan::{On, Plan, PlanForm, Planned, PrimaryRule, ScalingChange};

/// What an apply produced: whether it worked, and the tool's own messages. On X11 they are what
/// `xrandr` wrote to stdout and stderr.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// What a backend says about a plan before it applies it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The backend cannot check plans in advance.
    Untested,
    Accepted,
    /// Why the plan would fail.
    Rejected(String),
}

/// Arguments joined for a POSIX shell, each quoted when it holds anything beyond a safe set.
pub fn shell_words(args: &[String]) -> String {
    args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
}

fn quote(arg: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "-_.,:/=+@%".contains(c);
    if !arg.is_empty() && arg.chars().all(safe) {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

/// The command that carries out `plan`, as the confirm popup and `apply -n` show it: xrandr with
/// modes by XID on X11, the equivalent `wlr-randr` call on Wayland.
pub fn command_text(kind: Kind, plan: &Plan) -> String {
    match kind {
        Kind::X11 => xrandr::command::command_line(&xrandr::command::argv(plan)),
        Kind::Wayland => wayland::command::command_line(plan),
    }
}

/// The same command with modes by name and rate, which other machines understand: what `y`
/// copies.
pub fn portable_command_text(kind: Kind, plan: &Plan) -> String {
    match kind {
        Kind::X11 => xrandr::command::command_line(&xrandr::command::portable_argv(plan)),
        Kind::Wayland => wayland::command::command_line(plan),
    }
}

pub trait Backend {
    /// A full probe of the outputs (X11: `xrandr --verbose`).
    fn query(&self) -> Result<Snapshot>;

    /// A re-query that does not re-probe (X11: `xrandr --verbose --current`), used after an
    /// apply and by the watch.
    fn requery(&self) -> Result<Snapshot> {
        self.query()
    }

    /// Checks a plan without applying it.
    fn test(&self, _plan: &Plan) -> Result<Verdict> {
        Ok(Verdict::Untested)
    }

    /// Carries out a plan.
    fn apply(&self, plan: &Plan) -> Result<ApplyOutcome>;

    /// The capture text `--from-file` reads: raw `xrandr --verbose` on X11.
    fn dump(&self) -> Result<String>;

    /// Whether an apply changes the real screens. Only then does outlay write `revert.sh`, which
    /// the panic hook may run, and run the `post_apply` hooks.
    fn is_live(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(quote("1920x1080_60.00"), "1920x1080_60.00");
        assert_eq!(quote("DP-1-2.1"), "DP-1-2.1");
        assert_eq!(quote("my mode"), "'my mode'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote(""), "''");
    }
}
