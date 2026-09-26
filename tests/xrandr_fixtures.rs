//! Every captured and synthetic `xrandr --verbose` fixture parses into the values it contains.

use outlay::model::geometry::{Point, Rect, Size, effective_size};
use outlay::model::{Connection, Output, Reflection, Rotation, Snapshot};
use outlay::xrandr::parse_verbose;

const FIXTURES: &[&str] = &[
    "active-disconnected",
    "demo",
    "dock-mst-3",
    "laptop-edp-hdmi",
    "mirror",
    "no-edid",
    "panning",
    "rotated-left",
    "scaled",
    "unknown-connection",
];

fn load(name: &str) -> Snapshot {
    let path = format!(
        "{}/tests/fixtures/xrandr/{name}.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    parse_verbose(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn output<'a>(snap: &'a Snapshot, name: &str) -> &'a Output {
    &snap.outputs[snap
        .find(name)
        .unwrap_or_else(|| panic!("no output {name}"))]
}

#[test]
fn every_header_size_matches_mode_rotation_and_transform() {
    for name in FIXTURES {
        let snap = load(name);
        for out in &snap.outputs {
            let (Some(active), Some(mode)) = (&out.active, out.current_mode()) else {
                continue;
            };
            let size = effective_size(mode.size(), active.rotation, &active.transform);
            assert_eq!(size, active.size, "{name} {}", out.name);
        }
    }
}

#[test]
fn live_capture_of_the_laptop() {
    let snap = load("laptop-edp-hdmi");
    assert_eq!(snap.screen.max, Size::new(16384, 16384));
    assert_eq!(snap.screen.current, Size::new(3840, 1080));
    // 7 outputs on modesetting and 6 on NVIDIA-G0; only eDP-1 and HDMI-1-0 are connected.
    assert_eq!(snap.outputs.len(), 13);
    assert_eq!(
        snap.outputs
            .iter()
            .filter(|o| o.connection == Connection::Disconnected)
            .count(),
        11
    );

    let edp = output(&snap, "eDP-1");
    assert!(edp.primary);
    let edid = edp.edid.as_ref().unwrap();
    assert_eq!(edid.pnp, "AUO");
    assert_eq!(edid.model(), "B156HAN12.H");
    assert_eq!(edp.physical_mm, Some(Size::new(344, 193)));
    assert_eq!(edp.crtc, Some(0));
    assert_eq!(edp.crtcs, vec![0, 1, 2, 3]);
    let active = edp.active.as_ref().unwrap();
    assert_eq!(active.rect(), Rect::new(1920, 0, 1920, 1080));
    assert_eq!(active.xid, 0x4a);
    assert_eq!(edp.current_mode().unwrap().refresh, 165.01);
    assert!(edp.modes.iter().any(|m| m.double_scan));
    assert!(
        edp.modes
            .iter()
            .any(|m| m.interlaced && m.name == "1024x768i")
    );

    let hdmi = output(&snap, "HDMI-1-0");
    assert!(!hdmi.primary);
    assert_eq!(hdmi.edid.as_ref().unwrap().model(), "Philips FTV");
    assert_eq!(hdmi.edid.as_ref().unwrap().pnp, "PHL");
    assert_eq!(hdmi.physical_mm, Some(Size::new(1440, 810)));
    assert_eq!(hdmi.crtcs, vec![4, 5, 6, 7]);
    let active = hdmi.active.as_ref().unwrap();
    assert_eq!(active.pos, Point::new(0, 0));
    assert_eq!(hdmi.current_mode().unwrap().refresh, 59.94);
    assert_eq!(hdmi.preferred_mode().unwrap().name, "3840x2160");

    // Disconnected, yet it lists modes; they belong to it and not to HDMI-1-0 above it.
    let dp14 = output(&snap, "DP-1-4");
    assert_eq!(dp14.modes.len(), 7);
    assert!(!dp14.is_relevant());
    assert_eq!(
        snap.numbered(),
        vec![snap.find("eDP-1").unwrap(), snap.find("HDMI-1-0").unwrap()]
    );
}

#[test]
fn demo_fixture() {
    let snap = load("demo");
    let names: Vec<&str> = snap.outputs.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["HDMI-1-0", "DP-1-2", "eDP-1", "DP-1-3"]);
    let dp12 = output(&snap, "DP-1-2");
    assert!(dp12.primary);
    assert_eq!(
        dp12.active.as_ref().unwrap().rect(),
        Rect::new(1920, 0, 2560, 1440)
    );
    assert_eq!(dp12.current_mode().unwrap().refresh, 143.91);
    assert_eq!(
        output(&snap, "HDMI-1-0").active.as_ref().unwrap().pos,
        Point::new(0, 360)
    );
    let edp = output(&snap, "eDP-1");
    assert_eq!(
        edp.active.as_ref().unwrap().pos,
        Point::new(2240, 1440),
        "centred below DP-1-2"
    );
    assert_eq!(edp.current_mode().unwrap().refresh, 165.01);
    let dp13 = output(&snap, "DP-1-3");
    assert!(dp13.is_connected() && dp13.active.is_none());
    assert_eq!(dp13.label(), "LG HDR 4K");
    assert_eq!(dp13.preferred_mode().unwrap().summary(), "3840x2160@60.00");
}

