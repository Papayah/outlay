//! What the X server reports about its screen and outputs, as read from `xrandr --verbose`.

pub mod geometry;

use std::fmt;

use crate::xrandr::edid::Edid;
use geometry::{Point, Rect, Size};

/// One `xrandr --verbose` reading of the screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub screen: ScreenLimits,
    /// Outputs in xrandr order, connected or not.
    pub outputs: Vec<Output>,
}

impl Snapshot {
    pub fn find(&self, name: &str) -> Option<usize> {
        self.outputs.iter().position(|o| o.name == name)
    }

    /// Indices of the outputs worth showing: connected or active. A display's number is its
    /// position in this list plus one, so numbers follow xrandr order and stay fixed.
    pub fn numbered(&self) -> Vec<usize> {
        (0..self.outputs.len())
            .filter(|&i| self.outputs[i].is_relevant())
            .collect()
    }

    /// The 1-based display number of output `index`, if it is numbered.
    pub fn number_of(&self, index: usize) -> Option<usize> {
        self.numbered()
            .iter()
            .position(|&i| i == index)
            .map(|p| p + 1)
    }
}

/// The `Screen 0:` line: framebuffer size limits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScreenLimits {
    pub min: Size,
    pub current: Size,
    pub max: Size,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Connected,
    Disconnected,
    Unknown,
}

impl Connection {
    pub fn as_str(self) -> &'static str {
        match self {
            Connection::Connected => "connected",
            Connection::Disconnected => "disconnected",
            Connection::Unknown => "unknown connection",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub name: String,
    pub connection: Connection,
    pub primary: bool,
    pub modes: Vec<Mode>,
    pub edid: Option<Edid>,
    /// Physical size, from the header for active outputs, else from the EDID.
    pub physical_mm: Option<Size>,
    /// The CRTC driving this output, if active.
    pub crtc: Option<u32>,
    /// The CRTCs that can drive this output.
    pub crtcs: Vec<u32>,
    pub active: Option<ActiveConfig>,
}

impl Output {
    /// Connected, or of unknown connection but offering modes (some drivers never know).
    pub fn is_connected(&self) -> bool {
        match self.connection {
            Connection::Connected => true,
            Connection::Unknown => !self.modes.is_empty(),
            Connection::Disconnected => false,
        }
    }

    pub fn is_relevant(&self) -> bool {
        self.is_connected() || self.active.is_some()
    }

    /// A disconnected output that still holds a CRTC, typically left behind after undocking.
    pub fn is_stale(&self) -> bool {
        self.active.is_some() && self.connection == Connection::Disconnected
    }

    pub fn mode(&self, xid: u32) -> Option<&Mode> {
        self.modes.iter().find(|m| m.xid == xid)
    }

    pub fn current_mode(&self) -> Option<&Mode> {
        self.active.as_ref().and_then(|a| self.mode(a.xid))
    }

    /// The `+preferred` mode, else the first listed one.
    pub fn preferred_mode(&self) -> Option<&Mode> {
        self.modes
            .iter()
            .find(|m| m.preferred)
            .or_else(|| self.modes.first())
    }

    /// The mode xrandr would pick for `--mode name [--rate rate]`: the listed mode with that name
    /// whose refresh is nearest to `rate`, or the first one with that name when no rate is given.
    pub fn find_mode(&self, name: &str, rate: Option<f64>) -> Option<&Mode> {
        let mut named = self.modes.iter().filter(|m| m.name == name);
        match rate {
            None => named.next(),
            Some(r) => named.min_by(|a, b| (a.refresh - r).abs().total_cmp(&(b.refresh - r).abs())),
        }
    }

