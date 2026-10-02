//! Random edit sequences keep the layout engine's invariants.

mod common;

use common::{Out, desk, off, on};
use outlay::model::Snapshot;
use outlay::model::geometry::Dir;
use outlay::model::history::History;
use outlay::model::layout::Layout;
use outlay::model::links::{Align, Side};
use outlay::xrandr::command;
use outlay::xrandr::script::Profile;
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

const SIZES: &[(i32, i32)] = &[
    (1920, 1080),
    (2560, 1440),
    (1280, 1024),
    (3840, 2160),
    (1080, 1920),
    (1366, 768),
];

/// How display k (k ≥ 1) is placed relative to an earlier one.
#[derive(Clone, Debug)]
struct Placement {
    parent: usize,
    size: usize,
    /// 0..4: a side; 4: floating with a gap; 5: mirroring the parent.
    kind: u8,
    along: u32,
}

fn arb_desk() -> impl Strategy<Value = Vec<Out>> {
    let placement = (0usize..8, 0usize..SIZES.len(), 0u8..6, any::<u32>()).prop_map(
        |(parent, size, kind, along)| Placement {
            parent,
            size,
            kind,
            along,
        },
    );
    (
        0usize..SIZES.len(),
        prop::collection::vec(placement, 1..5),
        prop::option::of(0usize..5),
        0usize..2,
    )
        .prop_map(|(first, placements, primary, extra_off)| {
            let mut outs = vec![on("O0", SIZES[first].0, SIZES[first].1, 0, 0)];
            for (k, p) in placements.iter().enumerate() {
                let parent = outs[p.parent % outs.len()].clone();
                let (px, py) = parent.pos.expect("placed displays are on");
                let (pw, ph) = parent.size;
                let (mut w, mut h) = SIZES[p.size];
                // A shared edge of at least one pixel.
                let slide = |plen: i32, clen: i32| {
                    -(clen - 1) + (p.along % (plen + clen - 1) as u32) as i32
                };
                let (x, y) = match p.kind {
                    0 => (px + pw, py + slide(ph, h)),
                    1 => (px - w, py + slide(ph, h)),
                    2 => (px + slide(pw, w), py + ph),
                    3 => (px + slide(pw, w), py - h),
                    4 => (px + pw + 100 + (p.along % 500) as i32, py),
                    _ => {
                        (w, h) = (pw, ph);
                        (px, py)
                    }
                };
                outs.push(on(&format!("O{}", k + 1), w, h, x, y));
            }
            if let Some(p) = primary
                && p < outs.len()
            {
                outs[p].primary = true;
            }
            for k in 0..extra_off {
                outs.push(off(&format!("X{k}"), 2560, 1440));
            }
            outs
        })
}

#[derive(Clone, Debug)]
enum Edit {
    Nudge(usize, Dir, i32),
    Snap(usize, Dir),
    Stick(usize, usize, Side, Align),
    Unstick(usize),
    Toggle(usize),
    Resolution(usize, bool),
    Rate(usize, bool),
    Rotate(usize, bool),
    Primary(usize),
    MoveTo(usize, i32, i32),
    Undo,
    Redo,
}

fn arb_dir() -> impl Strategy<Value = Dir> {
    prop::sample::select(Dir::ALL.to_vec())
}

fn arb_edit() -> impl Strategy<Value = Edit> {
    let side = prop::sample::select(vec![
        Side::LeftOf,
        Side::RightOf,
        Side::Above,
        Side::Below,
        Side::Same,
    ]);
    let align = prop::sample::select(Align::ALL.to_vec());
    let i = 0usize..8;
    prop_oneof![
        3 => (i.clone(), arb_dir(), prop::sample::select(vec![1, 10, 100, 500])).prop_map(|(i, d, s)| Edit::Nudge(i, d, s)),
        4 => (i.clone(), arb_dir()).prop_map(|(i, d)| Edit::Snap(i, d)),
        2 => (i.clone(), i.clone(), side, align).prop_map(|(f, t, s, a)| Edit::Stick(f, t, s, a)),
        1 => i.clone().prop_map(Edit::Unstick),
        2 => i.clone().prop_map(Edit::Toggle),
        2 => (i.clone(), any::<bool>()).prop_map(|(i, b)| Edit::Resolution(i, b)),
        1 => (i.clone(), any::<bool>()).prop_map(|(i, b)| Edit::Rate(i, b)),
        2 => (i.clone(), any::<bool>()).prop_map(|(i, b)| Edit::Rotate(i, b)),
        1 => i.clone().prop_map(Edit::Primary),
        1 => (i, -3000i32..3000, -3000i32..3000).prop_map(|(i, x, y)| Edit::MoveTo(i, x, y)),
        1 => Just(Edit::Undo),
        1 => Just(Edit::Redo),
    ]
}

