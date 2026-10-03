//! Backends that never touch the real screens: [`FixtureBackend`] holds a snapshot in memory and
//! simulates each plan on it; [`DryRun`] does the same on a reading of the live state.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};

use super::{ApplyOutcome, Backend, Plan, PrimaryRule, ScalingChange};
use crate::model::geometry::{Size, effective_size};
use crate::model::{ActiveConfig, Scaling, Snapshot, Transform};
use crate::wayland::{self, capture};
use crate::xrandr::{DEMO, command, parse_verbose};

/// A snapshot held in memory, for `--demo`, `--from-file` and tests. An apply never touches the
/// screens: it records the plan and updates the snapshot the way the display server would, so
/// the whole apply → verify → revert flow can run without real displays.
pub struct FixtureBackend {
    state: Mutex<Snapshot>,
    applied: Mutex<Vec<Plan>>,
    /// The capture text the snapshot came from, for `dump`.
    source: Option<String>,
}

impl FixtureBackend {
    pub fn new(snapshot: Snapshot) -> Self {
        Self {
            state: Mutex::new(snapshot),
            applied: Mutex::new(Vec::new()),
            source: None,
        }
    }

    /// The snapshot a capture describes, keeping the text.
    pub fn from_text(text: String) -> Result<Self> {
        let snapshot = parse_capture(&text)?;
        Ok(Self {
            source: Some(text),
            ..Self::new(snapshot)
        })
    }

    /// The built-in four-output X11 fixture.
    pub fn demo() -> Self {
        Self::from_text(DEMO.to_owned()).expect("the built-in demo fixture parses")
    }

    /// The built-in four-head Wayland fixture.
    pub fn demo_wayland() -> Self {
        Self::from_text(wayland::DEMO.to_owned()).expect("the built-in Wayland demo parses")
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("could not read {}", path.display()))?;
        Self::from_text(text).with_context(|| format!("could not parse {}", path.display()))
    }

    /// Every plan passed to [`Backend::apply`] so far.
    pub fn applied(&self) -> Vec<Plan> {
        self.applied.lock().expect("fixture lock").clone()
    }

    /// The same plans as xrandr arguments, naming modes by XID.
    pub fn applied_argv(&self) -> Vec<Vec<String>> {
        self.applied().iter().map(command::argv).collect()
    }

    /// Replaces the state, as plugging a display in or unplugging one does.
    pub fn set_state(&self, snapshot: Snapshot) {
        *self.state.lock().expect("fixture lock") = snapshot;
    }
}

/// The snapshot a capture describes: `wlr-randr --json` (or `outlay dump` on Wayland) when it
/// starts like JSON, else `xrandr --verbose`.
pub fn parse_capture(text: &str) -> Result<Snapshot> {
    Ok(if capture::is_capture(text) {
        capture::parse(text)?
    } else {
        parse_verbose(text)?
    })
}

impl Backend for FixtureBackend {
    fn query(&self) -> Result<Snapshot> {
        Ok(self.state.lock().expect("fixture lock").clone())
    }

    fn apply(&self, plan: &Plan) -> Result<ApplyOutcome> {
        self.applied
            .lock()
            .expect("fixture lock")
            .push(plan.clone());
        let mut state = self.state.lock().expect("fixture lock");
        Ok(simulate(&mut state, plan))
    }

    fn dump(&self) -> Result<String> {
        self.source
            .clone()
            .ok_or_else(|| anyhow!("this snapshot was not read from a capture"))
    }
}

/// `-n`: the live state, read once, with applies simulated in memory. Nothing reaches the
/// screens; the editor shows the commands it would have run.
pub struct DryRun {
    inner: FixtureBackend,
}

impl DryRun {
    pub fn new(live: &dyn Backend) -> Result<Self> {
        Ok(Self {
            inner: FixtureBackend::from_text(live.dump()?)
                .context("could not read the live state")?,
        })
    }

    /// Every plan an apply would have carried out.
    pub fn applied(&self) -> Vec<Plan> {
        self.inner.applied()
    }

    pub fn applied_argv(&self) -> Vec<Vec<String>> {
        self.inner.applied_argv()
    }
}

impl Backend for DryRun {
    fn query(&self) -> Result<Snapshot> {
        self.inner.query()
    }

    fn apply(&self, plan: &Plan) -> Result<ApplyOutcome> {
        self.inner.apply(plan)
    }

    fn dump(&self) -> Result<String> {
        self.inner.dump()
    }
}

fn failure(stderr: String) -> ApplyOutcome {
    ApplyOutcome {
        success: false,
        stdout: String::new(),
        stderr,
    }
}

