//! Screenlayout profiles: arandr-style `#!/bin/sh` scripts of xrandr calls, as kept in
//! `~/.screenlayout`. Parsing is lenient, like the shell running the script: what outlay does not
//! understand becomes a warning. A script reads into the neutral [`Profile`].

use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::model::geometry::Point;
use crate::model::links::Side;
pub use crate::model::profile::{Entry, Profile, Remap};
use crate::model::profile::{ModeRequest, Target};
use crate::model::{Reflection, Rotation, Scaling, Transform};

use super::command;

/// Options outlay reads.
const UNDERSTOOD: &[&str] = &[
    "--output",
    "--mode",
    "--rate",
    "--refresh",
    "--pos",
    "--rotate",
    "--orientation",
    "--reflect",
    "--primary",
    "--noprimary",
    "--off",
    "--auto",
    "--preferred",
    "--scale",
    "--transform",
    "--filter",
    "--same-as",
    "--left-of",
    "--right-of",
    "--above",
    "--below",
];

/// Options outlay ignores, with how many values each takes, so the values are skipped too.
const IGNORED: &[(&str, usize)] = &[
    ("--verbose", 0),
    ("-q", 0),
    ("--query", 0),
    ("--current", 0),
    ("--nograb", 0),
    ("--dryrun", 0),
    ("-display", 1),
    ("-d", 1),
    ("--display", 1),
    ("--screen", 1),
    ("-s", 1),
    ("--size", 1),
    ("--dpi", 1),
    ("--fb", 1),
    ("--fbmm", 1),
    ("--crtc", 1),
    ("--panning", 1),
    ("--gamma", 1),
    ("--brightness", 1),
    ("--scale-from", 1),
    ("--set", 2),
    ("--addmode", 2),
    ("--delmode", 2),
    ("--rmmode", 1),
    ("--delmonitor", 1),
    ("--setmonitor", 3),
    ("--setprovideroutputsource", 2),
    ("--setprovideroffloadsink", 2),
];

/// Splits a script into logical lines: byte ranges that together cover the whole text, each one
/// command line with its backslash-newline continuations and its line break.
pub fn logical_lines(text: &str) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    let (mut single, mut double, mut comment, mut escape) = (false, false, false, false);
    let mut word_start = true;
    for (i, c) in text.char_indices() {
        if escape {
            escape = false;
            word_start = false;
            continue;
        }
        if comment {
            if c == '\n' {
                comment = false;
                lines.push(start..i + 1);
                start = i + 1;
                word_start = true;
            }
            continue;
        }
        match c {
            '\\' if !single => escape = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '#' if !single && !double && word_start => comment = true,
            '\n' if !single && !double => {
                lines.push(start..i + 1);
                start = i + 1;
            }
            _ => {}
        }
        word_start = c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(');
    }
    if start < text.len() {
        lines.push(start..text.len());
    }
    lines
}

/// The commands of one logical line, split at `;`, `&&`, `||`, `|` and `&`.
fn commands(line: &str) -> Result<Vec<Vec<String>>, shell_words::ParseError> {
    let mut out = vec![Vec::new()];
    for word in shell_words::split(line)? {
        if matches!(word.as_str(), ";" | "&&" | "||" | "|" | "&") {
            out.push(Vec::new());
            continue;
        }
        match word.strip_suffix(';') {
            Some(rest) => {
                if !rest.is_empty() {
                    out.last_mut().expect("never empty").push(rest.to_owned());
                }
                out.push(Vec::new());
            }
            None => out.last_mut().expect("never empty").push(word),
        }
    }
    out.retain(|c| !c.is_empty());
    Ok(out)
}

/// The arguments of an xrandr call, or `None` when the command runs something else. Leading
/// variable assignments and `exec` are skipped.
fn xrandr_args(command: &[String]) -> Option<&[String]> {
    let is_assignment = |w: &str| {
        w.split_once('=').is_some_and(|(name, _)| {
            !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit())
        })
    };
    let first = command
        .iter()
        .position(|w| !is_assignment(w) && w != "exec")?;
    let program = &command[first];
    (program == "xrandr" || program.ends_with("/xrandr")).then(|| &command[first + 1..])
}

/// Whether a logical line calls xrandr.
fn calls_xrandr(line: &str) -> bool {
    commands(line).is_ok_and(|cmds| cmds.iter().any(|c| xrandr_args(c).is_some()))
}

