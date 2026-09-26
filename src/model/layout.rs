//! The editable layout: one [`OutputState`] per output plus the stick links between them.
//!
//! Every edit goes through [`Layout::commit`], which re-flows the links, pushes displays out of
//! the way of a resize, and normalises the layout so its top-left corner is at 0,0.

use std::fmt;

use thiserror::Error;

use super::geometry::{Dir, Point, Rect, Size, bbox, effective_size};
use super::links::{Link, Restore, Side};
use super::{Mode, Output, Reflection, Rotation, Snapshot, Transform};

/// The pending configuration of one output.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputState {
    pub enabled: bool,
    /// The chosen mode; `None` for an output that has never been on.
    pub mode: Option<Mode>,
    pub pos: Point,
    pub rotation: Rotation,
    pub reflection: Reflection,
    pub transform: Transform,
    pub primary: bool,
}

impl OutputState {
    /// The effective size: the mode rotated and transformed.
    pub fn size(&self) -> Size {
        self.mode.as_ref().map_or(Size::default(), |m| {
            effective_size(m.size(), self.rotation, &self.transform)
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// Parallel to the snapshot's outputs.
    pub outputs: Vec<OutputState>,
    /// `links[child]`: what the child is stuck to. A display without a link is an anchor.
    pub links: Vec<Option<Link>>,
    /// How to bring back a display that was turned off in this session.
    pub restore: Vec<Option<Restore>>,
    pub names: Vec<String>,
    /// Display numbers, for outputs that have one.
    pub numbers: Vec<Option<usize>>,
    /// Outputs with panning, which outlay v0.1 leaves untouched.
    pub locked: Vec<bool>,
}

/// What an edit did besides the edit itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitReport {
    /// The translation normalisation applied to every display; the viewport follows it so that
    /// displays that did not move stay still on screen.
    pub shift: Point,
    /// Displays pushed out of the way of a resize.
    pub pushed: Vec<usize>,
    /// Status messages: failed pushes, a moved primary, a mirror that shows only part.
    pub notes: Vec<String>,
}

impl CommitReport {
    /// Puts notes gathered before the commit ran ahead of the commit's own.
    pub(super) fn with_notes(mut self, mut earlier: Vec<String>) -> Self {
        earlier.append(&mut self.notes);
        self.notes = earlier;
        self
    }
}

/// Why an edit did nothing. A failed or no-op edit adds no undo step.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum EditError {
    /// The edit would change nothing.
    #[error("{0}")]
    NoChange(String),
    /// The edit is not allowed.
    #[error("{0}")]
    Refused(String),
    /// A snap-move found no position in that direction.
    #[error("no snap spot further {}", .0.word())]
    NoSnapSpot(Dir),
}

/// What kind of edit a commit follows; it decides which pipeline steps run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// Positions were set directly (move, swap, nudge, `:pos`): links follow the new positions.
    Move,
    /// A size changed (mode, rotation, scale): neighbours re-flow and get pushed out of the way.
    Resize,
    /// Links changed (stick, unstick, on, off): positions follow the links.
    Relink,
}

impl Layout {
    /// The live layout exactly as the snapshot describes it, with inferred links. Normalised only
    /// when a position is negative, so a layout starting at 16,0 has no phantom pending change.
    pub fn inferred(snap: &Snapshot) -> Self {
        let n = snap.outputs.len();
        let outputs = snap.outputs.iter().map(state_from_output).collect();
        let mut layout = Layout {
            outputs,
            links: vec![None; n],
            restore: vec![None; n],
            names: snap.outputs.iter().map(|o| o.name.clone()).collect(),
            numbers: (0..n).map(|i| snap.number_of(i)).collect(),
            locked: snap.outputs.iter().map(Output::has_panning).collect(),
        };
        layout.infer_links();
        let min = layout.min_corner();
        if min.x < 0 || min.y < 0 {
            layout.normalise();
        }
        layout
    }

