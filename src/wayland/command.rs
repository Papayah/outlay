//! Plans as the equivalent `wlr-randr` call, for the confirm popup, `y` and `apply -n`. outlay
//! never runs it: it talks to the compositor itself. Pure.

use super::capture::mode_millihertz;
use super::transform_name;
use crate::backend::{Plan, Planned, shell_words};
use crate::model::Mode;

/// `1920x1080@60.000Hz`, or `1280x720` for a mode without a rate (headless outputs).
fn mode_arg(mode: &Mode) -> String {
    match mode_millihertz(mode) {
        0 => format!("{}x{}", mode.width, mode.height),
        mhz => format!(
            "{}x{}@{}.{:03}Hz",
            mode.width,
            mode.height,
            mhz / 1000,
            mhz % 1000
        ),
    }
}

/// The `--output` group for one planned head. Everything is spelled out: the compositor keeps
/// what a call leaves out, but a plan says what it wants.
fn output_args(planned: &Planned, args: &mut Vec<String>) {
    args.extend(["--output".to_owned(), planned.name.clone()]);
    let Some(on) = &planned.on else {
        args.push("--off".to_owned());
        return;
    };
    let flag = if on.mode.custom {
        "--custom-mode"
    } else {
        "--mode"
    };
    let scale = on.scaling.factor().unwrap_or(1.0);
    args.extend(
        [
            "--on",
            flag,
            &mode_arg(&on.mode),
            "--pos",
            &format!("{},{}", on.pos.x, on.pos.y),
            "--transform",
            transform_name(on.rotation, on.reflection),
            "--scale",
            &format!("{scale}"),
        ]
        .map(str::to_owned),
    );
}

/// The arguments of one `wlr-randr` call that carries out `plan`.
pub fn args(plan: &Plan) -> Vec<String> {
    let mut args = Vec::new();
    for planned in &plan.outputs {
        output_args(planned, &mut args);
    }
    args
}

/// `wlr-randr` followed by the arguments, quoted for a POSIX shell.
pub fn command_line(plan: &Plan) -> String {
    format!("wlr-randr {}", shell_words(&args(plan)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{On, PlanForm, PrimaryRule, ScalingChange};
    use crate::model::geometry::Point;
    use crate::model::{Reflection, Rotation, Scaling};
    use crate::wayland::capture::mode;

    fn on(mode: Mode, rotation: Rotation, reflection: Reflection, scale: f64) -> Option<On> {
        Some(On {
            mode,
            pos: Point::new(2480, 1440),
            rotation,
            reflection,
            scaling: Scaling::Logical(scale),
            scaling_change: ScalingChange::Keep,
            primary: false,
        })
    }

    #[test]
    fn plans_as_wlr_randr_arguments() {
        let plan = Plan {
            outputs: vec![
                Planned {
                    name: "eDP-1".to_owned(),
                    on: on(
                        mode(2880, 1800, 60_001, true, false),
                        Rotation::Normal,
                        Reflection::Normal,
                        2.0,
                    ),
                },
                Planned {
                    name: "DP-3".to_owned(),
                    on: on(
                        mode(2560, 1440, 143_912, false, false),
                        Rotation::Left,
                        Reflection::X,
                        1.25,
                    ),
                },
                Planned {
                    name: "HEADLESS-1".to_owned(),
                    on: on(
                        mode(1366, 768, 0, false, true),
                        Rotation::Normal,
                        Reflection::Y,
                        1.0,
                    ),
                },
                Planned {
                    name: "HDMI-A-1".to_owned(),
                    on: None,
                },
            ],
            primary: PrimaryRule::Clear,
            form: PlanForm::Apply,
        };
        assert_eq!(
            command_line(&plan),
            "wlr-randr --output eDP-1 --on --mode 2880x1800@60.001Hz --pos 2480,1440 \
             --transform normal --scale 2 \
             --output DP-3 --on --mode 2560x1440@143.912Hz --pos 2480,1440 \
             --transform flipped-90 --scale 1.25 \
             --output HEADLESS-1 --on --custom-mode 1366x768 --pos 2480,1440 \
             --transform flipped-180 --scale 1 \
             --output HDMI-A-1 --off"
        );
    }
}
