//! Checks a pending layout. Errors block the apply; warnings and info are shown live.

use std::fmt;

use super::layout::Layout;
use super::{Kind, Scaling, Snapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    /// The outputs the issue is about.
    pub outputs: Vec<usize>,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tag = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        write!(f, "{tag}: {}", self.message)
    }
}

fn issue(severity: Severity, outputs: Vec<usize>, message: String) -> Issue {
    Issue {
        severity,
        outputs,
        message,
    }
}

/// Every issue with `layout` against the live `snap`, errors first.
pub fn validate(layout: &Layout, snap: &Snapshot) -> Vec<Issue> {
    use Severity::{Error, Info, Warning};
    let mut issues = Vec::new();
    let on = layout.enabled();
    let name = |i: usize| layout.names[i].as_str();

    if on.is_empty() {
        issues.push(issue(Error, Vec::new(), "Nothing is enabled.".to_owned()));
    }
    for &i in &on {
        let out = &snap.outputs[i];
        // The live mode of a stale output is no longer listed; only a pending change counts. A
        // compositor sets a custom mode it does not list (kanshi's `mode --custom`).
        let live = out.active.as_ref().map(|a| a.mode);
        if let Some(mode) = &layout.outputs[i].mode
            && out.mode(mode.id).is_none()
            && Some(mode.id) != live
            && !(mode.custom && snap.caps.kind == Kind::Wayland)
        {
            issues.push(issue(
                Error,
                vec![i],
                format!("{}: {} is not one of its modes.", name(i), mode.summary()),
            ));
        }
    }
    if let (Some(b), Some(screen)) = (layout.bounds(), snap.screen) {
        let (w, h) = (b.right(), b.bottom());
        let max = screen.max;
        if w > max.w || h > max.h {
            issues.push(issue(
                Error,
                Vec::new(),
                format!(
                    "The layout needs {w}x{h}, more than the screen maximum of {}x{}.",
                    max.w, max.h
                ),
            ));
        }
    }
    for (i, out) in snap.outputs.iter().enumerate() {
        let (Some(live), true) = (&out.active, layout.locked[i]) else {
            continue;
        };
        let st = &layout.outputs[i];
        if !st.enabled || st.pos != live.pos {
            issues.push(issue(
                Error,
                vec![i],
                format!(
                    "{} uses panning and cannot move; the layout would move it.",
                    name(i)
                ),
            ));
        } else {
            issues.push(issue(
                Warning,
                vec![i],
                format!("{} uses panning; outlay leaves it as it is.", name(i)),
            ));
        }
    }

    for (a, b) in layout.overlapping_pairs() {
        issues.push(issue(
            Warning,
            vec![a, b],
            format!("{} and {} overlap.", name(a), name(b)),
        ));
    }
    let boxes: Vec<usize> = on
        .iter()
        .copied()
        .filter(|&i| !layout.is_mirror_child(i))
        .collect();
    if boxes.len() >= 2 {
        for &i in &boxes {
            // An overlapping display is not floating; the overlap has its own warning.
            let (r, others) = (layout.rect(i), boxes.iter().filter(|&&o| o != i));
            let touches = others
                .map(|&o| layout.rect(o))
                .any(|o| r.shared_edge(&o).is_some() || r.overlaps(&o));
            if !touches {
                issues.push(issue(
                    Warning,
                    vec![i],
                    format!("{} touches no other display.", name(i)),
                ));
            }
        }
    }
    issues.extend(crtc_issues(layout, snap));
    for &i in &on {
        if snap.outputs[i].connection == super::Connection::Disconnected {
            issues.push(issue(
                Warning,
                vec![i],
                format!("{} is disconnected but still active.", name(i)),
            ));
        }
    }
    issues.extend(fractional_issues(layout));
    if layout.caps.primary && !on.is_empty() && layout.primary().is_none() {
        issues.push(issue(Info, Vec::new(), "No display is primary.".to_owned()));
    }
    issues.sort_by(|a, b| b.severity.cmp(&a.severity));
    issues
}

/// A Wayland scale that does not divide the rotated mode evenly: the compositor rounds the
/// logical size, and the rounding can leave a 1 px gap or overlap that outlay cannot see.
fn fractional_issues(layout: &Layout) -> Vec<Issue> {
    let mut issues = Vec::new();
    for i in layout.enabled() {
        let st = &layout.outputs[i];
        let (Scaling::Logical(scale), Some(mode)) = (&st.scaling, &st.mode) else {
            continue;
        };
        let (w, h) = if st.rotation.swaps_axes() {
            (mode.height, mode.width)
        } else {
            (mode.width, mode.height)
        };
        let (lw, lh) = (f64::from(w) / scale, f64::from(h) / scale);
        let whole = |v: f64| (v - v.round()).abs() < 1e-6;
        let size = match (whole(lw), whole(lh)) {
            (true, true) => continue,
            (false, true) => format!("{lw:.1} px wide"),
            (true, false) => format!("{lh:.1} px high"),
            (false, false) => format!("{lw:.1}x{lh:.1} px"),
        };
        let badge = st.scaling.badge().unwrap_or_default();
        issues.push(issue(
            Severity::Warning,
            vec![i],
            format!(
                "{} at {badge} is {size}; the compositor rounds it, which can leave a 1 px gap \
                 or overlap.",
                layout.names[i]
            ),
        ));
    }
    issues
}

/// A heuristic from each output's `CRTCs:` list: for every distinct CRTC set, the enabled
/// outputs that can only use CRTCs from that set must not outnumber it.
fn crtc_issues(layout: &Layout, snap: &Snapshot) -> Vec<Issue> {
    let on: Vec<usize> = layout
        .enabled()
        .into_iter()
        .filter(|&i| !snap.outputs[i].crtcs.is_empty())
        .collect();
    let mut sets: Vec<&Vec<u32>> = on.iter().map(|&i| &snap.outputs[i].crtcs).collect();
    sets.sort();
    sets.dedup();
    let mut issues = Vec::new();
    for set in sets {
        let users: Vec<usize> = on
            .iter()
            .copied()
            .filter(|&i| snap.outputs[i].crtcs.iter().all(|c| set.contains(c)))
            .collect();
        if users.len() > set.len() {
            let names: Vec<&str> = users.iter().map(|&i| layout.names[i].as_str()).collect();
            issues.push(issue(
                Severity::Warning,
                users.clone(),
                format!(
                    "{} displays share {} CRTCs ({}); the GPU may not drive them all.",
                    users.len(),
                    set.len(),
                    names.join(", ")
                ),
            ));
        }
    }
    issues
}
