//! Golden desk layouts: a start layout, a few edits, and the expected rectangles and links.

mod common;

use common::{Out, ix, link, load, load_wl, off, on, rect, stuck};
use outlay::backend::{Backend, FixtureBackend, Plan};
use outlay::model::geometry::{Dir, Rect};
use outlay::model::history::History;
use outlay::model::layout::Layout;
use outlay::model::links::{Align, Side};
use outlay::model::snap::SnapKind;
use outlay::model::validate::{Severity, validate};
use outlay::model::{Reflection, Rotation, Snapshot};
use outlay::xrandr::{command, parse_verbose};

/// Builds a desk: [`load`] for X11, [`load_wl`] for Wayland. The scenarios that run on both use
/// no primary display except the largest one, which is the root of its links either way, and
/// no mirrors.
type Loader = fn(&[Out]) -> (Snapshot, Layout);

/// Runs a scenario on an X11 desk and on a Wayland one.
macro_rules! on_both {
    ($scenario:ident, $x11:ident, $wayland:ident) => {
        #[test]
        fn $x11() {
            $scenario(load);
        }

        #[test]
        fn $wayland() {
            $scenario(load_wl);
        }
    };
}

on_both!(
    laptop_centred,
    laptop_stays_centred_under_a_monitor_through_a_nudge_and_a_mode_change,
    laptop_stays_centred_on_wayland
);

fn laptop_centred(load: Loader) {
    let (snap, mut layout) = load(&[
        on("M", 2560, 1440, 0, 0).primary(),
        on("L", 1920, 1080, 320, 1440),
    ]);
    assert_eq!(
        link(&layout, "L"),
        stuck("M", Side::Below, Align::Center, 0)
    );

    layout.nudge(ix(&layout, "L"), Dir::Right, 10).unwrap();
    assert_eq!(rect(&layout, "L"), Rect::new(330, 1440, 1920, 1080));
    assert_eq!(
        link(&layout, "L"),
        stuck("M", Side::Below, Align::Center, 10)
    );

    layout
        .set_resolution(&snap, ix(&layout, "M"), 1920, 1080)
        .unwrap();
    assert_eq!(rect(&layout, "M"), Rect::new(0, 0, 1920, 1080));
    assert_eq!(
        rect(&layout, "L"),
        Rect::new(10, 1080, 1920, 1080),
        "still centred, still +10"
    );
    assert_eq!(
        link(&layout, "L"),
        stuck("M", Side::Below, Align::Center, 10)
    );
}

on_both!(
    row_reordered,
    a_row_is_reordered_by_swapping,
    a_row_is_reordered_by_swapping_on_wayland
);

fn row_reordered(load: Loader) {
    let (_, mut layout) = load(&[
        on("A", 1920, 1080, 0, 0),
        on("B", 1920, 1080, 1920, 0),
        on("C", 1920, 1080, 3840, 0),
    ]);
    let a = ix(&layout, "A");
    let moved = layout.snap_move(a, Dir::Right).unwrap();
    assert_eq!(moved.kind, SnapKind::Swapped(ix(&layout, "B")));
    assert_eq!(rect(&layout, "B"), Rect::new(0, 0, 1920, 1080));
    assert_eq!(rect(&layout, "A"), Rect::new(1920, 0, 1920, 1080));
    assert_eq!(rect(&layout, "C"), Rect::new(3840, 0, 1920, 1080));
    assert_eq!(
        link(&layout, "A"),
        None,
        "A was the root and stays an anchor"
    );
    assert_eq!(
        link(&layout, "B"),
        stuck("A", Side::LeftOf, Align::Start, 0)
    );
    assert_eq!(
        link(&layout, "C"),
        stuck("A", Side::RightOf, Align::Start, 0)
    );

    layout.snap_move(a, Dir::Right).unwrap();
    let order: Vec<i32> = ["B", "C", "A"].iter().map(|n| rect(&layout, n).x).collect();
    assert_eq!(order, [0, 1920, 3840]);

    let err = layout.snap_move(a, Dir::Right).unwrap_err();
    assert_eq!(err.to_string(), "no snap spot further right");
}

on_both!(
    root_moved,
    moving_the_root_display,
    moving_the_root_display_on_wayland
);

