//! Builds snapshots for tests without writing `xrandr --verbose` text.

#![allow(dead_code)]

pub mod sway;

use outlay::model::geometry::{Point, Rect, Size, effective_size};
use outlay::model::layout::Layout;
use outlay::model::links::{Align, Link, Side};
use outlay::model::{
    ActiveConfig, Caps, Connection, Mode, ModeId, Output, Reflection, Rotation, Scaling,
    ScreenLimits, Snapshot, Transform,
};

/// Every output offers these modes, so resizes are possible in any direction.
const MODES: &[(i32, i32, f64)] = &[
    (3840, 2160, 60.0),
    (2560, 1440, 143.91),
    (2560, 1440, 59.95),
    (1920, 1080, 165.01),
    (1920, 1080, 60.0),
    (1920, 1080, 50.0),
    (1280, 1024, 60.02),
    (1280, 720, 60.0),
];

/// One output of a test desk.
#[derive(Clone, Debug)]
pub struct Out {
    pub name: String,
    pub size: (i32, i32),
    pub pos: Option<(i32, i32)>,
    pub primary: bool,
    /// The Wayland scale; [`desk`] ignores it.
    pub scale: f64,
}

/// An enabled output of `w`x`h` at `x`,`y`.
pub fn on(name: &str, w: i32, h: i32, x: i32, y: i32) -> Out {
    Out {
        name: name.to_owned(),
        size: (w, h),
        pos: Some((x, y)),
        primary: false,
        scale: 1.0,
    }
}

/// A connected output that is off.
pub fn off(name: &str, w: i32, h: i32) -> Out {
    Out {
        name: name.to_owned(),
        size: (w, h),
        pos: None,
        primary: false,
        scale: 1.0,
    }
}

impl Out {
    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    /// On Wayland, at this scale: the position is in logical pixels, and the mode is still
    /// `size`.
    pub fn scaled(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }
}

fn mode(xid: u32, (w, h, refresh): (i32, i32, f64), preferred: bool) -> Mode {
    Mode {
        id: ModeId::from_xid(xid),
        name: format!("{w}x{h}"),
        width: w,
        height: h,
        refresh,
        interlaced: false,
        double_scan: false,
        preferred,
        custom: false,
    }
}

/// A snapshot with these outputs, all connected, each offering [`MODES`] plus its own size.
/// An enabled output runs its native size at the first listed rate.
pub fn desk(outs: &[Out]) -> Snapshot {
    let mut outputs = Vec::new();
    for (k, spec) in outs.iter().enumerate() {
        let (w, h) = spec.size;
        let mut modes: Vec<Mode> = MODES
            .iter()
            .enumerate()
            .map(|(m, &t)| mode(0x100 + m as u32, t, false))
            .collect();
        if !modes.iter().any(|m| m.width == w && m.height == h) {
            modes.insert(0, mode(0x200 + k as u32, (w, h, 60.0), false));
        }
        let native = modes
            .iter()
            .position(|m| m.width == w && m.height == h)
            .expect("native mode");
        modes[native].preferred = true;
        let active = spec.pos.map(|(x, y)| ActiveConfig {
            mode: modes[native].id,
            pos: Point::new(x, y),
            size: Size::new(w, h),
            rotation: Rotation::Normal,
            reflection: Reflection::Normal,
            scaling: Scaling::X11(Transform::identity()),
            panning: None,
        });
        outputs.push(Output {
            name: spec.name.clone(),
            connection: Connection::Connected,
            primary: spec.primary,
            modes,
            edid: None,
            identity: None,
            physical_mm: None,
            crtc: spec.pos.map(|_| k as u32),
            crtcs: (0..8).collect(),
            active,
            description: None,
            adaptive_sync: None,
        });
    }
    let current = outputs
        .iter()
        .filter_map(|o| o.active.as_ref())
        .fold(Size::default(), |s, a| {
            Size::new(s.w.max(a.pos.x + a.size.w), s.h.max(a.pos.y + a.size.h))
        });
    Snapshot {
        screen: Some(ScreenLimits {
            min: Size::new(320, 200),
            current,
            max: Size::new(16384, 16384),
        }),
        outputs,
        caps: Caps::x11(),
    }
}

