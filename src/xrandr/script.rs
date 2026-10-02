//! Screenlayout profiles: arandr-style `#!/bin/sh` scripts of xrandr calls, as kept in
//! `~/.screenlayout`. Parsing is lenient, like the shell running the script: what outlay does not
//! understand becomes a warning. Links are not stored; loading infers them from the positions.

use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::model::geometry::{Point, Rect};
use crate::model::layout::{Layout, OutputState};
use crate::model::links::{Align, Link, Side, place};
use crate::model::{Mode, ModeId, Reflection, Rotation, Scaling, Snapshot, Transform};

use super::command;

/// What a profile says about one output. Later options win when an output repeats.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub name: String,
    pub off: bool,
    /// `--auto` or `--preferred`: the preferred mode.
    pub auto: bool,
    /// A mode name, or an XID such as `0x1c3`.
    pub mode: Option<String>,
    pub rate: Option<f64>,
    pub pos: Option<Point>,
    pub rotation: Option<Rotation>,
    pub reflection: Option<Reflection>,
    pub transform: Option<Transform>,
    pub filter: Option<String>,
    /// `--left-of X` and friends; `--same-as X` is `Side::Same`.
    pub relative: Option<(Side, String)>,
}

/// A parsed profile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    /// In order of first mention.
    pub entries: Vec<Entry>,
    /// `Some(Some(name))` for `--primary`, `Some(None)` for `--noprimary`.
    pub primary: Option<Option<String>>,
    pub warnings: Vec<String>,
    /// How many xrandr calls the script makes.
    pub calls: usize,
}

/// A requested rate this close to the one found is the same rate (60.00 and 59.94).
const RATE_TOLERANCE: f64 = 0.5;

