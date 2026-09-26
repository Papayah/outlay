//! Golden desk layouts: a start layout, a few edits, and the expected rectangles and links.

mod common;

use common::{ix, link, load, off, on, rect, stuck};
use outlay::model::geometry::{Dir, Rect};
use outlay::model::history::History;
use outlay::model::layout::Layout;
use outlay::model::links::{Align, Side};
use outlay::model::snap::SnapKind;
use outlay::model::validate::validate;
use outlay::xrandr::{FixtureBackend, command, parse_verbose};

#[test]
fn laptop_stays_centred_under_a_monitor_through_a_nudge_and_a_mode_change() {
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

#[test]
fn a_row_is_reordered_by_swapping() {
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

#[test]
fn moving_the_root_display() {
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

#[test]
fn rotating_a_side_monitor_to_portrait_and_back() {
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

#[test]
fn a_resize_in_a_grid_pushes_the_display_below() {
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

    let script = command::script(&layout, &snap);
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

#[test]
fn undo_and_redo_restore_whole_layouts() {
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
    use outlay::xrandr::Backend;
    let outcome = backend.apply(&command::apply_args(&layout, &snap)).unwrap();
    assert!(outcome.success, "{}", outcome.stderr);
    let after = backend.query().unwrap();
    assert!(after.outputs[ix(&layout, "DP-1")].active.is_none());
    assert_eq!(
        Layout::inferred(&after).rect(ix(&layout, "eDP-1")),
        Rect::new(0, 0, 1920, 1080)
    );
}