fn apply(layout: &mut Layout, snap: &Snapshot, edit: &Edit) -> bool {
    let n = layout.len();
    let result = match *edit {
        Edit::Nudge(i, d, s) => layout.nudge(i % n, d, s).map(drop),
        Edit::Snap(i, d) => layout.snap_move(i % n, d).map(drop),
        Edit::Stick(f, t, s, a) => layout.stick(snap, f % n, t % n, s, a).map(drop),
        Edit::Unstick(i) => layout.unstick(i % n).map(drop),
        Edit::Toggle(i) => layout.toggle(snap, i % n).map(drop),
        Edit::Resolution(i, b) => layout.step_resolution(snap, i % n, b).map(drop),
        Edit::Rate(i, b) => layout.step_rate(snap, i % n, b).map(drop),
        Edit::Rotate(i, b) => layout.rotate(i % n, b).map(drop),
        Edit::Primary(i) => layout.set_primary(i % n).map(drop),
        Edit::MoveTo(i, x, y) => layout.move_to(i % n, x, y).map(drop),
        Edit::Undo | Edit::Redo => unreachable!("handled by the history"),
    };
    result.is_ok()
}

fn check_invariants(layout: &Layout) -> Result<(), TestCaseError> {
    let on = layout.enabled();
    prop_assert!(!on.is_empty(), "something stays enabled");
    let min_x = on.iter().map(|&i| layout.rect(i).x).min().unwrap();
    let min_y = on.iter().map(|&i| layout.rect(i).y).min().unwrap();
    prop_assert_eq!((min_x, min_y), (0, 0), "normalised");

    for (c, link) in layout.links.iter().enumerate() {
        let Some(link) = link else { continue };
        prop_assert!(
            layout.is_enabled(c) && layout.is_enabled(link.parent),
            "links join enabled displays: {c}"
        );
        let (child, parent) = (layout.rect(c), layout.rect(link.parent));
        match link.side {
            Side::Same => prop_assert_eq!(
                child.pos(),
                parent.pos(),
                "a mirror shares its parent's position"
            ),
            side => {
                let edge = parent.shared_edge(&child);
                prop_assert!(
                    edge.is_some_and(|(dir, len)| Side::from_dir(dir) == side && len >= 1),
                    "link {} → {} is flush: {:?} {:?} {:?}",
                    c,
                    link.parent,
                    side,
                    child,
                    parent
                );
            }
        }
        // No cycles: walking up from any display ends within n steps.
        let mut k = c;
        for _ in 0..=layout.len() {
            match layout.links[k] {
                Some(l) => k = l.parent,
                None => break,
            }
        }
        prop_assert!(layout.links[k].is_none(), "cycle through {c}");
    }

    let mut resolved = layout.clone();
    resolved.resolve();
    prop_assert_eq!(&resolved, layout, "resolve is idempotent");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]

    #[test]
    fn edit_sequences_keep_the_invariants(outs in arb_desk(), edits in prop::collection::vec(arb_edit(), 1..30)) {
        let snap = desk(&outs);
        let mut layout = Layout::inferred(&snap);
        let mut history = History::default();
        check_invariants(&layout)?;

        for edit in &edits {
            let before = layout.clone();
            let steps = history.undo_len();
            match edit {
                Edit::Undo => { history.undo(&mut layout); }
                Edit::Redo => { history.redo(&mut layout); }
                _ => {
                    if apply(&mut layout, &snap, edit) {
                        history.record(before.clone(), &layout);
                    } else {
                        prop_assert_eq!(&layout, &before, "a failed edit changes nothing: {:?}", edit);
                        prop_assert_eq!(history.undo_len(), steps, "a failed edit adds no undo step");
                    }
                }
            }
            check_invariants(&layout)?;

            if let Edit::Snap(..) = edit {
                let old = before.overlapping_pairs();
                prop_assert!(
                    layout.overlapping_pairs().iter().all(|p| old.contains(p)),
                    "a snap never creates an overlap: {:?}", edit
                );
            }

            if history.can_undo() {
                let (mut l, mut h) = (layout.clone(), history.clone());
                h.undo(&mut l);
                h.redo(&mut l);
                prop_assert_eq!(&l, &layout, "undo then redo is identical");
            }

            if layout.enabled_count() > 1 {
                for d in layout.enabled() {
                    let mut l = layout.clone();
                    l.turn_off(d).unwrap();
                    l.turn_on(&snap, d).unwrap();
                    prop_assert_eq!(&l, &layout, "off then on is identical for {}", d);
                }
            }
        }
    }

    #[test]
    fn a_refresh_keeps_what_did_not_change(
        outs in arb_desk(),
        edits in prop::collection::vec(arb_edit(), 0..20),
        gone in 0usize..8,
    ) {
        let snap = desk(&outs);
        let mut layout = Layout::inferred(&snap);
        for edit in edits.iter().filter(|e| !matches!(e, Edit::Undo | Edit::Redo)) {
            apply(&mut layout, &snap, edit);
        }
        prop_assert_eq!(&layout.remapped(&snap, &snap), &layout, "the same outputs: no change");

        // One display is unplugged, gone from the list, and comes back.
        let k = gone % snap.outputs.len();
        let mut without = snap.clone();
        without.outputs.remove(k);
        let there = layout.remapped(&snap, &without);
        prop_assert_eq!(there.len(), snap.outputs.len() - 1);
        let back = there.remapped(&without, &snap);
        prop_assert_eq!(&back.names, &layout.names);
        for i in (0..layout.len()).filter(|&i| i != k) {
            prop_assert_eq!(&back.outputs[i], &layout.outputs[i], "state of {}", i);
            prop_assert_eq!(back.numbers[i], layout.numbers[i], "number of {}", i);
            let link = layout.links[i].filter(|l| l.parent != k);
            prop_assert_eq!(back.links[i], link, "link of {}", i);
        }
        // The display that came back starts from its live state.
        let live = snap.outputs[k].active.as_ref();
        prop_assert_eq!(back.outputs[k].enabled, live.is_some());
        prop_assert_eq!(back.outputs[k].pos, live.map_or_else(Default::default, |a| a.pos));
        prop_assert_eq!(back.outputs[k].mode.as_ref().map(|m| m.id), live.map(|a| a.mode));
        prop_assert_eq!(back.links[k], None);
        prop_assert_eq!(back.numbers[k], layout.numbers[k], "its number is free again");
    }

    #[test]
    fn a_saved_layout_loads_back_the_same(outs in arb_desk(), edits in prop::collection::vec(arb_edit(), 0..20)) {
        let snap = desk(&outs);
        let mut layout = Layout::inferred(&snap);
        for edit in edits.iter().filter(|e| !matches!(e, Edit::Undo | Edit::Redo)) {
            apply(&mut layout, &snap, edit);
        }
        // Scripts do not store links; loading infers them from the positions.
        layout.infer_links();
        layout.restore = vec![None; layout.len()];
        let text = command::script(&layout, &snap);
        let profile = Profile::parse(&text);
        prop_assert!(profile.warnings.is_empty(), "{:?}", profile.warnings);
        let (mut loaded, notes) = profile.layout(&snap, &Vec::new());
        prop_assert!(notes.is_empty(), "{:?}", notes);
        // A script says only `--off` for an output that is off, not where it was.
        for i in 0..layout.len() {
            if !layout.is_enabled(i) && !loaded.is_enabled(i) {
                loaded.outputs[i] = layout.outputs[i].clone();
            }
        }
        prop_assert_eq!(loaded, layout, "{}", text);
    }
}
