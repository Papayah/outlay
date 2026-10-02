//! Plans → xrandr arguments and scripts, and back. Pure: nothing here runs a program.
//!
//! The live apply and the revert name modes by XID (`--mode 0x4a`), which is exact. Scripts and
//! the copied command name them the portable way (`--mode 1920x1080 --rate 165.01`).

use crate::backend::{On, Plan, PlanForm, Planned, PrimaryRule, ScalingChange};
use crate::model::geometry::Point;
use crate::model::layout::Layout;
use crate::model::{ModeId, Reflection, Rotation, Scaling, Snapshot, Transform};

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

/// The `--output` group for one planned output.
fn output_args(planned: &Planned, plan: PlanForm, form: Form, args: &mut Vec<String>) {
    let name = planned.name.as_str();
    let Some(on) = &planned.on else {
        push(args, &["--output", name, "--off"]);
        return;
    };
    push(args, &["--output", name]);
    if on.primary {
        push(args, &["--primary"]);
    }
    let mode = &on.mode;
    match form {
        Form::Xid => push(args, &["--mode", &format!("0x{:x}", mode.id)]),
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
            &format!("{}x{}", on.pos.x, on.pos.y),
            "--rotate",
            on.rotation.as_str(),
        ],
    );
    if form != Form::Script || on.reflection != Reflection::Normal {
        push(args, &["--reflect", on.reflection.as_str()]);
    }
    match (plan, on.scaling_change) {
        // A restore spells the scaling out, whatever it is.
        (PlanForm::Restore, _) if on.scaling.is_identity() => {
            push(args, &["--transform", "none"]);
        }
        (PlanForm::Restore, _) => transform_args(&on.scaling, args),
        (PlanForm::Apply, ScalingChange::Keep) => {}
        (PlanForm::Apply, ScalingChange::Reset) if form != Form::Script => {
            push(args, &["--transform", "none"]);
        }
        (PlanForm::Apply, ScalingChange::Reset) => {}
        (PlanForm::Apply, ScalingChange::Set) => match on.scaling.scale_factors() {
            Some((sx, sy)) => push(args, &["--scale", &format!("{sx}x{sy}")]),
            None => transform_args(&on.scaling, args),
        },
    }
}

/// `--transform` with the matrix, then `--filter` when the transform has one.
fn transform_args(scaling: &Scaling, args: &mut Vec<String>) {
    match scaling {
        Scaling::X11(t) => {
            push(args, &["--transform", &t.xrandr_arg()]);
            if !t.filter.is_empty() {
                push(args, &["--filter", &t.filter]);
            }
        }
        Scaling::Logical(s) => push(args, &["--scale", &format!("{s}x{s}")]),
    }
}

/// The arguments of one xrandr call that carries out `plan`.
fn plan_args(plan: &Plan, form: Form) -> Vec<String> {
    let mut args = Vec::new();
    for planned in &plan.outputs {
        output_args(planned, plan.form, form, &mut args);
    }
    if plan.primary == PrimaryRule::Clear {
        push(&mut args, &["--noprimary"]);
    }
    args
}

/// The arguments that carry out `plan`, naming modes by XID: what the apply and the revert run.
pub fn argv(plan: &Plan) -> Vec<String> {
    plan_args(plan, Form::Xid)
}

/// The same call, with modes by name and rate: what `y` copies.
pub fn portable_argv(plan: &Plan) -> Vec<String> {
    plan_args(plan, Form::NameRate)
}

/// What one `--output` group asks for, before it is resolved against the snapshot.
#[derive(Default)]
struct Request {
    off: bool,
    auto: bool,
    mode: Option<String>,
    rate: Option<f64>,
    pos: Option<Point>,
    rotation: Option<Rotation>,
    reflection: Option<Reflection>,
    transform: Option<Transform>,
    filter: Option<String>,
}

