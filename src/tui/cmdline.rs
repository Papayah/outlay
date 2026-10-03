//! The `:` command line: parsing only. The app runs the parsed command.

use crate::model::layout::{Layout, MAX_SCALE, MIN_SCALE};
use crate::model::links::{Align, Side};
use crate::model::{Cap, Caps, Reflection, Rotation};

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
    /// `:scale F`, or `:scale P%` (Wayland), as a factor.
    Scale {
        factor: f64,
        percent: bool,
    },
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
    /// `:w [name]`
    Save(Option<String>),
    /// `:e [name]`
    Open(Option<String>),
    /// `:apply`
    Apply,
}

/// The command words, with what the display server needs for them, for error messages and
/// completion.
pub const WORDS: &[(&str, Option<Cap>)] = &[
    ("pos", None),
    ("move", None),
    ("mode", None),
    ("rate", None),
    ("rotate", None),
    ("reflect", None),
    ("scale", None),
    ("stick", None),
    ("unstick", None),
    ("primary", Some(Cap::Primary)),
    ("on", None),
    ("off", None),
    ("w", None),
    ("e", None),
    ("apply", None),
    ("q", None),
    ("q!", None),
];

/// One command in help.
#[derive(Clone, Copy, Debug)]
pub struct Usage {
    pub command: &'static str,
    pub help: &'static str,
    /// What the display server needs for all of the command.
    pub needs: Option<Cap>,
    /// Without that: `None` leaves the command out, `Some` shows this command and help instead.
    pub without: Option<(&'static str, &'static str)>,
    /// What `outlay keys`, which knows no display server, adds to the help.
    pub note: &'static str,
}

const fn usage_row(command: &'static str, help: &'static str) -> Usage {
    Usage {
        command,
        help,
        needs: None,
        without: None,
        note: "",
    }
}

/// Every command with what it does, for help.
pub const USAGE: &[Usage] = &[
    usage_row(":pos X Y", "Move the focused display to X,Y"),
    usage_row(":move DX DY", "Move it by DX,DY"),
    usage_row(
        ":mode WxH[@R]",
        "Set the resolution, and the rate nearest to R",
    ),
    usage_row(":rate R", "Set the rate nearest to R"),
    usage_row(":rotate normal|left|right|inverted", "Rotate"),
    Usage {
        needs: Some(Cap::AllReflections),
        without: Some((":reflect normal|x", "Reflect")),
        note: "(y, xy: X11)",
        ..usage_row(":reflect normal|x|y|xy", "Reflect")
    },
    usage_row(":scale F", "Set the scale, 0.25 to 8; on Wayland also 150%"),
    Usage {
        needs: Some(Cap::Mirror),
        without: Some((
            ":stick A SIDE B [start|center|end]",
            "SIDE: left-of right-of above below",
        )),
        note: "(same-as: X11)",
        ..usage_row(
            ":stick A SIDE B [start|center|end]",
            "SIDE: left-of right-of above below same-as",
        )
    },
    usage_row(":unstick [OUT]", "Unstick"),
    Usage {
        needs: Some(Cap::Primary),
        note: "(X11)",
        ..usage_row(":primary [OUT]", "Make primary")
    },
    usage_row(":on [OUT]  :off [OUT]", "Turn on or off"),
    usage_row(
        ":w [NAME]",
        "Save as a profile (the one last opened, without NAME)",
    ),
    usage_row(":e [NAME]", "Open a profile; without NAME, the picker"),
    usage_row(":apply", "Apply, like a"),
    usage_row(":q  :q!", "Quit; :q! drops pending changes"),
];