fn root_moved(load: Loader) {
    // The demo desk: HDMI bottom-aligned left of the primary DP, the laptop centred below it.
    let (_, mut layout) = load(&[
        on("HDMI", 1920, 1080, 0, 360),
        on("DP", 2560, 1440, 1920, 0).primary(),
        on("eDP", 1920, 1080, 2240, 1440),
    ]);
    assert_eq!(link(&layout, "DP"), None);
    assert_eq!(
        link(&layout, "HDMI"),
        stuck("DP", Side::LeftOf, Align::End, 0)
    );
    assert_eq!(
        link(&layout, "eDP"),
        stuck("DP", Side::Below, Align::Center, 0)
    );

    // Everything hangs off DP, so a swap moves DP with the laptop below it.
    let mut swapped = layout.clone();
    let moved = swapped.snap_move(ix(&swapped, "DP"), Dir::Left).unwrap();
    assert_eq!(moved.kind, SnapKind::Swapped(ix(&swapped, "HDMI")));
    assert_eq!(rect(&swapped, "DP"), Rect::new(0, 0, 2560, 1440));
    assert_eq!(rect(&swapped, "eDP"), Rect::new(320, 1440, 1920, 1080));
    assert_eq!(rect(&swapped, "HDMI"), Rect::new(2560, 360, 1920, 1080));
    assert_eq!(
        link(&swapped, "HDMI"),
        stuck("DP", Side::RightOf, Align::End, 0)
    );
    assert_eq!(
        link(&swapped, "eDP"),
        stuck("DP", Side::Below, Align::Center, 0)
    );

    // A nudge moves the root alone: HDMI still touches it and follows, the laptop is left
    // floating as an anchor, and normalisation shifts everything down by 100.
    let dp = ix(&layout, "DP");
    let report = layout.nudge(dp, Dir::Up, 100).unwrap();
    assert_eq!(report.shift.y, 100);
    assert_eq!(rect(&layout, "DP"), Rect::new(1920, 0, 2560, 1440));
    assert_eq!(rect(&layout, "HDMI"), Rect::new(0, 460, 1920, 1080));
    assert_eq!(rect(&layout, "eDP"), Rect::new(2240, 1540, 1920, 1080));
    assert_eq!(
        link(&layout, "HDMI"),
        stuck("DP", Side::LeftOf, Align::End, 100)
    );
    assert_eq!(link(&layout, "eDP"), None);
}

on_both!(
    side_rotated,
    rotating_a_side_monitor_to_portrait_and_back,
    rotating_a_side_monitor_on_wayland
);

fn side_rotated(load: Loader) {
    let (_, mut layout) = load(&[
        on("A", 2560, 1440, 0, 0).primary(),
        on("B", 1920, 1080, 2560, 0),
        on("L", 1920, 1080, 320, 1440),
    ]);
    let original = layout.clone();
    let b = ix(&layout, "B");
    layout.rotate(b, true).unwrap();
    assert_eq!(rect(&layout, "B"), Rect::new(2560, 0, 1080, 1920));
    assert_eq!(
        link(&layout, "B"),
        stuck("A", Side::RightOf, Align::Start, 0)
    );
    assert_eq!(rect(&layout, "L"), Rect::new(320, 1440, 1920, 1080));
    layout.rotate(b, false).unwrap();
    assert_eq!(layout, original);
}

on_both!(
    grid_resized,
    a_resize_in_a_grid_pushes_the_display_below,
    a_resize_in_a_grid_pushes_on_wayland
);

fn grid_resized(load: Loader) {
    let (snap, mut layout) = load(&[
        on("A", 1920, 1080, 0, 0),
        on("B", 1920, 1080, 1920, 0),
        on("C", 1920, 1080, 0, 1080),
        on("D", 1920, 1080, 1920, 1080),
    ]);
    assert_eq!(
        link(&layout, "B"),
        stuck("A", Side::RightOf, Align::Start, 0)
    );
    assert_eq!(
        link(&layout, "C"),
        stuck("A", Side::Below, Align::Center, 0)
    );
    assert_eq!(
        link(&layout, "D"),
        stuck("C", Side::RightOf, Align::Start, 0)
    );

    let report = layout
        .set_resolution(&snap, ix(&layout, "B"), 2560, 1440)
        .unwrap();
    assert_eq!(report.pushed, vec![ix(&layout, "D")]);
    assert_eq!(rect(&layout, "B"), Rect::new(1920, 0, 2560, 1440));
    assert_eq!(
        rect(&layout, "D"),
        Rect::new(1920, 1440, 1920, 1080),
        "pushed down 360 px"
    );
    assert_eq!(
        link(&layout, "D"),
        stuck("C", Side::RightOf, Align::Start, 360)
    );
    assert!(layout.overlapping_pairs().is_empty());
}

