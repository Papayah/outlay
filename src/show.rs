//! `outlay show` and `outlay list`: plain-text views of a snapshot.

use crate::model::{Output, Snapshot};

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

/// `show`: a summary line, then one row per numbered output.
pub fn table(snap: &Snapshot) -> String {
    let numbered = snap.numbered();
    let on = numbered
        .iter()
        .filter(|&&i| snap.outputs[i].active.is_some())
        .count();
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
        ["#", "output", "mode", "position", "rotation", "display"]
            .map(str::to_owned)
            .to_vec(),
    ];
    for (n, &i) in numbered.iter().enumerate() {
        let out = &snap.outputs[i];
        let name = if out.primary {
            format!("{} ★", out.name)
        } else {
            out.name.clone()
        };
        let (mode, pos, rotation) = match &out.active {
            None => ("off".to_owned(), String::new(), String::new()),
            Some(a) => {
                let mode = match out.current_mode() {
                    Some(m) => format!("{}x{} @ {:.2}", m.width, m.height, m.refresh),
                    None => format!("{}x{}", a.size.w, a.size.h),
                };
                let mut rotation = a.rotation.to_string();
                if a.reflection != Default::default() {
                    rotation.push_str(&format!(", reflect {}", a.reflection));
                }
                if let Some((sx, sy)) = a.transform.scale_factors() {
                    if (sx - sy).abs() < 1e-6 && (sx - 1.0).abs() > 1e-6 {
                        rotation.push_str(&format!(" ×{sx}"));
                    } else if (sx - sy).abs() >= 1e-6 {
                        rotation.push_str(&format!(" ×{sx}x{sy}"));
                    }
                } else {
                    rotation.push_str(" transformed");
                }
                (mode, format!("{},{}", a.pos.x, a.pos.y), rotation)
            }
        };
        rows.push(vec![
            (n + 1).to_string(),
            name,
            mode,
            pos,
            rotation,
            display_details(out),
        ]);
    }
    text.push_str(&columns(&rows, ""));
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
