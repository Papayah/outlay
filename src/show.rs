//! `outlay show` and `outlay list`: text views of a snapshot. The `show` diagram is the editor's
//! canvas rendered into a buffer and printed, with colour only on a terminal.

use std::io::IsTerminal;

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use crate::model::layout::{Layout, OutputState};
use crate::model::orientation;
use crate::model::validate::validate;
use crate::model::{Kind, Output, Snapshot};
use crate::tui;
use crate::tui::canvas::{self, Scene, Viewport};
use crate::tui::theme::Theme;

/// Pads each column to its widest cell, two spaces apart, without trailing spaces.
fn columns(rows: &[Vec<String>], indent: &str) -> String {
    let widths: Vec<usize> = (0..rows.iter().map(Vec::len).max().unwrap_or(0))
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.get(c))
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for row in rows {
        let mut line = indent.to_owned();
        for (c, cell) in row.iter().enumerate() {
            line.push_str(cell);
            if c + 1 < row.len() {
                line.push_str(&" ".repeat(widths[c] - cell.chars().count() + 2));
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// `AUO B156HAN12.H · 344x193 mm · 141 dpi`
fn display_details(out: &Output) -> String {
    let mut parts = Vec::new();
    let label = out.label();
    if !label.is_empty() {
        parts.push(label);
    }
    if let Some(mm) = out.physical_mm {
        parts.push(format!("{}x{} mm", mm.w, mm.h));
        if let Some(mode) = out.current_mode()
            && mm.w > 0
        {
            parts.push(format!(
                "{:.0} dpi",
                f64::from(mode.width) * 25.4 / f64::from(mm.w)
            ));
        }
    }
    parts.join(" · ")
}

fn state(out: &Output) -> String {
    let mut state = out.connection.as_str().to_owned();
    if out.is_stale() {
        state.push_str(", still active");
    }
    if out.primary {
        state.push_str(", primary");
    }
    state
}

/// `list`: each numbered output with its resolutions and rates, then the outputs not connected.
pub fn list(snap: &Snapshot) -> String {
    let mut text = String::new();
    for (n, &i) in snap.numbered().iter().enumerate() {
        let out = &snap.outputs[i];
        let mut header = format!("{}  {}  {}", n + 1, out.name, state(out));
        let details = display_details(out);
        if !details.is_empty() {
            header.push_str(" · ");
            header.push_str(&details);
        }
        text.push_str(&header);
        text.push('\n');
        let current = out.active.as_ref().map(|a| a.mode);
        let rows: Vec<Vec<String>> = out
            .resolutions()
            .iter()
            .map(|r| {
                let rates = out.rates(r.width, r.height).into_iter().map(|m| {
                    let mut token = m.rate_label();
                    if Some(m.id) == current {
                        token.push('*');
                    }
                    if m.preferred {
                        token.push('+');
                    }
                    token
                });
                vec![
                    format!("{}x{}", r.width, r.height),
                    rates.collect::<Vec<_>>().join("  "),
                ]
            })
            .collect();
        if rows.is_empty() {
            text.push_str("    no modes\n");
        } else {
            text.push_str(&columns(&rows, "    "));
        }
    }
    // A Wayland head exists only while connected.
    let rest: Vec<&str> = snap
        .outputs
        .iter()
        .filter(|o| !o.is_relevant() && snap.caps.kind == Kind::X11)
        .map(|o| o.name.as_str())
        .collect();
    if !rest.is_empty() {
        text.push_str(&format!("\nnot connected: {}\n", rest.join(" ")));
    }
    text
}

/// The orientation, plus a scale badge when set: `left, reflect x ×1.5` on X11, `90 (left) 150%`
/// on Wayland.
fn orientation(st: &OutputState, kind: Kind) -> String {
    let mut text = orientation::label(kind, st.rotation, st.reflection);
    if !st.scaling.is_identity() {
        match st.scaling.badge() {
            Some(badge) => text.push_str(&format!(" {badge}")),
            None => text.push_str(" transformed"),
        }
    }
    text
}

/// How `show` draws its diagram.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiagramOptions {
    /// Columns available.
    pub width: u16,
    /// Cell height over cell width.
    pub aspect: f64,
    /// Emit ANSI colours.
    pub colour: bool,
}

impl DiagramOptions {
    /// Plain text, 80 columns, for pipes and tests.
    pub fn plain() -> Self {
        Self {
            width: 80,
            aspect: 2.0,
            colour: false,
        }
    }

    /// The terminal's width and cell shape, with colour, when stdout is a terminal; plain text
    /// otherwise. `aspect` overrides the detected cell shape.
    pub fn for_stdout(aspect: Option<f64>) -> Self {
        if !std::io::stdout().is_terminal() {
            return Self {
                aspect: aspect.unwrap_or(2.0),
                ..Self::plain()
            };
        }
        let width = ratatui::crossterm::terminal::size().map_or(80, |(w, _)| w);
        Self {
            width: width.clamp(40, 120),
            aspect: aspect.or_else(tui::detect_cell_aspect).unwrap_or(2.0),
            colour: Theme::from_env().colour,
        }
    }
}

/// The live layout drawn to scale, the same way the editor draws it.
pub fn diagram(snap: &Snapshot, options: &DiagramOptions) -> String {
    let layout = Layout::inferred(snap);
    let Some(bounds) = layout.bounds() else {
        return String::new();
    };
    let span = f64::from(options.width.saturating_sub(3).max(1));
    let scale = span / f64::from(bounds.w.max(1));
    let rows = (f64::from(bounds.h) * scale / options.aspect).round() as u16 + 1;
    let area = Rect::new(0, 0, options.width, rows.clamp(3, MAX_DIAGRAM_ROWS));
    let mut view = Viewport::default();
    view.fit(area, bounds, options.aspect);
    let mut buf = Buffer::empty(area);
    let theme = if options.colour {
        Theme::default()
    } else {
        Theme::plain()
    };
    let pending = vec![false; layout.len()];
    let scene = Scene {
        layout: &layout,
        snap,
        theme,
        focus: None,
        target: None,
        double_borders: false,
        ghosts: &[],
        pending: &pending,
    };
    canvas::render(&scene, &view, area, &mut buf);
    buffer_text(&buf, options.colour)
}

const MAX_DIAGRAM_ROWS: u16 = 30;

/// A buffer as lines of text without trailing spaces, with ANSI colours when `colour` is set.
pub fn buffer_text(buf: &Buffer, colour: bool) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        let cells: Vec<&Cell> = (area.left()..area.right()).map(|x| &buf[(x, y)]).collect();
        let end = cells
            .iter()
            .rposition(|c| c.symbol() != " ")
            .map_or(0, |k| k + 1);
        let mut line = String::new();
        let mut current = None;
        for cell in &cells[..end] {
            if colour {
                let style = (cell.fg, cell.modifier);
                if current != Some(style) {
                    line.push_str(&sgr(style.0, style.1));
                    current = Some(style);
                }
            }
            line.push_str(cell.symbol());
        }
        if colour && current.is_some() {
            line.push_str("\x1b[0m");
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// The SGR sequence that selects a foreground colour and modifiers, from a clean state.
fn sgr(fg: Color, modifier: Modifier) -> String {
    let mut codes = vec!["0".to_owned()];
    if modifier.contains(Modifier::BOLD) {
        codes.push("1".to_owned());
    }
    if modifier.contains(Modifier::DIM) {
        codes.push("2".to_owned());
    }
    let named = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
    ];
    let bright = [
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];
    if let Some(k) = named.iter().position(|&c| c == fg) {
        codes.push((30 + k).to_string());
    } else if let Some(k) = bright.iter().position(|&c| c == fg) {
        codes.push((90 + k).to_string());
    }
    format!("\x1b[{}m", codes.join(";"))
}

/// `show`: the summary line, the diagram, then the table.
pub fn show(snap: &Snapshot, options: &DiagramOptions) -> String {
    let mut text = summary(snap);
    text.push('\n');
    text.push_str(&diagram(snap, options));
    text.push('\n');
    text.push_str(&table_rows(snap));
    text
}

/// `outlay · 3 on · 1 off · screen 4480x2520 of max 16384x16384`
fn summary(snap: &Snapshot) -> String {
    let layout = Layout::inferred(snap);
    let numbered = snap.numbered();
    let on = numbered.iter().filter(|&&i| layout.is_enabled(i)).count();
    let room = match snap.screen {
        Some(s) => format!(
            "screen {}x{} of max {}x{}",
            s.current.w, s.current.h, s.max.w, s.max.h
        ),
        None => {
            let b = layout.bounds().unwrap_or_default();
            format!("layout {}x{}", b.right(), b.bottom())
        }
    };
    format!("outlay · {on} on · {} off · {room}\n", numbered.len() - on)
}

/// The summary line, one row per numbered output with its inferred link, then the validation
/// issues of the live layout.
pub fn table(snap: &Snapshot) -> String {
    format!("{}\n{}", summary(snap), table_rows(snap))
}

fn table_rows(snap: &Snapshot) -> String {
    let layout = Layout::inferred(snap);
    let numbered = snap.numbered();
    let mut text = String::new();
    let mut rows = vec![
        [
            "#", "output", "mode", "position", "rotation", "link", "display",
        ]
        .map(str::to_owned)
        .to_vec(),
    ];
    for (n, &i) in numbered.iter().enumerate() {
        let out = &snap.outputs[i];
        let st = &layout.outputs[i];
        let name = if st.primary {
            format!("{} ★", out.name)
        } else {
            out.name.clone()
        };
        let row = match (&st.mode, st.enabled) {
            (Some(mode), true) => {
                let mode = if out.mode(mode.id).is_some() {
                    format!("{}x{} @ {:.2}", mode.width, mode.height, mode.refresh)
                } else {
                    format!("{}x{}", mode.width, mode.height)
                };
                [
                    mode,
                    format!("{},{}", st.pos.x, st.pos.y),
                    orientation(st, snap.caps.kind),
                    layout.link_text(i),
                ]
            }
            _ => [
                "off".to_owned(),
                String::new(),
                String::new(),
                String::new(),
            ],
        };
        let mut cells = vec![(n + 1).to_string(), name];
        cells.extend(row);
        cells.push(display_details(out));
        rows.push(cells);
    }
    text.push_str(&columns(&rows, ""));
    let issues = validate(&layout, snap);
    if !issues.is_empty() {
        text.push('\n');
        for issue in issues {
            text.push_str(&format!("{issue}\n"));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_pad_and_trim() {
        let rows = vec![
            vec!["a".to_owned(), "bb".to_owned(), String::new()],
            vec!["ccc".to_owned(), "d".to_owned(), "e".to_owned()],
        ];
        assert_eq!(columns(&rows, " "), " a    bb\n ccc  d   e\n");
    }
}