    /// Distinct resolutions, largest first (by area, then width).
    pub fn resolutions(&self) -> Vec<Resolution> {
        let mut out: Vec<Resolution> = Vec::new();
        for m in &self.modes {
            let current = self.active.as_ref().is_some_and(|a| a.xid == m.xid);
            match out
                .iter_mut()
                .find(|r| r.width == m.width && r.height == m.height)
            {
                Some(r) => {
                    r.preferred |= m.preferred;
                    r.current |= current;
                }
                None => out.push(Resolution {
                    width: m.width,
                    height: m.height,
                    preferred: m.preferred,
                    current,
                }),
            }
        }
        out.sort_by_key(|r| std::cmp::Reverse((i64::from(r.width) * i64::from(r.height), r.width)));
        out
    }

    /// Every mode of one resolution, highest refresh first.
    pub fn rates(&self, width: i32, height: i32) -> Vec<&Mode> {
        let mut rates: Vec<&Mode> = self
            .modes
            .iter()
            .filter(|m| m.width == width && m.height == height)
            .collect();
        rates.sort_by(|a, b| b.refresh.total_cmp(&a.refresh));
        rates
    }

    /// Panning makes an output read-only for outlay v0.1.
    pub fn has_panning(&self) -> bool {
        self.active.as_ref().is_some_and(|a| a.panning.is_some())
    }

    /// The EDID-derived name ("Philips FTV", "AUO B156HAN12.H"), or an empty string.
    pub fn label(&self) -> String {
        self.edid
            .as_ref()
            .map(Edid::display_name)
            .unwrap_or_default()
    }
}

/// How an active output is configured.
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveConfig {
    /// XID of the current mode. It may be missing from the output's mode list when the output
    /// is disconnected but still active.
    pub xid: u32,
    pub pos: Point,
    /// The size in the header: the effective size, after rotation and transform.
    pub size: Size,
    pub rotation: Rotation,
    pub reflection: Reflection,
    pub transform: Transform,
    /// Set only when the panning area differs from the output's own rectangle.
    pub panning: Option<Rect>,
}

impl ActiveConfig {
    pub fn rect(&self) -> Rect {
        Rect::from_parts(self.pos, self.size)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mode {
    pub xid: u32,
    /// The mode name. Not necessarily a size: `1024x768i`, `1920x1080_60.00`.
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub refresh: f64,
    pub interlaced: bool,
    pub double_scan: bool,
    pub preferred: bool,
}

impl Mode {
    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    /// `1920x1080@165.01`
    pub fn summary(&self) -> String {
        format!("{}x{}@{:.2}", self.width, self.height, self.refresh)
    }

    /// The rate as the pickers show it: `59.94`, `60.00i`, `60.01dbl`.
    pub fn rate_label(&self) -> String {
        let suffix = match (self.interlaced, self.double_scan) {
            (true, _) => "i",
            (false, true) => "dbl",
            (false, false) => "",
        };
        format!("{:.2}{suffix}", self.refresh)
    }

    /// Plain progressive scan-out: neither interlaced nor DoubleScan.
    pub fn is_progressive(&self) -> bool {
        !self.interlaced && !self.double_scan
    }
}

/// One entry of the resolution picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub width: i32,
    pub height: i32,
    pub preferred: bool,
    pub current: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Rotation {
    #[default]
    Normal,
    Left,
    Inverted,
    Right,
}

impl Rotation {
    pub const ALL: [Rotation; 4] = [
        Rotation::Normal,
        Rotation::Left,
        Rotation::Inverted,
        Rotation::Right,
    ];

    /// The xrandr name, as used by `--rotate`.
    pub fn as_str(self) -> &'static str {
        match self {
            Rotation::Normal => "normal",
            Rotation::Left => "left",
            Rotation::Inverted => "inverted",
            Rotation::Right => "right",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == s)
    }

    /// `left` and `right` swap width and height.
    pub fn swaps_axes(self) -> bool {
        matches!(self, Rotation::Left | Rotation::Right)
    }

    /// The next rotation clockwise. xrandr's `right` turns the picture clockwise.
    pub fn clockwise(self) -> Self {
        match self {
            Rotation::Normal => Rotation::Right,
            Rotation::Right => Rotation::Inverted,
            Rotation::Inverted => Rotation::Left,
            Rotation::Left => Rotation::Normal,
        }
    }

    pub fn counter_clockwise(self) -> Self {
        match self {
            Rotation::Normal => Rotation::Left,
            Rotation::Left => Rotation::Inverted,
            Rotation::Inverted => Rotation::Right,
            Rotation::Right => Rotation::Normal,
        }
    }
}

impl fmt::Display for Rotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Reflection {
    #[default]
    Normal,
    X,
    Y,
    XY,
}

impl Reflection {
    pub const ALL: [Reflection; 4] = [
        Reflection::Normal,
        Reflection::X,
        Reflection::Y,
        Reflection::XY,
    ];