/// The commands as help shows them: with `caps`, as they work there; without, all of them, with
/// a note on what only X11 has.
pub fn usage(caps: Option<&Caps>) -> Vec<(String, String)> {
    USAGE
        .iter()
        .filter_map(|u| {
            let (command, help) = match (u.needs, caps) {
                (None, _) => (u.command, u.help.to_owned()),
                (Some(_), None) => (u.command, format!("{} {}", u.help, u.note)),
                (Some(cap), Some(caps)) if caps.has(cap) => (u.command, u.help.to_owned()),
                (Some(_), Some(_)) => {
                    let (command, help) = u.without?;
                    (command, help.to_owned())
                }
            };
            Some((command.to_owned(), help))
        })
        .collect()
}

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
        "scale" => {
            let usage = || format!("usage: :scale F, from {MIN_SCALE} to {MAX_SCALE}");
            let [s] = args else {
                return Err(usage());
            };
            let (number, percent) = match s.strip_suffix('%') {
                Some(n) => (n, true),
                None => (*s, false),
            };
            let value = number
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| format!("{s:?} is not a scale such as 1.5"))?;
            let factor = if percent { value / 100.0 } else { value };
            if !(MIN_SCALE..=MAX_SCALE).contains(&factor) {
                return Err(usage());
            }
            Ok(Cmd::Scale { factor, percent })
        }
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
        "w" => optional_output("w [NAME]").map(Cmd::Save),
        "e" => optional_output("e [NAME]").map(Cmd::Open),
        "apply" if args.is_empty() => Ok(Cmd::Apply),
        "q" | "q!" if args.is_empty() => Ok(Cmd::Quit {
            force: verb == "q!",
        }),
        _ => Err(format!(
            "unknown command {verb:?}; commands: {}",
            WORDS.iter().map(|(w, _)| *w).collect::<Vec<_>>().join(" ")
        )),
    }
}

