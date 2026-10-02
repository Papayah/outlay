//! `xrandr --verbose` output → [`Snapshot`].
//!
//! The format has four kinds of lines:
//! - `Screen 0: minimum W x H, current W x H, maximum W x H`;
//! - an output header, which starts in column 0;
//! - property lines: one tab and a non-space character start a key, anything else that starts
//!   with a tab continues the previous key (`Transform:` rows, `EDID:` hex, `supported:` lists);
//! - mode lines, which start with two spaces, each followed by indented `h:` and `v:` lines.

use thiserror::Error;

use crate::model::edid::{Edid, decode_hex};
use crate::model::geometry::{Point, Rect, Size};
use crate::model::{
    ActiveConfig, Caps, Connection, Mode, ModeId, Output, Reflection, Rotation, Scaling,
    ScreenLimits, Snapshot, Transform,
};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("no `Screen` line found; is this `xrandr --verbose` output?")]
    NoScreen,
    #[error("line {line}: {message}")]
    Line { line: usize, message: String },
}

fn line_error(line: usize, message: impl Into<String>) -> ParseError {
    ParseError::Line {
        line,
        message: message.into(),
    }
}

pub fn parse_verbose(text: &str) -> Result<Snapshot, ParseError> {
    let mut screen: Option<ScreenLimits> = None;
    let mut outputs: Vec<OutputBuilder> = Vec::new();

    for (index, raw) in text.lines().enumerate() {
        let lineno = index + 1;
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("Screen ") {
            if screen.is_some() {
                // A second X screen (Zaphod mode) is out of scope; keep the first one.
                break;
            }
            screen = Some(
                parse_screen(rest).ok_or_else(|| line_error(lineno, "malformed `Screen` line"))?,
            );
            continue;
        }
        if screen.is_none() {
            return Err(ParseError::NoScreen);
        }
        if !line.starts_with(char::is_whitespace) {
            if let Some(last) = outputs.last_mut() {
                last.finish_mode(lineno)?;
            }
            outputs.push(OutputBuilder::from_header(line, lineno)?);
            continue;
        }
        let Some(current) = outputs.last_mut() else {
            return Err(line_error(lineno, "indented line before the first output"));
        };
        if let Some(prop) = line.strip_prefix('\t') {
            current.property_line(prop, lineno)?;
        } else {
            current.mode_line(line, lineno)?;
        }
    }

    let screen = screen.ok_or(ParseError::NoScreen)?;
    let outputs = outputs
        .into_iter()
        .map(|b| b.build(text.lines().count()))
        .collect::<Result<_, _>>()?;
    Ok(Snapshot {
        screen: Some(screen),
        outputs,
        caps: Caps::x11(),
    })
}

/// `0: minimum 320 x 200, current 3840 x 1080, maximum 16384 x 16384`
fn parse_screen(rest: &str) -> Option<ScreenLimits> {
    let (_, fields) = rest.split_once(':')?;
    let mut limits = ScreenLimits::default();
    for field in fields.split(',') {
        let mut words = field.split_whitespace();
        let key = words.next()?;
        let w = words.next()?.parse().ok()?;
        (words.next()? == "x").then_some(())?;
        let h = words.next()?.parse().ok()?;
        let size = Size::new(w, h);
        match key {
            "minimum" => limits.min = size,
            "current" => limits.current = size,
            "maximum" => limits.max = size,
            _ => {}
        }
    }
    Some(limits)
}

