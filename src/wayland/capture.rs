//! The Wayland capture: the JSON `wlr-randr --json` prints, read into a [`Snapshot`] and written
//! back. `--from-file` and `--demo=wayland` read it, `outlay dump` prints it, and the Wayland
//! `revert.sh` carries one.
//!
//! The format is an array of heads, as `print_state_json` in wlr-randr's `main.c` writes it:
//! `name`, `description`, `make`, `model`, `serial`, `physical_size`, `enabled` and `modes`
//! (`width`, `height`, `refresh` in Hz, `preferred`, `current`); an enabled head also has
//! `position`, `transform`, `scale` and `adaptive_sync`. Unknown keys are ignored and missing
//! optional ones tolerated. outlay adds `custom` to a mode the head does not advertise. An enabled
//! head with no `"current": true` mode (Hyprland, on a mode outside its list) is read as running
//! the preferred size at rate 0, a mode marked `assumed`; `write` leaves that mode out again.

use std::fmt::Write as _;

use serde::Deserialize;
use thiserror::Error;

use super::{head_order, parse_transform, transform_name};
use crate::model::geometry::{Point, Size, effective_size};
use crate::model::{
    ActiveConfig, Caps, Connection, Identity, Mode, ModeId, Output, Reflection, Rotation, Scaling,
    Snapshot,
};

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("not a wlr-randr --json capture: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0} is enabled, but none of its modes is current")]
    NoCurrentMode(String),
    #[error("{0} has a mode of {1}x{2}, which is not a size")]
    BadSize(String, i32, i32),
    #[error("{0} has a mode at {1} Hz, which is not a rate")]
    BadRate(String, f64),
    #[error(
        "{0} has the transform {1:?}, which is not one of normal, 90, 180, 270, flipped, flipped-90, flipped-180, flipped-270"
    )]
    BadTransform(String, String),
    #[error("{0} has the scale {1}, which is not above 0")]
    BadScale(String, f64),
}

