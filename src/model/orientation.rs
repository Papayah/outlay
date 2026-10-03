//! How a picture is turned: xrandr's rotation and reflection, which the model keeps, and the
//! Wayland transform names (`90`, `flipped-270`) that wlr-randr, kanshi and the protocol use.

use super::{Kind, Reflection, Rotation, x_only};

/// The `wl_output` transforms in protocol order (the index is the enum value), with the names
/// `wlr-randr` and kanshi use, as outlay's rotation and reflection.
///
/// The protocol turns counter-clockwise, like RandR: `wayland.xml` defines `90` as "90 degrees
/// counter-clockwise" and the flipped values as "an initial flip around a vertical axis followed
/// by rotation", and XWayland's `wl_transform_to_xrandr` (`hw/xwayland/xwayland-output.c`) maps
/// `90` to `RR_Rotate_90`, which xrandr calls `left`, and `flipped-90` to `RR_Reflect_X |
/// RR_Rotate_90`. The test below checks the pixels through wlroots' `wlr_box_transform` and the X
/// server's `RRTransformCompute`.
///
/// sway's own commands and `swaymsg -t get_outputs` name the turns the other way round
/// (`sway/commands/output/transform.c`: "Sway uses clockwise transforms"): sway's `90` is the
/// protocol's `270`.
pub const TRANSFORMS: [(&str, Rotation, Reflection); 8] = [
    ("normal", Rotation::Normal, Reflection::Normal),
    ("90", Rotation::Left, Reflection::Normal),
    ("180", Rotation::Inverted, Reflection::Normal),
    ("270", Rotation::Right, Reflection::Normal),
    ("flipped", Rotation::Normal, Reflection::X),
    ("flipped-90", Rotation::Left, Reflection::X),
    ("flipped-180", Rotation::Inverted, Reflection::X),
    ("flipped-270", Rotation::Right, Reflection::X),
];

/// The protocol value of a rotation and a reflection. A reflection in `y` has no value of its
/// own; [`x_only`] turns it into one in `x`.
pub fn transform_value(rotation: Rotation, reflection: Reflection) -> u32 {
    let (rotation, reflection) = x_only(rotation, reflection);
    TRANSFORMS
        .iter()
        .position(|&(_, r, f)| r == rotation && f == reflection)
        .expect("every rotation with no reflection or one in x has a transform") as u32
}

/// The name of a rotation and a reflection: `normal`, `90`, `flipped-270`.
pub fn transform_name(rotation: Rotation, reflection: Reflection) -> &'static str {
    TRANSFORMS[transform_value(rotation, reflection) as usize].0
}

/// The rotation and reflection of a transform name.
pub fn parse_transform(name: &str) -> Option<(Rotation, Reflection)> {
    TRANSFORMS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|&(_, r, f)| (r, f))
}

/// The rotation and reflection of a protocol value.
pub fn transform_from_value(value: u32) -> Option<(Rotation, Reflection)> {
    TRANSFORMS.get(value as usize).map(|&(_, r, f)| (r, f))
}

/// The orientation as the editor names it. On X11, xrandr's words: `left, reflect x`. On
/// Wayland, the transform name, with xrandr's word for a turn after it: `90 (left)`,
/// `flipped-270 (right)`; "flipped" says the reflection.
pub fn label(kind: Kind, rotation: Rotation, reflection: Reflection) -> String {
    match kind {
        Kind::X11 if reflection == Reflection::Normal => rotation.to_string(),
        Kind::X11 => format!("{rotation}, reflect {reflection}"),
        Kind::Wayland => {
            let (turn, _) = x_only(rotation, reflection);
            let name = transform_name(rotation, reflection);
            if turn == Rotation::Normal {
                name.to_owned()
            } else {
                format!("{name} ({turn})")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_name_the_transform_and_the_turn() {
        let wl = |r, f| label(Kind::Wayland, r, f);
        assert_eq!(wl(Rotation::Normal, Reflection::Normal), "normal");
        assert_eq!(wl(Rotation::Left, Reflection::Normal), "90 (left)");
        assert_eq!(wl(Rotation::Inverted, Reflection::Normal), "180 (inverted)");
        assert_eq!(wl(Rotation::Normal, Reflection::X), "flipped");
        assert_eq!(wl(Rotation::Right, Reflection::X), "flipped-270 (right)");
        assert_eq!(
            wl(Rotation::Normal, Reflection::Y),
            "flipped-180 (inverted)",
            "a reflection in y is one in x turned upside down"
        );
        let x11 = |r, f| label(Kind::X11, r, f);
        assert_eq!(x11(Rotation::Normal, Reflection::Normal), "normal");
        assert_eq!(x11(Rotation::Left, Reflection::XY), "left, reflect xy");
    }
}
