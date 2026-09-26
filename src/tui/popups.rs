//! Popups drawn over the editor: pickers, questions and help.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, LineGauge, Padding, Paragraph, Widget, Wrap};

use crate::model::validate::Severity;

use super::app::{
    App, ApplyPlan, Countdown, Message, Picker, ProfilePicker, Question, RemapDialog, SavePlan,
};
use super::canvas::{self, Scene, Viewport, truncate};
use super::keys::Context;

/// A `width` x `height` rectangle centred in `area`, shrunk to fit.
pub fn centred(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

/// `⏎ choose · Esc cancel`, from the keymap.
fn hints(app: &App, context: Context) -> Line<'static> {
    let mut spans = Vec::new();
    for (k, (keys, label)) in app.keymap.hints(context, |_| true).into_iter().enumerate() {
        if k > 0 {
            spans.push(Span::styled(" · ", app.theme.dim()));
        }
        spans.push(Span::styled(keys, app.theme.key()));
        spans.push(Span::raw(format!(" {label}")));
    }
    Line::from(spans)
}

fn frame(title: String, area: Rect, buf: &mut Buffer) -> Rect {
    Clear.render(area, buf);
    let block = Block::bordered().title(Span::styled(
        format!(" {title} "),
        Style::new().add_modifier(Modifier::BOLD),
    ));
    let inner = block.inner(area);
    block.render(area, buf);
    inner
}

/// The resolution or rate picker: a list with the selection highlighted, kept in view.
pub fn picker(app: &App, picker: &Picker, area: Rect, buf: &mut Buffer) {
    let widest = picker
        .items
        .iter()
        .map(|it| it.label.chars().count())
        .max()
        .unwrap_or(0)
        .max(picker.title.chars().count() + 2)
        .max(hints(app, Context::Picker).width());
    let height = picker.items.len() as u16 + 3;
    let rect = centred(area, widest as u16 + 4, height);
    let inner = frame(picker.title.clone(), rect, buf);
    if inner.height < 2 {
        return;
    }
    let rows = usize::from(inner.height - 1);
    let first = picker.selected.saturating_sub(rows - 1);
    for (k, item) in picker.items.iter().enumerate().skip(first).take(rows) {
        let y = inner.y + (k - first) as u16;
        let style = if k == picker.selected {
            app.theme.selected()
        } else {
            Style::new()
        };
        let text = format!(" {:<w$}", item.label, w = usize::from(inner.width) - 1);
        buf.set_string(inner.x, y, truncate(&text, usize::from(inner.width)), style);
    }
    let hint_row = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
    hints(app, Context::Picker).render(hint_row, buf);
}

/// "Discard N pending changes and quit?" and friends.
pub fn confirm(app: &App, question: Question, area: Rect, buf: &mut Buffer) {
    let pending = app.pending().len();
    let changes = if pending == 1 {
        "1 pending change".to_owned()
    } else {
        format!("{pending} pending changes")
    };
    let (title, text) = match question {
        Question::Quit => ("Quit", format!("Discard {changes} and quit?")),
    };
    let hint = hints(app, Context::Confirm);
    let width = (text.chars().count().max(hint.width()) + 4) as u16;
    let rect = centred(area, width, 5);
    let inner = frame(title.to_owned(), rect, buf);
    Paragraph::new(vec![Line::from(text), Line::default(), hint])
        .wrap(Wrap { trim: false })
        .render(inner, buf);
}

/// Every key binding, grouped by mode, then the commands. Returns the scroll offset clamped to
/// the text.
pub fn help(app: &App, scroll: u16, area: Rect, buf: &mut Buffer) -> u16 {
    let rect = centred(area, 90, area.height.saturating_sub(2));
    let inner = frame("help".to_owned(), rect, buf);
    if inner.height < 2 {
        return 0;
    }
    let sections = app.keymap.reference();
    let mut lines: Vec<Line> = Vec::new();
    for (title, rows) in sections {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::styled(
            title.to_owned(),
            Style::new().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ));
        // Each section aligns its own column, so one long entry does not widen them all.
        let keys_width = rows
            .iter()
            .map(|(k, _)| k.chars().count())
            .max()
            .unwrap_or(0);
        for (keys, text) in rows {
            lines.push(Line::from(vec![
                Span::styled(format!("  {keys:<keys_width$}  "), app.theme.key()),
                Span::raw(text.to_owned()),
            ]));
        }
    }
    let visible = usize::from(inner.height - 1);
    let max_scroll = lines.len().saturating_sub(visible);
    let scroll = usize::from(scroll).min(max_scroll);
    let body = Rect::new(inner.x, inner.y, inner.width, inner.height - 1);
    Paragraph::new(
        lines
            .into_iter()
            .skip(scroll)
            .take(visible)
            .collect::<Vec<_>>(),
    )
    .render(body, buf);
    let hint_row = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
    hints(app, Context::Help).render(hint_row, buf);
    scroll as u16
}