#[test]
fn dock_with_mst_names() {
    let snap = load("dock-mst-3");
    assert_eq!(
        output(&snap, "DP-2.2").active.as_ref().unwrap().pos,
        Point::new(2560, 0)
    );
    assert!(output(&snap, "DP-2.2").primary);
    assert_eq!(
        output(&snap, "eDP-1").label(),
        "BOE NE156FHM-NX1",
        "the last 0xFE text is the part number"
    );
    let dp23 = output(&snap, "DP-2.3");
    assert!(dp23.is_connected() && dp23.active.is_none());
    assert_eq!(dp23.crtcs, vec![0, 1, 2]);
    assert!(!output(&snap, "DP-2").is_relevant());
}

#[test]
fn rotated_output() {
    let snap = load("rotated-left");
    let hdmi = output(&snap, "HDMI-1");
    let active = hdmi.active.as_ref().unwrap();
    assert_eq!(active.rotation, Rotation::Left);
    assert_eq!(active.reflection, Reflection::Normal);
    assert_eq!(active.size, Size::new(1440, 2560));
    assert_eq!(hdmi.current_mode().unwrap().size(), Size::new(2560, 1440));
}

#[test]
fn disconnected_but_active() {
    let snap = load("active-disconnected");
    let dp1 = output(&snap, "DP-1");
    assert!(dp1.is_stale() && dp1.is_relevant());
    assert!(dp1.modes.is_empty());
    assert_eq!(dp1.active.as_ref().unwrap().xid, 0x1f3);
    assert_eq!(dp1.active.as_ref().unwrap().size, Size::new(2560, 1440));
    assert_eq!(dp1.physical_mm, None);
    assert!(dp1.current_mode().is_none());
}

#[test]
fn outputs_without_edid() {
    let snap = load("no-edid");
    assert_eq!(snap.screen.max, Size::new(8192, 8192));
    let v1 = output(&snap, "Virtual-1");
    assert!(v1.edid.is_none());
    assert_eq!(v1.label(), "");
    assert_eq!(v1.physical_mm, None);
    assert!(v1.primary);
}

#[test]
fn scaled_output() {
    let snap = load("scaled");
    let active = output(&snap, "eDP-1").active.clone().unwrap();
    assert_eq!(active.transform.scale_factors(), Some((1.5, 1.5)));
    assert_eq!(active.transform.filter, "bilinear");
    assert_eq!(active.size, Size::new(2880, 1620));
    assert!(
        output(&snap, "HDMI-1")
            .active
            .as_ref()
            .unwrap()
            .transform
            .is_identity()
    );
}

#[test]
fn panning_output() {
    let snap = load("panning");
    let edp = output(&snap, "eDP-1");
    assert!(edp.has_panning());
    assert_eq!(
        edp.active.as_ref().unwrap().panning,
        Some(Rect::new(0, 0, 1920, 2160))
    );
    assert!(!output(&snap, "HDMI-1").has_panning());
}

#[test]
fn mirrored_outputs_share_a_rectangle() {
    let snap = load("mirror");
    let a = output(&snap, "eDP-1").active.as_ref().unwrap().rect();
    let b = output(&snap, "HDMI-1").active.as_ref().unwrap().rect();
    assert_eq!(a, b);
    assert_eq!(
        output(&snap, "HDMI-1").physical_mm,
        None,
        "a projector reports no size"
    );
}

#[test]
fn unknown_connection() {
    let snap = load("unknown-connection");
    let vga = output(&snap, "VGA-1");
    assert_eq!(vga.connection, Connection::Unknown);
    assert!(vga.is_connected());
    // The capture drops the `clock` of 800x600's `v:` line; the rate comes from the timings.
    let m800 = vga.modes.iter().find(|m| m.width == 800).unwrap();
    assert!((m800.refresh - 60.32).abs() < 0.01, "{}", m800.refresh);
    let dvi = output(&snap, "DVI-I-1");
    assert_eq!(dvi.connection, Connection::Unknown);
    assert!(!dvi.is_relevant(), "unknown and without modes");
}