fn parse_pos(v: &str) -> Option<Point> {
    let (x, y) = v.split_once('x')?;
    Some(Point::new(x.parse().ok()?, y.parse().ok()?))
}

/// `--mode`: an XID such as `0x1c3`, else a mode name.
fn parse_mode(v: &str) -> ModeRequest {
    match v
        .strip_prefix("0x")
        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
    {
        Some(xid) => ModeRequest::Xid(xid),
        None => ModeRequest::Name(v.to_owned()),
    }
}

fn parse_scale(v: &str) -> Option<Transform> {
    let (sx, sy) = v.split_once('x').unwrap_or((v, v));
    let (sx, sy): (f64, f64) = (sx.parse().ok()?, sy.parse().ok()?);
    (sx > 0.0 && sy > 0.0).then(|| {
        if (sx - 1.0).abs() < 1e-9 && (sy - 1.0).abs() < 1e-9 {
            Transform::identity()
        } else {
            Transform::scale(sx, sy)
        }
    })
}

impl Profile {
    /// Parses a screenlayout script. It never fails: what cannot be read becomes a warning.
    pub fn parse(text: &str) -> Profile {
        let mut profile = Profile::default();
        let mut calls = 0;
        for range in logical_lines(text) {
            let line = &text[range];
            let cmds = match commands(line) {
                Ok(cmds) => cmds,
                Err(_) => {
                    profile
                        .warnings
                        .push(format!("Unbalanced quotes in: {}", line.trim()));
                    continue;
                }
            };
            for cmd in &cmds {
                if let Some(args) = xrandr_args(cmd) {
                    calls += 1;
                    profile.read_call(args);
                }
            }
        }
        if calls == 0 {
            profile
                .warnings
                .push("The script does not call xrandr.".to_owned());
        }
        profile
    }

    fn entry(&mut self, name: &str) -> &mut Entry {
        let named = |e: &Entry| matches!(&e.target, Target::Name(n) if n == name);
        match self.entries.iter().position(named) {
            Some(k) => &mut self.entries[k],
            None => {
                self.entries.push(Entry::named(name));
                self.entries.last_mut().expect("just pushed")
            }
        }
    }

    fn read_call(&mut self, args: &[String]) {
        let mut current: Option<String> = None;
        let mut k = 0;
        while k < args.len() {
            let arg = args[k].as_str();
            k += 1;
            if let Some(&(_, arity)) = IGNORED.iter().find(|(o, _)| *o == arg) {
                self.warnings
                    .push(format!("{arg} is not supported; ignored."));
                k += arity;
                continue;
            }
            if !UNDERSTOOD.contains(&arg) {
                self.warnings
                    .push(format!("Unknown option {arg}; ignored."));
                continue;
            }
            let takes_value = !matches!(
                arg,
                "--primary" | "--noprimary" | "--off" | "--auto" | "--preferred"
            );
            let value = if takes_value {
                let Some(v) = args.get(k) else {
                    self.warnings.push(format!("{arg} is missing its value."));
                    break;
                };
                k += 1;
                v.clone()
            } else {
                String::new()
            };
            match arg {
                "--output" => {
                    self.entry(&value);
                    current = Some(value);
                    continue;
                }
                "--noprimary" => {
                    self.primary = Some(None);
                    continue;
                }
                _ => {}
            }
            let Some(name) = current.clone() else {
                self.warnings
                    .push(format!("{arg} comes before any --output; ignored."));
                continue;
            };
            let mut invalid = None;
            {
                let e = self.entry(&name);
                match arg {
                    "--off" => {
                        e.off = true;
                        e.mode = None;
                        e.rate = None;
                    }
                    "--auto" | "--preferred" => {
                        e.off = false;
                        e.mode = Some(ModeRequest::Preferred);
                    }
                    "--primary" => {}
                    "--mode" => {
                        e.off = false;
                        e.mode = Some(parse_mode(&value));
                    }
                    "--rate" | "--refresh" => match value.parse::<f64>() {
                        Ok(r) if r.is_finite() && r > 0.0 => e.rate = Some(r),
                        _ => invalid = Some("rate"),
                    },
                    "--pos" => match parse_pos(&value) {
                        Some(p) => {
                            e.pos = Some(p);
                            e.relative = None;
                        }
                        None => invalid = Some("position"),
                    },
                    "--rotate" | "--orientation" => match Rotation::parse(&value) {
                        Some(r) => e.rotation = Some(r),
                        None => invalid = Some("rotation"),
                    },
                    "--reflect" => match Reflection::parse(&value) {
                        Some(r) => e.reflection = Some(r),
                        None => invalid = Some("reflection"),
                    },
                    "--scale" => match parse_scale(&value) {
                        Some(t) => e.scaling = Some(Scaling::X11(t)),
                        None => invalid = Some("scale"),
                    },
                    "--transform" => match super::parse_transform_arg(&value) {
                        Some(t) => e.scaling = Some(Scaling::X11(t)),
                        None => invalid = Some("transform"),
                    },
                    "--filter" => e.filter = Some(value.clone()),
                    relative => {
                        let side = match relative {
                            "--left-of" => Side::LeftOf,
                            "--right-of" => Side::RightOf,
                            "--above" => Side::Above,
                            "--below" => Side::Below,
                            _ => Side::Same,
                        };
                        e.relative = Some((side, value.clone()));
                        e.pos = None;
                    }
                }
            }
            if arg == "--primary" {
                self.primary = Some(Some(name.clone()));
            }
            if let Some(what) = invalid {
                self.warnings
                    .push(format!("{name}: {value:?} is not a valid {what}; ignored."));
            }
        }
    }
}