#[test]
fn a_display_right_of_a_scaled_one_follows_it() {
    let (_, mut layout) = load(&[
        on("A", 1920, 1080, 0, 0).primary(),
        on("B", 1920, 1080, 1920, 0),
    ]);
    let original = layout.clone();
    let a = ix(&layout, "A");
    layout.set_scale(a, 1.5).unwrap();
    assert_eq!(rect(&layout, "A"), Rect::new(0, 0, 2880, 1620));
    assert_eq!(rect(&layout, "B"), Rect::new(2880, 0, 1920, 1080));
    assert_eq!(
        link(&layout, "B"),
        stuck("A", Side::RightOf, Align::Start, 0)
    );
    layout.step_scale(a, false).unwrap();
    assert_eq!(rect(&layout, "A"), Rect::new(0, 0, 2400, 1350));
    assert_eq!(rect(&layout, "B"), Rect::new(2400, 0, 1920, 1080));
    layout.reset_scale(a).unwrap();
    assert_eq!(layout, original);
}

#[test]
fn scaling_up_in_a_grid_pushes_the_display_below() {
    let (_, mut layout) = load(&[
        on("A", 1920, 1080, 0, 0),
        on("B", 1920, 1080, 1920, 0),
        on("C", 1920, 1080, 0, 1080),
        on("D", 1920, 1080, 1920, 1080),
    ]);
    let report = layout.set_scale(ix(&layout, "B"), 1.5).unwrap();
    assert_eq!(report.pushed, vec![ix(&layout, "D")]);
    assert_eq!(rect(&layout, "B"), Rect::new(1920, 0, 2880, 1620));
    assert_eq!(
        rect(&layout, "D"),
        Rect::new(1920, 1620, 1920, 1080),
        "pushed down 540 px"
    );
    assert!(layout.overlapping_pairs().is_empty());
}

#[test]
fn sticking_inserts_into_a_row() {
    // B is the root, so A must be re-rooted before F can go between them.
    let (snap, mut layout) = load(&[
        on("A", 1920, 1080, 0, 0),
        on("B", 1920, 1080, 1920, 0).primary(),
        on("F", 1280, 1024, 0, 1080),
    ]);
    assert_eq!(
        link(&layout, "A"),
        stuck("B", Side::LeftOf, Align::Start, 0)
    );
    assert_eq!(link(&layout, "F"), stuck("A", Side::Below, Align::Start, 0));

    let (f, a) = (ix(&layout, "F"), ix(&layout, "A"));
    layout
        .stick(&snap, f, a, Side::RightOf, Align::Start)
        .unwrap();
    assert_eq!(rect(&layout, "A"), Rect::new(0, 0, 1920, 1080));
    assert_eq!(rect(&layout, "F"), Rect::new(1920, 0, 1280, 1024));
    assert_eq!(rect(&layout, "B"), Rect::new(3200, 0, 1920, 1080));
    assert_eq!(link(&layout, "A"), None);
    assert_eq!(
        link(&layout, "F"),
        stuck("A", Side::RightOf, Align::Start, 0)
    );
    assert_eq!(
        link(&layout, "B"),
        stuck("F", Side::RightOf, Align::Start, 0)
    );
    assert!(
        layout.outputs[ix(&layout, "B")].primary,
        "sticking does not move primary"
    );
}