/// Splits on whitespace, keeping a parenthesised group such as
/// `(normal left inverted right x axis y axis)` as one token.
fn tokenize(line: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start: Option<usize> = None;
    let mut depth = 0usize;
    for (i, c) in line.char_indices() {
        match c {
            '(' => {
                if start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            ')' => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && depth == 0 => {
                if let Some(s) = start.take() {
                    tokens.push(&line[s..i]);
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
    }
    if let Some(s) = start {
        tokens.push(&line[s..]);
    }
    tokens
}

/// `WxH+X+Y`
fn parse_geometry(token: &str) -> Option<Rect> {
    let (size, pos) = token.split_once('+')?;
    let (w, h) = size.split_once('x')?;
    let (x, y) = pos.split_once('+')?;
    Some(Rect::new(
        x.parse().ok()?,
        y.parse().ok()?,
        w.parse().ok()?,
        h.parse().ok()?,
    ))
}

/// `(0x4a)`
fn parse_xid_token(token: &str) -> Option<u32> {
    let inner = token.strip_prefix("(0x")?.strip_suffix(')')?;
    u32::from_str_radix(inner, 16).ok()
}

struct Header {
    xid: u32,
    rect: Rect,
    rotation: Rotation,
    reflection: Reflection,
}

struct OutputBuilder {
    output: Output,
    header: Option<Header>,
    header_mm: Option<Size>,
    key: Key,
    edid_hex: String,
    transform_rows: Vec<String>,
    filter: String,
    panning: Option<Rect>,
    mode: Option<ModeBuilder>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    None,
    Edid,
    Transform,
    Other,
}

impl OutputBuilder {
    fn from_header(line: &str, lineno: usize) -> Result<Self, ParseError> {
        let tokens = tokenize(line);
        let err = |message: &str| line_error(lineno, format!("output header: {message}"));
        let name = tokens.first().ok_or_else(|| err("empty"))?.to_string();
        let mut i = 1;
        let connection = match tokens.get(i).copied() {
            Some("connected") => Connection::Connected,
            Some("disconnected") => Connection::Disconnected,
            Some("unknown") if tokens.get(i + 1) == Some(&"connection") => {
                i += 1;
                Connection::Unknown
            }
            _ => return Err(err("expected a connection state after the name")),
        };
        i += 1;
        let primary = tokens.get(i) == Some(&"primary");
        if primary {
            i += 1;
        }

        let mut header = None;
        if let Some(rect) = tokens.get(i).and_then(|t| parse_geometry(t)) {
            i += 1;
            let xid = tokens
                .get(i)
                .and_then(|t| parse_xid_token(t))
                .ok_or_else(|| {
                    err("active output without a mode XID; outlay needs `xrandr --verbose`")
                })?;
            i += 1;
            let rotation = match tokens.get(i).and_then(|t| Rotation::parse(t)) {
                Some(r) => {
                    i += 1;
                    r
                }
                None => Rotation::Normal,
            };
            let reflection = match tokens.get(i..).unwrap_or_default() {
                ["X", "and", "Y", "axis", ..] => {
                    i += 4;
                    Reflection::XY
                }
                ["X", "axis", ..] => {
                    i += 2;
                    Reflection::X
                }
                ["Y", "axis", ..] => {
                    i += 2;
                    Reflection::Y
                }
                _ => Reflection::Normal,
            };
            header = Some(Header {
                xid,
                rect,
                rotation,
                reflection,
            });
        }
        if tokens.get(i).is_some_and(|t| t.starts_with('(')) {
            i += 1;
        }
        let header_mm = match tokens.get(i..).unwrap_or_default() {
            [w, "x", h, ..] => parse_mm(w)
                .zip(parse_mm(h))
                .map(|(w, h)| Size::new(w, h))
                .filter(|s| s.w > 0 && s.h > 0),
            _ => None,
        };

        Ok(Self {
            output: Output {
                name,
                connection,
                primary,
                modes: Vec::new(),
                edid: None,
                identity: None,
                physical_mm: None,
                crtc: None,
                crtcs: Vec::new(),
                active: None,
            },
            header,
            header_mm,
            key: Key::None,
            edid_hex: String::new(),
            transform_rows: Vec::new(),
            filter: String::new(),
            panning: None,
            mode: None,
        })
    }

    /// A line that started with a tab, with that tab removed.
    fn property_line(&mut self, prop: &str, lineno: usize) -> Result<(), ParseError> {
        self.finish_mode(lineno)?;
        let starts_key = prop.chars().next().is_some_and(|c| !c.is_whitespace());
        if !starts_key {
            let value = prop.trim();
            match self.key {
                Key::Edid => self.edid_hex.push_str(value),
                Key::Transform => match value.strip_prefix("filter:") {
                    Some(filter) => self.filter = filter.trim().to_owned(),
                    None => self.transform_rows.push(value.to_owned()),
                },
                Key::None | Key::Other => {}
            }
            return Ok(());
        }
        let (key, value) = prop.split_once(':').unwrap_or((prop, ""));
        let value = value.trim();
        self.key = Key::Other;
        match key {
            "EDID" => {
                self.key = Key::Edid;
                self.edid_hex.push_str(value);
            }
            "Transform" => {
                self.key = Key::Transform;
                self.transform_rows.push(value.to_owned());
            }
            "CRTC" => {
                self.output.crtc = Some(
                    value
                        .parse()
                        .map_err(|_| line_error(lineno, "malformed CRTC"))?,
                );
            }
            "CRTCs" => {
                self.output.crtcs = value
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|_| line_error(lineno, "malformed CRTCs"))?;
            }
            "Panning" => {
                self.panning = Some(
                    parse_geometry(value).ok_or_else(|| line_error(lineno, "malformed Panning"))?,
                );
            }
            _ => {}
        }
        Ok(())
    }

    /// A line that started with spaces: a mode, or its `h:`/`v:` timing lines.
    fn mode_line(&mut self, line: &str, lineno: usize) -> Result<(), ParseError> {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("h:") {
            let mode = self
                .mode
                .as_mut()
                .ok_or_else(|| line_error(lineno, "`h:` line without a mode"))?;
            mode.width = field(rest, "width");
            mode.htotal = field(rest, "total");
            return Ok(());
        }
        if let Some(rest) = trimmed.strip_prefix("v:") {
            let mode = self
                .mode
                .as_mut()
                .ok_or_else(|| line_error(lineno, "`v:` line without a mode"))?;
            mode.height = field(rest, "height");
            mode.vtotal = field(rest, "total");
            mode.vclock = field::<String>(rest, "clock")
                .and_then(|c| c.strip_suffix("Hz").and_then(|v| v.parse().ok()));
            return Ok(());
        }
        self.finish_mode(lineno)?;
        self.mode = Some(ModeBuilder::from_line(trimmed, lineno)?);
        Ok(())
    }

    fn finish_mode(&mut self, lineno: usize) -> Result<(), ParseError> {
        if let Some(builder) = self.mode.take() {
            self.output.modes.push(builder.build(lineno)?);
        }
        Ok(())
    }

    fn build(mut self, last_line: usize) -> Result<Output, ParseError> {
        self.finish_mode(last_line)?;
        let mut out = self.output;
        if !self.edid_hex.is_empty() {
            out.edid = decode_hex(&self.edid_hex).and_then(|bytes| Edid::parse(&bytes));
            out.identity = out.edid.as_ref().map(Edid::identity);
        }
        out.physical_mm = self
            .header_mm
            .or_else(|| out.edid.as_ref().and_then(|e| e.size_mm));
        if let Some(h) = self.header {
            let transform = parse_transform(&self.transform_rows, self.filter);
            // Panning equal to the output's own rectangle has no effect, so it does not count.
            let panning = self.panning.filter(|p| *p != h.rect && p.w > 0 && p.h > 0);
            out.active = Some(ActiveConfig {
                mode: ModeId::from_xid(h.xid),
                pos: Point::new(h.rect.x, h.rect.y),
                size: Size::new(h.rect.w, h.rect.h),
                rotation: h.rotation,
                reflection: h.reflection,
                scaling: Scaling::X11(transform),
                panning,
            });
        }
        Ok(out)
    }
}

fn parse_mm(token: &str) -> Option<i32> {
    token.strip_suffix("mm")?.parse().ok()
}

fn parse_transform(rows: &[String], filter: String) -> Transform {
    let values: Vec<f64> = rows
        .iter()
        .flat_map(|r| r.split_whitespace())
        .filter_map(|v| v.parse().ok())
        .collect();
    if values.len() != 9 {
        return Transform::identity();
    }
    let mut matrix = [[0.0; 3]; 3];
    for (k, v) in values.into_iter().enumerate() {
        matrix[k / 3][k % 3] = v;
    }
    Transform { matrix, filter }
}

/// The word after `name` in a timing line such as `h: width  1920 start 1936 … total 2080`.
fn field<T: std::str::FromStr>(rest: &str, name: &str) -> Option<T> {
    let mut words = rest.split_whitespace();
    words.by_ref().find(|w| *w == name)?;
    words.next()?.parse().ok()
}

struct ModeBuilder {
    xid: u32,
    name: String,
    clock_mhz: f64,
    interlaced: bool,
    double_scan: bool,
    preferred: bool,
    width: Option<i32>,
    height: Option<i32>,
    htotal: Option<u32>,
    vtotal: Option<u32>,
    vclock: Option<f64>,
}

impl ModeBuilder {
    /// `1920x1080 (0x4a) 405.000MHz -HSync -VSync *current +preferred`
    fn from_line(line: &str, lineno: usize) -> Result<Self, ParseError> {
        let Some(open) = line.find(" (0x") else {
            return Err(line_error(
                lineno,
                "mode line without an XID; this looks like plain `xrandr` output, outlay needs `xrandr --verbose`",
            ));
        };
        let name = line[..open].trim().to_owned();
        let rest = &line[open + 1..];
        let close = rest
            .find(')')
            .ok_or_else(|| line_error(lineno, "unterminated mode XID"))?;
        let xid = parse_xid_token(&rest[..=close])
            .ok_or_else(|| line_error(lineno, "malformed mode XID"))?;
        let mut words = rest[close + 1..].split_whitespace();
        let clock_mhz = words
            .next()
            .and_then(|c| c.strip_suffix("MHz"))
            .and_then(|c| c.parse().ok())
            .ok_or_else(|| line_error(lineno, "mode line without a dot clock"))?;
        let mut mode = Self {
            xid,
            name,
            clock_mhz,
            interlaced: false,
            double_scan: false,
            preferred: false,
            width: None,
            height: None,
            htotal: None,
            vtotal: None,
            vclock: None,
        };
        for flag in words {
            match flag {
                "Interlace" => mode.interlaced = true,
                "DoubleScan" => mode.double_scan = true,
                "+preferred" => mode.preferred = true,
                _ => {}
            }
        }
        Ok(mode)
    }

    fn build(self, lineno: usize) -> Result<Mode, ParseError> {
        let (Some(width), Some(height)) = (self.width, self.height) else {
            return Err(line_error(
                lineno,
                format!("mode {} is missing its `h:`/`v:` lines", self.name),
            ));
        };
        let refresh = match (self.vclock, self.htotal, self.vtotal) {
            (Some(hz), _, _) => hz,
            (None, Some(ht), Some(vt)) if ht > 0 && vt > 0 => {
                let mut hz = self.clock_mhz * 1e6 / (f64::from(ht) * f64::from(vt));
                if self.interlaced {
                    hz *= 2.0;
                }
                if self.double_scan {
                    hz /= 2.0;
                }
                hz
            }
            _ => 0.0,
        };
        Ok(Mode {
            id: ModeId::from_xid(self.xid),
            name: self.name,
            width,
            height,
            refresh,
            interlaced: self.interlaced,
            double_scan: self.double_scan,
            preferred: self.preferred,
            custom: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: &str =
        "Screen 0: minimum 320 x 200, current 3840 x 1080, maximum 16384 x 16384\n";

    fn parse(body: &str) -> Snapshot {
        parse_verbose(&format!("{SCREEN}{body}")).unwrap()
    }

    #[test]
    fn reads_the_screen_limits() {
        let snap = parse("");
        assert_eq!(snap.screen.unwrap().min, Size::new(320, 200));
        assert_eq!(snap.screen.unwrap().current, Size::new(3840, 1080));
        assert_eq!(snap.screen.unwrap().max, Size::new(16384, 16384));
    }

    #[test]
    fn header_with_rotation_reflection_and_primary() {
        let snap = parse(
            "DP-2.1 connected primary 1440x2560+16+0 (0x1c8) left X and Y axis (normal left inverted right x axis y axis) 597mm x 336mm\n",
        );
        let out = &snap.outputs[0];
        assert_eq!(out.name, "DP-2.1");
        assert!(out.primary);
        assert_eq!(out.physical_mm, Some(Size::new(597, 336)));
        let active = out.active.as_ref().unwrap();
        assert_eq!(active.mode, ModeId(0x1c8));
        assert_eq!(active.rect(), Rect::new(16, 0, 1440, 2560));
        assert_eq!(active.rotation, Rotation::Left);
        assert_eq!(active.reflection, Reflection::XY);
    }

    #[test]
    fn header_variants() {
        let snap = parse(
            "VGA-1 unknown connection 1024x768+0+0 (0x47) normal Y axis (normal left inverted right x axis y axis) 0mm x 0mm\n\
             DP-1 disconnected (normal left inverted right x axis y axis)\n\
             DP-2 disconnected 2560x1440+0+0 (0x1f3) inverted (normal left inverted right x axis y axis) 0mm x 0mm\n",
        );
        let [vga, dp1, dp2] = &snap.outputs[..] else {
            panic!("three outputs")
        };
        assert_eq!(vga.connection, Connection::Unknown);
        assert_eq!(vga.active.as_ref().unwrap().reflection, Reflection::Y);
        assert_eq!(vga.physical_mm, None, "0mm x 0mm means unknown");
        assert_eq!(dp1.connection, Connection::Disconnected);
        assert!(dp1.active.is_none());
        assert!(dp2.is_stale());
        assert_eq!(dp2.active.as_ref().unwrap().rotation, Rotation::Inverted);
    }

    #[test]
    fn plain_xrandr_output_is_refused() {
        let err = parse_verbose(&format!(
            "{SCREEN}eDP-1 connected primary 1920x1080+0+0 (normal left inverted right x axis y axis) 344mm x 193mm\n"
        ))
        .unwrap_err();
        assert!(err.to_string().contains("--verbose"), "{err}");
        let err = parse_verbose(&format!(
            "{SCREEN}eDP-1 connected\n   1920x1080     60.00*+  59.94\n"
        ))
        .unwrap_err();
        assert!(err.to_string().contains("--verbose"), "{err}");
    }

    #[test]
    fn properties_transform_crtcs_and_panning() {
        let snap = parse(
            "eDP-1 connected 2880x1620+0+0 (0x4a) normal (normal left inverted right x axis y axis) 344mm x 193mm\n\
             \tCRTC:       2\n\
             \tCRTCs:      0 1 2 3\n\
             \tPanning:    2880x3240+0+0\n\
             \tTracking:   2880x1620+0+0\n\
             \tTransform:  1.500000 0.000000 0.000000\n\
             \t            0.000000 1.500000 0.000000\n\
             \t            0.000000 0.000000 1.000000\n\
             \t           filter: bilinear\n\
             \tPRIME Synchronization: \t\tsupported: ?, ?\n\
             \tCTM: \t1.000000 0.000000 0.000000\n\
             \t\t0.000000 1.000000 0.000000\n\
             \tnon-desktop: 0 \n\
             \t\trange: (0, 1)\n",
        );
        let out = &snap.outputs[0];
        assert_eq!(out.crtc, Some(2));
        assert_eq!(out.crtcs, vec![0, 1, 2, 3]);
        let active = out.active.as_ref().unwrap();
        assert_eq!(active.scaling.scale_factors(), Some((1.5, 1.5)));
        assert_eq!(active.scaling.transform().unwrap().filter, "bilinear");
        assert_eq!(active.panning, Some(Rect::new(0, 0, 2880, 3240)));
    }

    #[test]
    fn panning_equal_to_the_output_is_ignored() {
        let snap = parse(
            "eDP-1 connected 1920x1080+0+0 (0x4a) normal (normal left inverted right x axis y axis) 344mm x 193mm\n\
             \tPanning:    1920x1080+0+0\n",
        );
        assert_eq!(snap.outputs[0].active.as_ref().unwrap().panning, None);
    }

    #[test]
    fn modes_flags_and_refresh() {
        let snap = parse(
            "eDP-1 connected 1920x1080+0+0 (0x4a) normal (normal left inverted right x axis y axis) 344mm x 193mm\n\
             \x20 1920x1080 (0x4a) 405.000MHz -HSync -VSync *current +preferred\n\
             \x20       h: width  1920 start 1936 end 1952 total 2080 skew    0 clock 194.71KHz\n\
             \x20       v: height 1080 start 1083 end 1097 total 1180           clock 165.01Hz\n\
             \x20 1024x768i (0x72) 44.900MHz +HSync +VSync Interlace\n\
             \x20       h: width  1024 start 1032 end 1208 total 1264 skew    0 clock  35.52KHz\n\
             \x20       v: height  768 start  768 end  776 total  817\n\
             \x20 1920x1080 (0x4b) 356.375MHz -HSync +VSync DoubleScan\n\
             \x20       h: width  1920 start 2080 end 2288 total 2656 skew    0 clock 134.18KHz\n\
             \x20       v: height 1080 start 1081 end 1084 total 1118\n",
        );
        let modes = &snap.outputs[0].modes;
        assert_eq!(modes.len(), 3);
        assert_eq!(
            (modes[0].width, modes[0].height, modes[0].refresh),
            (1920, 1080, 165.01)
        );
        assert!(modes[0].preferred);
        assert_eq!(modes[1].name, "1024x768i");
        assert_eq!((modes[1].width, modes[1].height), (1024, 768));
        assert!(modes[1].interlaced);
        // Without a printed clock: 44.9 MHz / (1264 · 817), doubled for interlace.
        assert!(
            (modes[1].refresh - 86.96).abs() < 0.01,
            "{}",
            modes[1].refresh
        );
        assert!(modes[2].double_scan);
        assert!(
            (modes[2].refresh - 60.01).abs() < 0.01,
            "{}",
            modes[2].refresh
        );
    }

    #[test]
    fn a_mode_name_is_not_a_size() {
        let snap = parse(
            "HDMI-1 connected 1920x1080+0+0 (0x99) normal (normal left inverted right x axis y axis) 1mm x 1mm\n\
             \x20 1920x1080_60.00 (0x99) 173.000MHz -HSync +VSync *current\n\
             \x20       h: width  1920 start 2048 end 2248 total 2576 skew    0 clock  67.16KHz\n\
             \x20       v: height 1080 start 1083 end 1088 total 1120           clock  59.96Hz\n",
        );
        let mode = &snap.outputs[0].modes[0];
        assert_eq!(mode.name, "1920x1080_60.00");
        assert_eq!(mode.size(), Size::new(1920, 1080));
    }

    #[test]
    fn modes_of_a_disconnected_output_belong_to_it() {
        let snap = parse(
            "HDMI-1-0 connected 1920x1080+0+0 (0x720) normal (normal left inverted right x axis y axis) 1440mm x 810mm\n\
             \x20 1920x1080 (0x720) 148.350MHz +HSync +VSync *current\n\
             \x20       h: width  1920 start 2008 end 2052 total 2200 skew    0 clock  67.43KHz\n\
             \x20       v: height 1080 start 1084 end 1089 total 1125           clock  59.94Hz\n\
             DP-1-4 disconnected (normal left inverted right x axis y axis)\n\
             \tCRTCs:      4 5 6 7\n\
             \x20 1680x1050 (0x4f) 146.250MHz -HSync +VSync\n\
             \x20       h: width  1680 start 1784 end 1960 total 2240 skew    0 clock  65.29KHz\n\
             \x20       v: height 1050 start 1053 end 1059 total 1089           clock  59.95Hz\n",
        );
        assert_eq!(snap.outputs[0].modes.len(), 1);
        assert_eq!(snap.outputs[1].modes.len(), 1);
        assert_eq!(snap.outputs[1].modes[0].id, ModeId(0x4f));
        assert!(!snap.outputs[1].is_relevant());
    }

    #[test]
    fn missing_screen_line_is_an_error() {
        assert_eq!(
            parse_verbose("eDP-1 connected\n"),
            Err(ParseError::NoScreen)
        );
        assert_eq!(parse_verbose(""), Err(ParseError::NoScreen));
    }

    #[test]
    fn tokenizer_keeps_groups() {
        assert_eq!(
            tokenize("a b (c d (e)) f"),
            vec!["a", "b", "(c d (e))", "f"],
        );
    }
}
