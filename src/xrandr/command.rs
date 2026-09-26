//! Layout → xrandr arguments and scripts. Pure: nothing here runs a program.
//!
//! The live apply and the revert name modes by XID (`--mode 0x4a`), which is exact. Scripts and
//! the copied command name them the portable way (`--mode 1920x1080 --rate 165.01`).

use crate::model::layout::Layout;
use crate::model::{Reflection, Snapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    /// `--mode 0xXID`
    Xid,
    /// `--mode NAME --rate R`
    NameRate,
    /// `NameRate`, arandr-style: no `--reflect normal`, a transform written as `--scale`.
    Script,
}

fn push(args: &mut Vec<String>, items: &[&str]) {
    args.extend(items.iter().map(|s| (*s).to_owned()));
}

/// The `--output` group for output `i`.
fn output_args(layout: &Layout, snap: &Snapshot, i: usize, form: Form, args: &mut Vec<String>) {
    let st = &layout.outputs[i];
    let name = layout.names[i].as_str();
    let mode = match (&st.mode, st.enabled) {
        (Some(mode), true) => mode,
        _ => {
            push(args, &["--output", name, "--off"]);
            return;
        }
    };
    push(args, &["--output", name]);
    if st.primary {
        push(args, &["--primary"]);
    }
    match form {
        Form::Xid => push(args, &["--mode", &format!("0x{:x}", mode.xid)]),
        Form::NameRate | Form::Script => push(
            args,
            &[
                "--mode",
                &mode.name,
                "--rate",
                &format!("{:.2}", mode.refresh),
            ],
        ),
    }
    push(
        args,
        &[
            "--pos",
            &format!("{}x{}", st.pos.x, st.pos.y),
            "--rotate",
            st.rotation.as_str(),
        ],
    );
    if form != Form::Script || st.reflection != Reflection::Normal {
        push(args, &["--reflect", st.reflection.as_str()]);
    }
    let live_scaled = snap.outputs[i]
        .active
        .as_ref()
        .is_some_and(|a| !a.transform.is_identity());
    match form {
        Form::Xid | Form::NameRate => {
            if live_scaled && st.transform.is_identity() {
                push(args, &["--transform", "none"]);
            }
        }
        Form::Script => {
            if !st.transform.is_identity() {
                match st.transform.scale_factors() {
                    Some((sx, sy)) => push(args, &["--scale", &format!("{sx}x{sy}")]),
                    None => push(args, &["--transform", &st.transform.xrandr_arg()]),
                }
            }
        }
    }
}

/// Arguments for an apply: every connected or active output in snapshot order, in one call.
/// Panning outputs are left out, since outlay v0.1 does not touch them.
fn pending_args(layout: &Layout, snap: &Snapshot, form: Form) -> Vec<String> {
    let mut args = Vec::new();
    for (i, out) in snap.outputs.iter().enumerate() {
        if (out.is_relevant() || layout.outputs[i].enabled) && !layout.locked[i] {
            output_args(layout, snap, i, form, &mut args);
        }
    }
    if layout.primary().is_none() {
        push(&mut args, &["--noprimary"]);
    }
    args
}

/// The arguments that apply `layout`, naming modes by XID.
pub fn apply_args(layout: &Layout, snap: &Snapshot) -> Vec<String> {
    pending_args(layout, snap, Form::Xid)
}

/// The same apply, with modes by name and rate: what `y` copies.
pub fn portable_args(layout: &Layout, snap: &Snapshot) -> Vec<String> {
    pending_args(layout, snap, Form::NameRate)
}

/// The arguments that restore `live`, the snapshot taken just before an apply. It covers every
/// connected or active output with an explicit mode, position, rotation, reflection and
/// transform, turns off the outputs that were off, and restores the primary.
pub fn revert_args(live: &Snapshot) -> Vec<String> {
    let mut args = Vec::new();
    for out in live
        .outputs
        .iter()
        .filter(|o| o.is_relevant() && !o.has_panning())
    {
        let Some(a) = &out.active else {
            push(&mut args, &["--output", &out.name, "--off"]);
            continue;
        };
        push(&mut args, &["--output", &out.name]);
        if out.primary {
            push(&mut args, &["--primary"]);
        }
        push(
            &mut args,
            &[
                "--mode",
                &format!("0x{:x}", a.xid),
                "--pos",
                &format!("{}x{}", a.pos.x, a.pos.y),
                "--rotate",
                a.rotation.as_str(),
                "--reflect",
                a.reflection.as_str(),
            ],
        );
        if a.transform.is_identity() {
            push(&mut args, &["--transform", "none"]);
        } else {
            push(&mut args, &["--transform", &a.transform.xrandr_arg()]);
            if !a.transform.filter.is_empty() {
                push(&mut args, &["--filter", &a.transform.filter]);
            }
        }
    }
    if !live.outputs.iter().any(|o| o.primary && o.active.is_some()) {
        push(&mut args, &["--noprimary"]);
    }
    args
}

/// An arandr-compatible screenlayout script for `layout`: every known output appears, the ones
/// that are off with `--off`, one `--output` per line.
pub fn script(layout: &Layout, snap: &Snapshot) -> String {
    let mut groups: Vec<Vec<String>> = Vec::new();
    for i in 0..snap.outputs.len() {
        let mut args = Vec::new();
        output_args(layout, snap, i, Form::Script, &mut args);
        groups.push(args);
    }
    let mut text = format!(
        "#!/bin/sh\n# generated by outlay {}\n",
        env!("CARGO_PKG_VERSION")
    );
    for (k, group) in groups.iter().enumerate() {
        text.push_str(if k == 0 { "xrandr " } else { "       " });
        text.push_str(&join(group));
        text.push_str(if k + 1 < groups.len() { " \\\n" } else { "\n" });
    }
    text
}

/// `xrandr` followed by the arguments, quoted for a POSIX shell.
pub fn command_line(args: &[String]) -> String {
    format!("xrandr {}", join(args))
}

fn join(args: &[String]) -> String {
    args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
}

/// Quotes an argument for `sh` when it contains anything beyond a safe set.
fn quote(arg: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "-_.,:/=+@%".contains(c);
    if !arg.is_empty() && arg.chars().all(safe) {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(quote("1920x1080_60.00"), "1920x1080_60.00");
        assert_eq!(quote("DP-1-2.1"), "DP-1-2.1");
        assert_eq!(quote("my mode"), "'my mode'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote(""), "''");
    }
}
