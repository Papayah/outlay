//! `outlay watch`: what it decides for a set of displays, and the loop that decides when.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use common::{ix, unplugged};
use outlay::backend::{ApplyOutcome, Backend, FixtureBackend, Plan};
use outlay::model::Snapshot;
use outlay::model::layout::Layout;
use outlay::profiles::{ProfileStore, Stored};
use outlay::tui::app::Options;
use outlay::tui::session::Settings;
use outlay::watch::{Watch, decide};

fn capture(path: &str) -> Snapshot {
    let path = format!("{}/tests/fixtures/{path}", env!("CARGO_MANIFEST_DIR"));
    FixtureBackend::from_file(path.as_ref())
        .unwrap()
        .query()
        .unwrap()
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("outlay-watch-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Saves `layout` on `snap` as profile `name` in `dir`, as `w` does.
fn save(dir: &Path, name: &str, layout: &Layout, snap: &Snapshot) {
    let store = ProfileStore::Screenlayout(dir.to_owned());
    let save = store.save(name, layout, snap).unwrap();
    store.write(&save.path, &save.text).unwrap();
}

fn profiles(dir: &Path) -> Vec<Stored> {
    ProfileStore::Screenlayout(dir.to_owned()).list().unwrap()
}

/// The demo with every external monitor unplugged after the panel was turned off: the outputs
/// that were on stay active, as xrandr leaves them.
fn panel_dark() -> Snapshot {
    let backend = FixtureBackend::demo();
    let snap = backend.query().unwrap();
    let mut layout = Layout::inferred(&snap);
    layout.turn_off(ix(&layout, "eDP-1")).unwrap();
    backend.apply(&Plan::pending(&layout, &snap)).unwrap();
    let mut snap = backend.query().unwrap();
    for name in ["HDMI-1-0", "DP-1-2", "DP-1-3"] {
        snap = unplugged(&snap, name);
    }
    snap
}

fn on(layout: &Layout, name: &str) -> bool {
    layout.is_enabled(ix(layout, name))
}

#[test]
fn decide_takes_a_profile_that_fits() {
    let demo = capture("xrandr/demo.txt");
    let dir = scratch("fits");
    let mut layout = Layout::inferred(&demo);
    layout.turn_off(ix(&layout, "HDMI-1-0")).unwrap();
    save(&dir, "desk", &layout, &demo);
    let d = decide(&demo, &profiles(&dir));
    assert_eq!(d.profile.as_deref(), Some("desk"));
    assert!(!on(&d.layout, "HDMI-1-0"));
    assert_eq!(d.summary(), "desk fits.");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn decide_takes_the_first_by_name_when_two_fit() {
    let demo = capture("xrandr/demo.txt");
    let dir = scratch("two");
    let layout = Layout::inferred(&demo);
    save(&dir, "work", &layout, &demo);
    save(&dir, "home", &layout, &demo);
    // Fits nothing: saved with one monitor less.
    let three = unplugged(&demo, "DP-1-3");
    save(&dir, "away", &Layout::inferred(&three), &three);
    let d = decide(&demo, &profiles(&dir));
    assert_eq!(d.profile.as_deref(), Some("home"));
    assert_eq!(d.others, ["work"]);
    assert_eq!(d.summary(), "home fits. Also fits: work.");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn decide_turns_off_an_unplugged_output_when_no_profile_fits() {
    let snap = capture("xrandr/active-disconnected.txt");
    let d = decide(&snap, &[]);
    assert_eq!(d.profile, None);
    assert!(on(&d.layout, "eDP-1"));
    assert!(!on(&d.layout, "DP-1"));
    assert!(!d.layout.diff(&snap).is_empty());
    assert_eq!(
        d.summary(),
        "No profile fits. DP-1 is disconnected but still active; turning it off."
    );
}

#[test]
fn decide_changes_nothing_when_nothing_fits_and_nothing_is_stale() {
    let demo = capture("xrandr/demo.txt");
    let d = decide(&demo, &[]);
    assert_eq!(d.profile, None);
    assert!(d.layout.diff(&demo).is_empty());
}

#[test]
fn decide_turns_the_panel_on_when_the_last_external_goes() {
    let snap = panel_dark();
    let d = decide(&snap, &[]);
    assert_eq!(d.profile, None);
    for name in ["HDMI-1-0", "DP-1-2", "DP-1-3"] {
        assert!(!on(&d.layout, name), "{name}");
    }
    assert!(on(&d.layout, "eDP-1"));
    assert!(
        d.summary()
            .contains("Only eDP-1 is connected: turning it on."),
        "{}",
        d.summary()
    );
}

#[test]
fn decide_turns_the_panel_on_with_a_laptop_profile_and_against_one_that_leaves_it_off() {
    let snap = panel_dark();
    let dir = scratch("laptop");
    // Saved on the panel alone, from the editor's view of it: stale outputs off.
    let alone = unplugged(
        &unplugged(
            &unplugged(&capture("xrandr/demo.txt"), "HDMI-1-0"),
            "DP-1-2",
        ),
        "DP-1-3",
    );
    save(&dir, "laptop", &Layout::from_snapshot(&alone).0, &alone);
    let d = decide(&snap, &profiles(&dir));
    assert_eq!(d.profile.as_deref(), Some("laptop"));
    assert!(on(&d.layout, "eDP-1"));
    assert!(!on(&d.layout, "HDMI-1-0"));

    // A profile that fits the panel but turns it off: the fail-safe wins.
    let text = std::fs::read_to_string(dir.join("laptop.sh")).unwrap();
    let dark = text.replace("--output eDP-1 --primary", "--output eDP-1 --off --primary");
    assert_ne!(dark, text);
    std::fs::write(dir.join("laptop.sh"), dark).unwrap();
    let d = decide(&snap, &profiles(&dir));
    assert_eq!(d.profile.as_deref(), Some("laptop"));
    assert!(on(&d.layout, "eDP-1"), "{}", d.summary());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn decide_never_switches_profiles_on_wayland() {
    let wl = capture("wayland/demo.json");
    let dir = scratch("wayland");
    let mut layout = Layout::inferred(&wl);
    layout.turn_off(1).unwrap();
    save(&dir, "would-fit", &layout, &wl);
    let stored = profiles(&dir);
    assert!(stored[0].profile.fit(&wl).is_some(), "it would fit on X11");
    let d = decide(&wl, &stored);
    assert_eq!(d.profile, None);
    assert!(d.layout.diff(&wl).is_empty(), "kanshi decides on Wayland");
    std::fs::remove_dir_all(dir).unwrap();
}

/// A fixture whose next apply changes the screens and then reports a failure.
struct Flaky {
    inner: FixtureBackend,
    fail: AtomicBool,
}

impl Backend for Flaky {
    fn query(&self) -> Result<Snapshot> {
        self.inner.query()
    }

    fn apply(&self, plan: &Plan) -> Result<ApplyOutcome> {
        let mut outcome = self.inner.apply(plan)?;
        if self.fail.swap(false, std::sync::atomic::Ordering::SeqCst) {
            outcome.success = false;
            outcome.stderr = "xrandr: Configure crtc 2 failed\n".to_owned();
        }
        Ok(outcome)
    }

    fn dump(&self) -> Result<String> {
        self.inner.dump()
    }
}

fn watch(dir: &Path) -> Watch {
    let settings = Settings {
        revert_seconds: 0,
        layouts_dir: Some(dir.to_owned()),
        ..Settings::default()
    };
    Watch::new(
        Options::default(),
        settings,
        false,
        Arc::new(AtomicBool::new(false)),
    )
}

#[test]
fn the_loop_acts_on_the_second_equal_reading_only() {
    let dir = scratch("loop");
    let backend = Flaky {
        inner: FixtureBackend::demo(),
        fail: AtomicBool::new(false),
    };
    let demo = backend.query().unwrap();
    let mut w = watch(&dir);
    w.step(&backend).unwrap();
    w.step(&backend).unwrap();
    assert!(backend.inner.applied().is_empty(), "the demo needs nothing");

    backend.inner.set_state(unplugged(&demo, "DP-1-2"));
    w.step(&backend).unwrap();
    assert!(
        backend.inner.applied().is_empty(),
        "one reading is not enough"
    );
    w.step(&backend).unwrap();
    assert_eq!(backend.inner.applied().len(), 1);
    let now = backend.query().unwrap();
    assert!(now.outputs[now.find("DP-1-2").unwrap()].active.is_none());
    w.step(&backend).unwrap();
    assert_eq!(
        backend.inner.applied().len(),
        1,
        "acted on this set already"
    );

    // Moved by hand: the connections are the same, so the hand wins.
    let mut layout = Layout::inferred(&now);
    let hdmi = ix(&layout, "HDMI-1-0");
    layout.outputs[hdmi].pos.x += 100;
    backend.apply(&Plan::pending(&layout, &now)).unwrap();
    let moved = backend.query().unwrap();
    for _ in 0..3 {
        w.step(&backend).unwrap();
    }
    assert_eq!(backend.inner.applied().len(), 2, "only the move");
    assert_eq!(backend.query().unwrap(), moved);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_failed_apply_reverts_and_the_loop_goes_on() {
    let dir = scratch("failed");
    let backend = Flaky {
        inner: FixtureBackend::demo(),
        fail: AtomicBool::new(true),
    };
    let demo = backend.query().unwrap();
    let mut w = watch(&dir);
    let three = unplugged(&demo, "DP-1-2");
    backend.inner.set_state(three.clone());
    w.step(&backend).unwrap();
    assert!(w.step(&backend).is_err(), "the failure is reported");
    assert_eq!(backend.inner.applied().len(), 2, "the apply and its revert");
    assert_eq!(
        backend.inner.applied()[1],
        Plan::restore(&three),
        "reverted"
    );
    w.step(&backend).unwrap();
    assert_eq!(
        backend.inner.applied().len(),
        2,
        "not tried again on the same set"
    );

    backend.inner.set_state(unplugged(&three, "DP-1-3"));
    w.step(&backend).unwrap();
    w.step(&backend).unwrap();
    assert_eq!(
        backend.inner.applied().len(),
        3,
        "the next change is acted on"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