    /// The name `--reflect` takes.
    pub fn as_str(self) -> &'static str {
        match self {
            Reflection::Normal => "normal",
            Reflection::X => "x",
            Reflection::Y => "y",
            Reflection::XY => "xy",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == s)
    }
}

impl fmt::Display for Reflection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The 3x3 output transform (`--transform`, `--scale`) and its filter.
#[derive(Clone, Debug, PartialEq)]
pub struct Transform {
    pub matrix: [[f64; 3]; 3],
    /// `bilinear`, `nearest`, or empty.
    pub filter: String,
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

impl Transform {
    const EPSILON: f64 = 1e-6;

    pub fn identity() -> Self {
        Self {
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            filter: String::new(),
        }
    }

    /// What `xrandr --scale SXxSY` sets.
    pub fn scale(sx: f64, sy: f64) -> Self {
        Self {
            matrix: [[sx, 0.0, 0.0], [0.0, sy, 0.0], [0.0, 0.0, 1.0]],
            filter: "bilinear".to_owned(),
        }
    }

    pub fn is_identity(&self) -> bool {
        let id = Self::identity().matrix;
        self.matrix
            .iter()
            .flatten()
            .zip(id.iter().flatten())
            .all(|(a, b)| (a - b).abs() < Self::EPSILON)
    }

    /// The scale factors when the matrix is a pure scale, as `--scale` makes it.
    pub fn scale_factors(&self) -> Option<(f64, f64)> {
        let m = &self.matrix;
        let off_diagonal = [m[0][1], m[0][2], m[1][0], m[1][2], m[2][0], m[2][1]];
        let pure = off_diagonal.iter().all(|v| v.abs() < Self::EPSILON)
            && (m[2][2] - 1.0).abs() < Self::EPSILON;
        pure.then_some((m[0][0], m[1][1]))
    }

    /// The bounding size of a `size` rectangle at the origin after this transform, rounded outward
    /// like the server's `pixman_transform_bounds`. A small tolerance absorbs xrandr printing the
    /// fixed-point matrix with six decimals.
    pub fn bounds(&self, size: Size) -> Size {
        if self.is_identity() {
            return size;
        }
        const PRINT_TOLERANCE: f64 = 1e-3;
        let (w, h) = (f64::from(size.w), f64::from(size.h));
        let mut min = (f64::INFINITY, f64::INFINITY);
        let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for (x, y) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
            let m = &self.matrix;
            let wz = m[2][0] * x + m[2][1] * y + m[2][2];
            let tx = (m[0][0] * x + m[0][1] * y + m[0][2]) / wz;
            let ty = (m[1][0] * x + m[1][1] * y + m[1][2]) / wz;
            min = (min.0.min(tx), min.1.min(ty));
            max = (max.0.max(tx), max.1.max(ty));
        }
        let span = |lo: f64, hi: f64| {
            ((hi - PRINT_TOLERANCE).ceil() - (lo + PRINT_TOLERANCE).floor()) as i32
        };
        Size::new(span(min.0, max.0), span(min.1, max.1))
    }

    /// Row-major values for `--transform a,b,c,d,e,f,g,h,i`.
    pub fn xrandr_arg(&self) -> String {
        self.matrix
            .iter()
            .flatten()
            .map(|v| format!("{v}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}
