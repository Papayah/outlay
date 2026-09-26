//! Integer screen geometry in X pixels.

use super::{Rotation, Transform};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Size {
    pub w: i32,
    pub h: i32,
}

impl Size {
    pub const fn new(w: i32, h: i32) -> Self {
        Self { w, h }
    }
}

/// A rectangle covering `x..x+w` and `y..y+h`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn from_parts(pos: Point, size: Size) -> Self {
        Self::new(pos.x, pos.y, size.w, size.h)
    }

    pub const fn pos(&self) -> Point {
        Point::new(self.x, self.y)
    }

    pub const fn size(&self) -> Size {
        Size::new(self.w, self.h)
    }

    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }
}

/// The size an output covers on the screen: the mode rotated, then passed through the output's
/// transform, the way the X server computes a CRTC's footprint. For a plain `--scale` this is the
/// mode size times the scale, with width and height swapped for `left` and `right`.
pub fn effective_size(mode: Size, rotation: Rotation, transform: &Transform) -> Size {
    let rotated = if rotation.swaps_axes() {
        Size::new(mode.h, mode.w)
    } else {
        mode
    };
    transform.bounds(rotated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_size_rotates_then_scales() {
        let fhd = Size::new(1920, 1080);
        assert_eq!(
            effective_size(fhd, Rotation::Normal, &Transform::identity()),
            fhd
        );
        assert_eq!(
            effective_size(fhd, Rotation::Left, &Transform::identity()),
            Size::new(1080, 1920)
        );
        assert_eq!(
            effective_size(fhd, Rotation::Inverted, &Transform::identity()),
            fhd
        );
        let scaled = Transform::scale(1.5, 1.5);
        assert_eq!(
            effective_size(fhd, Rotation::Normal, &scaled),
            Size::new(2880, 1620)
        );
        assert_eq!(
            effective_size(fhd, Rotation::Right, &scaled),
            Size::new(1620, 2880)
        );
        // A non-uniform scale applies in screen space, after the rotation.
        let wide = Transform::scale(2.0, 1.0);
        assert_eq!(
            effective_size(fhd, Rotation::Left, &wide),
            Size::new(2160, 1920)
        );
    }

    #[test]
    fn effective_size_absorbs_printed_rounding() {
        // xrandr prints the fixed-point matrix with six decimals: 1.1 becomes 1.099991.
        let t = Transform::scale(1.099991, 1.099991);
        assert_eq!(
            effective_size(Size::new(1920, 1080), Rotation::Normal, &t),
            Size::new(2112, 1188)
        );
    }
}