/// Applies a plan to a snapshot the way xrandr would. A failed apply changes nothing; an output
/// the snapshot does not know is a warning and is skipped.
fn simulate(snapshot: &mut Snapshot, plan: &Plan) -> ApplyOutcome {
    let mut next = snapshot.clone();
    let mut warnings = String::new();
    // `Some(None)`: nobody is primary afterwards.
    let mut primary = match plan.primary {
        PrimaryRule::Keep => None,
        PrimaryRule::Clear => Some(None),
    };
    for planned in &plan.outputs {
        let Some(i) = next.find(&planned.name) else {
            warnings.push_str(&format!(
                "warning: output {} not found; ignoring\n",
                planned.name
            ));
            continue;
        };
        let out = &mut next.outputs[i];
        let Some(on) = &planned.on else {
            out.active = None;
            out.crtc = None;
            continue;
        };
        let Some(mode) = out.mode(on.mode.id).cloned() else {
            return failure(format!(
                "xrandr: cannot find mode 0x{:x} for output {}\n",
                on.mode.id, out.name
            ));
        };
        let old = out.active.take();
        let mut scaling = match (on.scaling_change, &old) {
            (ScalingChange::Keep, Some(a)) => a.scaling.clone(),
            (ScalingChange::Keep, None) => Scaling::unit(next.caps.kind),
            (ScalingChange::Set | ScalingChange::Reset, _) => on.scaling.clone(),
        };
        if matches!(&scaling, Scaling::X11(t) if t.is_identity()) {
            scaling = Scaling::X11(Transform::identity());
        }
        let size: Size = effective_size(mode.size(), on.rotation, &scaling);
        out.active = Some(ActiveConfig {
            mode: mode.id,
            pos: on.pos,
            size,
            rotation: on.rotation,
            reflection: on.reflection,
            scaling,
            panning: old.and_then(|a| a.panning),
        });
        if on.primary {
            primary = Some(Some(i));
        }
    }

    if let Some(p) = primary {
        for (k, out) in next.outputs.iter_mut().enumerate() {
            out.primary = Some(k) == p;
        }
    }

    // Hand out CRTCs where there are any: keep the ones in use, give newly enabled outputs the
    // first free one.
    if next.outputs.iter().any(|o| !o.crtcs.is_empty()) {
        let mut used: Vec<u32> = next
            .outputs
            .iter()
            .filter(|o| o.active.is_some())
            .filter_map(|o| o.crtc)
            .collect();
        for out in next
            .outputs
            .iter_mut()
            .filter(|o| o.active.is_some() && o.crtc.is_none())
        {
            let Some(&free) = out.crtcs.iter().find(|c| !used.contains(c)) else {
                return failure(format!(
                    "xrandr: cannot find crtc for output {}\n",
                    out.name
                ));
            };
            used.push(free);
            out.crtc = Some(free);
        }
    }

    let (w, h) = next
        .outputs
        .iter()
        .filter_map(|o| o.active.as_ref())
        .fold((0, 0), |(w, h), a| {
            (w.max(a.pos.x + a.size.w), h.max(a.pos.y + a.size.h))
        });
    if let Some(screen) = &mut next.screen {
        if w > screen.max.w || h > screen.max.h {
            return failure(format!(
                "xrandr: screen cannot be larger than {}x{} (desired size {w}x{h})\n",
                screen.max.w, screen.max.h
            ));
        }
        screen.current = Size::new(w.max(screen.min.w), h.max(screen.min.h));
    }
    *snapshot = next;
    ApplyOutcome {
        success: true,
        stdout: String::new(),
        stderr: warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Rotation;
    use crate::model::geometry::Point;
    use crate::xrandr::XrandrCli;
    use crate::xrandr::command::argv_to_plan;

    /// Applies an xrandr call, read the way xrandr reads it.
    fn apply(backend: &dyn Backend, argv: &str) -> ApplyOutcome {
        let plan = plan(backend, argv).unwrap();
        backend.apply(&plan).unwrap()
    }

    fn plan(backend: &dyn Backend, argv: &str) -> Result<Plan, String> {
        let argv: Vec<String> = argv.split_whitespace().map(str::to_owned).collect();
        argv_to_plan(&backend.query().unwrap(), &argv)
    }

    #[test]
    fn dry_run_simulates_without_touching_the_screens() {
        let dry = DryRun::new(&FixtureBackend::demo()).unwrap();
        assert!(!dry.is_live());
        assert!(XrandrCli::new().is_live());
        let outcome = apply(&dry, "--output HDMI-1-0 --off");
        assert!(outcome.success);
        assert!(dry.query().unwrap().outputs[0].active.is_none());
        assert_eq!(dry.applied().len(), 1);
        assert_eq!(dry.applied_argv(), [["--output", "HDMI-1-0", "--off"]]);
        assert_eq!(dry.dump().unwrap(), DEMO, "the capture it was read from");
    }

    #[test]
    fn demo_parses() {
        let snap = FixtureBackend::demo().query().unwrap();
        let names: Vec<&str> = snap.outputs.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["HDMI-1-0", "DP-1-2", "eDP-1", "DP-1-3"]);
        assert!(FixtureBackend::new(snap).dump().is_err());
    }

    #[test]
    fn fixture_apply_records_and_updates() {
        let backend = FixtureBackend::demo();
        let demo = backend.query().unwrap();
        let edp_xid = demo.outputs[demo.find("eDP-1").unwrap()]
            .preferred_mode()
            .unwrap()
            .id;
        let plan = plan(
            &backend,
            &format!(
                "--output HDMI-1-0 --off --output DP-1-3 --mode 3840x2160 --rate 60 --pos 4480x0 --rotate left \
                 --output eDP-1 --primary --mode 0x{edp_xid:x} --pos 2240x1440"
            ),
        )
        .unwrap();
        let outcome = backend.apply(&plan).unwrap();
        assert!(outcome.success, "{}", outcome.stderr);
        assert_eq!(backend.applied(), vec![plan]);

        let snap = backend.query().unwrap();
        let out = |name: &str| &snap.outputs[snap.find(name).unwrap()];
        assert!(out("HDMI-1-0").active.is_none());
        let dp13 = out("DP-1-3").active.as_ref().unwrap();
        assert_eq!(
            (dp13.pos, dp13.size, dp13.rotation),
            (Point::new(4480, 0), Size::new(2160, 3840), Rotation::Left)
        );
        assert_eq!(
            out("DP-1-3").crtc,
            Some(4),
            "takes the CRTC HDMI-1-0 released"
        );
        assert!(out("eDP-1").primary);
        assert!(!out("DP-1-2").primary);
        assert_eq!(snap.screen.unwrap().current, Size::new(6640, 3840));
    }

    #[test]
    fn fixture_apply_warns_like_xrandr() {
        let backend = FixtureBackend::demo();
        let before = backend.query().unwrap();
        let outcome = apply(&backend, "--output eDP-9 --off");
        assert!(outcome.success);
        assert_eq!(
            outcome.stderr,
            "warning: output eDP-9 not found; ignoring\n"
        );
        assert_eq!(backend.query().unwrap(), before);

        assert_eq!(
            plan(&backend, "--output eDP-1 --bogus"),
            Err("xrandr: unrecognized option '--bogus'\n".to_owned())
        );
        assert_eq!(
            plan(&backend, "--output eDP-1 --mode 123x45"),
            Err("xrandr: cannot find mode 123x45 for output eDP-1\n".to_owned())
        );
    }

    #[test]
    fn a_failed_apply_changes_nothing() {
        let backend = FixtureBackend::demo();
        let before = backend.query().unwrap();
        let mut plan = plan(&backend, "--output eDP-1 --pos 0x0").unwrap();
        let on = plan.outputs[0].on.as_mut().unwrap();
        on.mode.id = crate::model::ModeId(0x999);
        let outcome = backend.apply(&plan).unwrap();
        assert_eq!(
            outcome.stderr,
            "xrandr: cannot find mode 0x999 for output eDP-1\n"
        );
        assert!(!outcome.success);
        assert_eq!(backend.query().unwrap(), before);
    }

    #[test]
    fn fixture_apply_scale_and_transform_reset() {
        let backend = FixtureBackend::demo();
        apply(&backend, "--output eDP-1 --scale 1.5x1.5");
        let snap = backend.query().unwrap();
        let edp = snap.outputs[snap.find("eDP-1").unwrap()]
            .active
            .clone()
            .unwrap();
        assert_eq!(edp.size, Size::new(2880, 1620));
        assert_eq!(edp.scaling.transform().unwrap().filter, "bilinear");
        apply(&backend, "--output eDP-1 --transform none");
        let snap = backend.query().unwrap();
        let edp = snap.outputs[snap.find("eDP-1").unwrap()]
            .active
            .clone()
            .unwrap();
        assert_eq!(edp.size, Size::new(1920, 1080));
        assert!(edp.scaling.is_identity());
    }
}
