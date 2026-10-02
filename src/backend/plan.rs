//! What an apply sets, independent of the tool that sets it. A plan is self-contained: everything
//! a renderer or a backend needs is decided when the plan is built, so neither needs the snapshot.

use crate::model::geometry::Point;
use crate::model::layout::{Layout, OutputState};
use crate::model::{Kind, Mode, Output, Reflection, Rotation, Scaling, Snapshot};

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Only the outputs to set; the others stay as they are.
    pub outputs: Vec<Planned>,
    pub primary: PrimaryRule,
    pub form: PlanForm,
}

/// What happens to the primary display besides the outputs marked primary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimaryRule {
    /// Leave it: an output marked primary takes over, else nothing changes.
    Keep,
    /// No display is primary (X11 `--noprimary`).
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanForm {
    /// An apply of the pending layout: only what changes is spelled out.
    Apply,
    /// A restore of an earlier state: every setting is spelled out, the scaling too.
    Restore,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Planned {
    pub name: String,
    /// `None` turns the output off.
    pub on: Option<On>,
}

/// The settings of an output that is on.
#[derive(Clone, Debug, PartialEq)]
pub struct On {
    pub mode: Mode,
    pub pos: Point,
    pub rotation: Rotation,
    pub reflection: Reflection,
    /// The scaling the output has afterwards.
    pub scaling: Scaling,
    pub scaling_change: ScalingChange,
    pub primary: bool,
}

/// Whether an apply touches an output's scaling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalingChange {
    /// The output keeps the scaling it has, and nothing is written for it.
    Keep,
    /// The scaling is set to `On::scaling`.
    Set,
    /// The scaling goes back to none.
    Reset,
}

impl Plan {
    /// The apply of `layout` on top of `live`: every connected or active output, plus any turned
    /// on, in snapshot order. Outputs with panning are left out, since outlay leaves them alone,
    /// but they still count for the primary rule.
    pub fn pending(layout: &Layout, live: &Snapshot) -> Self {
        let outputs = live
            .outputs
            .iter()
            .enumerate()
            .filter(|&(i, out)| {
                (out.is_relevant() || layout.outputs[i].enabled) && !layout.locked[i]
            })
            .map(|(i, out)| Planned {
                name: layout.names[i].clone(),
                on: pending_on(&layout.outputs[i], out, layout.caps.kind),
            })
            .collect();
        Self {
            outputs,
            primary: rule(layout.primary().is_none()),
            form: PlanForm::Apply,
        }
    }

    /// The restore of `live`, the state taken just before an apply: every connected or active
    /// output except those with panning, with each setting explicit. Outputs that were off are
    /// turned off.
    pub fn restore(live: &Snapshot) -> Self {
        let outputs = live
            .outputs
            .iter()
            .filter(|o| o.is_relevant() && !o.has_panning())
            .map(|out| Planned {
                name: out.name.clone(),
                on: restore_on(out),
            })
            .collect();
        let primary = live.outputs.iter().any(|o| o.primary && o.active.is_some());
        Self {
            outputs,
            primary: rule(!primary),
            form: PlanForm::Restore,
        }
    }

    /// Every output of `layout`, connected or not, as a profile records it. The scaling counts
    /// as set whenever there is one.
    pub fn whole(layout: &Layout) -> Self {
        let outputs = (0..layout.len())
            .map(|i| {
                let st = &layout.outputs[i];
                let change = if st.scaling.is_identity() {
                    ScalingChange::Keep
                } else {
                    ScalingChange::Set
                };
                Planned {
                    name: layout.names[i].clone(),
                    on: on(st, st.scaling.clone(), change),
                }
            })
            .collect();
        Self {
            outputs,
            primary: rule(layout.primary().is_none()),
            form: PlanForm::Apply,
        }
    }

    pub fn find(&self, name: &str) -> Option<&Planned> {
        self.outputs.iter().find(|p| p.name == name)
    }
}

fn rule(no_primary: bool) -> PrimaryRule {
    if no_primary {
        PrimaryRule::Clear
    } else {
        PrimaryRule::Keep
    }
}

/// `st` as an output that is on, or `None` when it is off or has no mode.
fn on(st: &OutputState, scaling: Scaling, scaling_change: ScalingChange) -> Option<On> {
    let mode = st.mode.clone().filter(|_| st.enabled)?;
    Some(On {
        mode,
        pos: st.pos,
        rotation: st.rotation,
        reflection: st.reflection,
        scaling,
        scaling_change,
        primary: st.primary,
    })
}

/// The scaling is written only when it changes, so an unchanged output keeps its filter and any
/// transform that is more than a scale.
fn pending_on(st: &OutputState, live: &Output, kind: Kind) -> Option<On> {
    let live = live
        .active
        .as_ref()
        .map_or_else(|| Scaling::unit(kind), |a| a.scaling.clone());
    if st.scaling.approx_eq(&live) {
        on(st, live, ScalingChange::Keep)
    } else if st.scaling.is_identity() {
        on(st, st.scaling.clone(), ScalingChange::Reset)
    } else {
        on(st, st.scaling.clone(), ScalingChange::Set)
    }
}

fn restore_on(out: &Output) -> Option<On> {
    let active = out.active.as_ref()?;
    let scaling_change = if active.scaling.is_identity() {
        ScalingChange::Reset
    } else {
        ScalingChange::Set
    };
    Some(On {
        mode: out.live_mode()?,
        pos: active.pos,
        rotation: active.rotation,
        reflection: active.reflection,
        scaling: active.scaling.clone(),
        scaling_change,
        primary: out.primary,
    })
}
