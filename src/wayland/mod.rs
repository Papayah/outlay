//! Wayland compositors that speak `zwlr_output_manager_v1` (sway, Hyprland, niri, river …): the
//! capture format, the equivalent `wlr-randr` command, and what the protocol's names mean.

pub mod capture;
pub mod client;
pub mod command;
pub mod kanshi;

pub use client::WlrBackend;

use std::cmp::Ordering;
use std::path::Path;

use crate::model::Snapshot;
pub use crate::model::orientation::{
    TRANSFORMS, parse_transform, transform_from_value, transform_name, transform_value,
};

/// The built-in fixture behind `--demo=wayland`.
pub const DEMO: &str = include_str!("../../tests/fixtures/wayland/demo.json");

/// The `revert.sh` written before an apply on Wayland: it hands the state from before the apply,
/// as a capture, to `outlay restore`, which applies it back. `program` is outlay itself.
pub fn revert_script(program: &Path, before: &Snapshot) -> String {
    let program = program.to_string_lossy().replace('\'', r"'\''");
    format!(
        "#!/bin/sh\n# Written by outlay {} before an apply. It restores the layout from before \
         it.\nexec '{program}' restore - <<'OUTLAY-CAPTURE'\n{}OUTLAY-CAPTURE\n",
        env!("CARGO_PKG_VERSION"),
        capture::write(before),
    )
}