    /// The initial pending layout: the live one, with stale outputs (disconnected but still
    /// active) turned off. Returns status messages about what was changed.
    pub fn from_snapshot(snap: &Snapshot) -> (Self, Vec<String>) {
        let mut layout = Self::inferred(snap);
        let mut notes = Vec::new();
        let stale: Vec<usize> = (0..layout.len())
            .filter(|&i| layout.outputs[i].enabled && snap.outputs[i].is_stale())
            .collect();
        let keeps_one = (0..layout.len()).any(|i| layout.outputs[i].enabled && !stale.contains(&i));
        if keeps_one {
            for &d in &stale {
                if layout.turn_off(d).is_ok() {
                    notes.push(format!(
                        "{} is disconnected but still active; turning it off.",
                        layout.names[d]
                    ));
                }
            }
        }
        (layout, notes)
    }

    pub fn len(&self) -> usize {
        self.outputs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.outputs.is_empty()
    }

    pub fn is_enabled(&self, i: usize) -> bool {
        self.outputs[i].enabled
    }

    pub fn enabled(&self) -> Vec<usize> {
        (0..self.len()).filter(|&i| self.is_enabled(i)).collect()
    }

    pub fn enabled_count(&self) -> usize {
        self.outputs.iter().filter(|o| o.enabled).count()
    }

    pub fn size(&self, i: usize) -> Size {
        self.outputs[i].size()
    }

    pub fn rect(&self, i: usize) -> Rect {
        Rect::from_parts(self.outputs[i].pos, self.size(i))
    }

    /// The bounding box of the enabled displays.
    pub fn bounds(&self) -> Option<Rect> {
        bbox(self.enabled().into_iter().map(|i| self.rect(i)))
    }

    pub fn primary(&self) -> Option<usize> {
        (0..self.len()).find(|&i| self.outputs[i].enabled && self.outputs[i].primary)
    }

    /// `2 DP-1-2`, or just the name for an unnumbered output.
    pub fn label(&self, i: usize) -> String {
        match self.numbers[i] {
            Some(n) => format!("{n} {}", self.names[i]),
            None => self.names[i].clone(),
        }
    }

    /// Enabled pairs that overlap and are not mirrors of each other, lower index first.
    pub fn overlapping_pairs(&self) -> Vec<(usize, usize)> {
        let on = self.enabled();
        let mut pairs = Vec::new();
        for (k, &a) in on.iter().enumerate() {
            for &b in &on[k + 1..] {
                if !self.are_mirrors(a, b) && self.rect(a).overlaps(&self.rect(b)) {
                    pairs.push((a, b));
                }
            }
        }
        pairs
    }

    pub(super) fn min_corner(&self) -> Point {
        self.bounds().map_or(Point::default(), |b| b.pos())
    }

    /// Runs the edit pipeline after an edit has been applied to `self`:
    /// 1. for a move, `relink()` so links follow the new positions;
    /// 2. `resolve()` places every stuck display from its parent;
    /// 3. for a resize, the push pass moves neighbours out of the way, then relinks and
    ///    resolves again if it pushed anything;
    /// 4. normalise, so the top-left corner is at 0,0.
    pub fn commit(&mut self, before: &Layout, kind: EditKind) -> CommitReport {
        let mut report = CommitReport::default();
        if kind == EditKind::Move {
            self.relink();
        }
        self.resolve();
        if kind == EditKind::Resize && self.push_pass(before, &mut report) {
            self.relink();
            self.resolve();
        }
        report.shift = self.normalise();
        report
    }

    /// Translates every enabled display so the minimum x and y are 0, and returns the shift.
    /// A locked (panning) output pins the origin unless a position went negative.
    pub(super) fn normalise(&mut self) -> Point {
        let min = self.min_corner();
        let pinned = self.enabled().iter().any(|&i| self.locked[i]);
        let axis = |m: i32| if m < 0 || !pinned { -m } else { 0 };
        let shift = Point::new(axis(min.x), axis(min.y));
        if shift != Point::default() {
            for o in self.outputs.iter_mut().filter(|o| o.enabled) {
                o.pos = Point::new(o.pos.x + shift.x, o.pos.y + shift.y);
            }
        }
        shift
    }

    pub(super) fn check_editable(&self, i: usize) -> Result<(), EditError> {
        if self.locked[i] {
            return Err(EditError::Refused(format!(
                "{} uses panning; outlay leaves it as it is.",
                self.names[i]
            )));
        }
        Ok(())
    }

    fn check_enabled(&self, i: usize) -> Result<(), EditError> {
        self.check_editable(i)?;
        if !self.is_enabled(i) {
            return Err(EditError::Refused(format!("{} is off.", self.names[i])));
        }
        Ok(())
    }

