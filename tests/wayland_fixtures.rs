//! Every `wlr-randr --json` fixture parses into the values it contains, and writes back to a
//! capture that reads the same.

use outlay::backend::{Backend, FixtureBackend};
use outlay::model::geometry::{Point, Size, effective_size};
use outlay::model::{Caps, ModeId, Output, Reflection, Rotation, Scaling, Snapshot};
use outlay::wayland::capture::{parse, write};

const FIXTURES: &[&str] = &[
    "custom-mode",
    "demo",
    "disabled-head",
    "headless-sway",
    "laptop-scaled",
    "no-serial",
    "rotated",
];

fn text(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/wayland/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn load(name: &str) -> Snapshot {
    parse(&text(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn output<'a>(snap: &'a Snapshot, name: &str) -> &'a Output {
    &snap.outputs[snap
        .find(name)
        .unwrap_or_else(|| panic!("no output {name}"))]
}

fn names(snap: &Snapshot) -> Vec<&str> {
    snap.outputs.iter().map(|o| o.name.as_str()).collect()
}

#[test]
fn every_capture_round_trips() {
    for name in FIXTURES {
        let snap = load(name);
        assert_eq!(snap.caps, Caps::wayland(), "{name}");
        assert_eq!(snap.screen, None, "{name}");
        let again = write(&snap);
        assert_eq!(parse(&again).unwrap(), snap, "{name}:\n{again}");
        assert_eq!(write(&parse(&again).unwrap()), again, "{name}: stable");
        for out in &snap.outputs {
            assert!(out.crtcs.is_empty() && out.crtc.is_none() && !out.primary);
            let (Some(active), Some(mode)) = (&out.active, out.current_mode()) else {
                continue;
            };
            let size = effective_size(mode.size(), active.rotation, &active.scaling);
            assert_eq!(size, active.size, "{name} {}", out.name);
        }
    }
}

#[test]
fn the_demo() {
    let snap = load("demo");
    assert_eq!(
        names(&snap),
        ["eDP-1", "DP-3", "DP-4", "HDMI-A-1"],
        "the built-in panel first, then by name, not as listed"
    );
    assert_eq!(
        FixtureBackend::demo_wayland().query().unwrap(),
        snap,
        "--demo=wayland"
    );

    let edp = output(&snap, "eDP-1");
    let id = edp.identity.as_ref().unwrap();
    assert_eq!(id.make.as_deref(), Some("Samsung Display Corp."));
    assert_eq!(id.serial, None);
    assert_eq!(id.label, "Samsung Display Corp. ATNA40YK07-1");
    assert_eq!(edp.physical_mm, Some(Size::new(302, 189)));
    let a = edp.active.as_ref().unwrap();
    assert_eq!(a.mode, ModeId::wayland(2880, 1800, 60_001));
    assert_eq!(a.scaling, Scaling::Logical(2.0));
    assert_eq!(a.size, Size::new(1440, 900), "200 % halves it");
    assert_eq!(a.pos, Point::new(2480, 1440));
    let mode = edp.current_mode().unwrap();
    assert_eq!(mode.name, "2880x1800");
    assert_eq!(mode.summary(), "2880x1800@60.00");
    assert!(mode.preferred && !mode.custom);

    let dell = output(&snap, "DP-3");
    assert_eq!(dell.label(), "DELL U2719D");
    assert_eq!(dell.adaptive_sync, Some(true));
    assert_eq!(
        dell.description.as_deref(),
        Some("Dell Inc. DELL U2719D 7LQ2M43 (DP-3)")
    );
    let current = dell.current_mode().unwrap();
    assert_eq!(current.id, ModeId::wayland(2560, 1440, 143_912));
    assert_eq!(current.refresh, 143.912, "143.912003 read back as mHz");
    assert_eq!(dell.preferred_mode().unwrap().refresh, 59.951);
    assert_eq!(dell.modes.len(), 9);

    let lg = output(&snap, "DP-4");
    assert!(lg.active.is_none() && lg.is_relevant());
    assert_eq!(lg.adaptive_sync, None, "a disabled head has none");
    assert_eq!(snap.numbered(), [0, 1, 2, 3], "every head is connected");
}

#[test]
fn scaled_heads_take_their_logical_size() {
    let snap = load("laptop-scaled");
    assert_eq!(names(&snap), ["eDP-1", "DP-1"]);
    let edp = output(&snap, "eDP-1").active.clone().unwrap();
    assert_eq!(edp.scaling, Scaling::Logical(1.25));
    assert_eq!(edp.size, Size::new(2048, 1280));
    let dp = output(&snap, "DP-1").active.clone().unwrap();
    assert_eq!(dp.scaling, Scaling::Logical(1.5));
    assert_eq!(dp.size, Size::new(2560, 1440));
    assert_eq!(dp.pos, Point::new(2048, 0));
}

#[test]
fn rotated_heads_read_the_protocol_names() {
    let snap = load("rotated");
    let dp1 = output(&snap, "DP-1").active.clone().unwrap();
    assert_eq!(
        (dp1.rotation, dp1.reflection),
        (Rotation::Left, Reflection::Normal)
    );
    assert_eq!(dp1.size, Size::new(1080, 1920));
    let dp2 = output(&snap, "DP-2").active.clone().unwrap();
    assert_eq!(
        (dp2.rotation, dp2.reflection),
        (Rotation::Right, Reflection::X)
    );
    assert_eq!(dp2.size, Size::new(1080, 1920));
    assert!(write(&snap).contains("\"transform\": \"flipped-270\""));
}

#[test]
fn disabled_heads_have_no_state_and_may_have_no_modes() {
    let snap = load("disabled-head");
    assert_eq!(names(&snap), ["eDP-1", "DP-2", "HDMI-A-1"]);
    let edp = output(&snap, "eDP-1");
    assert_eq!(edp.adaptive_sync, None, "adaptive_sync is null before v4");
    assert!(edp.active.is_some());
    let tv = output(&snap, "HDMI-A-1");
    assert!(tv.active.is_none());
    assert_eq!(tv.preferred_mode().unwrap().summary(), "3840x2160@30.00");
    let dp2 = output(&snap, "DP-2");
    assert!(dp2.modes.is_empty() && dp2.active.is_none());
    assert_eq!(dp2.identity, None);
    assert_eq!(dp2.description.as_deref(), Some("Unknown"));
    assert!(
        write(&snap).contains("\"modes\": []\n  }"),
        "as wlr-randr prints it"
    );
}

#[test]
fn custom_modes() {
    let snap = load("custom-mode");
    let dp = output(&snap, "DP-1");
    let current = dp.current_mode().unwrap();
    assert_eq!(current.id, ModeId::wayland(1600, 900, 59_978));
    assert!(current.custom, "outlay's own key");
    assert!(dp.modes.iter().filter(|m| m.custom).count() == 1);
    let virt = output(&snap, "HDMI-A-1");
    let current = virt.current_mode().unwrap();
    assert!(current.custom, "a rate of 0 is no real mode");
    assert_eq!(
        (current.width, current.height, current.refresh),
        (1366, 768, 0.0)
    );
    assert_eq!(virt.label(), "");
    assert_eq!(
        virt.description.as_deref(),
        Some("Virtual output (HDMI-A-1)")
    );
}

#[test]
fn heads_without_serial_or_maker() {
    let snap = load("no-serial");
    let edp = output(&snap, "eDP-1");
    assert_eq!(edp.label(), "Sharp Corporation 0x1516");
    assert_eq!(edp.identity.as_ref().unwrap().serial, None);
    let unknown = output(&snap, "DP-1");
    assert_eq!(unknown.identity, None);
    assert_eq!(unknown.physical_mm, None, "0x0 mm is no size");
    let lg = output(&snap, "HDMI-A-1");
    assert_eq!(lg.label(), "Goldstar Company Ltd LG ULTRAGEAR");
    assert_eq!(
        lg.current_mode().unwrap().summary(),
        "2560x1440@59.95",
        "not the preferred one"
    );
}

#[test]
fn headless_sway_as_outlay_dump_wrote_it() {
    let snap = load("headless-sway");
    assert_eq!(names(&snap), ["HEADLESS-1", "HEADLESS-2"]);
    for out in &snap.outputs {
        assert_eq!(out.identity, None);
        assert_eq!(out.modes.len(), 1, "a virtual mode only");
        let mode = out.current_mode().unwrap();
        assert!(mode.custom && !mode.preferred);
        assert_eq!((mode.width, mode.height, mode.refresh), (1280, 720, 0.0));
        assert_eq!(out.adaptive_sync, Some(false));
    }
    let pos = |name| output(&snap, name).active.as_ref().unwrap().pos;
    assert_eq!(
        pos("HEADLESS-2"),
        Point::new(0, 0),
        "sway puts the second one first"
    );
    assert_eq!(pos("HEADLESS-1"), Point::new(1280, 0));
    assert_eq!(write(&snap), text("headless-sway"), "byte for byte");
}
