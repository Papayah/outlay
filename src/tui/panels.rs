//! The panels around the canvas: title bar, details, off tray, status line and hint line.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::symbols::merge::MergeStrategy;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph, Widget, Wrap};

use crate::model::{Kind, Reflection, Scaling};

use super::app::{App, UiMode};
use super::canvas::truncate;
use super::keys::{Action, Context};

/// ` outlay · 3 on · 1 off · demo` on the left, `● 2 pending` on the right.
pub fn title_bar(app: &App, area: Rect, buf: &mut Buffer) {
    let numbered = app.snap.numbered();
    let on = numbered
        .iter()
        .filter(|&&i| app.layout.is_enabled(i))
        .count();
    let mut left = vec![
        Span::styled(" outlay", Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(format!(" · {on} on · {} off", numbered.len() - on)),
    ];
    if let Some(source) = &app.source {
        left.push(Span::styled(format!(" · {source}"), app.theme.dim()));
    }
    if let Some(profile) = &app.profile {
        left.push(Span::raw(format!(" · profile {profile}")));
    }
    Line::from(left).render(area, buf);

    let pending = app.pending().len();
    let right = if pending == 0 {
        Line::from(Span::styled("no changes ", app.theme.dim()))
    } else {
        let apply = app
            .keymap
            .key_for(Context::Normal, Action::Apply)
            .unwrap_or_default();
        Line::from(vec![
            Span::styled("● ", app.theme.pending()),
            Span::raw(format!("{pending} pending · ")),
            Span::styled(apply, app.theme.key()),
            Span::raw(" apply "),
        ])
    };
    let width = right.width() as u16;
    if width < area.width {
        let x = area.right() - width;
        right.render(Rect::new(x, area.y, width, 1), buf);
    }
}

/// `2560x1440@143.91`; for a change, `2560x1440` and `rate 59.95 → 143.91`, or the old and the
/// new mode on two lines.
fn mode_lines(app: &App, i: usize) -> Vec<String> {
    let st = &app.layout.outputs[i];
    let Some(mode) = &st.mode else {
        return vec!["mode  none".to_owned()];
    };
    let live = app.snap.outputs[i]
        .current_mode()
        .filter(|_| app.snap.outputs[i].active.is_some());
    match live {
        Some(old) if old.id != mode.id && old.size() == mode.size() => vec![
            format!("mode  {}x{}", mode.width, mode.height),
            format!("rate  {:.2} → {:.2}", old.refresh, mode.refresh),
        ],
        Some(old) if old.id != mode.id => vec![
            format!("mode  {}", old.summary()),
            format!("   →  {}", mode.summary()),
        ],
        _ if app.snap.outputs[i].mode(mode.id).is_none() => {
            vec![format!("mode  {}x{}", mode.width, mode.height)]
        }
        _ => vec![format!("mode  {}", mode.summary())],
    }
}

/// `scale ×1.5`, `scale ×1.25 → ×1.5`, or on Wayland `scale 150% · 1280x720 logical`. Left out
/// on X11 while the display has no scale and none is pending.
fn scale_line(app: &App, i: usize) -> Option<String> {
    let st = &app.layout.outputs[i];
    let live = app.snap.outputs[i].active.as_ref().map(|a| &a.scaling);
    let wayland = app.layout.caps.kind == Kind::Wayland;
    if !wayland && st.scaling.is_identity() && live.is_none_or(Scaling::is_identity) {
        return None;
    }
    let text = |s: &Scaling| s.badge().unwrap_or_else(|| "custom".to_owned());
    let mut line = match live {
        Some(l) if !l.approx_eq(&st.scaling) => {
            format!("scale {} → {}", text(l), text(&st.scaling))
        }
        _ => format!("scale {}", text(&st.scaling)),
    };
    if wayland {
        let size = app.layout.size(i);
        line.push_str(&format!(" · {}x{} logical", size.w, size.h));
    }
    Some(line)
}

/// The focused display's details, and below them the tray of connected displays that are off.
pub fn details(app: &App, area: Rect, buf: &mut Buffer) {
    let i = app.focus;
    let layout = &app.layout;
    let out = &app.snap.outputs[i];
    let st = &layout.outputs[i];
    let colour = app.theme.output(layout.numbers[i]);

    let off: Vec<usize> = app
        .snap
        .numbered()
        .into_iter()
        .filter(|&k| !layout.is_enabled(k))
        .collect();
    let tray_height = if off.is_empty() {
        0
    } else {
        (off.len() as u16 + 2).min(area.height / 2)
    };
    // The tray's top border is the details' bottom border.
    let top = if tray_height == 0 {
        area
    } else {
        Rect::new(area.x, area.y, area.width, area.height - tray_height + 1)
    };

    let mut title = format!(" {}", layout.label(i));
    if st.primary && st.enabled {
        title.push_str(" ★");
    }
    title.push(' ');
    let mut lines: Vec<Line> = Vec::new();
    match &out.identity {
        Some(id) => {
            lines.push(Line::from(id.label.clone()));
            if let Some(serial) = &id.serial {
                lines.push(Line::from(format!("serial {serial}")));
            }
        }
        None => match &out.description {
            Some(description) => lines.push(Line::from(description.clone())),
            None if app.layout.caps.kind == Kind::Wayland => {
                lines.push(Line::styled("unknown display", app.theme.dim()))
            }
            None => lines.push(Line::styled("no EDID", app.theme.dim())),
        },
    }
    if st.enabled {
        lines.extend(mode_lines(app, i).into_iter().map(Line::from));
        let pos = match &out.active {
            Some(live) if live.pos != st.pos => format!(
                "pos   {},{} → {},{}",
                live.pos.x, live.pos.y, st.pos.x, st.pos.y
            ),
            _ => format!("pos   {},{}", st.pos.x, st.pos.y),
        };
        lines.push(Line::from(pos));
        let mut rot = format!("rot   {}", st.rotation);
        if st.reflection != Reflection::Normal {
            rot.push_str(&format!(" · reflect {}", st.reflection));
        }
        lines.push(Line::from(rot));
        if let Some(scale) = scale_line(app, i) {
            lines.push(Line::from(scale));
        }
        // Shown, never changed: outlay leaves variable refresh as the compositor has it.
        if let Some(sync) = out.adaptive_sync.filter(|_| out.active.is_some()) {
            lines.push(Line::from(format!(
                "vrr   {}",
                if sync { "on" } else { "off" }
            )));
        }
        lines.push(Line::from(format!("link  {}", layout.link_text(i))));
    } else {
        let toggle = app
            .keymap
            .key_for(Context::Normal, Action::Toggle)
            .unwrap_or_default();
        lines.push(Line::styled("off", app.theme.dim()));
        if out.is_connected() {
            lines.push(Line::from(format!("{toggle} turns it on")));
        } else {
            lines.push(Line::from("disconnected"));
        }
    }
    if let Some(mm) = out.physical_mm {
        let mut size = format!("size  {}x{} mm", mm.w, mm.h);
        if let Some(mode) = &st.mode
            && mm.w > 0
        {
            size.push_str(&format!(
                " · {:.0} dpi",
                f64::from(mode.width) * 25.4 / f64::from(mm.w)
            ));
        }
        lines.push(Line::from(size));
    }
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(
            Block::bordered()
                .title(Span::styled(title, colour.add_modifier(Modifier::BOLD)))
                .padding(Padding::horizontal(1))
                .merge_borders(MergeStrategy::Exact),
        )
        .render(top, buf);

    if tray_height > 0 {
        let tray = Rect::new(area.x, area.bottom() - tray_height, area.width, tray_height);
        let inner_width = usize::from(tray.width.saturating_sub(4));
        let rows: Vec<Line> = off
            .iter()
            .map(|&k| {
                let text = format!("{:<11} {}", layout.label(k), app.snap.outputs[k].label());
                let mut style = app.theme.output(layout.numbers[k]);
                if k == i {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                Line::styled(truncate(&text, inner_width), style)
            })
            .collect();
        Paragraph::new(rows)
            .block(
                Block::bordered()
                    .title(" off ")
                    .padding(Padding::horizontal(1))
                    .merge_borders(MergeStrategy::Exact),
            )
            .render(tray, buf);
    }
}

/// The last message, coloured by severity, then the most severe validation issue, and the
/// nudge step on the right.
pub fn status_line(app: &App, area: Rect, buf: &mut Buffer) {
    let step = match app.nudge_multiplier() {
        1 => format!("step {}px ", app.step),
        m => format!("step {}px ×{m} ", app.step),
    };
    let mut spans = vec![Span::raw(" ")];
    let flow_text = match &app.mode {
        UiMode::Stick(flow) => Some(app.stick_summary(flow)),
        _ => None,
    };
    if let Some(text) = flow_text {
        spans.push(Span::styled(
            text,
            Style::new().add_modifier(Modifier::BOLD),
        ));
    } else if let Some(status) = &app.status {
        spans.push(Span::styled(
            status.text.clone(),
            app.theme.severity(status.severity),
        ));
    }
    if let Some(issue) = app.issues.first() {
        if spans.len() > 1 {
            spans.push(Span::styled(" · ", app.theme.dim()));
        }
        let mut text = issue.message.clone();
        if app.issues.len() > 1 {
            text.push_str(&format!(" (+{} more)", app.issues.len() - 1));
        }
        spans.push(Span::styled(text, app.theme.severity(issue.severity)));
    }
    let room = area.width.saturating_sub(step.chars().count() as u16 + 1);
    Line::from(spans).render(Rect::new(area.x, area.y, room, 1), buf);
    let x = area.right().saturating_sub(step.chars().count() as u16);
    buf.set_string(x, area.y, step, app.theme.dim());
}

/// Key hints for the current mode, generated from the keymap. When they do not all fit, entries
/// before the last one are left out, so the last (help, cancel, close) always shows.
pub fn hint_line(app: &App, area: Rect, buf: &mut Buffer) {
    let context = app.context();
    let hints = app.keymap.hints(context, |b| match b.does {
        _ if !b.available(&app.layout.caps) => false,
        super::keys::Does::Act(Action::Undo) => app.history.can_undo(),
        _ => true,
    });
    let cost = |(keys, label): &(String, &str)| keys.chars().count() + 1 + label.chars().count();
    let room = usize::from(area.width).saturating_sub(1);
    let total: usize = hints.iter().map(cost).sum::<usize>() + 3 * hints.len().saturating_sub(1);
    let mut shown: Vec<&(String, &str)> = Vec::new();
    if total <= room {
        shown.extend(&hints);
    } else if let Some((last, rest)) = hints.split_last() {
        let mut width = cost(last);
        for hint in rest {
            if width + cost(hint) + 3 > room {
                break;
            }
            width += cost(hint) + 3;
            shown.push(hint);
        }
        shown.push(last);
    }
    let mut spans = vec![Span::raw(" ")];
    for (k, (keys, label)) in shown.into_iter().enumerate() {
        if k > 0 {
            spans.push(Span::styled(" · ", app.theme.dim()));
        }
        spans.push(Span::styled(keys.clone(), app.theme.key()));
        spans.push(Span::raw(format!(" {label}")));
    }
    Line::from(spans).render(area, buf);
}

/// `:pos 10 20█` or `save as home█` in place of the hint line.
pub fn command_line(
    app: &App,
    prompt: &str,
    line: &str,
    context: Context,
    area: Rect,
    buf: &mut Buffer,
) {
    let hints: Vec<String> = app
        .keymap
        .hints(context, |b| b.available(&app.layout.caps))
        .into_iter()
        .map(|(k, h)| format!("{k} {h}"))
        .collect();
    let hints = hints.join(" · ");
    let text = Line::from(vec![
        Span::styled(prompt.to_owned(), app.theme.key()),
        Span::raw(line.to_owned()),
        Span::styled("█", Style::new().add_modifier(Modifier::SLOW_BLINK)),
    ]);
    text.render(area, buf);
    let used = prompt.chars().count() + line.chars().count() + 2;
    let hint_width = hints.chars().count() + 1;
    if used + hint_width < usize::from(area.width) {
        let x = area.right() - hint_width as u16;
        buf.set_string(x, area.y, hints, app.theme.dim());
    }
}
