//! Popups drawn over the editor: pickers, questions and help.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Widget, Wrap};

use super::app::{App, Picker, Question};
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