    /// Changes one output's state, then commits a resize.
    fn reshape(
        &mut self,
        i: usize,
        edit: impl FnOnce(&mut OutputState),
    ) -> Result<CommitReport, EditError> {
        self.check_enabled(i)?;
        let before = self.clone();
        edit(&mut self.outputs[i]);
        if self.outputs[i] == before.outputs[i] {
            return Err(EditError::NoChange("nothing to change".to_owned()));
        }
        Ok(self.commit(&before, EditKind::Resize))
    }

    /// Switches output `i` to the mode with this XID.
    pub fn set_mode(
        &mut self,
        snap: &Snapshot,
        i: usize,
        xid: u32,
    ) -> Result<CommitReport, EditError> {
        let mode = snap.outputs[i].mode(xid).cloned().ok_or_else(|| {
            EditError::Refused(format!("{} has no mode 0x{xid:x}.", self.names[i]))
        })?;
        self.reshape(i, |o| o.mode = Some(mode))
    }

    /// Switches output `i` to a resolution, choosing the rate by [`pick_rate`].
    pub fn set_resolution(
        &mut self,
        snap: &Snapshot,
        i: usize,
        w: i32,
        h: i32,
    ) -> Result<CommitReport, EditError> {
        let previous = self.outputs[i].mode.as_ref().map(|m| m.refresh);
        let mode = pick_rate(&snap.outputs[i], w, h, previous)
            .cloned()
            .ok_or_else(|| EditError::Refused(format!("{} has no {w}x{h} mode.", self.names[i])))?;
        self.reshape(i, |o| o.mode = Some(mode))
    }

    /// The next larger (`larger`) or smaller resolution, skipping interlaced and DoubleScan-only
    /// ones.
    pub fn step_resolution(
        &mut self,
        snap: &Snapshot,
        i: usize,
        larger: bool,
    ) -> Result<CommitReport, EditError> {
        self.check_enabled(i)?;
        let out = &snap.outputs[i];
        let list: Vec<_> = out
            .resolutions()
            .into_iter()
            .filter(|r| {
                out.rates(r.width, r.height)
                    .iter()
                    .any(|m| m.is_progressive())
            })
            .collect();
        let current = self.outputs[i]
            .mode
            .as_ref()
            .map(Mode::size)
            .unwrap_or_default();
        let area = |w: i32, h: i32| i64::from(w) * i64::from(h);
        // The list runs from largest to smallest.
        let next = if larger {
            list.iter().rev().find(|r| {
                (area(r.width, r.height), r.width) > (area(current.w, current.h), current.w)
            })
        } else {
            list.iter().find(|r| {
                (area(r.width, r.height), r.width) < (area(current.w, current.h), current.w)
            })
        };
        let Some(next) = next else {
            let which = if larger { "largest" } else { "smallest" };
            return Err(EditError::NoChange(format!(
                "{} is at its {which} resolution.",
                self.names[i]
            )));
        };
        self.set_resolution(snap, i, next.width, next.height)
    }

    /// The next higher (`higher`) or lower rate of the current resolution, including interlaced
    /// and DoubleScan modes.
    pub fn step_rate(
        &mut self,
        snap: &Snapshot,
        i: usize,
        higher: bool,
    ) -> Result<CommitReport, EditError> {
        self.check_enabled(i)?;
        let Some(current) = self.outputs[i].mode.clone() else {
            return Err(EditError::Refused(format!(
                "{} has no mode.",
                self.names[i]
            )));
        };
        // Lowest first; at equal refresh the progressive mode still comes first.
        let mut rates = snap.outputs[i].rates(current.width, current.height);
        rates.sort_by(|a, b| {
            a.refresh
                .total_cmp(&b.refresh)
                .then(b.is_progressive().cmp(&a.is_progressive()))
        });
        let at = rates.iter().position(|m| m.xid == current.xid);
        let next = match (at, higher) {
            (Some(k), true) => rates.get(k + 1),
            (Some(k), false) => k.checked_sub(1).and_then(|k| rates.get(k)),
            (None, _) => rates.last(),
        };
        let Some(next) = next else {
            let which = if higher { "highest" } else { "lowest" };
            return Err(EditError::NoChange(format!(
                "{} is at its {which} rate.",
                self.names[i]
            )));
        };
        let xid = next.xid;
        self.set_mode(snap, i, xid)
    }

