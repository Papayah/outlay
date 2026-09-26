//! The whole screen: title bar, canvas, details, status and hint lines, and the popup on top.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout as Split, Rect};
use ratatui::widgets::{Block, Paragraph, Widget, Wrap};

use super::app::{App, UiMode};
use super::canvas::{self, Scene};
use super::{panels, popups};

/// Below this the editor shows a "terminal too small" screen.
pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 16;
/// The details panel needs this many columns in total.
const DETAILS_MIN_WIDTH: u16 = 90;
const DETAILS_WIDTH: u16 = 32;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let buf = frame.buffer_mut();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let text = format!(
            "The terminal is too small: {}x{}. outlay needs {MIN_WIDTH}x{MIN_HEIGHT}.",
            area.width, area.height
        );
        let rect = popups::centred(area, area.width, 3);
        Paragraph::new(text)
            .centered()
            .wrap(Wrap { trim: true })
            .render(rect, buf);
        return;
    }
    let [title, main, status, hints] = Split::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let details = app.show_details && area.width >= DETAILS_MIN_WIDTH;
    let (canvas_area, side) = if details {
        let [c, s] =
            Split::horizontal([Constraint::Fill(1), Constraint::Length(DETAILS_WIDTH)]).areas(main);
        (c, Some(s))
    } else {
        (main, None)
    };

    panels::title_bar(app, title, buf);
    let block = Block::bordered().title(" layout ");
    let inner = block.inner(canvas_area);
    block.render(canvas_area, buf);
    draw_canvas(app, inner, buf);
    if let Some(side) = side {
        panels::details(app, side, buf);
    }
    panels::status_line(app, status, buf);
    match &app.mode {
        UiMode::Command(line) => panels::command_line(app, line, hints, buf),
        _ => panels::hint_line(app, hints, buf),
    }

    match &app.mode {
        UiMode::Picker(picker) => popups::picker(app, picker, main, buf),
        UiMode::Confirm(question) => popups::confirm(app, *question, main, buf),
        UiMode::Help { scroll } => {
            let clamped = popups::help(app, *scroll, area, buf);
            app.mode = UiMode::Help { scroll: clamped };
        }
        UiMode::ConfirmApply(plan) => popups::confirm_apply(app, plan, main, buf),
        UiMode::Applying => popups::applying(main, buf),
        UiMode::Countdown(c) => popups::countdown(app, c, main, buf),
        UiMode::Message(m) => popups::message(app, m, main, buf),
        _ => {}
    }
}

fn draw_canvas(app: &mut App, area: Rect, buf: &mut ratatui::buffer::Buffer) {
    let pending = app.pending_flags();
    let (target, ghosts) = match &app.mode {
        UiMode::Stick(flow) => (Some(flow.target), flow.ghosts.clone()),
        _ => (
            app.layout.links[app.focus]
                .filter(|_| app.layout.is_enabled(app.focus))
                .map(|l| l.parent),
            Vec::new(),
        ),
    };
    let scene = Scene {
        layout: &app.layout,
        snap: &app.snap,
        theme: app.theme,
        focus: Some(app.focus),
        target,
        ghosts: &ghosts,
        pending: &pending,
    };
    app.viewport.update(area, scene.bounds(), app.cell_aspect);
    canvas::render(&scene, &app.viewport, area, buf);
}