/// Whether a head is a built-in panel: `eDP`, `LVDS` or `DSI`.
pub fn built_in(name: &str) -> bool {
    ["eDP", "LVDS", "DSI"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// How heads are numbered: built-in panels first, then by name with the numbers in names
/// compared as numbers, so numbers do not depend on what was plugged in first.
pub fn head_order(a: &str, b: &str) -> Ordering {
    (!built_in(a))
        .cmp(&!built_in(b))
        .then_with(|| natural(a, b))
        .then_with(|| a.cmp(b))
}

/// Compares names piece by piece, runs of digits by value: `DP-2` < `DP-10`.
fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a, b);
    loop {
        let (Some(ca), Some(cb)) = (a.chars().next(), b.chars().next()) else {
            return a.len().cmp(&b.len());
        };
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let digits = |s: &str| s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
            let (na, nb) = (digits(a), digits(b));
            let value = |s: &str| s.trim_start_matches('0').to_owned();
            let (va, vb) = (value(&a[..na]), value(&b[..nb]));
            let order = va.len().cmp(&vb.len()).then_with(|| va.cmp(&vb));
            if order != Ordering::Equal {
                return order;
            }
            (a, b) = (&a[na..], &b[nb..]);
        } else {
            if ca != cb {
                return ca.cmp(&cb);
            }
            (a, b) = (&a[ca.len_utf8()..], &b[cb.len_utf8()..]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Reflection, Rotation, x_only};

    const W: i32 = 4;
    const H: i32 = 3;

    /// The framebuffer pixel a panel pixel shows under RandR, as the X server's
    /// `RRTransformCompute` (`randr/rrtransform.c`) builds the CRTC-to-framebuffer transform:
    /// rotate by `RR_Rotate_*` (`left` is `RR_Rotate_90`), then reflect. The panel is `W`x`H`.
    fn randr(rotation: Rotation, reflection: Reflection, (x, y): (i32, i32)) -> (i32, i32) {
        let (mut fx, mut fy) = match rotation {
            Rotation::Normal => (x, y),
            Rotation::Left => (H - y, x),
            Rotation::Inverted => (W - x, H - y),
            Rotation::Right => (y, W - x),
        };
        let (span_x, span_y) = if rotation.swaps_axes() {
            (H, W)
        } else {
            (W, H)
        };
        if matches!(reflection, Reflection::X | Reflection::XY) {
            fx = span_x - fx;
        }
        if matches!(reflection, Reflection::Y | Reflection::XY) {
            fy = span_y - fy;
        }
        (fx, fy)
    }

    /// The logical pixel a panel pixel shows under wlroots: the scene goes to the buffer through
    /// `wlr_box_transform(box, wlr_output_transform_invert(T), trans_width, trans_height)`
    /// (`types/scene/wlr_scene.c`, `transform_output_box`; `util/box.c`). Returns the inverse
    /// of that map, from the panel back to the logical picture.
    fn wlroots(value: u32, panel: (i32, i32)) -> (i32, i32) {
        let swaps = value % 2 == 1;
        let (tw, th) = if swaps { (H, W) } else { (W, H) };
        // `wlr_output_transform_invert` swaps 90 and 270 unless flipped.
        let inverted = if value & 1 == 1 && value & 4 == 0 {
            value ^ 2
        } else {
            value
        };
        let to_buffer = |(x, y): (i32, i32)| match inverted {
            0 => (x, y),
            1 => (th - y, x),
            2 => (tw - x, th - y),
            3 => (y, tw - x),
            4 => (tw - x, y),
            5 => (y, x),
            6 => (x, th - y),
            7 => (th - y, tw - x),
            _ => unreachable!(),
        };
        let mut found = None;
        for lx in 0..=tw {
            for ly in 0..=th {
                if to_buffer((lx, ly)) == panel {
                    assert!(found.is_none(), "the map is one to one");
                    found = Some((lx, ly));
                }
            }
        }
        found.expect("every panel pixel shows a logical one")
    }

    fn panel() -> Vec<(i32, i32)> {
        (0..=W).flat_map(|x| (0..=H).map(move |y| (x, y))).collect()
    }

    #[test]
    fn transforms_match_randr_pixel_for_pixel() {
        for (value, &(name, rotation, reflection)) in TRANSFORMS.iter().enumerate() {
            for p in panel() {
                assert_eq!(
                    wlroots(value as u32, p),
                    randr(rotation, reflection, p),
                    "{name} at {p:?}"
                );
            }
        }
        assert_eq!(transform_name(Rotation::Left, Reflection::X), "flipped-90");
        assert_eq!(transform_name(Rotation::Right, Reflection::Normal), "270");
        assert_eq!(
            parse_transform("flipped-270"),
            Some((Rotation::Right, Reflection::X))
        );
        assert_eq!(parse_transform("left"), None);
        assert_eq!(
            transform_from_value(5),
            Some((Rotation::Left, Reflection::X))
        );
        assert_eq!(transform_from_value(8), None);
    }

    #[test]
    fn reflections_in_y_become_reflections_in_x() {
        for rotation in Rotation::ALL {
            for reflection in Reflection::ALL {
                let (r, f) = x_only(rotation, reflection);
                assert!(matches!(f, Reflection::Normal | Reflection::X));
                for p in panel() {
                    assert_eq!(
                        randr(r, f, p),
                        randr(rotation, reflection, p),
                        "{rotation} {reflection} at {p:?}"
                    );
                }
            }
        }
        assert_eq!(
            transform_name(Rotation::Normal, Reflection::Y),
            "flipped-180"
        );
        assert_eq!(transform_name(Rotation::Left, Reflection::XY), "270");
    }

    #[test]
    fn built_in_panels_first_then_natural_order() {
        let mut names = vec![
            "HDMI-A-1",
            "DP-10",
            "eDP-1",
            "DP-2",
            "DSI-1",
            "DP-1",
            "LVDS-1",
            "HEADLESS-2",
            "HEADLESS-1",
        ];
        names.sort_by(|a, b| head_order(a, b));
        assert_eq!(
            names,
            [
                "DSI-1",
                "LVDS-1",
                "eDP-1",
                "DP-1",
                "DP-2",
                "DP-10",
                "HDMI-A-1",
                "HEADLESS-1",
                "HEADLESS-2"
            ]
        );
        assert_eq!(natural("DP-02", "DP-2"), Ordering::Equal);
        assert_eq!(
            head_order("DP-02", "DP-2"),
            Ordering::Less,
            "still a total order"
        );
    }
}