#[test]
fn a_mirror_pair() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/xrandr/mirror.txt"
    ))
    .unwrap();
    let snap = parse_verbose(&text).unwrap();
    let mut layout = Layout::inferred(&snap);
    assert_eq!(
        link(&layout, "HDMI-1"),
        stuck("eDP-1", Side::Same, Align::Start, 0)
    );
    assert_eq!(
        link(&layout, "DP-1"),
        stuck("eDP-1", Side::RightOf, Align::Start, 0)
    );
    assert!(
        validate(&layout, &snap).is_empty(),
        "a mirror pair is not an overlap"
    );

    // Mirroring DP-1 too switches it to eDP-1's resolution.
    let (dp, edp) = (ix(&layout, "DP-1"), ix(&layout, "eDP-1"));
    layout
        .stick(&snap, dp, edp, Side::Same, Align::Start)
        .unwrap();
    assert_eq!(rect(&layout, "DP-1"), Rect::new(0, 0, 1920, 1080));
    assert_eq!(
        layout.outputs[dp].mode.as_ref().unwrap().summary(),
        "1920x1080@60.00"
    );

    // Turning off the mirrored display hands everything to its first mirror.
    let before = layout.clone();
    layout.turn_off(edp).unwrap();
    assert_eq!(link(&layout, "HDMI-1"), None);
    assert_eq!(
        link(&layout, "DP-1"),
        stuck("HDMI-1", Side::Same, Align::Start, 0)
    );
    assert!(layout.outputs[ix(&layout, "HDMI-1")].primary);
    layout.turn_on(&snap, edp).unwrap();
    assert_eq!(layout, before);

    let script = command::script(&layout);
    assert_eq!(
        script.matches("--pos 0x0").count(),
        3,
        "mirrors share a position:\n{script}"
    );
}

#[test]
fn turning_a_display_off_and_back_on_restores_it() {
    let (snap, mut layout) = load(&[
        on("HDMI", 1920, 1080, 0, 360),
        on("DP", 2560, 1440, 1920, 0).primary(),
        on("eDP", 1920, 1080, 2240, 1440),
        off("DP3", 3840, 2160),
    ]);
    let original = layout.clone();
    let dp = ix(&layout, "DP");
    let report = layout.turn_off(dp).unwrap();
    assert_eq!(report.notes, vec!["HDMI is primary now.".to_owned()]);
    // The largest child inherits; the laptop sticks below it the same way (centred).
    assert_eq!(rect(&layout, "HDMI"), Rect::new(0, 0, 1920, 1080));
    assert_eq!(link(&layout, "HDMI"), None);
    assert_eq!(rect(&layout, "eDP"), Rect::new(0, 1080, 1920, 1080));
    assert_eq!(
        link(&layout, "eDP"),
        stuck("HDMI", Side::Below, Align::Center, 0)
    );

    layout.turn_on(&snap, dp).unwrap();
    assert_eq!(layout, original);

    // An output that was never on goes right of the rightmost display, top-aligned.
    let dp3 = ix(&layout, "DP3");
    layout.turn_on(&snap, dp3).unwrap();
    assert_eq!(rect(&layout, "DP3"), Rect::new(4480, 0, 3840, 2160));
    assert_eq!(
        link(&layout, "DP3"),
        stuck("DP", Side::RightOf, Align::Start, 0)
    );

    let mut last = load(&[on("Only", 1920, 1080, 0, 0)]).1;
    assert_eq!(
        last.turn_off(0).unwrap_err().to_string(),
        "The last enabled display stays on."
    );
}

on_both!(
    undo_redo,
    undo_and_redo_restore_whole_layouts,
    undo_and_redo_on_wayland
);

fn undo_redo(load: Loader) {
    let (snap, mut layout) = load(&[on("A", 1920, 1080, 0, 0), on("B", 1920, 1080, 1920, 0)]);
    let mut history = History::default();
    let start = layout.clone();

    let before = layout.clone();
    layout.set_resolution(&snap, 1, 2560, 1440).unwrap();
    assert!(history.record(before, &layout));
    let resized = layout.clone();

    // A failed snap changes nothing and records nothing.
    let before = layout.clone();
    assert!(layout.snap_move(1, Dir::Right).is_err());
    assert!(!history.record(before, &layout));
    assert_eq!(history.undo_len(), 1);

    assert!(history.undo(&mut layout));
    assert_eq!(layout, start);
    assert!(history.redo(&mut layout));
    assert_eq!(layout, resized);
    assert!(!history.redo(&mut layout));
}