/// Completes the last word of a command line: a command word, then output names, sides,
/// alignments or rotations, as the command takes them. Words the display server cannot use
/// (`primary`, `same-as`, `y`, `xy` on Wayland) are not offered. Returns the new line and, when
/// more than one word fits, the candidates. A single fit is completed with a space after it.
pub fn complete(line: &str, layout: &Layout) -> (String, Vec<String>) {
    let caps = layout.caps;
    let words: Vec<&str> = line.split_whitespace().collect();
    let fresh = line.is_empty() || line.ends_with(char::is_whitespace);
    let (done, partial) = match words.split_last() {
        Some((last, rest)) if !fresh => (rest, *last),
        _ => (&words[..], ""),
    };
    let outputs = || -> Vec<String> {
        (0..layout.len())
            .filter(|&i| layout.numbers[i].is_some())
            .map(|i| layout.names[i].clone())
            .collect()
    };
    let owned = |list: &[&str]| list.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    let candidates: Vec<String> = match done {
        [] => WORDS
            .iter()
            .filter(|(_, needs)| needs.is_none_or(|cap| caps.has(cap)))
            .map(|(w, _)| (*w).to_owned())
            .collect(),
        [verb, rest @ ..] => match (*verb, rest.len()) {
            ("primary", 0) if !caps.primary => Vec::new(),
            ("stick", 0 | 2) | ("unstick" | "primary" | "on" | "off", 0) => outputs(),
            ("stick", 1) if caps.mirror => {
                owned(&["left-of", "right-of", "above", "below", "same-as"])
            }
            ("stick", 1) => owned(&["left-of", "right-of", "above", "below"]),
            ("stick", 3) => owned(&["start", "center", "end"]),
            ("rotate", 0) => Rotation::ALL
                .iter()
                .map(|r| r.as_str().to_owned())
                .collect(),
            ("reflect", 0) => Reflection::ALL
                .iter()
                .filter(|r| caps.all_reflections || matches!(r, Reflection::Normal | Reflection::X))
                .map(|r| r.as_str().to_owned())
                .collect(),
            _ => Vec::new(),
        },
    };
    let lower = partial.to_lowercase();
    let fits: Vec<String> = candidates
        .into_iter()
        .filter(|c| c.to_lowercase().starts_with(&lower))
        .collect();
    let head = &line[..line.len() - partial.len()];
    match fits.as_slice() {
        [] => (line.to_owned(), Vec::new()),
        [one] => (format!("{head}{one} "), Vec::new()),
        many => {
            let first = &many[0];
            let common = many.iter().skip(1).fold(first.len(), |n, w| {
                first
                    .char_indices()
                    .zip(w.chars())
                    .take_while(|((_, a), b)| a.eq_ignore_ascii_case(b))
                    .last()
                    .map_or(0, |((k, a), _)| (k + a.len_utf8()).min(n))
            });
            let prefix = if common >= partial.len() {
                &first[..common]
            } else {
                partial
            };
            (format!("{head}{prefix}"), many.to_vec())
        }
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
        let scale = |factor: f64, percent: bool| Ok(Cmd::Scale { factor, percent });
        assert_eq!(parse("scale 1"), scale(1.0, false));
        assert_eq!(parse("scale 1.25"), scale(1.25, false));
        assert_eq!(parse("scale 150%"), scale(1.5, true));
        assert_eq!(parse("scale 0.25"), scale(0.25, false));
        assert_eq!(parse("scale 8"), scale(8.0, false));
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
        assert_eq!(parse("w"), Ok(Cmd::Save(None)));
        assert_eq!(parse("w home"), Ok(Cmd::Save(Some("home".to_owned()))));
        assert_eq!(
            parse("e tv-home"),
            Ok(Cmd::Open(Some("tv-home".to_owned())))
        );
        assert_eq!(parse("e"), Ok(Cmd::Open(None)));
        assert_eq!(parse("apply"), Ok(Cmd::Apply));
    }

    #[test]
    fn completes_words_outputs_and_keywords() {
        use crate::model::layout::Layout;
        let snap = crate::xrandr::parse_verbose(crate::xrandr::DEMO).unwrap();
        let layout = Layout::inferred(&snap);
        let c = |line: &str| complete(line, &layout);
        assert_eq!(c("sti"), ("stick ".to_owned(), vec![]));
        assert_eq!(c("r").1, ["rate", "rotate", "reflect"]);
        assert_eq!(c("r").0, "r");
        assert_eq!(c("re"), ("reflect ".to_owned(), vec![]));
        assert_eq!(c("stick h"), ("stick HDMI-1-0 ".to_owned(), vec![]));
        assert_eq!(
            c("stick 3 below dp"),
            (
                "stick 3 below DP-1-".to_owned(),
                vec!["DP-1-2".to_owned(), "DP-1-3".to_owned()]
            )
        );
        assert_eq!(
            c("stick eDP-1 ri"),
            ("stick eDP-1 right-of ".to_owned(), vec![])
        );
        assert_eq!(
            c("stick 3 below 2 c"),
            ("stick 3 below 2 center ".to_owned(), vec![])
        );
        assert_eq!(c("rotate l"), ("rotate left ".to_owned(), vec![]));
        assert_eq!(c("off ").1.len(), 4, "every numbered output");
        assert_eq!(
            c("pos 1"),
            ("pos 1".to_owned(), vec![]),
            "nothing to complete"
        );
        assert_eq!(c("").1.len(), WORDS.len());
    }

    #[test]
    fn wayland_completes_only_what_it_can_do() {
        use crate::model::layout::Layout;
        let snap = crate::wayland::capture::parse(crate::wayland::DEMO).unwrap();
        let layout = Layout::inferred(&snap);
        let c = |line: &str| complete(line, &layout);
        assert_eq!(c("").1.len(), WORDS.len() - 1);
        assert!(!c("").1.contains(&"primary".to_owned()));
        assert_eq!(c("p"), ("pos ".to_owned(), vec![]));
        assert_eq!(c("primary ").1, Vec::<String>::new());
        assert_eq!(c("stick 1 ").1, ["left-of", "right-of", "above", "below"]);
        assert_eq!(c("reflect ").1, ["normal", "x"]);

        let rows = usage(Some(&crate::model::Caps::wayland()));
        assert_eq!(rows.len(), USAGE.len() - 1, "no :primary");
        assert!(rows.contains(&(":reflect normal|x".to_owned(), "Reflect".to_owned())));
        assert!(rows.iter().all(|(_, h)| !h.contains("same-as")));
        let all = usage(None);
        assert!(all.contains(&(":primary [OUT]".to_owned(), "Make primary (X11)".to_owned())));
        assert!(
            all.iter()
                .any(|(_, h)| h.ends_with("same-as (same-as: X11)"))
        );
        assert_eq!(
            usage(Some(&crate::model::Caps::x11())),
            USAGE
                .iter()
                .map(|u| (u.command.to_owned(), u.help.to_owned()))
                .collect::<Vec<_>>()
        );
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
        assert_eq!(
            parse("scale 9"),
            Err("usage: :scale F, from 0.25 to 8".to_owned())
        );
        assert!(parse("scale 10%").unwrap_err().starts_with("usage"));
        assert!(parse("scale big").unwrap_err().contains("not a scale"));
        assert!(parse("scale").unwrap_err().starts_with("usage"));
        assert!(parse("stick 1 beside 2").unwrap_err().starts_with("usage"));
        assert!(
            parse("frobnicate")
                .unwrap_err()
                .starts_with("unknown command")
        );
        assert_eq!(parse(""), Err("empty command".to_owned()));
    }
}
