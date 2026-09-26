//! The `:` command line: parsing only. The app runs the parsed command.

use crate::model::layout::Layout;
use crate::model::links::{Align, Side};
use crate::model::{Reflection, Rotation};

#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    /// `:pos X Y`
    Pos(i32, i32),
    /// `:move DX DY`
    Move(i32, i32),
    /// `:mode WxH[@R]`
    Mode {
        w: i32,
        h: i32,
        rate: Option<f64>,
    },
    /// `:rate R`
    Rate(f64),
    Rotate(Rotation),
    Reflect(Reflection),
    /// `:scale 1`: editing the scale is left for a later version.
    ResetScale,
    /// `:stick A left-of|right-of|above|below|same-as B [start|center|end]`
    Stick {
        child: String,
        side: Side,
        parent: String,
        align: Option<Align>,
    },
    Unstick(Option<String>),
    Primary(Option<String>),
    On(Option<String>),
    Off(Option<String>),
    /// `:q`, or `:q!` to skip the question about pending changes.
    Quit {
        force: bool,
    },
}

/// The command words, for error messages.
pub const WORDS: &[&str] = &[
    "pos", "move", "mode", "rate", "rotate", "reflect", "scale", "stick", "unstick", "primary",
    "on", "off", "q", "q!",
];

/// Every command with what it does, for help.
pub const USAGE: &[(&str, &str)] = &[
    (":pos X Y", "Move the focused display to X,Y"),
    (":move DX DY", "Move it by DX,DY"),
    (
        ":mode WxH[@R]",
        "Set the resolution, and the rate nearest to R",
    ),
    (":rate R", "Set the rate nearest to R"),
    (":rotate normal|left|right|inverted", "Rotate"),
    (":reflect normal|x|y|xy", "Reflect"),
    (":scale 1", "Reset the scale"),
    (
        ":stick A SIDE B [start|center|end]",
        "SIDE: left-of right-of above below same-as",
    ),
    (":unstick [OUT]", "Unstick"),
    (":primary [OUT]", "Make primary"),
    (":on [OUT]  :off [OUT]", "Turn on or off"),
    (":q  :q!", "Quit; :q! drops pending changes"),
];

pub fn parse(line: &str) -> Result<Cmd, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some((&verb, args)) = words.split_first() else {
        return Err("empty command".to_owned());
    };
    let int = |s: &str| {
        s.parse::<i32>()
            .map_err(|_| format!("{s:?} is not a whole number"))
    };
    let rate = |s: &str| {
        s.parse::<f64>()
            .ok()
            .filter(|r| r.is_finite() && *r > 0.0)
            .ok_or_else(|| format!("{s:?} is not a refresh rate"))
    };
    let optional_output = |usage: &str| match args {
        [] => Ok(None),
        [out] => Ok(Some((*out).to_owned())),
        _ => Err(format!("usage: :{usage}")),
    };
    match verb {
        "pos" | "move" => {
            let [x, y] = args else {
                return Err(format!(
                    "usage: :{verb} {}",
                    if verb == "pos" { "X Y" } else { "DX DY" }
                ));
            };
            let (x, y) = (int(x)?, int(y)?);
            Ok(if verb == "pos" {
                Cmd::Pos(x, y)
            } else {
                Cmd::Move(x, y)
            })
        }
        "mode" => {
            let [spec] = args else {
                return Err("usage: :mode WxH[@RATE]".to_owned());
            };
            let (size, r) = match spec.split_once('@') {
                Some((size, r)) => (size, Some(rate(r)?)),
                None => (*spec, None),
            };
            let (w, h) = size
                .split_once('x')
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                .filter(|&(w, h): &(i32, i32)| w > 0 && h > 0)
                .ok_or_else(|| format!("{size:?} is not a resolution such as 1920x1080"))?;
            Ok(Cmd::Mode { w, h, rate: r })
        }
        "rate" => match args {
            [r] => Ok(Cmd::Rate(rate(r)?)),
            _ => Err("usage: :rate RATE".to_owned()),
        },
        "rotate" => match args {
            [r] => Rotation::parse(r)
                .map(Cmd::Rotate)
                .ok_or_else(|| "usage: :rotate normal|left|right|inverted".to_owned()),
            _ => Err("usage: :rotate normal|left|right|inverted".to_owned()),
        },
        "reflect" => match args {
            [r] => Reflection::parse(r)
                .map(Cmd::Reflect)
                .ok_or_else(|| "usage: :reflect normal|x|y|xy".to_owned()),
            _ => Err("usage: :reflect normal|x|y|xy".to_owned()),
        },
        "scale" => match args {
            [s] if s.parse::<f64>().is_ok_and(|v| (v - 1.0).abs() < 1e-9) => Ok(Cmd::ResetScale),
            _ => Err(":scale 1 resets the scale; other scales are not supported yet".to_owned()),
        },
        "stick" => {
            let usage = || {
                "usage: :stick A left-of|right-of|above|below|same-as B [start|center|end]"
                    .to_owned()
            };
            let (child, side, parent, align) = match args {
                [c, s, p] => (c, s, p, None),
                [c, s, p, a] => (c, s, p, Some(a)),
                _ => return Err(usage()),
            };
            let side = match *side {
                "left-of" => Side::LeftOf,
                "right-of" => Side::RightOf,
                "above" => Side::Above,
                "below" => Side::Below,
                "same-as" => Side::Same,
                _ => return Err(usage()),
            };
            let align = match align.copied() {
                None => None,
                Some("start") => Some(Align::Start),
                Some("center" | "centre") => Some(Align::Center),
                Some("end") => Some(Align::End),
                Some(_) => return Err(usage()),
            };
            Ok(Cmd::Stick {
                child: (*child).to_owned(),
                side,
                parent: (*parent).to_owned(),
                align,
            })
        }
        "unstick" => optional_output("unstick [OUTPUT]").map(Cmd::Unstick),
        "primary" => optional_output("primary [OUTPUT]").map(Cmd::Primary),
        "on" => optional_output("on [OUTPUT]").map(Cmd::On),
        "off" => optional_output("off [OUTPUT]").map(Cmd::Off),
        "q" | "q!" if args.is_empty() => Ok(Cmd::Quit {
            force: verb == "q!",
        }),
        _ => Err(format!(
            "unknown command {verb:?}; commands: {}",
            WORDS.join(" ")
        )),
    }
}