    pub fn set_rotation(
        &mut self,
        i: usize,
        rotation: Rotation,
    ) -> Result<CommitReport, EditError> {
        self.reshape(i, |o| o.rotation = rotation)
    }

    pub fn rotate(&mut self, i: usize, clockwise: bool) -> Result<CommitReport, EditError> {
        let r = self.outputs[i].rotation;
        self.set_rotation(
            i,
            if clockwise {
                r.clockwise()
            } else {
                r.counter_clockwise()
            },
        )
    }

    pub fn set_reflection(
        &mut self,
        i: usize,
        reflection: Reflection,
    ) -> Result<CommitReport, EditError> {
        self.reshape(i, |o| o.reflection = reflection)
    }

    /// `:scale 1`: drops the output's transform.
    pub fn reset_scale(&mut self, i: usize) -> Result<CommitReport, EditError> {
        self.reshape(i, |o| o.transform = Transform::identity())
    }

    pub fn set_primary(&mut self, i: usize) -> Result<CommitReport, EditError> {
        self.check_enabled(i)?;
        if self.outputs[i].primary {
            return Err(EditError::NoChange(format!(
                "{} is already primary.",
                self.names[i]
            )));
        }
        let before = self.clone();
        for (k, o) in self.outputs.iter_mut().enumerate() {
            o.primary = k == i;
        }
        Ok(self.commit(&before, EditKind::Relink))
    }

    /// Turns output `i` on or off.
    pub fn toggle(&mut self, snap: &Snapshot, i: usize) -> Result<CommitReport, EditError> {
        if self.is_enabled(i) {
            self.turn_off(i)
        } else {
            self.turn_on(snap, i)
        }
    }

    /// How output `i` is stuck: `right-of 1 HDMI-1-0, top`, `mirror of 2 DP-1-2`, or `anchor`.
    pub fn link_text(&self, i: usize) -> String {
        let Some(link) = self.links[i] else {
            return "anchor".to_owned();
        };
        let parent = self.label(link.parent);
        if link.side == Side::Same {
            return format!("mirror of {parent}");
        }
        let mut text = format!(
            "{} {parent}, {}",
            link.side.as_str(),
            link.align.label(link.side)
        );
        if link.offset != 0 {
            text.push_str(&format!(" {:+}", link.offset));
        }
        text
    }

    /// Per-output differences from the live snapshot, in xrandr order.
    pub fn diff(&self, snap: &Snapshot) -> Vec<OutputDiff> {
        let gained = (0..self.len()).any(|i| {
            self.outputs[i].enabled && self.outputs[i].primary && !snap.outputs[i].primary
        });
        let mut diffs = Vec::new();
        for (i, (st, out)) in self.outputs.iter().zip(&snap.outputs).enumerate() {
            let mut changes = Vec::new();
            match (&out.active, st.enabled) {
                (None, false) => {}
                (None, true) => changes.push(Change::On(
                    st.mode.as_ref().map(Mode::summary).unwrap_or_default(),
                )),
                (Some(_), false) => changes.push(Change::Off),
                (Some(live), true) => {
                    let live_mode = out.mode(live.xid);
                    if let Some(mode) = &st.mode
                        && mode.xid != live.xid
                    {
                        match live_mode {
                            Some(old) if old.size() == mode.size() => {
                                changes.push(Change::Rate(old.refresh, mode.refresh))
                            }
                            Some(old) => changes.push(Change::Mode(old.summary(), mode.summary())),
                            None => changes.push(Change::Mode(
                                format!("{}x{}", live.size.w, live.size.h),
                                mode.summary(),
                            )),
                        }
                    }
                    if st.pos != live.pos {
                        changes.push(Change::Position(live.pos, st.pos));
                    }
                    if st.rotation != live.rotation {
                        changes.push(Change::Rotation(live.rotation, st.rotation));
                    }
                    if st.reflection != live.reflection {
                        changes.push(Change::Reflection(live.reflection, st.reflection));
                    }
                    if st.transform.is_identity() != live.transform.is_identity() {
                        changes.push(Change::Scale(
                            scale_text(&live.transform),
                            scale_text(&st.transform),
                        ));
                    }
                }
            }
            if st.enabled {
                if st.primary && !out.primary {
                    changes.push(Change::Primary(true));
                } else if !st.primary && out.primary && !gained {
                    changes.push(Change::Primary(false));
                }
            }
            if !changes.is_empty() {
                diffs.push(OutputDiff {
                    index: i,
                    name: self.names[i].clone(),
                    changes,
                });
            }
        }
        diffs
    }
}

