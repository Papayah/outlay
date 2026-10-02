//! Focused checks of movement, focus, validation and mode rules.

mod common;

use common::{desk, ix, link, load, off, on, rect, stuck};
use outlay::model::geometry::{Dir, Rect};
use outlay::model::layout::{EditError, Layout, pick_rate};
use outlay::model::links::{Align, Side};
use outlay::model::snap::SnapKind;
use outlay::model::validate::{Severity, validate};
use outlay::xrandr::parse_verbose;

fn fixture(name: &str) -> outlay::model::Snapshot {
    let path = format!(
        "{}/tests/fixtures/xrandr/{name}.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    parse_verbose(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn messages(layout: &Layout, snap: &outlay::model::Snapshot) -> Vec<String> {
    validate(layout, snap)
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn spatial_focus_prefers_the_aligned_neighbour() {
    // A monitor with a laptop below it and a tall display to the right.
    let (_, layout) = load(&[
        on("M", 2560, 1440, 0, 0),
        on("L", 1920, 1080, 320, 1440),
        on("R", 1080, 1920, 2560, 0),
    ]);
    let (m, l, r) = (ix(&layout, "M"), ix(&layout, "L"), ix(&layout, "R"));
    assert_eq!(layout.focus_towards(m, Dir::Down), Some(l));
    assert_eq!(layout.focus_towards(m, Dir::Right), Some(r));
    assert_eq!(layout.focus_towards(l, Dir::Up), Some(m));
    // From the laptop, R's centre is right and up; M is not to the right of L at all.
    assert_eq!(layout.focus_towards(l, Dir::Right), Some(r));
    assert_eq!(layout.focus_towards(m, Dir::Left), None);
    assert_eq!(layout.focus_towards(r, Dir::Left), Some(m));
}

#[test]
fn alignment_stops_slide_a_laptop_along_its_monitor() {
    let (_, mut layout) = load(&[
        on("M", 2560, 1440, 0, 0).primary(),
        on("L", 1920, 1080, 320, 1440),
    ]);
    let l = ix(&layout, "L");
    let moved = layout.snap_move(l, Dir::Right).unwrap();
    assert_eq!(moved.kind, SnapKind::Aligned);
    assert_eq!(
        rect(&layout, "L"),
        Rect::new(640, 1440, 1920, 1080),
        "right-aligned"
    );
    assert_eq!(link(&layout, "L"), stuck("M", Side::Below, Align::End, 0));
    assert_eq!(
        layout.snap_move(l, Dir::Right),
        Err(EditError::NoSnapSpot(Dir::Right))
    );
    layout.snap_move(l, Dir::Left).unwrap();
    assert_eq!(
        rect(&layout, "L"),
        Rect::new(320, 1440, 1920, 1080),
        "back to centred"
    );
    layout.snap_move(l, Dir::Left).unwrap();
    assert_eq!(
        rect(&layout, "L"),
        Rect::new(0, 1440, 1920, 1080),
        "left-aligned"
    );
}

#[test]
fn a_nudge_may_overlap_and_validation_says_so() {
    let (snap, mut layout) = load(&[on("A", 1920, 1080, 0, 0), on("B", 1920, 1080, 1920, 0)]);
    layout.nudge(ix(&layout, "A"), Dir::Right, 10).unwrap();
    assert_eq!(rect(&layout, "A"), Rect::new(0, 0, 1920, 1080));
    assert_eq!(
        rect(&layout, "B"),
        Rect::new(1910, 0, 1920, 1080),
        "no push after a move"
    );
    assert_eq!(
        messages(&layout, &snap),
        ["warning: A and B overlap.", "info: No display is primary."]
    );
}

#[test]
fn validation_flags_floating_crtcs_size_and_stale_outputs() {
    let (snap, mut layout) = load(&[
        on("A", 1920, 1080, 0, 0).primary(),
        on("B", 1920, 1080, 1920, 0),
    ]);
    layout.nudge(ix(&layout, "B"), Dir::Right, 50).unwrap();
    assert_eq!(
        messages(&layout, &snap),
        [
            "warning: A touches no other display.",
            "warning: B touches no other display."
        ]
    );

    // Three CRTCs on the dock's GPU: a fourth display is too many.
    let snap = fixture("dock-mst-3");
    let (mut layout, _) = Layout::from_snapshot(&snap);
    assert!(validate(&layout, &snap).is_empty());
    layout.turn_on(&snap, snap.find("DP-2.3").unwrap()).unwrap();
    assert_eq!(
        messages(&layout, &snap),
        [
            "warning: 4 displays share 3 CRTCs (eDP-1, DP-2.1, DP-2.2, DP-2.3); the GPU may not drive them all."
        ]
    );

    let mut small = desk(&[on("A", 3840, 2160, 0, 0), on("B", 3840, 2160, 3840, 0)]);
    small.screen.as_mut().unwrap().max = outlay::model::geometry::Size::new(4096, 4096);
    let layout = Layout::inferred(&small);
    assert_eq!(
        validate(&layout, &small)[0].to_string(),
        "error: The layout needs 7680x2160, more than the screen maximum of 4096x4096."
    );

    let snap = fixture("active-disconnected");
    let issues = validate(&Layout::inferred(&snap), &snap);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, Severity::Warning);
    assert_eq!(issues[0].message, "DP-1 is disconnected but still active.");
}

#[test]
fn rate_rule_after_a_resolution_change() {
    let snap = fixture("demo");
    let dp = &snap.outputs[snap.find("DP-1-2").unwrap()];
    // 1. The previous rate within 0.5 Hz.
    assert_eq!(
        pick_rate(dp, 1920, 1080, Some(59.95)).unwrap().refresh,
        59.94
    );
    assert_eq!(
        pick_rate(dp, 1920, 1080, Some(119.9)).unwrap().refresh,
        119.88
    );
    // 2. The preferred rate, on the preferred resolution.
    assert_eq!(
        pick_rate(dp, 2560, 1440, Some(165.01)).unwrap().refresh,
        59.95
    );
    // 3. Otherwise the highest progressive rate.
    assert_eq!(
        pick_rate(dp, 1920, 1080, Some(165.01)).unwrap().refresh,
        119.88
    );
    let hdmi = &snap.outputs[snap.find("HDMI-1-0").unwrap()];
    assert!(
        pick_rate(hdmi, 1920, 1080, Some(60.0))
            .unwrap()
            .is_progressive(),
        "never the interlaced 60"
    );
}

#[test]
fn resolution_steps_skip_interlaced_and_stop_at_the_ends() {
    let snap = fixture("demo");
    let (mut layout, _) = Layout::from_snapshot(&snap);
    let edp = snap.find("eDP-1").unwrap();
    let err = layout.step_resolution(&snap, edp, true).unwrap_err();
    assert_eq!(err.to_string(), "eDP-1 is at its largest resolution.");
    layout.step_resolution(&snap, edp, false).unwrap();
    assert_eq!(
        layout.outputs[edp].mode.as_ref().unwrap().summary(),
        "1680x1050@59.95"
    );
    layout.step_resolution(&snap, edp, true).unwrap();
    // Back at 1920x1080: 59.95 is within 0.5 Hz of 59.96.
    assert_eq!(
        layout.outputs[edp].mode.as_ref().unwrap().summary(),
        "1920x1080@59.96"
    );
}

#[test]
fn panning_outputs_are_read_only() {
    let snap = fixture("panning");
    let (mut layout, _) = Layout::from_snapshot(&snap);
    let (edp, hdmi) = (snap.find("eDP-1").unwrap(), snap.find("HDMI-1").unwrap());
    let refused = EditError::Refused("eDP-1 uses panning; outlay leaves it as it is.".to_owned());
    assert_eq!(layout.rotate(edp, true).unwrap_err(), refused);
    assert_eq!(layout.nudge(edp, Dir::Right, 10).unwrap_err(), refused);
    assert_eq!(layout.turn_off(edp).unwrap_err(), refused);
    // Its neighbour can still move, and the panning display stays put.
    layout.snap_move(hdmi, Dir::Right).unwrap_err();
    layout.nudge(hdmi, Dir::Down, 100).unwrap();
    assert_eq!(layout.rect(edp), Rect::new(0, 0, 1920, 1080));
    assert_eq!(
        layout.rect(hdmi),
        Rect::new(1920, 100, 1920, 1080),
        "the origin stays pinned"
    );
}

#[test]
fn scale_steps_stop_at_the_ends_and_refuse_nonsense() {
    let (_, mut layout) = load(&[on("A", 1920, 1080, 0, 0), off("B", 1920, 1080)]);
    let (a, b) = (ix(&layout, "A"), ix(&layout, "B"));
    assert_eq!(
        layout.set_scale(a, 1.0).unwrap_err(),
        EditError::NoChange("A is at ×1 already.".to_owned())
    );
    assert_eq!(
        layout.set_scale(a, 9.0).unwrap_err(),
        EditError::Refused("A scale must be between 0.25 and 8.".to_owned())
    );
    assert_eq!(
        layout.set_scale(b, 1.5).unwrap_err(),
        EditError::Refused("B is off.".to_owned())
    );
    for _ in 0..6 {
        layout.step_scale(a, true).unwrap();
    }
    assert_eq!(layout.outputs[a].scaling.factor(), Some(3.0));
    assert_eq!(
        layout.step_scale(a, true).unwrap_err(),
        EditError::NoChange("A is at its largest scale.".to_owned())
    );
    // A scale off the list steps to its neighbours on the list.
    layout.set_scale(a, 1.1).unwrap();
    layout.step_scale(a, true).unwrap();
    assert_eq!(layout.outputs[a].scaling.factor(), Some(1.25));
    layout.set_scale(a, 1.1).unwrap();
    layout.step_scale(a, false).unwrap();
    assert_eq!(layout.outputs[a].scaling.factor(), Some(1.0));
    layout.set_scale(a, 0.5).unwrap();
    assert_eq!(
        layout.step_scale(a, false).unwrap_err(),
        EditError::NoChange("A is at its smallest scale.".to_owned())
    );
}

#[test]
fn a_transform_that_is_more_than_a_scale_survives_until_a_scale_replaces_it() {
    use outlay::backend::Plan;
    use outlay::model::{Scaling, Transform};
    let mut snap = fixture("scaled");
    let edp = snap.find("eDP-1").unwrap();
    let mut keystone = Transform::scale(1.5, 1.5);
    keystone.matrix[0][1] = 0.1;
    snap.outputs[edp].active.as_mut().unwrap().scaling = Scaling::X11(keystone);
    let (mut layout, _) = Layout::from_snapshot(&snap);
    let plan = Plan::pending(&layout, &snap);
    let args = outlay::xrandr::command::argv(&plan);
    assert!(
        !args.iter().any(|a| a == "--transform" || a == "--scale"),
        "an unchanged output writes nothing: {args:?}"
    );
    let report = layout.set_scale(edp, 1.5).unwrap();
    assert_eq!(
        report.notes,
        ["eDP-1's transform was not a uniform scale; ×1.5 replaces it."]
    );
    let args = outlay::xrandr::command::argv(&Plan::pending(&layout, &snap));
    assert!(
        args.windows(2).any(|w| w == ["--scale", "1.5x1.5"]),
        "{args:?}"
    );
}

#[test]
fn stick_refuses_nonsense() {
    let (snap, mut layout) = load(&[
        on("A", 1920, 1080, 0, 0),
        on("B", 1920, 1080, 1920, 0),
        off("C", 1920, 1080),
    ]);
    let (a, c) = (ix(&layout, "A"), ix(&layout, "C"));
    assert!(matches!(
        layout.stick(&snap, a, a, Side::RightOf, Align::Start),
        Err(EditError::Refused(_))
    ));
    assert!(matches!(
        layout.stick(&snap, a, c, Side::RightOf, Align::Start),
        Err(EditError::Refused(_))
    ));
    assert!(matches!(layout.unstick(a), Err(EditError::NoChange(_))));
}

#[test]
fn sticking_below_centres_by_default() {
    let (snap, mut layout) = load(&[on("M", 2560, 1440, 0, 0), on("L", 1920, 1080, 2560, 0)]);
    let (l, m) = (ix(&layout, "L"), ix(&layout, "M"));
    layout
        .stick(&snap, l, m, Side::Below, Align::default_for(Side::Below))
        .unwrap();
    assert_eq!(rect(&layout, "L"), Rect::new(320, 1440, 1920, 1080));
    assert_eq!(
        link(&layout, "L"),
        stuck("M", Side::Below, Align::Center, 0)
    );
    assert_eq!(layout.link_text(l), "below 1 M, centre");
}

#[test]
fn diff_reports_a_lost_primary_only_when_nobody_gains_it() {
    let (snap, mut layout) = load(&[
        on("A", 1920, 1080, 0, 0).primary(),
        on("B", 1920, 1080, 1920, 0),
    ]);
    layout.set_primary(1).unwrap();
    let diff: Vec<String> = layout.diff(&snap).iter().map(ToString::to_string).collect();
    assert_eq!(diff, ["B  primary"]);
    layout.outputs[1].primary = false;
    let diff: Vec<String> = layout.diff(&snap).iter().map(ToString::to_string).collect();
    assert_eq!(diff, ["A  not primary"]);
}

#[test]
fn history_keeps_the_last_200_steps() {
    use outlay::model::history::History;
    let (_, mut layout) = load(&[on("A", 1920, 1080, 0, 0), on("B", 1920, 1080, 1920, 0)]);
    let mut history = History::default();
    for _ in 0..250 {
        let before = layout.clone();
        layout.nudge(1, Dir::Down, 1).unwrap();
        assert!(history.record(before, &layout));
    }
    assert_eq!(history.undo_len(), History::CAP);
    while history.undo(&mut layout) {}
    assert_eq!(rect(&layout, "B").y, 50, "the oldest 50 steps are gone");
}