/// The number of rows `lines` take when wrapped at `width` columns.
fn wrapped_rows(lines: &[Line], width: u16) -> u16 {
    let width = usize::from(width.max(1));
    lines
        .iter()
        .map(|l| l.width().max(1).div_ceil(width) as u16)
        .sum()
}

/// A popup of wrapped lines with the hints of `context` at the bottom.
fn text_popup(
    app: &App,
    title: String,
    mut lines: Vec<Line<'static>>,
    context: Context,
    area: Rect,
    buf: &mut Buffer,
) {
    let width = area.width.saturating_sub(4).min(96);
    lines.push(Line::default());
    lines.push(hints(app, context));
    let rows = wrapped_rows(&lines, width.saturating_sub(4)) + 2;
    let rect = centred(area, width, rows);
    let inner = frame(title, rect, buf);
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(Block::new().padding(Padding::horizontal(1)))
        .render(inner, buf);
}

fn heading(text: &str) -> Line<'static> {
    Line::styled(text.to_owned(), Style::new().add_modifier(Modifier::BOLD))
}

/// The apply confirmation: what changes, what blocks the apply, what to watch for, and the
/// exact command.
pub fn confirm_apply(app: &App, plan: &ApplyPlan, area: Rect, buf: &mut Buffer) {
    let mut lines = vec![heading("Changes")];
    lines.extend(plan.changes.iter().map(|c| Line::from(format!("  {c}"))));
    if !plan.errors.is_empty() {
        lines.push(Line::default());
        lines.push(heading("Errors (these block the apply)"));
        let style = app.theme.severity(Severity::Error);
        lines.extend(
            plan.errors
                .iter()
                .map(|e| Line::styled(format!("  {e}"), style)),
        );
    }
    if !plan.warnings.is_empty() {
        lines.push(Line::default());
        lines.push(heading("Warnings"));
        let style = app.theme.severity(Severity::Warning);
        lines.extend(
            plan.warnings
                .iter()
                .map(|w| Line::styled(format!("  {w}"), style)),
        );
    }
    lines.push(Line::default());
    lines.push(heading("Command"));
    lines.push(Line::styled(plan.command.clone(), app.theme.dim()));
    text_popup(
        app,
        "Apply".to_owned(),
        lines,
        Context::ConfirmApply,
        area,
        buf,
    );
}

/// While xrandr runs.
pub fn applying(area: Rect, buf: &mut Buffer) {
    let text = "Applying… the screens may go dark for a moment.";
    let rect = centred(area, text.chars().count() as u16 + 4, 3);
    let inner = frame("xrandr".to_owned(), rect, buf);
    Paragraph::new(text).render(inner, buf);
}