fn scale_text(t: &Transform) -> String {
    match t.scale_factors() {
        Some((sx, sy)) if (sx - sy).abs() < 1e-6 => format!("×{sx}"),
        Some((sx, sy)) => format!("×{sx}x{sy}"),
        None => "custom".to_owned(),
    }
}

fn state_from_output(out: &Output) -> OutputState {
    let Some(active) = &out.active else {
        return OutputState {
            enabled: false,
            mode: None,
            pos: Point::default(),
            rotation: Rotation::Normal,
            reflection: Reflection::Normal,
            transform: Transform::identity(),
            primary: false,
        };
    };
    let mode = out.mode(active.xid).cloned().unwrap_or_else(|| {
        // A stale output's mode is no longer listed: rebuild its size from the header.
        let mut size = active.size;
        if active.rotation.swaps_axes() {
            size = Size::new(size.h, size.w);
        }
        if let Some((sx, sy)) = active.transform.scale_factors() {
            size = Size::new(
                (f64::from(size.w) / sx).round() as i32,
                (f64::from(size.h) / sy).round() as i32,
            );
        }
        Mode {
            xid: active.xid,
            name: format!("{}x{}", size.w, size.h),
            width: size.w,
            height: size.h,
            refresh: 0.0,
            interlaced: false,
            double_scan: false,
            preferred: false,
        }
    });
    OutputState {
        enabled: true,
        mode: Some(mode),
        pos: active.pos,
        rotation: active.rotation,
        reflection: active.reflection,
        transform: active.transform.clone(),
        primary: out.primary,
    }
}

/// The mode to use after switching to a `w`x`h` resolution:
/// 1. the previous rate, if one within 0.5 Hz exists;
/// 2. otherwise the preferred rate, if this resolution is the preferred one;
/// 3. otherwise the highest progressive rate.
pub fn pick_rate(out: &Output, w: i32, h: i32, previous: Option<f64>) -> Option<&Mode> {
    let all = out.rates(w, h);
    let progressive: Vec<&Mode> = all.iter().copied().filter(|m| m.is_progressive()).collect();
    let pool = if progressive.is_empty() {
        &all
    } else {
        &progressive
    };
    if let Some(prev) = previous
        && let Some(m) = pool
            .iter()
            .filter(|m| (m.refresh - prev).abs() <= 0.5)
            .min_by(|a, b| {
                (a.refresh - prev)
                    .abs()
                    .total_cmp(&(b.refresh - prev).abs())
            })
    {
        return Some(m);
    }
    pool.iter()
        .find(|m| m.preferred)
        .or_else(|| pool.first())
        .copied()
}

/// One output's pending changes.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputDiff {
    pub index: usize,
    pub name: String,
    pub changes: Vec<Change>,
}

impl fmt::Display for OutputDiff {
    /// `DP-1-2  rate 59.95 → 143.91, pos 0,0 → 10,0`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let changes: Vec<String> = self.changes.iter().map(Change::to_string).collect();
        write!(f, "{}  {}", self.name, changes.join(", "))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    On(String),
    Off,
    Mode(String, String),
    Rate(f64, f64),
    Position(Point, Point),
    Rotation(Rotation, Rotation),
    Reflection(Reflection, Reflection),
    Scale(String, String),
    Primary(bool),
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Change::On(mode) => write!(f, "off → {mode}"),
            Change::Off => write!(f, "on → off"),
            Change::Mode(a, b) => write!(f, "mode {a} → {b}"),
            Change::Rate(a, b) => write!(f, "rate {a:.2} → {b:.2}"),
            Change::Position(a, b) => write!(f, "pos {},{} → {},{}", a.x, a.y, b.x, b.y),
            Change::Rotation(a, b) => write!(f, "rotate {a} → {b}"),
            Change::Reflection(a, b) => write!(f, "reflect {a} → {b}"),
            Change::Scale(a, b) => write!(f, "scale {a} → {b}"),
            Change::Primary(true) => write!(f, "primary"),
            Change::Primary(false) => write!(f, "not primary"),
        }
    }
}