#[test]
fn stale_outputs_are_turned_off_in_the_initial_layout() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/xrandr/active-disconnected.txt"
    ))
    .unwrap();
    let snap = parse_verbose(&text).unwrap();
    let (layout, notes) = Layout::from_snapshot(&snap);
    assert_eq!(
        notes,
        vec!["DP-1 is disconnected but still active; turning it off.".to_owned()]
    );
    assert!(!layout.is_enabled(ix(&layout, "DP-1")));
    assert_eq!(
        rect(&layout, "eDP-1"),
        Rect::new(0, 0, 1920, 1080),
        "normalised once the ghost is gone"
    );
    let diff: Vec<String> = layout.diff(&snap).iter().map(ToString::to_string).collect();
    assert_eq!(diff, ["eDP-1  pos 2560,0 → 0,0", "DP-1  on → off"]);

    // The command that applies it round-trips through the fixture backend.
    let backend = FixtureBackend::new(snap.clone());
    let outcome = backend.apply(&Plan::pending(&layout, &snap)).unwrap();
    assert!(outcome.success, "{}", outcome.stderr);
    let after = backend.query().unwrap();
    assert!(after.outputs[ix(&layout, "DP-1")].active.is_none());
    assert_eq!(
        Layout::inferred(&after).rect(ix(&layout, "eDP-1")),
        Rect::new(0, 0, 1920, 1080)
    );
}

#[test]
fn a_wayland_scale_shrinks_the_display_and_its_neighbour_follows() {
    let (_, mut layout) = load_wl(&[on("A", 1920, 1080, 0, 0), on("B", 1920, 1080, 1920, 0)]);
    let original = layout.clone();
    let a = ix(&layout, "A");
    layout.set_scale(a, 1.5).unwrap();
    assert_eq!(
        rect(&layout, "A"),
        Rect::new(0, 0, 1280, 720),
        "150 % is fewer pixels"
    );
    assert_eq!(rect(&layout, "B"), Rect::new(1280, 0, 1920, 1080));
    assert_eq!(
        link(&layout, "B"),
        stuck("A", Side::RightOf, Align::Start, 0)
    );
    layout.step_scale(a, false).unwrap();
    assert_eq!(rect(&layout, "A"), Rect::new(0, 0, 1536, 864));
    assert_eq!(rect(&layout, "B"), Rect::new(1536, 0, 1920, 1080));
    layout.reset_scale(a).unwrap();
    assert_eq!(layout, original);
}

#[test]
fn a_wayland_scale_below_one_grows_the_display_and_pushes() {
    let (_, mut layout) = load_wl(&[
        on("A", 1920, 1080, 0, 0),
        on("B", 1920, 1080, 1920, 0),
        on("C", 1920, 1080, 0, 1080),
        on("D", 1920, 1080, 1920, 1080),
    ]);
    let report = layout.set_scale(ix(&layout, "B"), 0.75).unwrap();
    assert_eq!(report.pushed, vec![ix(&layout, "D")]);
    assert_eq!(rect(&layout, "B"), Rect::new(1920, 0, 2560, 1440));
    assert_eq!(rect(&layout, "D"), Rect::new(1920, 1440, 1920, 1080));
    assert!(layout.overlapping_pairs().is_empty());
}

#[test]
fn scaled_wayland_heads_are_placed_by_their_logical_size() {
    // A 4K monitor at 150 % right of a 2560x1600 laptop at 125 %, bottom-aligned.
    let (_, layout) = load_wl(&[
        on("eDP-1", 2560, 1600, 0, 160).scaled(1.25),
        on("DP-1", 3840, 2160, 2048, 0).scaled(1.5),
    ]);
    assert_eq!(rect(&layout, "eDP-1"), Rect::new(0, 160, 2048, 1280));
    assert_eq!(rect(&layout, "DP-1"), Rect::new(2048, 0, 2560, 1440));
    assert_eq!(link(&layout, "DP-1"), None, "the largest is the root");
    assert_eq!(
        link(&layout, "eDP-1"),
        stuck("DP-1", Side::LeftOf, Align::End, 0)
    );
}

