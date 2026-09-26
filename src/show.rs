//! `outlay show` and `outlay list`: plain-text views of a snapshot.

use crate::model::layout::{Layout, OutputState};
use crate::model::validate::validate;
use crate::model::{Output, Reflection, Snapshot};

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
        let current = out.active.as_ref().map(|a| a.xid);
        let rows: Vec<Vec<String>> = out
            .resolutions()
            .iter()
            .map(|r| {
                let rates = out.rates(r.width, r.height).into_iter().map(|m| {
                    let mut token = m.rate_label();
                    if Some(m.xid) == current {
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
    let rest: Vec<&str> = snap
        .outputs
        .iter()
        .filter(|o| !o.is_relevant())
        .map(|o| o.name.as_str())
        .collect();
    if !rest.is_empty() {
        text.push_str(&format!("\nnot connected: {}\n", rest.join(" ")));
    }
    text
}

/// `rotation`, plus a reflection and a scale badge when set: `left, reflect x ×1.5`.
fn orientation(st: &OutputState) -> String {
    let mut text = st.rotation.to_string();
    if st.reflection != Reflection::Normal {
        text.push_str(&format!(", reflect {}", st.reflection));
    }
    match st.transform.scale_factors() {
        Some((sx, sy)) if (sx - sy).abs() < 1e-6 && (sx - 1.0).abs() > 1e-6 => {
            text.push_str(&format!(" ×{sx}"))
        }
        Some((sx, sy)) if (sx - sy).abs() >= 1e-6 => text.push_str(&format!(" ×{sx}x{sy}")),
        Some(_) => {}
        None => text.push_str(" transformed"),
    }
    text
}

/// `show`: a summary line, one row per numbered output with its inferred link, then the
/// validation issues of the live layout.
pub fn table(snap: &Snapshot) -> String {
    let layout = Layout::inferred(snap);
    let numbered = snap.numbered();
    let on = numbered.iter().filter(|&&i| layout.is_enabled(i)).count();
    let s = snap.screen;
    let mut text = format!(
        "outlay · {on} on · {} off · screen {}x{} of max {}x{}\n\n",
        numbered.len() - on,
        s.current.w,
        s.current.h,
        s.max.w,
        s.max.h
    );
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
                let mode = if out.mode(mode.xid).is_some() {
                    format!("{}x{} @ {:.2}", mode.width, mode.height, mode.refresh)
                } else {
                    format!("{}x{}", mode.width, mode.height)
                };
                [
                    mode,
                    format!("{},{}", st.pos.x, st.pos.y),
                    orientation(st),
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