/// "Keep this layout?" with a shrinking gauge.
pub fn countdown(app: &App, c: &Countdown, area: Rect, buf: &mut Buffer) {
    let total = c
        .deadline
        .saturating_duration_since(c.started)
        .as_secs_f64();
    let left = c.deadline.saturating_duration_since(app.now).as_secs_f64();
    let ratio = if total > 0.0 {
        (left / total).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let hint = hints(app, Context::Countdown);
    let width = (hint.width() as u16 + 12).max(52);
    let rect = centred(area, width, 6);
    let inner = frame("Keep this layout?".to_owned(), rect, buf);
    if inner.height < 4 {
        return;
    }
    let row = |k: u16| Rect::new(inner.x + 1, inner.y + k, inner.width.saturating_sub(2), 1);
    let blocked = app.now < c.blocked_until;
    let first = if blocked {
        Line::styled(
            "One moment: keys pressed just now are ignored.",
            app.theme.dim(),
        )
    } else {
        Line::from("It reverts on its own unless you keep it.")
    };
    first.render(row(0), buf);
    LineGauge::default()
        .ratio(ratio)
        .label(format!("{:>2.0} s ", left.ceil()))
        .filled_symbol("━")
        .unfilled_symbol("─")
        .filled_style(app.theme.severity(Severity::Warning))
        .unfilled_style(app.theme.dim())
        .render(row(1), buf);
    let hint = if blocked {
        hint.style(app.theme.dim())
    } else {
        hint
    };
    hint.render(row(3), buf);
}

/// A report, such as why an apply failed.
pub fn message(app: &App, message: &Message, area: Rect, buf: &mut Buffer) {
    let lines = message
        .lines
        .iter()
        .map(|l| Line::from(l.clone()))
        .collect();
    text_popup(
        app,
        message.title.clone(),
        lines,
        Context::Message,
        area,
        buf,
    );
}

/// The profile picker: names on the left, the selected profile drawn to scale on the right, and
/// what loading it would change or skip below.
pub fn profiles(app: &App, picker: &ProfilePicker, area: Rect, buf: &mut Buffer) {
    let names_width = picker
        .items
        .iter()
        .map(|it| it.name.chars().count() + 4)
        .max()
        .unwrap_or(10)
        .clamp(12, 32) as u16;
    let width = area.width.saturating_sub(4).min(names_width + 52);
    let height = area.height.saturating_sub(2).min(20);
    let rect = centred(area, width, height);
    let inner = frame(format!("profiles · {}", picker.items.len()), rect, buf);
    if inner.height < 4 || inner.width < names_width + 12 {
        return;
    }
    let body = Rect::new(inner.x, inner.y, inner.width, inner.height - 2);
    let list = Rect::new(body.x, body.y, names_width, body.height);
    let rows = usize::from(list.height);
    let first = picker.selected.saturating_sub(rows.saturating_sub(1));
    for (k, item) in picker.items.iter().enumerate().skip(first).take(rows) {
        let y = list.y + (k - first) as u16;
        let mark = if item.unmatched.is_empty() { ' ' } else { '~' };
        let text = format!(" {mark}{:<w$}", item.name, w = usize::from(list.width) - 2);
        let style = if k == picker.selected {
            app.theme.selected()
        } else {
            Style::new()
        };
        buf.set_string(list.x, y, truncate(&text, usize::from(list.width)), style);
    }

    for y in body.top()..body.bottom() {
        buf.set_string(list.right(), y, "│", app.theme.dim());
    }

    let item = &picker.items[picker.selected];
    let side = Rect::new(
        list.right() + 2,
        body.y,
        body.width - list.width - 2,
        body.height,
    );
    let mut notes: Vec<Line> = Vec::new();
    if !item.unmatched.is_empty() {
        notes.push(Line::styled(
            format!("~ not connected: {}", item.unmatched.join(" ")),
            app.theme.severity(Severity::Warning),
        ));
    }
    let changes = item.preview.diff(&app.snap).len();
    notes.push(Line::styled(
        match changes {
            0 => "no changes".to_owned(),
            1 => "1 output changes".to_owned(),
            n => format!("{n} outputs change"),
        },
        app.theme.dim(),
    ));
    let note_rows = (notes.len() as u16).min(side.height.saturating_sub(3));
    let drawing = Rect::new(side.x, side.y, side.width, side.height - note_rows);
    let pending = vec![false; item.preview.len()];
    let scene = Scene {
        layout: &item.preview,
        snap: &app.snap,
        theme: app.theme,
        focus: None,
        target: None,
        double_borders: false,
        ghosts: &[],
        pending: &pending,
    };
    let mut view = Viewport::default();
    view.update(drawing, scene.bounds(), app.cell_aspect);
    canvas::render(&scene, &view, drawing, buf);
    let below = Rect::new(side.x, drawing.bottom(), side.width, note_rows);
    Paragraph::new(notes).render(below, buf);

    let hint_row = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
    hints(app, Context::Profiles).render(hint_row, buf);
}

/// Where each of a profile's missing outputs goes.
pub fn remap(app: &App, dialog: &RemapDialog, area: Rect, buf: &mut Buffer) {
    let mut lines = vec![Line::from(format!(
        "{} names outputs that are not connected. Choose where each one goes:",
        dialog.item.name
    ))];
    lines.push(Line::default());
    let width = dialog
        .rows
        .iter()
        .map(|(from, _)| from.chars().count())
        .max()
        .unwrap_or(0);
    for (k, (from, to)) in dialog.rows.iter().enumerate() {
        let target = match to {
            Some(i) => {
                let out = &app.snap.outputs[*i];
                format!("{} {}", app.layout.label(*i), out.label())
                    .trim_end()
                    .to_owned()
            }
            None => "skip".to_owned(),
        };
        let text = format!("  {from:<width$}  →  ‹ {target} ›");
        let style = if k == dialog.selected {
            app.theme.selected()
        } else {
            Style::new()
        };
        lines.push(Line::styled(text, style));
    }
    text_popup(
        app,
        format!("open {}", dialog.item.name),
        lines,
        Context::Remap,
        area,
        buf,
    );
}

/// "Overwrite this profile?" with the line diff.
pub fn overwrite(app: &App, plan: &SavePlan, area: Rect, buf: &mut Buffer) {
    let mut lines = vec![Line::from(format!(
        "{} exists and differs:",
        super::session::tilde(&plan.path)
    ))];
    lines.push(Line::default());
    let rows = usize::from(area.height.saturating_sub(9));
    let changed: Vec<&String> = plan.diff.iter().filter(|l| !l.starts_with("  ")).collect();
    for line in changed.iter().take(rows) {
        let style = if line.starts_with('+') {
            app.theme.success()
        } else {
            app.theme.severity(Severity::Error)
        };
        lines.push(Line::styled((*line).clone(), style));
    }
    if changed.len() > rows {
        lines.push(Line::styled(
            format!("… {} more", changed.len() - rows),
            app.theme.dim(),
        ));
    }
    lines.push(Line::default());
    lines.push(Line::from("Overwrite it? Other lines stay as they are."));
    text_popup(app, "save".to_owned(), lines, Context::Confirm, area, buf);
}