/// The plan an xrandr call carries out on `snapshot`, the way xrandr reads its arguments: an
/// unknown output is planned with nothing set and its options are skipped; an unknown option, a
/// malformed value or a mode the output lacks is an error, in xrandr's words. What the call does
/// not mention keeps its live value. The plan has the `Apply` form.
#[doc(hidden)]
pub fn argv_to_plan(snapshot: &Snapshot, argv: &[String]) -> Result<Plan, String> {
    // Per output, in order of first mention; `None` for an unknown output.
    let mut requests: Vec<(String, Option<usize>, Request)> = Vec::new();
    // `Some(None)` for `--noprimary`.
    let mut primary: Option<Option<usize>> = None;
    // The request the options belong to.
    let mut current: Option<usize> = None;
    let mut args = argv.iter();
    while let Some(arg) = args.next() {
        let mut value = |what: &str| {
            args.next()
                .cloned()
                .ok_or_else(|| format!("xrandr: {arg} requires {what}\n"))
        };
        match arg.as_str() {
            "--output" => {
                let name = value("an output name")?;
                let index = snapshot.find(&name);
                current = Some(
                    match requests
                        .iter()
                        .position(|(_, i, _)| index.is_some() && *i == index)
                    {
                        Some(k) => k,
                        None => {
                            requests.push((name, index, Request::default()));
                            requests.len() - 1
                        }
                    },
                );
                continue;
            }
            "--noprimary" => {
                primary = Some(None);
                continue;
            }
            _ => {}
        }
        let Some(k) = current else {
            return Err(format!("xrandr: {arg} must follow --output\n"));
        };
        let target = requests[k].1;
        let req = &mut requests[k].2;
        let invalid = |what: &str, v: &str| format!("xrandr: invalid {what} {v}\n");
        match arg.as_str() {
            "--off" => req.off = true,
            "--auto" => req.auto = true,
            "--primary" => primary = Some(target),
            "--mode" => req.mode = Some(value("a mode")?),
            "--rate" | "--refresh" => {
                let v = value("a rate")?;
                req.rate = Some(v.parse().map_err(|_| invalid("rate", &v))?);
            }
            "--pos" => {
                let v = value("a position")?;
                let parsed = v
                    .split_once('x')
                    .and_then(|(x, y)| Some(Point::new(x.parse().ok()?, y.parse().ok()?)));
                req.pos = Some(parsed.ok_or_else(|| invalid("position", &v))?);
            }
            "--rotate" | "--orientation" => {
                let v = value("a rotation")?;
                req.rotation = Some(Rotation::parse(&v).ok_or_else(|| invalid("rotation", &v))?);
            }
            "--reflect" => {
                let v = value("a reflection")?;
                req.reflection =
                    Some(Reflection::parse(&v).ok_or_else(|| invalid("reflection", &v))?);
            }
            "--transform" => {
                let v = value("a transform")?;
                req.transform =
                    Some(super::parse_transform_arg(&v).ok_or_else(|| invalid("transform", &v))?);
            }
            "--scale" => {
                let v = value("a scale")?;
                let (sx, sy) = v.split_once('x').unwrap_or((v.as_str(), v.as_str()));
                let parsed = sx
                    .parse()
                    .ok()
                    .zip(sy.parse().ok())
                    .map(|(sx, sy)| Transform::scale(sx, sy));
                req.transform = Some(parsed.ok_or_else(|| invalid("scale", &v))?);
            }
            "--filter" => req.filter = Some(value("a filter")?),
            other => return Err(format!("xrandr: unrecognized option '{other}'\n")),
        }
    }

    let mut outputs = Vec::new();
    for (name, index, req) in requests {
        let Some(i) = index else {
            outputs.push(Planned { name, on: None });
            continue;
        };
        let on = if req.off {
            None
        } else {
            Some(resolve(snapshot, i, req, primary == Some(Some(i)))?)
        };
        outputs.push(Planned { name, on });
    }
    Ok(Plan {
        outputs,
        primary: if primary == Some(None) {
            PrimaryRule::Clear
        } else {
            PrimaryRule::Keep
        },
        form: PlanForm::Apply,
    })
}

/// One output's request, completed from its live state as xrandr completes it.
fn resolve(snapshot: &Snapshot, i: usize, req: Request, primary: bool) -> Result<On, String> {
    let out = &snapshot.outputs[i];
    let mode = match (&req.mode, req.auto) {
        (Some(m), _) => match m
            .strip_prefix("0x")
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
        {
            Some(xid) => out.mode(ModeId::from_xid(xid)),
            None => out.find_mode(m, req.rate),
        },
        (None, true) => out.preferred_mode(),
        (None, false) => out.current_mode().or_else(|| out.preferred_mode()),
    };
    let Some(mode) = mode.cloned() else {
        return Err(format!(
            "xrandr: cannot find mode {} for output {}\n",
            req.mode.unwrap_or_default(),
            out.name
        ));
    };
    let old = out.active.as_ref();
    let live = old.map(|a| a.scaling.clone());
    let (scaling, scaling_change) = if req.transform.is_none() && req.filter.is_none() {
        let kept = live.unwrap_or_else(|| Scaling::unit(snapshot.caps.kind));
        (kept, ScalingChange::Keep)
    } else {
        let mut transform = req
            .transform
            .or_else(|| live.as_ref().and_then(|s| s.transform().cloned()))
            .unwrap_or_default();
        if let Some(filter) = req.filter {
            transform.filter = filter;
        }
        if transform.is_identity() {
            (Scaling::X11(Transform::identity()), ScalingChange::Reset)
        } else {
            (Scaling::X11(transform), ScalingChange::Set)
        }
    };
    Ok(On {
        mode,
        pos: req.pos.or(old.map(|a| a.pos)).unwrap_or_default(),
        rotation: req.rotation.or(old.map(|a| a.rotation)).unwrap_or_default(),
        reflection: req
            .reflection
            .or(old.map(|a| a.reflection))
            .unwrap_or_default(),
        scaling,
        scaling_change,
        primary,
    })
}

/// The first line of a script outlay wrote, after the shebang.
pub fn script_header() -> String {
    format!("# generated by outlay {}", env!("CARGO_PKG_VERSION"))
}

/// An arandr-compatible screenlayout script for `layout`: every known output appears, the ones
/// that are off with `--off`, one `--output` per line.
pub fn script(layout: &Layout) -> String {
    format!(
        "#!/bin/sh\n{}\n{}\n",
        script_header(),
        script_command(layout)
    )
}

/// The xrandr call of [`script`], without a final newline: `xrandr --output … \` and one more
/// line per output.
pub fn script_command(layout: &Layout) -> String {
    let plan = Plan::whole(layout);
    let mut groups: Vec<Vec<String>> = Vec::new();
    for planned in &plan.outputs {
        let mut args = Vec::new();
        output_args(planned, plan.form, Form::Script, &mut args);
        groups.push(args);
    }
    let mut text = String::new();
    for (k, group) in groups.iter().enumerate() {
        text.push_str(if k == 0 { "xrandr " } else { "       " });
        text.push_str(&join(group));
        if k + 1 < groups.len() {
            text.push_str(" \\\n");
        }
    }
    text
}

/// The `revert.sh` written before an apply: running it carries out `restore`, which brings back
/// the layout that was live.
pub fn revert_script(restore: &Plan) -> String {
    format!(
        "#!/bin/sh\n# Written by outlay {} just before it applied a layout.\n\
         # Running it restores the layout that was live before that apply.\n{}\n",
        env!("CARGO_PKG_VERSION"),
        command_line(&argv(restore))
    )
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
