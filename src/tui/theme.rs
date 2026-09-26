//! Colours. Each display gets its own named ANSI colour; red, green and yellow are reserved for
//! overlaps, seams and warnings. `NO_COLOR` turns every colour off.

use ratatui::style::{Color, Modifier, Style};

use crate::model::validate::Severity;

/// Display colours, by display number. Red, green and yellow never appear here.
const OUTPUT_COLOURS: [Color; 8] = [
    Color::Cyan,
    Color::Magenta,
    Color::Blue,
    Color::White,
    Color::LightCyan,
    Color::LightMagenta,
    Color::LightBlue,
    Color::Gray,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub colour: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self { colour: true }
    }
}

impl Theme {
    /// Colours unless `NO_COLOR` is set to a non-empty value.
    pub fn from_env() -> Self {
        let no_colour = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        Self { colour: !no_colour }
    }

    pub fn plain() -> Self {
        Self { colour: false }
    }

    fn fg(&self, colour: Color) -> Style {
        if self.colour {
            Style::new().fg(colour)
        } else {
            Style::new()
        }
    }

    /// The colour of display number `n` (1-based).
    pub fn output(&self, number: Option<usize>) -> Style {
        let k = number.map_or(0, |n| n.saturating_sub(1));
        self.fg(OUTPUT_COLOURS[k % OUTPUT_COLOURS.len()])
    }

    /// Shared edges, where the mouse crosses from one display to the next.
    pub fn seam(&self) -> Style {
        self.fg(Color::Green)
    }

    pub fn overlap(&self) -> Style {
        self.fg(Color::Red)
    }

    pub fn severity(&self, severity: Severity) -> Style {
        match severity {
            Severity::Error => self.fg(Color::Red).add_modifier(Modifier::BOLD),
            Severity::Warning => self.fg(Color::Yellow),
            Severity::Info => Style::new(),
        }
    }

    pub fn success(&self) -> Style {
        self.fg(Color::Green)
    }

    pub fn dim(&self) -> Style {
        if self.colour {
            Style::new().fg(Color::DarkGray)
        } else {
            Style::new().add_modifier(Modifier::DIM)
        }
    }

    pub fn key(&self) -> Style {
        Style::new().add_modifier(Modifier::BOLD)
    }

    pub fn selected(&self) -> Style {
        Style::new().add_modifier(Modifier::REVERSED)
    }

    pub fn pending(&self) -> Style {
        Style::new().add_modifier(Modifier::BOLD)
    }
}