/// Finds an output by display number, exact name, or a unique name prefix (any case).
pub fn resolve_output(layout: &Layout, token: &str) -> Result<usize, String> {
    let numbered: Vec<usize> = (0..layout.len())
        .filter(|&i| layout.numbers[i].is_some())
        .collect();
    if let Ok(n) = token.parse::<usize>() {
        return numbered
            .iter()
            .copied()
            .find(|&i| layout.numbers[i] == Some(n))
            .ok_or_else(|| format!("there is no display {n}"));
    }
    if let Some(i) = numbered.iter().copied().find(|&i| layout.names[i] == token) {
        return Ok(i);
    }
    let lower = token.to_lowercase();
    let matches: Vec<usize> = numbered
        .into_iter()
        .filter(|&i| layout.names[i].to_lowercase().starts_with(&lower))
        .collect();
    match matches[..] {
        [i] => Ok(i),
        [] => Err(format!("there is no output {token:?}")),
        _ => {
            let names: Vec<&str> = matches.iter().map(|&i| layout.names[i].as_str()).collect();
            Err(format!("{token:?} could be {}", names.join(" or ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_command() {
        assert_eq!(parse("pos 10 -20"), Ok(Cmd::Pos(10, -20)));
        assert_eq!(parse(" move  0 5 "), Ok(Cmd::Move(0, 5)));
        assert_eq!(
            parse("mode 2560x1440@143.91"),
            Ok(Cmd::Mode {
                w: 2560,
                h: 1440,
                rate: Some(143.91)
            })
        );
        assert_eq!(
            parse("mode 1920x1080"),
            Ok(Cmd::Mode {
                w: 1920,
                h: 1080,
                rate: None
            })
        );
        assert_eq!(parse("rate 60"), Ok(Cmd::Rate(60.0)));
        assert_eq!(parse("rotate left"), Ok(Cmd::Rotate(Rotation::Left)));
        assert_eq!(parse("reflect xy"), Ok(Cmd::Reflect(Reflection::XY)));
        assert_eq!(parse("scale 1"), Ok(Cmd::ResetScale));
        assert_eq!(parse("scale 1.0"), Ok(Cmd::ResetScale));
        assert_eq!(
            parse("stick 3 left-of DP-1-2 end"),
            Ok(Cmd::Stick {
                child: "3".to_owned(),
                side: Side::LeftOf,
                parent: "DP-1-2".to_owned(),
                align: Some(Align::End)
            })
        );
        assert_eq!(parse("unstick"), Ok(Cmd::Unstick(None)));
        assert_eq!(parse("on 4"), Ok(Cmd::On(Some("4".to_owned()))));
        assert_eq!(parse("off"), Ok(Cmd::Off(None)));
        assert_eq!(
            parse("primary eDP"),
            Ok(Cmd::Primary(Some("eDP".to_owned())))
        );
        assert_eq!(parse("q"), Ok(Cmd::Quit { force: false }));
        assert_eq!(parse("q!"), Ok(Cmd::Quit { force: true }));
    }

    #[test]
    fn explains_what_is_wrong() {
        assert_eq!(parse("pos 10"), Err("usage: :pos X Y".to_owned()));
        assert_eq!(
            parse("pos 10 x"),
            Err("\"x\" is not a whole number".to_owned())
        );
        assert!(parse("mode 1920").unwrap_err().contains("not a resolution"));
        assert!(parse("rate -1").unwrap_err().contains("not a refresh rate"));
        assert!(parse("scale 1.5").unwrap_err().contains("not supported"));
        assert!(parse("stick 1 beside 2").unwrap_err().starts_with("usage"));
        assert!(
            parse("frobnicate")
                .unwrap_err()
                .starts_with("unknown command")
        );
        assert_eq!(parse(""), Err("empty command".to_owned()));
    }
}