/// A Wayland snapshot with these heads, in this order: the same modes as [`desk`] with Wayland
/// ids, logical scaling, no CRTCs, no screen limits and no primary.
pub fn desk_wl(outs: &[Out]) -> Snapshot {
    let mut outputs = Vec::new();
    for spec in outs {
        let (w, h) = spec.size;
        let wl = |(w, h, refresh): (i32, i32, f64), preferred: bool| {
            outlay::wayland::capture::mode(
                w as u16,
                h as u16,
                (refresh * 1000.0).round() as u32,
                preferred,
                false,
            )
        };
        let mut modes: Vec<Mode> = MODES.iter().map(|&t| wl(t, false)).collect();
        if !modes.iter().any(|m| m.width == w && m.height == h) {
            modes.insert(0, wl((w, h, 60.0), false));
        }
        let native = modes
            .iter()
            .position(|m| m.width == w && m.height == h)
            .expect("native mode");
        modes[native].preferred = true;
        let scaling = Scaling::Logical(spec.scale);
        let active = spec.pos.map(|(x, y)| ActiveConfig {
            mode: modes[native].id,
            pos: Point::new(x, y),
            size: effective_size(Size::new(w, h), Rotation::Normal, &scaling),
            rotation: Rotation::Normal,
            reflection: Reflection::Normal,
            scaling,
            panning: None,
        });
        outputs.push(Output {
            name: spec.name.clone(),
            connection: Connection::Connected,
            primary: false,
            modes,
            edid: None,
            identity: None,
            physical_mm: None,
            crtc: None,
            crtcs: Vec::new(),
            active,
            description: None,
            adaptive_sync: None,
        });
    }
    Snapshot {
        screen: None,
        outputs,
        caps: Caps::wayland(),
    }
}

pub fn load_wl(outs: &[Out]) -> (Snapshot, Layout) {
    let snap = desk_wl(outs);
    let layout = Layout::inferred(&snap);
    (snap, layout)
}

/// `snap` with output `name` unplugged: disconnected, with no modes and no EDID. An output that
/// was on stays active, as xrandr leaves it until something turns it off.
pub fn unplugged(snap: &Snapshot, name: &str) -> Snapshot {
    let mut snap = snap.clone();
    let i = snap
        .find(name)
        .unwrap_or_else(|| panic!("no output {name}"));
    let out = &mut snap.outputs[i];
    out.connection = Connection::Disconnected;
    out.modes.clear();
    out.edid = None;
    out.identity = None;
    out.physical_mm = None;
    snap
}

pub fn load(outs: &[Out]) -> (Snapshot, Layout) {
    let snap = desk(outs);
    let layout = Layout::inferred(&snap);
    (snap, layout)
}

/// Index of the output called `name`.
pub fn ix(layout: &Layout, name: &str) -> usize {
    layout
        .names
        .iter()
        .position(|n| n == name)
        .unwrap_or_else(|| panic!("no output {name}"))
}

pub fn rect(layout: &Layout, name: &str) -> Rect {
    layout.rect(ix(layout, name))
}

/// `Some((parent name, side, align, offset))`, or `None` for an anchor.
pub fn link(layout: &Layout, name: &str) -> Option<(String, Side, Align, i32)> {
    layout.links[ix(layout, name)]
        .map(|l: Link| (layout.names[l.parent].clone(), l.side, l.align, l.offset))
}

pub fn stuck(
    parent: &str,
    side: Side,
    align: Align,
    offset: i32,
) -> Option<(String, Side, Align, i32)> {
    Some((parent.to_owned(), side, align, offset))
}

/// Key events from a script: plain characters, plus `<Enter>`, `<Esc>`, `<Tab>`, `<S-Tab>`,
/// `<BS>`, arrows (`<Left>`, `<S-Up>` …), `<A-h>` (Alt) and `<C-r>` (Ctrl). `<lt>` types `<`.
pub fn keys(script: &str) -> Vec<ratatui::crossterm::event::KeyEvent> {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut out = Vec::new();
    let mut rest = script;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(end) = rest.find('>')
        {
            let name = &rest[1..end];
            rest = &rest[end + 1..];
            let (mods, base) = match name.split_once('-') {
                Some(("A", b)) => (KeyModifiers::ALT, b),
                Some(("C", b)) => (KeyModifiers::CONTROL, b),
                Some(("S", b)) => (KeyModifiers::SHIFT, b),
                _ => (KeyModifiers::NONE, name),
            };
            let code = match base {
                "Enter" => KeyCode::Enter,
                "Esc" => KeyCode::Esc,
                "Tab" if mods == KeyModifiers::SHIFT => KeyCode::BackTab,
                "Tab" => KeyCode::Tab,
                "BS" => KeyCode::Backspace,
                "Left" => KeyCode::Left,
                "Right" => KeyCode::Right,
                "Up" => KeyCode::Up,
                "Down" => KeyCode::Down,
                "lt" => KeyCode::Char('<'),
                b if b.chars().count() == 1 => KeyCode::Char(b.chars().next().unwrap()),
                other => panic!("unknown key <{other}>"),
            };
            out.push(KeyEvent::new(code, mods));
        } else {
            rest = &rest[c.len_utf8()..];
            out.push(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }
    out
}
