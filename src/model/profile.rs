//! A saved layout, whatever file it came from: what a profile says about each output, matched
//! onto the outputs that are connected now. Screenlayout scripts (`crate::xrandr::script`) and
//! kanshi configs both read into this form. Parsing is lenient: what a reader does not understand
//! becomes a warning. Links are not stored; loading infers them from the positions.

use super::geometry::{Point, Rect};
use super::layout::{Layout, OutputState};
use super::links::{Align, Link, Side, place};
use super::{Kind, Mode, ModeId, Output, Reflection, Rotation, Scaling, Snapshot};

/// Which output an entry is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A connector name: `eDP-1`.
    Name(String),
    /// A glob on the display's description, `Make Model Serial` with `Unknown` for a missing
    /// field, as kanshi builds it.
    Description(String),
    /// `*`: any one connected output no other entry matches.
    Any,
}

impl Default for Target {
    fn default() -> Self {
        Self::Name(String::new())
    }
}

impl Target {
    /// How the remap dialog and the notes name the entry: the name, the description glob, or
    /// `*`.
    pub fn label(&self) -> String {
        match self {
            Self::Name(name) | Self::Description(name) => name.clone(),
            Self::Any => "*".to_owned(),
        }
    }
}

/// The mode an entry asks for. The rate, if any, is the entry's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModeRequest {
    /// The preferred mode: xrandr's `--auto` and `--preferred`, kanshi's `mode preferred`.
    Preferred,
    /// A mode by name (`1920x1080`, `1920x1080_60.00`), at the rate nearest the entry's, or the
    /// first one with that name.
    Name(String),
    /// An X11 mode by its XID: `--mode 0x1c3`.
    Xid(u32),
}

impl ModeRequest {
    /// The mode as the profile wrote it.
    pub fn label(&self) -> String {
        match self {
            Self::Preferred => "preferred".to_owned(),
            Self::Name(name) => name.clone(),
            Self::Xid(xid) => format!("0x{xid:x}"),
        }
    }
}

/// What a profile says about one output. Readers merge repeated mentions; later ones win.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub target: Target,
    pub off: bool,
    /// `None` keeps the mode the output runs, or the preferred one when it is off.
    pub mode: Option<ModeRequest>,
    pub rate: Option<f64>,
    pub pos: Option<Point>,
    pub rotation: Option<Rotation>,
    pub reflection: Option<Reflection>,
    pub scaling: Option<Scaling>,
    /// The X11 filter of a scaled output.
    pub filter: Option<String>,
    /// `--left-of X` and friends; `--same-as X` is `Side::Same`.
    pub relative: Option<(Side, String)>,
}

impl Entry {
    /// An entry about the output named `name`.
    pub fn named(name: &str) -> Self {
        Self {
            target: Target::Name(name.to_owned()),
            ..Self::default()
        }
    }

    fn turns_on(&self) -> bool {
        !self.off
    }
}

/// A parsed profile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    /// In order of first mention.
    pub entries: Vec<Entry>,
    /// `Some(Some(name))` for `--primary`, `Some(None)` for `--noprimary`.
    pub primary: Option<Option<String>>,
    pub warnings: Vec<String>,
}

/// A requested rate this close to the one found is the same rate (60.00 and 59.94).
const RATE_TOLERANCE: f64 = 0.5;

/// Where each profile output that is not connected goes: a connected output, or nowhere. Rows
/// are labelled as [`Target::label`] labels the entries, in the order of
/// [`Profile::unmatched`].
pub type Remap = Vec<(String, Option<usize>)>;

/// The description kanshi matches its globs against: `Make Model Serial`, with `Unknown` for a
/// field the display does not report.
pub fn description(out: &Output) -> String {
    let id = out.identity.as_ref();
    let field = |f: Option<&Option<String>>| {
        f.and_then(Option::as_deref)
            .filter(|v| !v.is_empty())
            .unwrap_or("Unknown")
            .to_owned()
    };
    format!(
        "{} {} {}",
        field(id.map(|i| &i.make)),
        field(id.map(|i| &i.model)),
        field(id.map(|i| &i.serial))
    )
}