/// Where each profile output that is not connected goes: a connected output, or nowhere.
pub type Remap = Vec<(String, Option<usize>)>;

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
    /// Parses a script. It never fails: what cannot be read becomes a warning.
    pub fn parse(text: &str) -> Profile {
        let mut profile = Profile::default();
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
                    profile.calls += 1;
                    profile.read_call(args);
                }
            }
        }
        if profile.calls == 0 {
            profile
                .warnings
                .push("The script does not call xrandr.".to_owned());
        }
        profile
    }

    fn entry(&mut self, name: &str) -> &mut Entry {
        match self.entries.iter().position(|e| e.name == name) {
            Some(k) => &mut self.entries[k],
            None => {
                self.entries.push(Entry {
                    name: name.to_owned(),
                    ..Entry::default()
                });
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
                        e.auto = false;
                        e.mode = None;
                        e.rate = None;
                    }
                    "--auto" | "--preferred" => {
                        e.off = false;
                        e.auto = true;
                        e.mode = None;
                    }
                    "--primary" => {}
                    "--mode" => {
                        e.off = false;
                        e.auto = false;
                        e.mode = Some(value.clone());
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
                        Some(t) => e.transform = Some(t),
                        None => invalid = Some("scale"),
                    },
                    "--transform" => match super::parse_transform_arg(&value) {
                        Some(t) => e.transform = Some(t),
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

    fn turns_on(entry: &Entry) -> bool {
        !entry.off
    }

    /// Profile outputs that would be on but are not connected: they need a remap.
    pub fn unmatched(&self, snap: &Snapshot) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| Self::turns_on(e))
            .filter(|e| {
                snap.find(&e.name)
                    .is_none_or(|i| !snap.outputs[i].is_connected())
            })
            .map(|e| e.name.clone())
            .collect()
    }

    /// Connected outputs the profile does not turn on: the ones a remap may use.
    pub fn free_outputs(&self, snap: &Snapshot) -> Vec<usize> {
        snap.numbered()
            .into_iter()
            .filter(|&i| snap.outputs[i].is_connected())
            .filter(|&i| {
                !self
                    .entries
                    .iter()
                    .any(|e| Self::turns_on(e) && e.name == snap.outputs[i].name)
            })
            .collect()
    }

    /// The remap dialog's starting point: each unmatched output goes to a free output with the
    /// same connector prefix (`eDP-2` → `eDP-1`) that no earlier one took, else nowhere.
    pub fn default_remap(&self, snap: &Snapshot) -> Remap {
        let free = self.free_outputs(snap);
        let mut taken: Vec<usize> = Vec::new();
        self.unmatched(snap)
            .into_iter()
            .map(|name| {
                let pick = free.iter().copied().find(|&i| {
                    !taken.contains(&i) && connector(&snap.outputs[i].name) == connector(&name)
                });
                taken.extend(pick);
                (name, pick)
            })
            .collect()
    }

    /// The output a profile name stands for on `snap`, after the remap.
    fn index_of(&self, snap: &Snapshot, remap: &Remap, name: &str) -> Option<usize> {
        if let Some((_, to)) = remap.iter().find(|(from, _)| from == name) {
            return *to;
        }
        snap.find(name)
    }

    /// The layout this profile describes on `snap`, with `remap` for the outputs that are not
    /// connected, and notes about everything that did not fit.
    pub fn layout(&self, snap: &Snapshot, remap: &Remap) -> (Layout, Vec<String>) {
        let live = Layout::inferred(snap);
        let mut states: Vec<OutputState> = live.outputs.clone();
        let mut notes = self.warnings.clone();

        // Outputs named directly first, remapped ones after, so a remap overrides an --off.
        let mut targets: Vec<(usize, &Entry)> = Vec::new();
        for e in &self.entries {
            let direct = snap
                .find(&e.name)
                .filter(|&i| snap.outputs[i].is_connected() || e.off);
            if let Some(i) = direct.filter(|_| !remap.iter().any(|(from, _)| *from == e.name)) {
                targets.push((i, e));
            }
        }
        for (from, to) in remap {
            let Some(e) = self.entries.iter().find(|e| e.name == *from) else {
                continue;
            };
            match to {
                Some(i) => {
                    targets.retain(|(k, _)| k != i);
                    targets.push((*i, e));
                }
                None => notes.push(format!("{from} is not connected; skipped.")),
            }
        }

        for &(i, e) in &targets {
            if live.locked[i] {
                notes.push(format!(
                    "{} uses panning; outlay leaves it as it is.",
                    snap.outputs[i].name
                ));
                continue;
            }
            let st = &mut states[i];
            if e.off {
                st.enabled = false;
                st.primary = false;
                continue;
            }
            let out = &snap.outputs[i];
            let was_on = st.enabled;
            let keep = st.mode.clone().filter(|m| out.mode(m.id).is_some());
            let mode: Option<Mode> = match &e.mode {
                Some(m) => m
                    .strip_prefix("0x")
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .and_then(|xid| out.mode(ModeId::from_xid(xid)))
                    .or_else(|| out.find_mode(m, e.rate))
                    .cloned(),
                None if e.auto => out.preferred_mode().cloned(),
                None => {
                    let base = keep.or_else(|| out.preferred_mode().cloned());
                    match (base, e.rate) {
                        (Some(b), Some(r)) => out
                            .rates(b.width, b.height)
                            .into_iter()
                            .min_by(|x, y| (x.refresh - r).abs().total_cmp(&(y.refresh - r).abs()))
                            .cloned()
                            .or(Some(b)),
                        (base, _) => base,
                    }
                }
            };
            let mode = match mode {
                Some(m) => m,
                None => {
                    let Some(preferred) = out.preferred_mode().cloned() else {
                        notes.push(format!("{} lists no modes; skipped.", out.name));
                        continue;
                    };
                    notes.push(format!(
                        "{} has no mode {}; using {}.",
                        out.name,
                        e.mode.as_deref().unwrap_or("?"),
                        preferred.summary()
                    ));
                    preferred
                }
            };
            if let Some(r) = e.rate
                && (mode.refresh - r).abs() > RATE_TOLERANCE
            {
                notes.push(format!(
                    "{} has no {r:.2} Hz at {}x{}; using {:.2} Hz.",
                    out.name, mode.width, mode.height, mode.refresh
                ));
            }
            st.enabled = true;
            st.mode = Some(mode);
            st.pos = e
                .pos
                .unwrap_or(if was_on { st.pos } else { Point::default() });
            if let Some(r) = e.rotation {
                st.rotation = r;
            }
            if let Some(r) = e.reflection {
                st.reflection = r;
            }
            if let Some(t) = &e.transform {
                st.scaling = Scaling::X11(t.clone());
            }
            if let (Some(f), Scaling::X11(t)) = (&e.filter, &mut st.scaling)
                && !t.is_identity()
            {
                t.filter = f.clone();
            }
        }

        match &self.primary {
            Some(Some(name)) => match self.index_of(snap, remap, name) {
                Some(p) if states[p].enabled => {
                    for (k, st) in states.iter_mut().enumerate() {
                        st.primary = k == p;
                    }
                }
                _ => {}
            },
            Some(None) => states.iter_mut().for_each(|st| st.primary = false),
            None => {}
        }

        // Relative placement, as xrandr does it: next to the parent, edges aligned at the start.
        let mut sticks: Vec<(usize, usize, Side)> = Vec::new();
        let relative: Vec<(usize, Side, &str)> = targets
            .iter()
            .filter_map(|&(i, e)| e.relative.as_ref().map(|(s, p)| (i, *s, p.as_str())))
            .filter(|&(i, _, _)| states[i].enabled)
            .collect();
        let mut placed: Vec<usize> = Vec::new();
        for _ in 0..relative.len() {
            for &(c, side, parent) in &relative {
                if placed.contains(&c) {
                    continue;
                }
                let Some(p) = self
                    .index_of(snap, remap, parent)
                    .filter(|&p| states[p].enabled && p != c)
                else {
                    continue;
                };
                let waits = relative.iter().any(|&(o, _, _)| o == p) && !placed.contains(&p);
                if waits {
                    continue;
                }
                let link = Link {
                    parent: p,
                    side,
                    align: Align::Start,
                    offset: 0,
                };
                let parent_rect = Rect::from_parts(states[p].pos, states[p].size());
                states[c].pos = place(states[c].size(), parent_rect, &link);
                sticks.push((c, p, side));
                placed.push(c);
            }
        }
        for &(c, _, parent) in &relative {
            if !placed.contains(&c) {
                notes.push(format!(
                    "{}: cannot place it next to {parent}; it stays at {},{}.",
                    snap.outputs[c].name, states[c].pos.x, states[c].pos.y
                ));
            }
        }

        if !states.iter().any(|s| s.enabled) {
            notes.push("The profile turns every display off; keeping the live layout.".to_owned());
            return (live, notes);
        }
        let (layout, more) = Layout::from_states(snap, states, &sticks);
        notes.extend(more);
        (layout, notes)
    }
}