/// Whether `text` looks like a Wayland capture rather than `xrandr --verbose`: its first
/// character other than white space opens a JSON array or object.
pub fn is_capture(text: &str) -> bool {
    matches!(text.trim_start().chars().next(), Some('[' | '{'))
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Heads {
    Many(Vec<Head>),
    One(Box<Head>),
}

#[derive(Deserialize)]
struct Head {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    make: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    serial: Option<String>,
    #[serde(default)]
    physical_size: Option<Dimensions>,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    modes: Vec<HeadMode>,
    #[serde(default)]
    position: Option<Position>,
    #[serde(default)]
    transform: Option<String>,
    #[serde(default)]
    scale: Option<f64>,
    #[serde(default)]
    adaptive_sync: Option<bool>,
}

#[derive(Deserialize)]
struct Dimensions {
    width: i32,
    height: i32,
}

#[derive(Deserialize)]
struct Position {
    x: i32,
    y: i32,
}

#[derive(Deserialize)]
struct HeadMode {
    width: i32,
    height: i32,
    #[serde(default)]
    refresh: f64,
    #[serde(default)]
    preferred: bool,
    #[serde(default)]
    current: bool,
    #[serde(default)]
    custom: Option<bool>,
}

/// What the compositor says about one head, from a capture or from the protocol.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeadReport {
    pub name: String,
    pub description: Option<String>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub physical_size: Option<Size>,
    pub enabled: bool,
    pub modes: Vec<ModeReport>,
    /// Position, transform and scale mean something only while the head is enabled.
    pub position: Point,
    pub rotation: Rotation,
    pub reflection: Reflection,
    pub scale: f64,
    pub adaptive_sync: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModeReport {
    pub width: i32,
    pub height: i32,
    pub millihertz: u32,
    pub preferred: bool,
    pub current: bool,
    /// Whether the head does not advertise the mode; unknown when `None`, and then a rate of 0
    /// (which no real mode has) means custom.
    pub custom: Option<bool>,
}

/// Reads a capture. Heads come out in [`head_order`], so display numbers do not depend on the
/// order the compositor listed them in.
pub fn parse(text: &str) -> Result<Snapshot, CaptureError> {
    let heads = match serde_json::from_str(text)? {
        Heads::Many(heads) => heads,
        Heads::One(head) => vec![*head],
    };
    let outputs = heads
        .into_iter()
        .map(|head| output(report(head)?))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(snapshot(outputs))
}

/// The Wayland snapshot of these outputs, in [`head_order`], without Hyprland's stand-in
/// ([`is_fallback`]).
pub fn snapshot(mut outputs: Vec<Output>) -> Snapshot {
    outputs.retain(|o| !is_fallback(o));
    outputs.sort_by(|a, b| head_order(&a.name, &b.name));
    Snapshot {
        screen: None,
        outputs,
        caps: Caps::wayland(),
    }
}

/// Hyprland's stand-in while no monitor is on: a headless head named `FALLBACK`, enabled, with
/// no make, model or serial. After an undock that leaves the laptop panel off, it is the only
/// enabled head, and it would count as an external display. It is not one, so a snapshot leaves
/// it out. An apply still sends it back as it is (a head outside the plan), and Hyprland drops
/// it once a real output is on.
fn is_fallback(o: &Output) -> bool {
    o.name == "FALLBACK" && o.identity.is_none()
}

fn report(head: Head) -> Result<HeadReport, CaptureError> {
    let name = head.name;
    let (rotation, reflection) = match head.transform.as_deref() {
        None => Default::default(),
        Some(t) => parse_transform(t)
            .ok_or_else(|| CaptureError::BadTransform(name.clone(), t.to_owned()))?,
    };
    let modes = head
        .modes
        .iter()
        .map(|m| {
            Ok(ModeReport {
                width: m.width,
                height: m.height,
                millihertz: millihertz(&name, m.refresh)?,
                preferred: m.preferred,
                current: m.current,
                custom: m.custom,
            })
        })
        .collect::<Result<_, CaptureError>>()?;
    Ok(HeadReport {
        description: head.description,
        make: head.make,
        model: head.model,
        serial: head.serial,
        physical_size: head.physical_size.map(|d| Size::new(d.width, d.height)),
        enabled: head.enabled,
        modes,
        position: head
            .position
            .map_or_else(Point::default, |p| Point::new(p.x, p.y)),
        rotation,
        reflection,
        scale: head.scale.unwrap_or(1.0),
        adaptive_sync: head.adaptive_sync,
        name,
    })
}

/// A rate in Hz as the protocol's mHz: `round(Hz × 1000)`, as wlr-randr reads `--mode`.
fn millihertz(name: &str, hz: f64) -> Result<u32, CaptureError> {
    let mhz = (hz * 1000.0).round();
    if !(0.0..=f64::from(u32::MAX)).contains(&mhz) {
        return Err(CaptureError::BadRate(name.to_owned(), hz));
    }
    Ok(mhz as u32)
}

/// A mode of a Wayland head, named by its size so `find_mode` works on it.
pub fn mode(width: u16, height: u16, millihertz: u32, preferred: bool, custom: bool) -> Mode {
    Mode::wayland(width, height, millihertz, preferred, custom)
}

/// The rate of a Wayland mode in mHz.
pub fn mode_millihertz(mode: &Mode) -> u32 {
    (mode.id.0 & 0xFFFF_FFFF) as u32
}

/// What the display says it is: make and model as the compositor reports them. The label is
/// the model alone when it already names the maker (`DELL U2719D` from `Dell Inc.`).
pub fn identity(make: Option<&str>, model: Option<&str>, serial: Option<&str>) -> Option<Identity> {
    let owned = |s: Option<&str>| s.map(str::to_owned);
    if make.is_none() && model.is_none() && serial.is_none() {
        return None;
    }
    let label = match (make, model) {
        (Some(make), Some(model)) => {
            let maker = make
                .split_whitespace()
                .next()
                .unwrap_or(make)
                .to_lowercase();
            if model.to_lowercase().contains(&maker) {
                model.to_owned()
            } else {
                format!("{make} {model}")
            }
        }
        (Some(one), None) | (None, Some(one)) => one.to_owned(),
        (None, None) => String::new(),
    };
    Some(Identity {
        make: owned(make),
        model: owned(model),
        serial: owned(serial),
        label,
    })
}

/// The mode an enabled head is taken to run when the compositor names none as current. Hyprland
/// names none for a mode the head does not advertise. The size is the preferred mode's, else
/// the first one's; the rate is 0, unknown.
// shortcut: a custom mode of another size is drawn at the preferred size; wl_output's current mode has the real one.
pub fn assumed_mode(head: &HeadReport) -> Option<ModeReport> {
    if !head.enabled || head.modes.iter().any(|m| m.current) {
        return None;
    }
    let sized = || head.modes.iter().filter(|m| m.width > 0 && m.height > 0);
    let base = sized().find(|m| m.preferred).or_else(|| sized().next())?;
    Some(ModeReport {
        millihertz: 0,
        preferred: false,
        current: true,
        custom: Some(true),
        ..*base
    })
}

/// The output a head report describes. Modes are merged by size and rate; a duplicate the head
/// runs keeps its own `custom`. A mode of no size is left out: wlroots 0.17 gives a client that
/// binds while a custom-mode head is off a virtual mode whose size it never sends. An enabled
/// head with no current mode runs the [`assumed_mode`], marked `assumed`.
pub fn output(mut head: HeadReport) -> Result<Output, CaptureError> {
    let assumed = assumed_mode(&head);
    let assumed_id = assumed.as_ref().and_then(|m| {
        let (w, h) = (u16::try_from(m.width).ok()?, u16::try_from(m.height).ok()?);
        Some(ModeId::wayland(w, h, m.millihertz))
    });
    head.modes.extend(assumed);
    let name = head.name;
    let mut modes: Vec<Mode> = Vec::new();
    let mut current = None;
    for m in &head.modes {
        let (Ok(w), Ok(h)) = (u16::try_from(m.width), u16::try_from(m.height)) else {
            return Err(CaptureError::BadSize(name, m.width, m.height));
        };
        if w == 0 && h == 0 {
            continue;
        }
        if w == 0 || h == 0 {
            return Err(CaptureError::BadSize(name, m.width, m.height));
        }
        let mhz = m.millihertz;
        let custom = m.custom.unwrap_or(mhz == 0);
        let new = mode(w, h, mhz, m.preferred, custom);
        let id = new.id;
        match modes.iter_mut().find(|k| k.id == id) {
            Some(known) => {
                known.preferred |= m.preferred;
                if m.current {
                    known.custom = custom;
                }
            }
            None => modes.push(new),
        }
        if m.current && current.is_none() {
            current = Some(id);
        }
    }
    if let Some(m) = modes.iter_mut().find(|m| Some(m.id) == assumed_id) {
        m.assumed = true;
    }
    let active = if head.enabled {
        let Some(id) = current else {
            return Err(CaptureError::NoCurrentMode(name));
        };
        let size = modes.iter().find(|m| m.id == id).expect("listed").size();
        let (rotation, reflection) = (head.rotation, head.reflection);
        let scale = head.scale;
        if !(scale.is_finite() && scale > 0.0) {
            return Err(CaptureError::BadScale(name, scale));
        }
        let scaling = Scaling::Logical(scale);
        Some(ActiveConfig {
            mode: id,
            pos: head.position,
            size: effective_size(size, rotation, &scaling),
            rotation,
            reflection,
            scaling,
            panning: None,
        })
    } else {
        None
    };
    Ok(Output {
        identity: identity(
            head.make.as_deref(),
            head.model.as_deref(),
            head.serial.as_deref(),
        ),
        name,
        connection: Connection::Connected,
        primary: false,
        modes,
        edid: None,
        physical_mm: head.physical_size.filter(|s| s.w > 0 && s.h > 0),
        crtc: None,
        crtcs: Vec::new(),
        active,
        description: head.description,
        adaptive_sync: head.adaptive_sync,
    })
}

/// A number the way wlr-randr prints it (`%f`), unless six decimals would change it: scales are
/// multiples of 1/256, which can need eight.
fn number(v: f64) -> String {
    let fixed = format!("{v:.6}");
    if fixed.parse::<f64>().ok() == Some(v) {
        fixed
    } else {
        format!("{v}")
    }
}

fn string(s: Option<&str>) -> String {
    s.map_or_else(
        || "null".to_owned(),
        |s| serde_json::to_string(s).expect("a string serialises"),
    )
}

/// Writes a snapshot as a capture, laid out the way `wlr-randr --json` lays it out. Only what a
/// Wayland head has is written; X11-only data (EDID, CRTCs, panning) is left out.
pub fn write(snap: &Snapshot) -> String {
    let mut out = String::from("[");
    for (k, o) in snap.outputs.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        let id = o.identity.as_ref();
        let field = |f: fn(&Identity) -> &Option<String>| id.and_then(|i| f(i).as_deref());
        let mm = o.physical_mm.unwrap_or_default();
        let _ = write!(
            out,
            "\n  {{\n    \"name\": {},\n    \"description\": {},\n    \"make\": {},\n    \
             \"model\": {},\n    \"serial\": {},\n    \"physical_size\": {{\n      \
             \"width\": {},\n      \"height\": {}\n    }},\n    \"enabled\": {},\n    \
             \"modes\": [",
            string(Some(&o.name)),
            string(o.description.as_deref()),
            string(field(|i| &i.make)),
            string(field(|i| &i.model)),
            string(field(|i| &i.serial)),
            mm.w,
            mm.h,
            o.active.is_some(),
        );
        let current = o.active.as_ref().map(|a| a.mode);
        // An assumed mode is left out, as the compositor leaves it out; `parse` assumes it again.
        let modes = || o.modes.iter().filter(|m| !m.assumed);
        for (j, m) in modes().enumerate() {
            if j > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "\n      {{\n        \"width\": {},\n        \"height\": {},\n        \
                 \"refresh\": {},\n        \"preferred\": {},\n        \"current\": {}",
                m.width,
                m.height,
                number(m.refresh),
                m.preferred,
                current == Some(m.id),
            );
            if m.custom {
                out.push_str(",\n        \"custom\": true");
            }
            out.push_str("\n      }");
        }
        if modes().next().is_some() {
            out.push_str("\n    ");
        }
        out.push(']');
        match &o.active {
            None => out.push('\n'),
            Some(a) => {
                let scale = a.scaling.factor().unwrap_or(1.0);
                let sync = o
                    .adaptive_sync
                    .map_or_else(|| "null".to_owned(), |s| s.to_string());
                let _ = write!(
                    out,
                    ",\n    \"position\": {{\n      \"x\": {},\n      \"y\": {}\n    }},\n    \
                     \"transform\": \"{}\",\n    \"scale\": {},\n    \"adaptive_sync\": {}\n",
                    a.pos.x,
                    a.pos.y,
                    transform_name(a.rotation, a.reflection),
                    number(scale),
                    sync,
                );
            }
        }
        out.push_str("  }");
    }
    out.push_str("\n]\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ModeId, Reflection, Rotation};

    const HEADLESS: &str = r#"[
  {
    "name": "HEADLESS-2",
    "description": "Headless output 1",
    "make": null,
    "model": null,
    "serial": null,
    "physical_size": {
      "width": 0,
      "height": 0
    },
    "enabled": true,
    "modes": [
      {
        "width": 1280,
        "height": 720,
        "refresh": 0.000000,
        "preferred": false,
        "current": true
      }
    ],
    "position": {
      "x": 0,
      "y": 0
    },
    "transform": "normal",
    "scale": 1.000000,
    "adaptive_sync": false
  },
  {
    "name": "HEADLESS-1",
    "description": "Headless output 2",
    "make": null,
    "model": null,
    "serial": null,
    "physical_size": {
      "width": 0,
      "height": 0
    },
    "enabled": true,
    "modes": [
      {
        "width": 1280,
        "height": 720,
        "refresh": 0.000000,
        "preferred": false,
        "current": true
      }
    ],
    "position": {
      "x": 1280,
      "y": 0
    },
    "transform": "270",
    "scale": 1.750000,
    "adaptive_sync": false
  }
]
"#;

    #[test]
    fn reads_what_wlr_randr_prints() {
        assert!(is_capture(HEADLESS));
        assert!(is_capture(" \n{\"name\": \"X\"}"));
        assert!(!is_capture("Screen 0: minimum 320 x 200"));
        let snap = parse(HEADLESS).unwrap();
        assert_eq!(snap.caps, Caps::wayland());
        assert_eq!(snap.screen, None);
        let names: Vec<&str> = snap.outputs.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["HEADLESS-1", "HEADLESS-2"], "sorted, not as listed");
        let one = &snap.outputs[0];
        assert_eq!(one.identity, None);
        assert_eq!(one.description.as_deref(), Some("Headless output 2"));
        assert_eq!(one.physical_mm, None);
        assert_eq!(one.adaptive_sync, Some(false));
        assert!(one.modes[0].custom, "a rate of 0 is no real mode");
        assert_eq!(one.modes[0].id, ModeId::wayland(1280, 720, 0));
        let a = one.active.as_ref().unwrap();
        assert_eq!(
            (a.rotation, a.reflection),
            (Rotation::Right, Reflection::Normal)
        );
        assert_eq!(a.scaling, Scaling::Logical(1.75));
        assert_eq!(a.size, Size::new(411, 731), "720x1280 / 1.75, truncated");
        assert_eq!(a.pos, Point::new(1280, 0));
    }

    #[test]
    fn writes_what_it_reads() {
        let snap = parse(HEADLESS).unwrap();
        let modes = || snap.outputs.iter().flat_map(|o| &o.modes);
        assert!(modes().any(|m| m.custom && m.refresh == 0.0));
        assert!(
            modes().all(|m| !m.assumed),
            "a custom mode named current is not assumed"
        );
        let text = write(&snap);
        assert_eq!(parse(&text).unwrap(), snap);
        assert!(text.contains("\"refresh\": 0.000000,"), "{text}");
        assert!(text.contains("\"custom\": true"), "{text}");
        assert!(text.contains("\"scale\": 1.750000,"), "{text}");
        // A scale six decimals cannot hold keeps all of its digits.
        assert_eq!(number(341.0 / 256.0), "1.33203125");
        assert_eq!(number(1.25), "1.250000");
    }

    #[test]
    fn modes_are_merged_by_size_and_rate() {
        let text = r#"[{"name": "DP-1", "enabled": true, "extra": 1, "modes": [
            {"width": 1920, "height": 1080, "refresh": 60.000000, "preferred": true},
            {"width": 1920, "height": 1080, "refresh": 59.999998, "current": true, "custom": true},
            {"width": 1280, "height": 720, "refresh": 60.0}]}]"#;
        let snap = parse(text).unwrap();
        let out = &snap.outputs[0];
        assert_eq!(out.modes.len(), 2);
        assert!(out.modes[0].preferred && out.modes[0].custom);
        let a = out.active.as_ref().unwrap();
        assert_eq!(a.mode, ModeId::wayland(1920, 1080, 60_000));
        assert_eq!(a.scaling, Scaling::Logical(1.0), "no scale means 1");
        assert_eq!(a.pos, Point::default());
        assert_eq!(mode_millihertz(&out.modes[0]), 60_000);

        let off = r#"[{"name": "HEADLESS-2", "enabled": false,
            "modes": [{"width": 0, "height": 0, "refresh": 0}]}]"#;
        assert!(
            parse(off).unwrap().outputs[0].modes.is_empty(),
            "a mode wlroots 0.17 never sized"
        );
    }

    #[test]
    fn a_head_with_no_current_mode_runs_the_preferred_size_at_rate_0() {
        let text = |preferred: &str, enabled: bool| {
            format!(
                r#"[{{"name":"eDP-1","enabled":{enabled},"scale":1.5,"transform":"90","modes":[
                {{"width":1280,"height":720,"refresh":60.0}},
                {{"width":1920,"height":1080,"refresh":165.009995,"preferred":{preferred}}}]}}]"#
            )
        };
        let snap = parse(&text("true", true)).unwrap();
        let out = &snap.outputs[0];
        assert_eq!(out.modes.len(), 3);
        let active = out.active.as_ref().unwrap();
        assert_eq!(active.mode, ModeId::wayland(1920, 1080, 0));
        let current = out.current_mode().unwrap();
        assert!(current.custom && !current.preferred && current.assumed);
        assert_eq!(
            out.modes.iter().filter(|m| m.assumed).count(),
            1,
            "only the added mode is assumed"
        );
        assert_eq!(active.size, Size::new(720, 1280));
        assert_eq!(out.preferred_mode().unwrap().refresh, 165.01);
        let written = write(&snap);
        assert_eq!(
            parse(&written).unwrap(),
            snap,
            "parse assumes the mode again"
        );
        assert!(!written.contains("\"current\": true"), "{written}");
        assert!(!written.contains("\"custom\": true"), "{written}");
        assert_eq!(
            written.matches("\"width\": ").count(),
            1 + 2,
            "physical_size and the two listed modes: {written}"
        );

        let first = parse(&text("false", true)).unwrap();
        assert_eq!(
            first.outputs[0].active.as_ref().unwrap().mode,
            ModeId::wayland(1280, 720, 0),
            "no preferred mode: the first one"
        );

        let off = parse(&text("true", false)).unwrap();
        assert_eq!(off.outputs[0].modes.len(), 2);
        assert!(off.outputs[0].active.is_none());
    }

    #[test]
    fn explains_a_bad_capture() {
        let err = |text: &str| parse(text).unwrap_err().to_string();
        assert!(err("[").starts_with("not a wlr-randr --json capture"));
        assert_eq!(
            err(r#"[{"name": "DP-1", "enabled": true, "modes": []}]"#),
            "DP-1 is enabled, but none of its modes is current"
        );
        assert!(
            err(r#"{"name": "DP-1", "enabled": true, "transform": "left",
                "modes": [{"width": 8, "height": 8, "current": true}]}"#)
            .contains("\"left\", which is not one of")
        );
        assert_eq!(
            err(r#"[{"name": "DP-1", "modes": [{"width": 0, "height": 8}]}]"#),
            "DP-1 has a mode of 0x8, which is not a size"
        );
        assert_eq!(
            err(r#"[{"name": "DP-1", "enabled": true, "scale": 0,
                "modes": [{"width": 8, "height": 8, "current": true}]}]"#),
            "DP-1 has the scale 0, which is not above 0"
        );
    }

    #[test]
    fn labels_name_the_maker_once() {
        let label = |make, model| identity(make, model, None).unwrap().label;
        assert_eq!(label(Some("Dell Inc."), Some("DELL U2719D")), "DELL U2719D");
        assert_eq!(label(Some("BOE"), Some("0x0BCA")), "BOE 0x0BCA");
        assert_eq!(label(None, Some("0x0BCA")), "0x0BCA");
        assert_eq!(identity(None, None, None), None);
        assert_eq!(identity(None, None, Some("7LQ2M43")).unwrap().label, "");
    }
}