#[test]
fn identical_rectangles_overlap_on_wayland() {
    let (snap, mut layout) = load_wl(&[on("A", 1920, 1080, 0, 0), on("B", 1920, 1080, 0, 0)]);
    assert_eq!(link(&layout, "B"), None, "no mirror without the ability");
    assert_eq!(layout.overlapping_pairs(), vec![(0, 1)]);
    let issues = validate(&layout, &snap);
    assert!(
        issues
            .iter()
            .any(|i| i.severity == Severity::Warning && i.message == "A and B overlap."),
        "{issues:?}"
    );
    let err = layout
        .stick(&snap, 1, 0, Side::Same, Align::Start)
        .unwrap_err();
    assert_eq!(err.to_string(), "This compositor cannot mirror displays.");
    assert_eq!(
        layout.set_primary(0).unwrap_err().to_string(),
        "Wayland compositors have no primary display."
    );
}

#[test]
fn wayland_has_no_screen_maximum_and_no_primary_to_miss() {
    let (snap, layout) = load_wl(&[
        on("A", 3840, 2160, 0, 0).scaled(0.25),
        on("B", 3840, 2160, 15360, 0).scaled(0.25),
    ]);
    assert_eq!(layout.bounds().unwrap().w, 30720, "beyond any X screen");
    let issues = validate(&layout, &snap);
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn a_fractional_logical_size_is_a_warning() {
    let (snap, mut layout) = load_wl(&[on("eDP-1", 1920, 1080, 0, 0)]);
    let edp = ix(&layout, "eDP-1");
    layout.set_scale(edp, 1.75).unwrap();
    assert_eq!(
        rect(&layout, "eDP-1"),
        Rect::new(0, 0, 1097, 617),
        "truncated"
    );
    let messages: Vec<String> = validate(&layout, &snap)
        .into_iter()
        .map(|i| i.message)
        .collect();
    assert_eq!(
        messages,
        [
            "eDP-1 at 175% is 1097.1x617.1 px; the compositor rounds it, which can leave a 1 px \
             gap or overlap."
        ]
    );
    layout.set_scale(edp, 1.25).unwrap();
    assert!(validate(&layout, &snap).is_empty(), "1536x864 is whole");
    layout.set_scale(edp, 1.5).unwrap();
    assert!(validate(&layout, &snap).is_empty(), "1280x720 is whole");
}

#[test]
fn wayland_reflects_in_y_by_turning_a_reflection_in_x_upside_down() {
    let (_, mut layout) = load_wl(&[on("A", 1920, 1080, 0, 0)]);
    layout.rotate(0, true).unwrap();
    let report = layout.set_reflection(0, Reflection::Y).unwrap();
    let st = &layout.outputs[0];
    assert_eq!(
        (st.rotation, st.reflection),
        (Rotation::Left, Reflection::X)
    );
    assert_eq!(
        report.notes,
        ["Wayland reflects only in x: reflect y is the same picture as rotate left, reflect x."]
    );
    layout.set_reflection(0, Reflection::XY).unwrap();
    let st = &layout.outputs[0];
    assert_eq!(
        (st.rotation, st.reflection),
        (Rotation::Right, Reflection::Normal)
    );

    let (_, mut x11) = load(&[on("A", 1920, 1080, 0, 0)]);
    let report = x11.set_reflection(0, Reflection::Y).unwrap();
    assert!(report.notes.is_empty());
    assert_eq!(x11.outputs[0].reflection, Reflection::Y);
}

#[test]
fn turning_a_wayland_head_off_and_back_on_restores_it() {
    let (snap, mut layout) = load_wl(&[
        on("HDMI", 1920, 1080, 0, 360),
        on("DP", 2560, 1440, 1920, 0),
        on("eDP", 1920, 1080, 2240, 1440),
        off("DP3", 3840, 2160),
    ]);
    let original = layout.clone();
    let dp = ix(&layout, "DP");
    let report = layout.turn_off(dp).unwrap();
    assert!(report.notes.is_empty(), "no primary to hand on");
    assert_eq!(rect(&layout, "HDMI"), Rect::new(0, 0, 1920, 1080));
    assert_eq!(
        link(&layout, "eDP"),
        stuck("HDMI", Side::Below, Align::Center, 0)
    );
    layout.turn_on(&snap, dp).unwrap();
    assert_eq!(layout, original);

    let dp3 = ix(&layout, "DP3");
    layout.turn_on(&snap, dp3).unwrap();
    assert_eq!(rect(&layout, "DP3"), Rect::new(4480, 0, 3840, 2160));
    assert!(
        layout.outputs[dp3].scaling.is_identity(),
        "a new head starts at 100 %"
    );
}
