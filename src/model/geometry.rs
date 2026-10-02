//! Integer screen geometry in X pixels.

use super::{Rotation, Scaling};

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

    pub const fn area(&self) -> i64 {
        self.w as i64 * self.h as i64
    }

    pub const fn translated(&self, dx: i32, dy: i32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// The centre in doubled coordinates, so it stays an integer.
    pub const fn center2(&self) -> (i64, i64) {
        (
            2 * self.x as i64 + self.w as i64,
            2 * self.y as i64 + self.h as i64,
        )
    }

    /// True when the two rectangles share an area; touching edges do not overlap.
    pub const fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// Where `other` touches this rectangle edge to edge: the side of `self` it sits on and the
    /// length of the shared edge. A corner touch shares no edge and returns `None`.
    pub fn shared_edge(&self, other: &Rect) -> Option<(Dir, i32)> {
        let along_y = overlap_len(self.y, self.bottom(), other.y, other.bottom());
        let along_x = overlap_len(self.x, self.right(), other.x, other.right());
        if other.x == self.right() && along_y > 0 {
            Some((Dir::Right, along_y))
        } else if other.right() == self.x && along_y > 0 {
            Some((Dir::Left, along_y))
        } else if other.y == self.bottom() && along_x > 0 {
            Some((Dir::Down, along_x))
        } else if other.bottom() == self.y && along_x > 0 {
            Some((Dir::Up, along_x))
        } else {
            None
        }
    }
}

/// The length of `a0..a1 ∩ b0..b1`, or 0.
pub fn overlap_len(a0: i32, a1: i32, b0: i32, b1: i32) -> i32 {
    (a1.min(b1) - a0.max(b0)).max(0)
}

/// The bounding box of some rectangles.
pub fn bbox(rects: impl IntoIterator<Item = Rect>) -> Option<Rect> {
    rects.into_iter().reduce(|a, b| {
        let (x, y) = (a.x.min(b.x), a.y.min(b.y));
        Rect::new(
            x,
            y,
            a.right().max(b.right()) - x,
            a.bottom().max(b.bottom()) - y,
        )
    })
}

/// A direction on screen, as the `h j k l` keys name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    Left,
    Down,
    Up,
    Right,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::Left, Dir::Down, Dir::Up, Dir::Right];

    pub const fn opposite(self) -> Dir {
        match self {
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
        }
    }

    pub const fn is_horizontal(self) -> bool {
        matches!(self, Dir::Left | Dir::Right)
    }

    /// +1 for right and down, -1 for left and up.
    pub const fn sign(self) -> i32 {
        match self {
            Dir::Right | Dir::Down => 1,
            Dir::Left | Dir::Up => -1,
        }
    }

    /// The offset of a step of `amount` pixels in this direction.
    pub const fn delta(self, amount: i32) -> (i32, i32) {
        if self.is_horizontal() {
            (self.sign() * amount, 0)
        } else {
            (0, self.sign() * amount)
        }
    }

    pub const fn word(self) -> &'static str {
        match self {
            Dir::Left => "left",
            Dir::Down => "down",
            Dir::Up => "up",
            Dir::Right => "right",
        }
    }
}

/// The size an output covers on the screen: the mode rotated, then scaled. On X11 the scaling is
/// the output's transform, the way the X server computes a CRTC's footprint; for a plain
/// `--scale` this is the mode size times the scale, with width and height swapped for `left` and
/// `right`. A Wayland scale divides the rotated size instead.
pub fn effective_size(mode: Size, rotation: Rotation, scaling: &Scaling) -> Size {
    let rotated = if rotation.swaps_axes() {
        Size::new(mode.h, mode.w)
    } else {
        mode
    };
    scaling.bounds(rotated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Transform;

    #[test]
    fn effective_size_rotates_then_scales() {
        let fhd = Size::new(1920, 1080);
        assert_eq!(
            effective_size(fhd, Rotation::Normal, &Scaling::X11(Transform::identity())),
            fhd
        );
        assert_eq!(
            effective_size(fhd, Rotation::Left, &Scaling::X11(Transform::identity())),
            Size::new(1080, 1920)
        );
        assert_eq!(
            effective_size(
                fhd,
                Rotation::Inverted,
                &Scaling::X11(Transform::identity())
            ),
            fhd
        );
        let scaled = Scaling::X11(Transform::scale(1.5, 1.5));
        assert_eq!(
            effective_size(fhd, Rotation::Normal, &scaled),
            Size::new(2880, 1620)
        );
        assert_eq!(
            effective_size(fhd, Rotation::Right, &scaled),
            Size::new(1620, 2880)
        );
        // A non-uniform scale applies in screen space, after the rotation.
        let wide = Scaling::X11(Transform::scale(2.0, 1.0));
        assert_eq!(
            effective_size(fhd, Rotation::Left, &wide),
            Size::new(2160, 1920)
        );
    }

    #[test]
    fn a_logical_scale_divides_and_truncates() {
        let panel = Size::new(2880, 1800);
        assert_eq!(
            effective_size(panel, Rotation::Normal, &Scaling::Logical(2.0)),
            Size::new(1440, 900)
        );
        // 2880 / 1.75 = 1645.7 and 1800 / 1.75 = 1028.6: wlroots truncates both.
        assert_eq!(
            effective_size(panel, Rotation::Right, &Scaling::Logical(1.75)),
            Size::new(1028, 1645)
        );
    }

    #[test]
    fn shared_edges_and_overlaps() {
        let a = Rect::new(0, 0, 1920, 1080);
        assert_eq!(
            a.shared_edge(&Rect::new(1920, 540, 1920, 1080)),
            Some((Dir::Right, 540))
        );
        assert_eq!(
            a.shared_edge(&Rect::new(-100, 0, 100, 10)),
            Some((Dir::Left, 10))
        );
        assert_eq!(
            a.shared_edge(&Rect::new(320, 1080, 1280, 720)),
            Some((Dir::Down, 1280))
        );
        assert_eq!(a.shared_edge(&Rect::new(0, -10, 5, 10)), Some((Dir::Up, 5)));
        // A corner touch or a gap shares no edge.
        assert_eq!(a.shared_edge(&Rect::new(1920, 1080, 10, 10)), None);
        assert_eq!(a.shared_edge(&Rect::new(1921, 0, 10, 10)), None);
        assert!(!a.overlaps(&Rect::new(1920, 0, 10, 10)));
        assert!(a.overlaps(&Rect::new(1919, 1079, 10, 10)));
        assert_eq!(
            bbox([a, Rect::new(1920, 540, 100, 1000)]),
            Some(Rect::new(0, 0, 2020, 1540))
        );
        assert_eq!(bbox([]), None);
    }

    #[test]
    fn effective_size_absorbs_printed_rounding() {
        // xrandr prints the fixed-point matrix with six decimals: 1.1 becomes 1.099991.
        let t = Scaling::X11(Transform::scale(1.099991, 1.099991));
        assert_eq!(
            effective_size(Size::new(1920, 1080), Rotation::Normal, &t),
            Size::new(2112, 1188)
        );
    }
}