/// The connector type of an output name: `eDP` for `eDP-1`, `DP` for `DP-1-2.1`.
pub fn connector(name: &str) -> &str {
    let end = name
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(name.len());
    &name[..end]
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

/// A line diff from `old` to `new`: kept lines start with two spaces, removed ones with `- `,
/// added ones with `+ `.
pub fn line_diff(old: &str, new: &str) -> Vec<String> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    // Longest common subsequence, filled from the end.
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push(format!("  {}", a[i]));
            i += 1;
            j += 1;
        } else if i < a.len() && (j == b.len() || lcs[i + 1][j] >= lcs[i][j + 1]) {
            out.push(format!("- {}", a[i]));
            i += 1;
        } else {
            out.push(format!("+ {}", b[j]));
            j += 1;
        }
    }
    out
}

/// Writes a file atomically: a temporary file next to it, then a rename. A new file gets
/// `mode`; an existing one keeps its permissions.
pub fn write_atomic(path: &Path, text: &str, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let permissions = match std::fs::metadata(path) {
        Ok(meta) => meta.permissions(),
        Err(_) => std::fs::Permissions::from_mode(mode),
    };
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".outlay-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::set_permissions(&tmp, permissions)?;
    std::fs::rename(&tmp, path)
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
        assert_eq!(p.calls, 1);
        let a = &p.entries[0];
        assert_eq!(a.mode.as_deref(), Some("1920x1080"));
        assert_eq!(a.rate, Some(60.0));
        assert_eq!(a.pos, Some(Point::new(10, -5)));
        assert_eq!(a.rotation, Some(Rotation::Left));
        assert_eq!(a.reflection, Some(Reflection::XY));
        assert_eq!(a.transform, Some(Transform::scale(1.5, 1.5)));
        let b = &p.entries[1];
        assert!(!b.off && b.auto, "--auto after --off turns it on: {b:?}");
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
    fn connectors_and_paths() {
        assert_eq!(connector("eDP-2"), "eDP");
        assert_eq!(connector("DP-1-2.1"), "DP");
        assert_eq!(connector("DVI-I-2-1"), "DVI");
        assert_eq!(connector("HDMI-1-0"), "HDMI");
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

    #[test]
    fn line_diffs() {
        assert_eq!(
            line_diff("a\nb\nc\n", "a\nx\nc\nd\n"),
            ["  a", "- b", "+ x", "  c", "+ d"]
        );
        assert_eq!(line_diff("", "a"), ["+ a"]);
    }

    #[test]
    fn atomic_writes_keep_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("outlay-atomic-{}", std::process::id()));
        let path = dir.join("nested").join("p.sh");
        write_atomic(&path, "one\n", 0o755).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o755);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        write_atomic(&path, "two\n", 0o755).unwrap();
        assert_eq!(mode(&path), 0o700);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two\n");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
