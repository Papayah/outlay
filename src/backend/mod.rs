//! The neutral side of talking to a display server: the [`Backend`] trait, the [`Plan`] an apply
//! carries out, and the in-memory backends behind `--demo`, `--from-file` and `-n`.

pub mod fixture;
pub mod plan;

use anyhow::Result;

use crate::model::Snapshot;
pub use fixture::{DryRun, FixtureBackend};
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