/// `<dir>/<name>.sh`, or `name` itself when it contains `/` or ends in `.sh`.
pub fn profile_path(dir: &Path, name: &str) -> PathBuf {
    if name.contains('/') || name.ends_with(".sh") {
        PathBuf::from(name)
    } else {
        dir.join(format!("{name}.sh"))
    }
}

/// The profile name of a path: its file name without `.sh`.
pub fn profile_name(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.strip_suffix(".sh").unwrap_or(&name).to_owned()
}

/// The scripts in `dir`, sorted by name.
pub fn list_profiles(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "sh"))
        .collect();
    paths.sort();
    Ok(paths)
}

/// The text to save: a fresh script, or `old` with its xrandr call replaced by `command` and
/// every other line kept byte for byte. The `# generated by outlay` line is added or updated.
pub fn save_text(old: Option<&str>, command: &str) -> String {
    let header = command::script_header();
    let Some(old) = old else {
        return format!("#!/bin/sh\n{header}\n{command}\n");
    };
    let is_header = |line: &str| line.trim_end().starts_with("# generated by outlay");
    let mut out = String::new();
    let mut replaced = false;
    let mut has_header = false;
    let lines = logical_lines(old);
    for (k, range) in lines.iter().enumerate() {
        let line = &old[range.clone()];
        let newline = if line.ends_with('\n') { "\n" } else { "" };
        if is_header(line) {
            if !has_header {
                out.push_str(&header);
                out.push_str(newline);
            }
            has_header = true;
            continue;
        }
        if calls_xrandr(line) {
            if !replaced {
                if !has_header {
                    out.push_str(&header);
                    out.push('\n');
                    has_header = true;
                }
                out.push_str(command);
                out.push('\n');
                replaced = true;
            }
            continue;
        }
        out.push_str(line);
        if k == 0 && line.starts_with("#!") && !newline.is_empty() {
            // The header goes right after the shebang, unless the file already has one.
            let later = lines[1..].iter().any(|r| is_header(&old[r.clone()]));
            if !later {
                out.push_str(&header);
                out.push('\n');
                has_header = true;
            }
        }
    }
    if !replaced {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !has_header {
            out.push_str(&header);
            out.push('\n');
        }
        out.push_str(command);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_lines_cover_the_text_and_join_continuations() {
        let text = "#!/bin/sh\nxrandr --output A \\\n  --off # done\n\necho 'a\nb'\nlast";
        let lines = logical_lines(text);
        let parts: Vec<&str> = lines.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(
            parts,
            [
                "#!/bin/sh\n",
                "xrandr --output A \\\n  --off # done\n",
                "\n",
                "echo 'a\nb'\n",
                "last"
            ]
        );
        assert_eq!(parts.concat(), text);
    }

    #[test]
    fn finds_xrandr_calls_among_other_commands() {
        assert!(calls_xrandr("xrandr --output A --off\n"));
        assert!(calls_xrandr("/usr/bin/xrandr --auto"));
        assert!(calls_xrandr("DISPLAY=:0 exec xrandr --output A --off"));
        assert!(calls_xrandr("sleep 1; xrandr --output A --off"));
        assert!(!calls_xrandr("# xrandr --output A --off"));
        assert!(!calls_xrandr("feh --bg-fill x.png"));
        assert!(!calls_xrandr("echo xrandr"));
    }

    #[test]
    fn parses_options_and_later_ones_win() {
        let p = Profile::parse(
            "xrandr --output A --mode 1920x1080 --rate 60 --pos 10x-5 --rotate left \
             --reflect xy --primary --output B --off --output A --scale 1.5 --output B --auto \
             --right-of A --output C --same-as A --dpi 96 --frob\n",
        );
        let a = &p.entries[0];
        assert_eq!(a.target, Target::Name("A".to_owned()));
        assert_eq!(a.mode, Some(ModeRequest::Name("1920x1080".to_owned())));
        assert_eq!(a.rate, Some(60.0));
        assert_eq!(a.pos, Some(Point::new(10, -5)));
        assert_eq!(a.rotation, Some(Rotation::Left));
        assert_eq!(a.reflection, Some(Reflection::XY));
        assert_eq!(a.scaling, Some(Scaling::X11(Transform::scale(1.5, 1.5))));
        let b = &p.entries[1];
        assert!(
            !b.off && b.mode == Some(ModeRequest::Preferred),
            "--auto after --off turns it on: {b:?}"
        );
        assert_eq!(b.relative, Some((Side::RightOf, "A".to_owned())));
        assert_eq!(p.entries[2].relative, Some((Side::Same, "A".to_owned())));
        assert_eq!(p.primary, Some(Some("A".to_owned())));
        assert_eq!(
            p.warnings,
            [
                "--dpi is not supported; ignored.",
                "Unknown option --frob; ignored."
            ]
        );
    }

    #[test]
    fn warns_about_bad_values_and_scripts_without_xrandr() {
        let p = Profile::parse("xrandr --output A --pos 10,20 --rotate sideways --mode");
        assert_eq!(
            p.warnings,
            [
                "A: \"10,20\" is not a valid position; ignored.",
                "A: \"sideways\" is not a valid rotation; ignored.",
                "--mode is missing its value."
            ]
        );
        let p = Profile::parse("#!/bin/sh\nautorandr --change\n");
        assert_eq!(p.warnings, ["The script does not call xrandr."]);
    }

    #[test]
    fn modes_by_name_or_xid() {
        let p = Profile::parse("xrandr --output A --mode 0x1c3 --output B --mode 0xZZ");
        assert_eq!(p.entries[0].mode, Some(ModeRequest::Xid(0x1c3)));
        assert_eq!(
            p.entries[1].mode,
            Some(ModeRequest::Name("0xZZ".to_owned()))
        );
    }

    #[test]
    fn paths() {
        let dir = Path::new("/layouts");
        assert_eq!(profile_path(dir, "home"), Path::new("/layouts/home.sh"));
        assert_eq!(profile_path(dir, "x/home"), Path::new("x/home"));
        assert_eq!(profile_path(dir, "home.sh"), Path::new("home.sh"));
        assert_eq!(profile_name(Path::new("/layouts/tv-home.sh")), "tv-home");
    }

    #[test]
    fn saving_keeps_other_lines_byte_for_byte() {
        let old = "#!/bin/sh\n# my setup\nxrandr --output A \\\n  --off\nfeh --bg-fill  x.png\n";
        let text = save_text(Some(old), "xrandr --output A --auto");
        assert_eq!(
            text,
            format!(
                "#!/bin/sh\n{}\n# my setup\nxrandr --output A --auto\nfeh --bg-fill  x.png\n",
                command::script_header()
            )
        );
        // Saving again updates the header in place and changes nothing else.
        assert_eq!(save_text(Some(&text), "xrandr --output A --auto"), text);

        let fresh = save_text(None, "xrandr --output A --auto");
        assert!(fresh.starts_with("#!/bin/sh\n# generated by outlay "));
        let appended = save_text(Some("echo hi"), "xrandr --output A --auto");
        assert!(
            appended.starts_with("echo hi\n# generated by outlay"),
            "{appended}"
        );
        assert!(appended.ends_with("xrandr --output A --auto\n"));
    }
}
