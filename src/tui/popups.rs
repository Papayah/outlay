//! Popups drawn over the editor: pickers, questions and help.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, LineGauge, Padding, Paragraph, Widget, Wrap};

use crate::model::validate::Severity;

use super::app::{App, ApplyPlan, Countdown, Message, Picker, Question};
use super::canvas::truncate;
use super::cmdline::USAGE;
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
        Question::Reload => (
            "Reload",
            format!("Discard {changes} and reload the live state?"),
        ),
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
    let sections: Vec<(&str, Vec<(String, &str)>)> = Context::ALL
        .iter()
        .map(|&ctx| (ctx.title(), app.keymap.help(ctx)))
        .filter(|(_, rows)| !rows.is_empty())
        .chain(std::iter::once((
            "Commands (after :)",
            USAGE.iter().map(|&(c, h)| (c.to_owned(), h)).collect(),
        )))
        .collect();
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