/// Shell-style glob matching, as `fnmatch` with no flags does it: `*`, `?`, bracket expressions
/// (`[a-z]`, `[!0-9]`) and backslash escapes.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    // Where the last `*` was, and the text position it matched up to.
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        let step = match p.get(pi) {
            Some('*') => {
                star = Some((pi, ti));
                pi += 1;
                continue;
            }
            Some('?') => Some(1),
            Some('[') => bracket(&p[pi..], t[ti]),
            Some('\\') if pi + 1 < p.len() => (p[pi + 1] == t[ti]).then_some(2),
            Some(&c) => (c == t[ti]).then_some(1),
            None => None,
        };
        match (step, star) {
            (Some(n), _) => {
                pi += n;
                ti += 1;
            }
            (None, Some((sp, st))) => {
                pi = sp + 1;
                ti = st + 1;
                star = Some((sp, st + 1));
            }
            (None, None) => return false,
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// Whether the bracket expression at the start of `p` matches `c`; on a match, how many pattern
/// characters it spans. An unclosed `[` is a plain character.
fn bracket(p: &[char], c: char) -> Option<usize> {
    let mut k = 1;
    let negated = matches!(p.get(k), Some('!' | '^'));
    if negated {
        k += 1;
    }
    let mut hit = false;
    let mut first = true;
    loop {
        let Some(&lo) = p.get(k) else {
            return (c == '[').then_some(1);
        };
        if lo == ']' && !first {
            break;
        }
        first = false;
        if p.get(k + 1) == Some(&'-') && p.get(k + 2).is_some_and(|&hi| hi != ']') {
            hit |= (lo..=p[k + 2]).contains(&c);
            k += 3;
        } else {
            hit |= lo == c;
            k += 1;
        }
    }
    (hit != negated).then_some(k + 1)
}

/// The connector type of an output name: `eDP` for `eDP-1`, `DP` for `DP-1-2.1`.
pub fn connector(name: &str) -> &str {
    let end = name
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(name.len());
    &name[..end]
}

impl Profile {
    /// The output each entry stands for on `snap`, before any remap. A name matches that output
    /// when it is connected, or whenever the entry turns it off; a description matches a
    /// connected output no other entry took; `*` takes one that is left. Names go first, then
    /// descriptions, then `*`, so a wildcard never takes a display a later entry names.
    pub fn matches(&self, snap: &Snapshot) -> Vec<Option<usize>> {
        let mut found = vec![None; self.entries.len()];
        let mut taken: Vec<usize> = Vec::new();
        let connected = |i: usize| snap.outputs[i].is_connected();
        for pass in 0..3 {
            for (k, e) in self.entries.iter().enumerate() {
                let free = |i: &usize| connected(*i) && !taken.contains(i);
                let pick = match (&e.target, pass) {
                    (Target::Name(name), 0) => snap.find(name).filter(|&i| connected(i) || e.off),
                    (Target::Description(glob), 1) => snap
                        .numbered()
                        .into_iter()
                        .filter(free)
                        .find(|&i| glob_match(glob, &description(&snap.outputs[i]))),
                    (Target::Any, 2) => snap.numbered().into_iter().find(free),
                    _ => continue,
                };
                found[k] = pick;
                taken.extend(pick);
            }
        }
        found
    }

    /// Profile outputs that would be on but are not connected: they need a remap.
    pub fn unmatched(&self, snap: &Snapshot) -> Vec<String> {
        self.entries
            .iter()
            .zip(self.matches(snap))
            .filter(|(e, found)| e.turns_on() && found.is_none())
            .map(|(e, _)| e.target.label())
            .collect()
    }

    /// Connected outputs the profile does not turn on: the ones a remap may use.
    pub fn free_outputs(&self, snap: &Snapshot) -> Vec<usize> {
        let found = self.matches(snap);
        let claimed = |i: usize| {
            self.entries
                .iter()
                .zip(&found)
                .any(|(e, f)| e.turns_on() && *f == Some(i))
        };
        snap.numbered()
            .into_iter()
            .filter(|&i| snap.outputs[i].is_connected() && !claimed(i))
            .collect()
    }

    /// The remap dialog's starting point: each unmatched output goes to a free output no earlier
    /// one took: one with the same connector prefix (`eDP-2` → `eDP-1`) for a name, the same
    /// make and model for a description; else nowhere.
    pub fn default_remap(&self, snap: &Snapshot) -> Remap {
        let free = self.free_outputs(snap);
        let found = self.matches(snap);
        let mut taken: Vec<usize> = Vec::new();
        self.entries
            .iter()
            .zip(found)
            .filter(|(e, found)| e.turns_on() && found.is_none())
            .map(|(e, _)| {
                let fits = |i: usize| {
                    let out = &snap.outputs[i];
                    match &e.target {
                        Target::Name(name) => connector(&out.name) == connector(name),
                        Target::Description(glob) => out.identity.as_ref().is_some_and(|id| {
                            let make_model = format!(
                                "{} {} ",
                                id.make.as_deref().unwrap_or("Unknown"),
                                id.model.as_deref().unwrap_or("Unknown")
                            );
                            glob.starts_with(&make_model)
                        }),
                        Target::Any => true,
                    }
                };
                let pick = free
                    .iter()
                    .copied()
                    .find(|&i| !taken.contains(&i) && fits(i));
                taken.extend(pick);
                (e.target.label(), pick)
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

        // Each remap row stands for the first entry with its label that no earlier row took.
        let mut row_of: Vec<Option<usize>> = vec![None; self.entries.len()];
        for (r, (from, _)) in remap.iter().enumerate() {
            if let Some(k) = (0..self.entries.len())
                .find(|&k| row_of[k].is_none() && self.entries[k].target.label() == *from)
            {
                row_of[k] = Some(r);
            }
        }
        // Outputs matched directly first, remapped ones after, so a remap overrides an --off.
        let mut targets: Vec<(usize, &Entry)> = Vec::new();
        for ((e, found), row) in self.entries.iter().zip(self.matches(snap)).zip(&row_of) {
            if let (Some(i), None) = (found, row) {
                targets.push((i, e));
            }
        }
        for (r, (from, to)) in remap.iter().enumerate() {
            let Some(k) = row_of.iter().position(|&row| row == Some(r)) else {
                continue;
            };
            match to {
                Some(i) => {
                    targets.retain(|(t, _)| t != i);
                    targets.push((*i, &self.entries[k]));
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
                Some(ModeRequest::Xid(xid)) => out.mode(ModeId::from_xid(*xid)).cloned(),
                Some(ModeRequest::Name(name)) => out.find_mode(name, e.rate).cloned(),
                Some(ModeRequest::Preferred) => out.preferred_mode().cloned(),
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
                        e.mode.as_ref().map_or("?".to_owned(), ModeRequest::label),
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
            match (&e.scaling, snap.caps.kind) {
                (Some(s @ Scaling::X11(_)), Kind::X11)
                | (Some(s @ Scaling::Logical(_)), Kind::Wayland) => st.scaling = s.clone(),
                (Some(_), kind) => notes.push(format!(
                    "{}: the profile's scale is not a {} scale; ignored.",
                    out.name,
                    kind.name()
                )),
                (None, _) => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connectors() {
        assert_eq!(connector("eDP-2"), "eDP");
        assert_eq!(connector("DP-1-2.1"), "DP");
        assert_eq!(connector("DVI-I-2-1"), "DVI");
        assert_eq!(connector("HDMI-1-0"), "HDMI");
    }

    #[test]
    fn globs_match_as_fnmatch_does() {
        let yes = [
            ("*", ""),
            ("*", "Dell Inc. DELL U2719D 7LQ2M43"),
            ("Dell Inc. *", "Dell Inc. DELL U2719D 7LQ2M43"),
            ("* U2719D *", "Dell Inc. DELL U2719D 7LQ2M43"),
            ("Dell?Inc.*", "Dell Inc. X Y"),
            ("[A-F]ell*", "Dell"),
            ("[!a-z]ell", "Dell"),
            ("a\\*b", "a*b"),
            ("a[*]b", "a*b"),
            ("[]]", "]"),
            ("x[", "x["),
        ];
        for (p, t) in yes {
            assert!(glob_match(p, t), "{p:?} should match {t:?}");
        }
        let no = [
            ("Dell", "Dell Inc."),
            ("*U2720*", "Dell Inc. DELL U2719D 7LQ2M43"),
            ("?", ""),
            ("[!A-F]ell", "Dell"),
            ("a\\*b", "axb"),
        ];
        for (p, t) in no {
            assert!(!glob_match(p, t), "{p:?} should not match {t:?}");
        }
    }

    #[test]
    fn mode_labels() {
        assert_eq!(ModeRequest::Xid(0x1c3).label(), "0x1c3");
        assert_eq!(ModeRequest::Preferred.label(), "preferred");
        assert_eq!(Target::Any.label(), "*");
    }
}
