//! The editor state and its key handling. Pure: [`App::handle_key`] changes the state and returns
//! the effects (refresh, quit …) for the event loop to carry out, so tests drive the whole editor
//! without a terminal.

use std::borrow::Cow;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

use crate::backend::{Plan, Verdict, command_text, portable_command_text};
use crate::model::geometry::effective_size;
use crate::model::geometry::{Dir, Point, Rect};
use crate::model::history::History;
use crate::model::layout::SCALES;
use crate::model::layout::{CommitReport, EditError, Layout, OutputDiff};
use crate::model::links::{Align, Side, best_align};
use crate::model::profile::{Profile, Remap};
use crate::model::snap::SnapKind;
use crate::model::validate::{Issue, Severity, validate};
use crate::model::{Cap, Kind, Mode, ModeId, Output, Scaling, Snapshot};
use crate::profiles::{NO_WAYLAND_PROFILES, Stored};

use super::canvas::Viewport;
use super::cmdline::{self, Cmd};
use super::keys::{Action, Context, Keymap, normalise};
use super::theme::Theme;

/// Work for the event loop.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Re-read the live state and merge it into the pending layout: a full probe for `R`, a
    /// re-query that does not wake the hardware for the watch.
    Refresh {
        probe: bool,
    },
    Quit,
    /// Ask the display server whether it would take this plan; the answer goes to
    /// [`App::tested`].
    Test(Plan),
    /// Carry out the plan, verify the result, and start the countdown.
    Apply(ApplyRequest),
    /// Restore the layout that was live before the apply.
    Revert(RevertReason),
    /// Keep the applied layout.
    Keep,
    /// Put text on the clipboard with OSC 52.
    Copy(String),
    /// Read the profiles in the store and open the picker.
    ListProfiles,
    /// Read one profile, by name or path, and open it.
    OpenProfile(String),
    /// Save the pending layout under this profile name or path; asks before overwriting.
    SaveProfile(String),
    /// Write a profile that the user agreed to overwrite.
    WriteProfile(SavePlan),
}

/// A profile read from disk, with what it would do on the current outputs.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileItem {
    pub name: String,
    pub path: PathBuf,
    pub profile: Profile,
    /// The layout it gives with the default remap, for the preview.
    pub preview: Layout,
    /// What did not fit, with the default remap.
    pub notes: Vec<String>,
    /// Profile outputs that are not connected.
    pub unmatched: Vec<String>,
}

impl ProfileItem {
    pub fn new(snap: &Snapshot, stored: Stored) -> Self {
        let Stored {
            name,
            path,
            profile,
        } = stored;
        let remap = profile.default_remap(snap);
        let (preview, notes) = profile.layout(snap, &remap);
        Self {
            name,
            unmatched: profile.unmatched(snap),
            path,
            profile,
            preview,
            notes,
        }
    }
}

/// The profile picker (`e`).
#[derive(Clone, Debug, PartialEq)]
pub struct ProfilePicker {
    pub items: Vec<ProfileItem>,
    pub selected: usize,
}

/// Where a profile's missing outputs go: one row per output that is not connected.
#[derive(Clone, Debug, PartialEq)]
pub struct RemapDialog {
    pub item: ProfileItem,
    pub rows: Remap,
    pub selected: usize,
    /// Connected outputs the profile leaves free.
    pub free: Vec<usize>,
}

/// A save that would overwrite a different file: the new text and the diff to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavePlan {
    /// The profile's name.
    pub name: String,
    pub path: PathBuf,
    pub text: String,
    pub diff: Vec<String>,
}

/// What to apply: the plan, and the layout it should produce, for verification.
#[derive(Clone, Debug, PartialEq)]
pub struct ApplyRequest {
    pub plan: Plan,
    pub layout: Layout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevertReason {
    /// Nobody answered before the countdown ran out.
    Timeout,
    /// `n`, `Esc` or Ctrl-C.
    Declined,
    /// SIGHUP or SIGTERM.
    Signal,
}

/// The apply confirmation: the per-output diff, what blocks the apply, what to watch for, the
/// equivalent command, and the plan to carry out.
#[derive(Clone, Debug, PartialEq)]
pub struct ApplyPreview {
    pub changes: Vec<String>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub command: String,
    pub plan: Plan,
}

/// The "keep this layout?" countdown after an apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Countdown {
    pub started: Instant,
    pub deadline: Instant,
    /// Keys are ignored until then, so one pressed while the screens were dark cannot answer.
    pub blocked_until: Instant,
}

/// A popup with a report, such as why an apply failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub title: String,
    pub lines: Vec<String>,
}

/// How long keys are ignored after xrandr returns.
pub const INPUT_BLOCK: Duration = Duration::from_secs(1);

/// A nudge continues a held key's burst when it comes at most this long after the previous one.
/// The terminal's autorepeat delay is longer, so a single press always moves one step.
pub const NUDGE_BURST_GAP: Duration = Duration::from_millis(150);

/// How a held nudge speeds up: the step multiplier until each time since the burst started,
/// then [`NUDGE_RAMP_TOP`].
pub const NUDGE_RAMP: [(Duration, i32); 3] = [
    (Duration::from_millis(400), 1),
    (Duration::from_millis(800), 2),
    (Duration::from_millis(1200), 5),
];
pub const NUDGE_RAMP_TOP: i32 = 10;

/// The nudge steps `+` and `-` go through, in pixels.
pub const NUDGE_STEPS: [i32; 5] = [1, 5, 10, 50, 100];

/// How long displays take to glide to a new position after a discrete action.
pub const ANIMATION: Duration = Duration::from_millis(120);

/// How often the editor re-reads the live state, to pick up displays that were plugged in.
pub const WATCH_INTERVAL: Duration = Duration::from_secs(2);

/// Displays gliding from where they were drawn to where the edit put them.
#[derive(Clone, Debug, PartialEq)]
struct Animation {
    /// Per output: the position to start from, in the new coordinates; `None` for outputs that
    /// do not glide (turned on or off).
    from: Vec<Option<Point>>,
    started: Instant,
}

/// Ease-out: fast first, then settling. `t` runs from 0 to 1.
fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Nudges of one display in one direction, each within [`NUDGE_BURST_GAP`] of the last: a held
/// key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Burst {
    display: usize,
    dir: Dir,
    started: Instant,
    last: Instant,
}

impl Burst {
    fn multiplier(&self, now: Instant) -> i32 {
        let held = now.saturating_duration_since(self.started);
        NUDGE_RAMP
            .iter()
            .find(|&&(until, _)| held < until)
            .map_or(NUDGE_RAMP_TOP, |&(_, m)| m)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    pub severity: Severity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickStep {
    Target,
    Side,
}

/// The `s` flow: pick a target, then a side and an alignment.
#[derive(Clone, Debug, PartialEq)]
pub struct StickFlow {
    pub step: StickStep,
    /// The display being stuck.
    pub display: usize,
    pub target: usize,
    pub side: Side,
    pub align: Align,
    /// Where displays would go, in the current coordinates. Always includes the stuck display.
    pub ghosts: Vec<(usize, Rect)>,
    /// Why the stick would be refused.
    pub problem: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    Resolution(i32, i32),
    Mode(ModeId),
    Scale(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PickItem {
    pub label: String,
    pub pick: Pick,
}

/// The resolution picker (`m`), the rate picker (`r`) or the scale picker (`x`).
#[derive(Clone, Debug, PartialEq)]
pub struct Picker {
    pub output: usize,
    pub title: String,
    pub items: Vec<PickItem>,
    pub selected: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Question {
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UiMode {
    Normal,
    Stick(StickFlow),
    Picker(Picker),
    Command(String),
    Confirm(Question),
    Help {
        scroll: u16,
    },
    ConfirmApply(ApplyPreview),
    /// The apply is running.
    Applying,
    Countdown(Countdown),
    Message(Message),
    /// Typing a profile name to save under.
    SavePrompt(String),
    Profiles(ProfilePicker),
    Remap(Box<RemapDialog>),
    /// "Overwrite this profile?" with the diff.
    Overwrite(SavePlan),
}

/// Settings the editor starts with.
#[derive(Clone, Debug)]
pub struct Options {
    pub keymap: Keymap,
    pub theme: Theme,
    pub nudge_step: i32,
    /// The old look: double borders on the focused display's parent and the stick target.
    pub double_borders: bool,
    /// Displays glide to new positions. Off by default here, so tests draw final positions; the
    /// command line turns it on from the config.
    pub animations: bool,
    /// Cell height divided by cell width; `None` detects it from the terminal.
    pub cell_aspect: Option<f64>,
    /// Where the state comes from when it is not the live display server: `demo`, a file name.
    pub source: Option<String>,
    /// The Wayland compositor's name (`sway`), a label for the title bar.
    pub compositor: Option<String>,
    /// How often to look for displays that were plugged in or unplugged; `None` never looks.
    pub watch: Option<Duration>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            keymap: Keymap::default(),
            theme: Theme::default(),
            nudge_step: 10,
            double_borders: false,
            animations: false,
            cell_aspect: None,
            source: None,
            compositor: None,
            watch: None,
        }
    }
}

pub struct App {
    /// The live state, as last read.
    pub snap: Snapshot,
    /// The pending layout.
    pub layout: Layout,
    pub history: History,
    /// The focused output (an index into the snapshot). It may be off.
    pub focus: usize,
    pub mode: UiMode,
    pub status: Option<Status>,
    /// Validation of the pending layout, errors first.
    pub issues: Vec<Issue>,
    pub step: i32,
    pub show_details: bool,
    pub keymap: Keymap,
    pub theme: Theme,
    pub double_borders: bool,
    pub viewport: Viewport,
    pub cell_aspect: f64,
    pub source: Option<String>,
    pub compositor: Option<String>,
    /// The time of the last tick; the countdown and the input block compare against it.
    pub now: Instant,
    /// The held nudge key, if any.
    burst: Option<Burst>,
    /// The display and direction of the last nudges when nothing else happened since: they
    /// share one undo step.
    nudge_run: Option<(usize, Dir)>,
    /// The profile last opened or saved: the save prompt starts with it.
    pub profile: Option<String>,
    pub animations: bool,
    animation: Option<Animation>,
    watch: Option<Duration>,
    /// When the watch next re-reads the live state.
    next_watch: Instant,
}

impl App {
    pub fn new(snap: Snapshot, options: Options) -> Self {
        let (layout, notes) = Layout::from_snapshot(&snap);
        let now = Instant::now();
        let mut app = App {
            focus: 0,
            layout,
            snap,
            history: History::default(),
            mode: UiMode::Normal,
            status: None,
            issues: Vec::new(),
            step: options.nudge_step,
            show_details: true,
            keymap: options.keymap,
            theme: options.theme,
            double_borders: options.double_borders,
            viewport: Viewport::default(),
            cell_aspect: options.cell_aspect.unwrap_or(2.0),
            source: options.source,
            compositor: options.compositor,
            now,
            burst: None,
            nudge_run: None,
            profile: None,
            animations: options.animations,
            animation: None,
            watch: options.watch,
            next_watch: now + options.watch.unwrap_or_default(),
        };
        app.focus = app.default_focus();
        app.revalidate();
        if !notes.is_empty() {
            app.say(Severity::Warning, notes.join(" "));
        }
        app
    }

    /// The primary display, else the first enabled one, else the first numbered one.
    fn default_focus(&self) -> usize {
        self.layout
            .primary()
            .or_else(|| self.layout.enabled().first().copied())
            .or_else(|| self.snap.numbered().first().copied())
            .unwrap_or(0)
    }

    pub fn say(&mut self, severity: Severity, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            severity,
        });
    }

    fn revalidate(&mut self) {
        self.issues = validate(&self.layout, &self.snap);
    }

    /// Per-output differences between the pending layout and the live state.
    pub fn pending(&self) -> Vec<OutputDiff> {
        self.layout.diff(&self.snap)
    }

    /// Which outputs have pending changes, indexed like the layout.
    pub fn pending_flags(&self) -> Vec<bool> {
        let mut flags = vec![false; self.layout.len()];
        for d in self.pending() {
            flags[d.index] = true;
        }
        flags
    }

    pub fn context(&self) -> Context {
        match &self.mode {
            UiMode::Normal => Context::Normal,
            UiMode::Stick(flow) if flow.step == StickStep::Target => Context::StickTarget,
            UiMode::Stick(_) => Context::StickSide,
            UiMode::Picker(_) => Context::Picker,
            UiMode::Command(_) => Context::Command,
            UiMode::Confirm(_) => Context::Confirm,
            UiMode::ConfirmApply(_) => Context::ConfirmApply,
            UiMode::Help { .. } => Context::Help,
            UiMode::Applying => Context::Applying,
            UiMode::Countdown(_) => Context::Countdown,
            UiMode::Message(_) => Context::Message,
            UiMode::SavePrompt(_) => Context::SavePrompt,
            UiMode::Profiles(_) => Context::Profiles,
            UiMode::Remap(_) => Context::Remap,
            UiMode::Overwrite(_) => Context::Confirm,
        }
    }

    /// Advances the clock; returns a revert when the countdown has run out, and a refresh when
    /// the watch is due. The watch only runs in normal mode: popups and the countdown hold output
    /// indices that a refresh would move.
    pub fn tick(&mut self, now: Instant) -> Vec<Effect> {
        self.now = now;
        if !self.animating() {
            self.animation = None;
        }
        match (&self.mode, self.watch) {
            (UiMode::Countdown(c), _) if now >= c.deadline => {
                self.mode = UiMode::Applying;
                vec![Effect::Revert(RevertReason::Timeout)]
            }
            (UiMode::Normal, Some(every)) if now >= self.next_watch => {
                self.next_watch = now + every;
                vec![Effect::Refresh { probe: false }]
            }
            _ => Vec::new(),
        }
    }

    /// Whether displays are still gliding; the event loop then redraws often.
    pub fn animating(&self) -> bool {
        self.animation
            .as_ref()
            .is_some_and(|a| self.now < a.started + ANIMATION)
    }

    /// Where each output is drawn now: on its way to its position while gliding.
    fn drawn_positions(&self) -> Vec<Point> {
        self.drawn_layout().outputs.iter().map(|o| o.pos).collect()
    }

    /// The layout as it is drawn at this moment: the pending one, with gliding displays part of
    /// the way there.
    pub fn drawn_layout(&self) -> Cow<'_, Layout> {
        let Some(a) = self.animation.as_ref().filter(|_| self.animating()) else {
            return Cow::Borrowed(&self.layout);
        };
        let t =
            self.now.saturating_duration_since(a.started).as_secs_f64() / ANIMATION.as_secs_f64();
        let e = ease_out(t);
        let mut layout = self.layout.clone();
        for (st, from) in layout.outputs.iter_mut().zip(&a.from) {
            if let Some(p) = from {
                let lerp = |a: i32, b: i32| a + (f64::from(b - a) * e).round() as i32;
                st.pos = Point::new(lerp(p.x, st.pos.x), lerp(p.y, st.pos.y));
            }
        }
        Cow::Owned(layout)
    }

    /// Starts a glide from `drawn` (where outputs were drawn before the edit, in the old
    /// coordinates) to the pending layout, which is `shift` away.
    fn glide(&mut self, before: &Layout, drawn: &[Point], shift: Point) {
        if !self.animations {
            return;
        }
        let from: Vec<Option<Point>> = (0..self.layout.len())
            .map(|i| {
                (before.is_enabled(i) && self.layout.is_enabled(i))
                    .then(|| Point::new(drawn[i].x + shift.x, drawn[i].y + shift.y))
            })
            .collect();
        let moves = from
            .iter()
            .zip(&self.layout.outputs)
            .any(|(f, st)| f.is_some_and(|p| p != st.pos));
        self.animation = moves.then_some(Animation {
            from,
            started: self.now,
        });
    }

    /// Whether the countdown is running.
    pub fn counting_down(&self) -> bool {
        matches!(self.mode, UiMode::Countdown(_))
    }

    pub fn set_cell_aspect(&mut self, aspect: f64) {
        self.cell_aspect = aspect;
    }

    /// Moves the pending layout and every undo step onto `snap`, matching outputs by name (see
    /// [`Layout::remapped`]). Focus stays on the same output while it is still relevant.
    fn rebase(&mut self, snap: Snapshot) {
        let focused = self.layout.names.get(self.focus).cloned();
        let layout = self.layout.remapped(&self.snap, &snap);
        let numbers = layout.numbers.clone();
        self.history.remap(|l| Layout {
            numbers: numbers.clone(),
            ..l.remapped(&self.snap, &snap)
        });
        self.layout = layout;
        self.snap = snap;
        self.focus = focused
            .and_then(|name| self.snap.find(&name))
            .filter(|&i| self.snap.outputs[i].is_relevant())
            .unwrap_or_else(|| self.default_focus());
        self.burst = None;
        self.nudge_run = None;
        self.animation = None;
    }

    /// Merges a fresh reading of the live state, keeping the pending edits and the undo
    /// history. `probe` is set for `R`, which also reports that nothing changed; the watch stays
    /// silent then.
    ///
    /// A display unplugged while on is turned off in the pending layout, as one undo step; focus
    /// moves to a display that was just connected, so Space turns it on. When nothing was
    /// pending and the live layout changed outside outlay, the pending layout follows it.
    pub fn refreshed(&mut self, snap: Snapshot, probe: bool) {
        if self.mode != UiMode::Normal {
            // A key opened a popup after the watch asked; try again on the next tick.
            self.next_watch = self.now;
            return;
        }
        if snap == self.snap {
            if probe {
                self.say(Severity::Info, "No display changes.");
            }
            return;
        }
        let plugs = Plugs::between(&self.snap, &snap);
        let untouched = self.pending().is_empty();
        let enabled_before = self.enabled_names();
        self.rebase(snap);
        if plugs.outside && untouched {
            let mut layout = Layout::inferred(&self.snap);
            layout.numbers = self.layout.numbers.clone();
            self.layout = layout;
        }

        let unplugged: Vec<usize> = plugs
            .disconnected
            .iter()
            .filter_map(|name| self.snap.find(name))
            .filter(|&i| self.layout.is_enabled(i))
            .collect();
        let keeps_one = self.layout.enabled().iter().any(|i| !unplugged.contains(i));
        let mut turned_off = Vec::new();
        let mut notes = Vec::new();
        if keeps_one {
            let before = self.layout.clone();
            for &d in &unplugged {
                if let Ok(report) = self.layout.turn_off(d) {
                    turned_off.push(self.layout.names[d].clone());
                    notes.extend(report.notes);
                }
            }
            self.history.record(before, &self.layout);
        }

        let connected: Vec<usize> = plugs
            .connected
            .iter()
            .filter_map(|name| self.snap.find(name))
            .collect();
        let off = connected
            .iter()
            .copied()
            .find(|&i| !self.layout.is_enabled(i));
        if let Some(&first) = connected.first() {
            self.focus = off.unwrap_or(first);
        }
        if self.enabled_names() != enabled_before {
            self.viewport.refit();
        }
        self.revalidate();

        let mut text = Vec::new();
        if !turned_off.is_empty() {
            let (were, they) = if turned_off.len() == 1 {
                ("was", "it is")
            } else {
                ("were", "they are")
            };
            text.push(format!(
                "{} {were} unplugged while on; {they} turned off in the pending layout.",
                listing(&turned_off)
            ));
        }
        let gone: Vec<String> = plugs
            .disconnected
            .iter()
            .filter(|name| !turned_off.contains(name))
            .cloned()
            .collect();
        if !gone.is_empty() {
            text.push(format!("{} disconnected.", listing(&gone)));
        }
        if !connected.is_empty() {
            let names: Vec<String> = connected
                .iter()
                .map(|&i| self.layout.names[i].clone())
                .collect();
            let toggle = self
                .keymap
                .key_for(Context::Normal, Action::Toggle)
                .unwrap_or_default();
            text.push(match off {
                Some(_) if connected.len() == 1 => {
                    format!("{} connected: {toggle} turns it on.", names[0])
                }
                Some(i) => format!(
                    "{} connected: {toggle} turns {} on.",
                    listing(&names),
                    self.layout.names[i]
                ),
                None => format!("{} connected.", listing(&names)),
            });
        }
        if text.is_empty() {
            text.push("The live state changed outside outlay.".to_owned());
            if !untouched {
                text.push("Your edits are still pending.".to_owned());
            }
        }
        text.extend(notes);
        let severity = if turned_off.is_empty() {
            Severity::Info
        } else {
            Severity::Warning
        };
        self.say(severity, text.join(" "));
    }

    fn enabled_names(&self) -> Vec<String> {
        self.layout
            .enabled()
            .into_iter()
            .map(|i| self.layout.names[i].clone())
            .collect()
    }

    pub fn handle_key(&mut self, ev: KeyEvent) -> Vec<Effect> {
        let Some(key) = normalise(&ev) else {
            return Vec::new();
        };
        let context = self.context();
        let found = self.keymap.lookup(context, key);
        let action = found.map(|(action, _)| action);
        // Any other key ends a held nudge; any other action ends a run of nudges.
        if !(context == Context::Normal && matches!(action, Some(Action::Nudge(_)))) {
            self.burst = None;
            if action.is_some() {
                self.nudge_run = None;
            }
        }
        if let UiMode::Countdown(c) = self.mode
            && self.now < c.blocked_until
        {
            return Vec::new();
        }
        if let Some((_, Some(cap))) = found
            && !self.layout.caps.has(cap)
        {
            self.missing(cap);
            return Vec::new();
        }
        match action {
            Some(action) => self.perform(context, action),
            None => {
                if let (
                    UiMode::Command(line) | UiMode::SavePrompt(line),
                    ratatui::crossterm::event::KeyCode::Char(c),
                ) = (&mut self.mode, key.code)
                    && (key.mods - KeyModifiers::SHIFT).is_empty()
                {
                    line.push(c);
                }
                Vec::new()
            }
        }
    }

    /// A key or command needs what the display server cannot do: say so, and change nothing.
    fn missing(&mut self, cap: Cap) {
        if let UiMode::Stick(flow) = &mut self.mode {
            // The stick flow's own line hides the status.
            flow.problem = Some(cap.missing().trim_end_matches('.').to_owned());
        }
        self.say(Severity::Warning, cap.missing());
    }

    fn perform(&mut self, context: Context, action: Action) -> Vec<Effect> {
        match context {
            Context::Normal => self.normal(action),
            Context::StickTarget | Context::StickSide => {
                self.stick_key(action);
                Vec::new()
            }
            Context::Picker => {
                self.picker_key(action);
                Vec::new()
            }
            Context::Command => self.command_key(action),
            Context::SavePrompt => self.save_prompt_key(action),
            Context::Profiles => {
                self.profiles_key(action);
                Vec::new()
            }
            Context::Remap => {
                self.remap_key(action);
                Vec::new()
            }
            Context::Confirm | Context::ConfirmApply => self.confirm_key(action),
            Context::Countdown => {
                self.mode = UiMode::Applying;
                match action {
                    Action::Keep => vec![Effect::Keep],
                    Action::Revert => vec![Effect::Revert(RevertReason::Declined)],
                    Action::RevertQuit => {
                        vec![Effect::Revert(RevertReason::Declined), Effect::Quit]
                    }
                    _ => Vec::new(),
                }
            }
            Context::Message => {
                self.mode = UiMode::Normal;
                Vec::new()
            }
            Context::Applying => Vec::new(),
            Context::Help => {
                if let UiMode::Help { scroll } = &mut self.mode {
                    match action {
                        Action::Move(Dir::Down) => *scroll = scroll.saturating_add(1),
                        Action::Move(Dir::Up) => *scroll = scroll.saturating_sub(1),
                        Action::Cancel => self.mode = UiMode::Normal,
                        _ => {}
                    }
                }
                Vec::new()
            }
        }
    }

    /// Runs one edit. A successful edit becomes an undo step and the viewport follows its
    /// normalisation shift; a failed or no-op edit only sets the status.
    fn edit<T>(
        &mut self,
        f: impl FnOnce(&mut Layout, &Snapshot) -> Result<T, EditError>,
        report: impl Fn(&T) -> &CommitReport,
    ) -> Option<T> {
        self.edit_step(true, true, f, report)
    }

    /// [`App::edit`], adding an undo step only when `record` is set (an edit that continues the
    /// previous one shares its step), and gliding into place only when `animate` is set.
    fn edit_step<T>(
        &mut self,
        record: bool,
        animate: bool,
        f: impl FnOnce(&mut Layout, &Snapshot) -> Result<T, EditError>,
        report: impl Fn(&T) -> &CommitReport,
    ) -> Option<T> {
        let before = self.layout.clone();
        let drawn = self.drawn_positions();
        match f(&mut self.layout, &self.snap) {
            Ok(done) => {
                let r = report(&done);
                self.viewport.shift(r.shift);
                if animate {
                    self.glide(&before, &drawn, r.shift);
                } else {
                    self.animation = None;
                }
                let mut notes = r.notes.clone();
                if !r.pushed.is_empty() {
                    let names: Vec<String> =
                        r.pushed.iter().map(|&i| self.layout.label(i)).collect();
                    notes.insert(0, format!("Pushed {} out of the way.", names.join(", ")));
                }
                if record {
                    self.history.record(before, &self.layout);
                }
                self.revalidate();
                self.status = None;
                if !notes.is_empty() {
                    self.say(Severity::Info, notes.join(" "));
                }
                Some(done)
            }
            Err(err) => {
                self.layout = before;
                let severity = match err {
                    EditError::Refused(_) => Severity::Warning,
                    EditError::NoChange(_) | EditError::NoSnapSpot(_) => Severity::Info,
                };
                let text = match err {
                    EditError::NoSnapSpot(dir) => {
                        let nudge = self
                            .keymap
                            .key_for(Context::Normal, Action::Nudge(dir))
                            .unwrap_or_default();
                        format!(
                            "No snap spot further {}. {nudge} nudges freely.",
                            dir.word()
                        )
                    }
                    EditError::NoChange(text) => capitalise(&text),
                    EditError::Refused(text) => text,
                };
                self.say(severity, text);
                None
            }
        }
    }

    fn simple_edit(
        &mut self,
        f: impl FnOnce(&mut Layout, &Snapshot) -> Result<CommitReport, EditError>,
    ) -> bool {
        self.edit(f, |r| r).is_some()
    }

    /// Undo and redo restore whole layouts; the viewport follows the shift most displays made,
    /// so the ones that stay put do not jump.
    fn follow(&mut self, before: &Layout) -> Point {
        let mut deltas: Vec<Point> = (0..self.layout.len())
            .filter(|&i| before.is_enabled(i) && self.layout.is_enabled(i))
            .filter(|&i| before.size(i) == self.layout.size(i))
            .map(|i| {
                let (a, b) = (before.outputs[i].pos, self.layout.outputs[i].pos);
                Point::new(b.x - a.x, b.y - a.y)
            })
            .collect();
        deltas.sort_by_key(|p| (p.x, p.y));
        let common = deltas
            .chunk_by(|a, b| a == b)
            .max_by_key(|run| run.len())
            .map(|run| run[0]);
        let shift = common.unwrap_or_default();
        self.viewport.shift(shift);
        shift
    }

    fn normal(&mut self, action: Action) -> Vec<Effect> {
        let f = self.focus;
        match action {
            Action::Focus(dir) => {
                if self.layout.is_enabled(f) {
                    if let Some(next) = self.layout.focus_towards(f, dir) {
                        self.focus = next;
                    }
                } else {
                    self.focus = self.default_focus();
                }
            }
            Action::FocusNext | Action::FocusPrev => {
                // Mirror children share their root's box, so focus skips them.
                let order: Vec<usize> = self
                    .snap
                    .numbered()
                    .into_iter()
                    .filter(|&i| !self.layout.is_mirror_child(i))
                    .collect();
                let n = order.len();
                let at = order.iter().position(|&i| i == self.layout.mirror_root(f));
                self.focus = match at {
                    Some(k) if action == Action::FocusNext => order[(k + 1) % n],
                    Some(k) => order[(k + n - 1) % n],
                    None => order.first().copied().unwrap_or(f),
                };
            }
            Action::FocusNumber(n) => match self.by_number(n) {
                Some(i) => self.focus = i,
                None => self.say(Severity::Info, format!("There is no display {n}.")),
            },
            Action::Snap(dir) => {
                if let Some(done) = self.edit(|l, _| l.snap_move(f, dir), |s| &s.report)
                    && let SnapKind::Swapped(n) = done.kind
                {
                    let text = format!(
                        "Swapped {} and {}.",
                        self.layout.label(f),
                        self.layout.label(n)
                    );
                    self.say(Severity::Info, text);
                }
            }
            Action::Nudge(dir) => self.nudge(dir),
            Action::Stick => self.start_stick(),
            Action::Unstick => {
                if self.simple_edit(|l, _| l.unstick(f)) {
                    let text = format!("{} is an anchor now.", self.layout.label(f));
                    self.say(Severity::Info, text);
                }
            }
            Action::ModePicker => self.open_resolutions(),
            Action::RatePicker => self.open_rates(),
            Action::Smaller | Action::Larger => {
                let larger = action == Action::Larger;
                self.simple_edit(|l, s| l.step_resolution(s, f, larger));
            }
            Action::SlowerRate | Action::FasterRate => {
                let higher = action == Action::FasterRate;
                self.simple_edit(|l, s| l.step_rate(s, f, higher));
            }
            Action::ScalePicker => self.open_scales(),
            Action::SmallerScale | Action::LargerScale => {
                let larger = action == Action::LargerScale;
                self.simple_edit(|l, _| l.step_scale(f, larger));
            }
            Action::RotateCw | Action::RotateCcw => {
                let cw = action == Action::RotateCw;
                self.simple_edit(|l, _| l.rotate(f, cw));
            }
            Action::Primary => {
                self.simple_edit(|l, _| l.set_primary(f));
            }
            Action::Toggle => self.toggle(f),
            Action::Undo | Action::Redo => {
                let before = self.layout.clone();
                let drawn = self.drawn_positions();
                let done = if action == Action::Undo {
                    self.history.undo(&mut self.layout)
                } else {
                    self.history.redo(&mut self.layout)
                };
                let word = if action == Action::Undo {
                    "undo"
                } else {
                    "redo"
                };
                if done {
                    let shift = self.follow(&before);
                    self.glide(&before, &drawn, shift);
                    self.revalidate();
                    self.status = None;
                } else {
                    self.say(Severity::Info, format!("Nothing to {word}."));
                }
            }
            Action::Refresh => return vec![Effect::Refresh { probe: true }],
            Action::Command => self.mode = UiMode::Command(String::new()),
            Action::Refit => self.viewport.refit(),
            Action::Details => self.show_details = !self.show_details,
            Action::Help => self.mode = UiMode::Help { scroll: 0 },
            Action::Quit => return self.quit(false),
            Action::Apply => return self.open_apply(),
            Action::Copy => {
                let plan = Plan::pending(&self.layout, &self.snap);
                let text = portable_command_text(self.layout.caps.kind, &plan);
                return vec![Effect::Copy(text)];
            }
            Action::Save | Action::Open if !self.has_profiles() => {}
            Action::Save => {
                self.mode = UiMode::SavePrompt(self.profile.clone().unwrap_or_default())
            }
            Action::Open => return vec![Effect::ListProfiles],
            Action::StepUp | Action::StepDown => {
                let up = action == Action::StepUp;
                let next = if up {
                    NUDGE_STEPS.iter().find(|&&s| s > self.step)
                } else {
                    NUDGE_STEPS.iter().rev().find(|&&s| s < self.step)
                };
                match next {
                    Some(&step) => {
                        self.step = step;
                        self.say(Severity::Info, format!("Nudge step {step} px."));
                    }
                    None => {
                        let end = if up { "largest" } else { "smallest" };
                        let text = format!("{} px is the {end} nudge step.", self.step);
                        self.say(Severity::Info, text);
                    }
                }
            }
            _ => {}
        }
        Vec::new()
    }

    /// Nudges the focused display by the step, times the multiplier of a held key. Nudges of one
    /// display in one direction with nothing in between are one undo step.
    fn nudge(&mut self, dir: Dir) {
        let f = self.focus;
        let now = self.now;
        let burst = match self.burst {
            Some(b)
                if b.display == f
                    && b.dir == dir
                    && now.saturating_duration_since(b.last) <= NUDGE_BURST_GAP =>
            {
                Burst { last: now, ..b }
            }
            _ => Burst {
                display: f,
                dir,
                started: now,
                last: now,
            },
        };
        self.burst = Some(burst);
        let step = self.step * burst.multiplier(now);
        let continues = self.nudge_run == Some((f, dir));
        // Nudges never glide, so a held key does not lag behind.
        let moved = self
            .edit_step(!continues, false, |l, _| l.nudge(f, dir, step), |r| r)
            .is_some();
        if moved {
            self.nudge_run = Some((f, dir));
        } else if !continues {
            self.nudge_run = None;
        }
    }

    /// The multiplier of a held nudge key, or 1 when no key is held.
    pub fn nudge_multiplier(&self) -> i32 {
        match self.burst {
            Some(b) if self.now.saturating_duration_since(b.last) <= NUDGE_BURST_GAP => {
                b.multiplier(b.last)
            }
            _ => 1,
        }
    }

    /// The apply confirmation for the pending layout, and a test of its plan. With no changes it
    /// re-applies the live layout, which is a safe way to try the apply flow.
    fn open_apply(&mut self) -> Vec<Effect> {
        let mut changes: Vec<String> = self.pending().iter().map(ToString::to_string).collect();
        if changes.is_empty() {
            changes.push("No changes: this re-applies the current layout.".to_owned());
        }
        let text = |i: &Issue| i.message.clone();
        let errors = self
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(text)
            .collect();
        let warnings = self
            .issues
            .iter()
            .filter(|i| i.severity != Severity::Error)
            .map(text)
            .collect();
        let plan = Plan::pending(&self.layout, &self.snap);
        self.mode = UiMode::ConfirmApply(ApplyPreview {
            changes,
            errors,
            warnings,
            command: command_text(self.layout.caps.kind, &plan),
            plan: plan.clone(),
        });
        vec![Effect::Test(plan)]
    }

    /// The display server's answer to the plan in the confirmation: a rejection blocks the
    /// apply like a validation error.
    pub fn tested(&mut self, verdict: Verdict) {
        if let (UiMode::ConfirmApply(preview), Verdict::Rejected(why)) = (&mut self.mode, verdict)
            && !preview.errors.contains(&why)
        {
            preview.errors.push(why);
        }
    }

    /// The countdown starts: the apply went through and came out as asked.
    pub fn countdown(&mut self, now: Instant, seconds: u64) {
        self.now = now;
        self.mode = UiMode::Countdown(Countdown {
            started: now,
            deadline: now + Duration::from_secs(seconds),
            blocked_until: now + INPUT_BLOCK,
        });
        self.status = None;
    }

    /// Shows a report and leaves the pending edits alone.
    pub fn report(&mut self, title: impl Into<String>, lines: Vec<String>) {
        self.mode = UiMode::Message(Message {
            title: title.into(),
            lines,
        });
    }

    /// Takes a fresh reading of the live state after an apply or a revert, without touching the
    /// pending layout. Outputs that came or went meanwhile are merged as a refresh merges them.
    pub fn adopt(&mut self, snap: Snapshot) {
        self.rebase(snap);
        self.revalidate();
    }

    /// The user kept the applied layout, now read back as `snap`. The in-memory links survive
    /// when the geometry matches; otherwise they are inferred again.
    pub fn kept(&mut self, snap: Snapshot) {
        let before = self.layout.clone();
        let matches = self.layout.mismatches(&snap).is_empty();
        if matches {
            // A Wayland scale the compositor rounded is what the display has now.
            for (st, out) in self.layout.outputs.iter_mut().zip(&snap.outputs) {
                if let (Some(live), true) = (&out.active, st.enabled)
                    && matches!(live.scaling, Scaling::Logical(_))
                {
                    st.scaling = live.scaling.clone();
                }
            }
        }
        self.adopt(snap);
        if !matches {
            let mut layout = Layout::from_snapshot(&self.snap).0;
            layout.numbers = self.layout.numbers.clone();
            self.layout = layout;
            if before.names == self.layout.names {
                let _ = self.follow(&before);
            } else {
                self.viewport.refit();
            }
            self.revalidate();
        }
        self.mode = UiMode::Normal;
        self.say(Severity::Info, "Kept the new layout.");
    }

    /// The previous layout is back, read as `snap` if the read worked. The edits stay pending.
    pub fn reverted(&mut self, snap: Option<Snapshot>, reason: RevertReason, seconds: u64) {
        if let Some(snap) = snap {
            self.adopt(snap);
        }
        self.mode = UiMode::Normal;
        let text = match reason {
            RevertReason::Timeout => format!(
                "No answer in {seconds} s: reverted to the previous layout. Your edits are still pending."
            ),
            RevertReason::Declined | RevertReason::Signal => {
                "Reverted to the previous layout. Your edits are still pending.".to_owned()
            }
        };
        self.say(Severity::Warning, text);
    }

    fn quit(&mut self, force: bool) -> Vec<Effect> {
        if force || self.pending().is_empty() {
            return vec![Effect::Quit];
        }
        self.mode = UiMode::Confirm(Question::Quit);
        Vec::new()
    }

    fn by_number(&self, n: usize) -> Option<usize> {
        (0..self.layout.len()).find(|&i| self.layout.numbers[i] == Some(n))
    }

    fn confirm_key(&mut self, action: Action) -> Vec<Effect> {
        if let UiMode::Overwrite(plan) = &self.mode {
            let plan = plan.clone();
            self.mode = UiMode::Normal;
            return match action {
                Action::Accept => vec![Effect::WriteProfile(plan)],
                _ => {
                    self.say(Severity::Info, "Not saved.");
                    Vec::new()
                }
            };
        }
        if let UiMode::ConfirmApply(preview) = &self.mode {
            return match action {
                Action::Accept if !preview.errors.is_empty() => {
                    self.say(Severity::Error, "Fix the errors before applying.");
                    Vec::new()
                }
                Action::Accept => {
                    let plan = preview.plan.clone();
                    self.mode = UiMode::Applying;
                    vec![Effect::Apply(ApplyRequest {
                        plan,
                        layout: self.layout.clone(),
                    })]
                }
                _ => {
                    self.mode = UiMode::Normal;
                    Vec::new()
                }
            };
        }
        let UiMode::Confirm(question) = self.mode else {
            return Vec::new();
        };
        self.mode = UiMode::Normal;
        match (action, question) {
            (Action::Accept, Question::Quit) => vec![Effect::Quit],
            _ => Vec::new(),
        }
    }

    // --- Stick flow -------------------------------------------------------------------------

    /// Displays `f` can stick to: every box but its own.
    fn stick_targets(&self, f: usize) -> Vec<usize> {
        self.snap
            .numbered()
            .into_iter()
            .filter(|&i| i != f && self.layout.is_enabled(i) && !self.layout.is_mirror_child(i))
            .collect()
    }

    fn start_stick(&mut self) {
        let f = self.focus;
        if !self.layout.is_enabled(f) {
            let text = format!("{} is off.", self.layout.label(f));
            self.say(Severity::Info, text);
            return;
        }
        let targets = self.stick_targets(f);
        let fr = self.layout.rect(f);
        let parent = self.layout.links[f]
            .map(|l| self.layout.mirror_root(l.parent))
            .filter(|p| targets.contains(p));
        let nearest = targets.iter().copied().min_by_key(|&t| {
            let (tx, ty) = self.layout.rect(t).center2();
            let (fx, fy) = fr.center2();
            (tx - fx).pow(2) + (ty - fy).pow(2)
        });
        let Some(target) = parent.or(nearest) else {
            self.say(Severity::Info, "There is nothing to stick to.");
            return;
        };
        let mut flow = StickFlow {
            step: StickStep::Target,
            display: f,
            target,
            side: Side::RightOf,
            align: Align::Start,
            ghosts: Vec::new(),
            problem: None,
        };
        if targets.len() == 1 {
            self.enter_side(&mut flow);
        }
        self.status = None;
        self.mode = UiMode::Stick(flow);
    }

    /// Moves to the side step, starting from the current link to the target, else from where
    /// the display sits now.
    fn enter_side(&self, flow: &mut StickFlow) {
        let (f, t) = (flow.display, flow.target);
        let (fr, tr) = (self.layout.rect(f), self.layout.rect(t));
        flow.step = StickStep::Side;
        match self.layout.links[f] {
            Some(link) if self.layout.mirror_root(link.parent) == t => {
                flow.side = link.side;
                flow.align = link.align;
            }
            _ => {
                if let Some((dir, _)) = tr.shared_edge(&fr) {
                    flow.side = Side::from_dir(dir);
                    flow.align = best_align(flow.side, fr, tr).0;
                } else {
                    let (fx, fy) = fr.center2();
                    let (tx, ty) = tr.center2();
                    let (dx, dy) = (fx - tx, fy - ty);
                    let across =
                        dx.abs() * i64::from(tr.h.max(1)) >= dy.abs() * i64::from(tr.w.max(1));
                    flow.side = match (across, dx >= 0, dy >= 0) {
                        (true, true, _) => Side::RightOf,
                        (true, false, _) => Side::LeftOf,
                        (false, _, true) => Side::Below,
                        (false, _, false) => Side::Above,
                    };
                    flow.align = Align::default_for(flow.side);
                }
            }
        }
        self.preview(flow);
    }

    /// Fills in where the displays would go if the flow were committed now.
    fn preview(&self, flow: &mut StickFlow) {
        let (f, t) = (flow.display, flow.target);
        let mut trial = self.layout.clone();
        flow.ghosts.clear();
        flow.problem = None;
        match trial.stick(&self.snap, f, t, flow.side, flow.align) {
            Ok(report) => {
                let back = |r: Rect| r.translated(-report.shift.x, -report.shift.y);
                for i in trial.enabled() {
                    if trial.is_mirror_child(i) {
                        continue;
                    }
                    let r = back(trial.rect(i));
                    if i == f || r != self.layout.rect(i) {
                        flow.ghosts.push((i, r));
                    }
                }
            }
            Err(err) => flow.problem = Some(err.to_string()),
        }
    }

    fn stick_key(&mut self, action: Action) {
        let UiMode::Stick(mut flow) = std::mem::replace(&mut self.mode, UiMode::Normal) else {
            return;
        };
        let targets = self.stick_targets(flow.display);
        let keep = match (flow.step, action) {
            (_, Action::Cancel) => false,
            (StickStep::Target, Action::TargetNumber(n)) => match self.by_number(n) {
                Some(t) if targets.contains(&t) => {
                    flow.target = t;
                    self.enter_side(&mut flow);
                    true
                }
                _ => {
                    self.say(Severity::Info, format!("Display {n} cannot be the target."));
                    true
                }
            },
            (StickStep::Target, Action::Target(dir)) => {
                let skip: Vec<usize> = (0..self.layout.len())
                    .filter(|i| !targets.contains(i))
                    .collect();
                if let Some(t) = self.layout.nearest_towards(flow.target, dir, &skip) {
                    flow.target = t;
                }
                true
            }
            (StickStep::Target, Action::TargetNext | Action::TargetPrev) => {
                if let Some(at) = targets.iter().position(|&t| t == flow.target) {
                    let n = targets.len();
                    let next = if action == Action::TargetNext {
                        (at + 1) % n
                    } else {
                        (at + n - 1) % n
                    };
                    flow.target = targets[next];
                }
                true
            }
            (StickStep::Target, Action::Accept) => {
                self.enter_side(&mut flow);
                true
            }
            (StickStep::Side, Action::Side(dir)) => {
                flow.side = Side::from_dir(dir);
                flow.align = Align::default_for(flow.side);
                self.preview(&mut flow);
                true
            }
            (StickStep::Side, Action::Mirror) => {
                flow.side = Side::Same;
                self.preview(&mut flow);
                true
            }
            (StickStep::Side, Action::AlignNext | Action::AlignPrev) => {
                if flow.side != Side::Same {
                    flow.align = if action == Action::AlignNext {
                        flow.align.next()
                    } else {
                        flow.align.previous()
                    };
                    self.preview(&mut flow);
                }
                true
            }
            (StickStep::Side, Action::Back) => {
                flow.step = StickStep::Target;
                flow.ghosts.clear();
                flow.problem = None;
                targets.len() > 1
            }
            (StickStep::Side, Action::Accept) => {
                let (f, t, side, align) = (flow.display, flow.target, flow.side, flow.align);
                if self.simple_edit(|l, s| l.stick(s, f, t, side, align)) {
                    let text = format!(
                        "Stuck {} {}.",
                        self.layout.label(f),
                        self.layout.link_text(f)
                    );
                    if self.status.is_none() {
                        self.say(Severity::Info, text);
                    }
                }
                false
            }
            _ => true,
        };
        if keep {
            self.mode = UiMode::Stick(flow);
        }
    }

    /// The status line text of the stick flow.
    pub fn stick_summary(&self, flow: &StickFlow) -> String {
        let who = self.layout.label(flow.display);
        let target = self.layout.label(flow.target);
        let text = match flow.step {
            StickStep::Target => format!("Stick {who} to {target}?"),
            StickStep::Side if flow.side == Side::Same => format!("Stick {who}: mirror {target}"),
            StickStep::Side => format!(
                "Stick {who} {} {target}, {}",
                flow.side.as_str(),
                flow.align.label(flow.side)
            ),
        };
        match &flow.problem {
            Some(problem) => format!("{text} ({problem})"),
            None => text,
        }
    }

    // --- Pickers ----------------------------------------------------------------------------

    fn picker_mode(&mut self) -> Option<Mode> {
        let f = self.focus;
        if !self.layout.is_enabled(f) {
            let toggle = self
                .keymap
                .key_for(Context::Normal, Action::Toggle)
                .unwrap_or_default();
            let text = format!("{} is off; {toggle} turns it on.", self.layout.label(f));
            self.say(Severity::Info, text);
            return None;
        }
        self.layout.outputs[f].mode.clone()
    }

    fn open_resolutions(&mut self) {
        let Some(current) = self.picker_mode() else {
            return;
        };
        let f = self.focus;
        let out = &self.snap.outputs[f];
        let items: Vec<PickItem> = out
            .resolutions()
            .into_iter()
            .map(|r| {
                let top = out
                    .rates(r.width, r.height)
                    .first()
                    .map(|m| m.refresh)
                    .unwrap_or_default();
                let mark = |on: bool, c: char| if on { c } else { ' ' };
                let is_current = r.width == current.width && r.height == current.height;
                PickItem {
                    label: format!(
                        "{}{} {:>9}  up to {top:.2} Hz",
                        mark(is_current, '•'),
                        mark(r.preferred, '+'),
                        format!("{}x{}", r.width, r.height),
                    ),
                    pick: Pick::Resolution(r.width, r.height),
                }
            })
            .collect();
        let selected = items
            .iter()
            .position(|it| it.pick == Pick::Resolution(current.width, current.height))
            .unwrap_or(0);
        self.mode = UiMode::Picker(Picker {
            output: f,
            title: format!("{} resolution", self.layout.label(f)),
            items,
            selected,
        });
    }

    fn open_rates(&mut self) {
        let Some(current) = self.picker_mode() else {
            return;
        };
        let f = self.focus;
        let items: Vec<PickItem> = self.snap.outputs[f]
            .rates(current.width, current.height)
            .into_iter()
            .map(|m| {
                let mark = |on: bool, c: char| if on { c } else { ' ' };
                PickItem {
                    label: format!(
                        "{}{} {:>9} Hz",
                        mark(m.id == current.id, '•'),
                        mark(m.preferred, '+'),
                        m.rate_label()
                    ),
                    pick: Pick::Mode(m.id),
                }
            })
            .collect();
        if items.is_empty() {
            let text = format!("{} lists no other rates.", self.layout.label(f));
            self.say(Severity::Info, text);
            return;
        }
        let selected = items
            .iter()
            .position(|it| it.pick == Pick::Mode(current.id))
            .unwrap_or(0);
        self.mode = UiMode::Picker(Picker {
            output: f,
            title: format!(
                "{} rate at {}x{}",
                self.layout.label(f),
                current.width,
                current.height
            ),
            items,
            selected,
        });
    }

    /// Each row shows what the scale makes of the mode: on X11 a larger scale gives a larger
    /// desktop, so things look smaller; on Wayland it gives fewer logical pixels, so things look
    /// larger. A size that is not a whole number gets `~`.
    fn open_scales(&mut self) {
        let Some(mode) = self.picker_mode() else {
            return;
        };
        let f = self.focus;
        let st = &self.layout.outputs[f];
        let kind = self.layout.caps.kind;
        let current = st.scaling.factor();
        let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
        let mut factors = SCALES.to_vec();
        if let Some(c) = current
            && !factors.iter().any(|&s| near(s, c))
        {
            factors.push(c);
            factors.sort_by(f64::total_cmp);
        }
        let rotated = if st.rotation.swaps_axes() {
            (mode.height, mode.width)
        } else {
            (mode.width, mode.height)
        };
        let items: Vec<PickItem> = factors
            .iter()
            .map(|&s| {
                let scaling = Scaling::uniform(kind, s);
                let size = effective_size(mode.size(), st.rotation, &scaling);
                let (exact, noun) = match kind {
                    Kind::X11 => (whole(rotated, |v| v * s), "desktop"),
                    Kind::Wayland => (whole(rotated, |v| v / s), "logical"),
                };
                let mark = if current.is_some_and(|c| near(c, s)) {
                    '•'
                } else {
                    ' '
                };
                PickItem {
                    label: format!(
                        "{mark} {:>5} → {}{}x{} {noun}",
                        scaling.badge().unwrap_or_default(),
                        if exact { "" } else { "~" },
                        size.w,
                        size.h
                    ),
                    pick: Pick::Scale(s),
                }
            })
            .collect();
        let at = current.unwrap_or(1.0);
        let selected = items
            .iter()
            .position(|it| matches!(it.pick, Pick::Scale(s) if near(s, at)))
            .unwrap_or(0);
        self.mode = UiMode::Picker(Picker {
            output: f,
            title: format!("{} scale", self.layout.label(f)),
            items,
            selected,
        });
    }

    fn picker_key(&mut self, action: Action) {
        let UiMode::Picker(picker) = &mut self.mode else {
            return;
        };
        match action {
            Action::Move(Dir::Down) => {
                picker.selected = (picker.selected + 1).min(picker.items.len().saturating_sub(1));
            }
            Action::Move(Dir::Up) => picker.selected = picker.selected.saturating_sub(1),
            Action::Accept => {
                let (i, pick) = (picker.output, picker.items[picker.selected].pick);
                self.mode = UiMode::Normal;
                match pick {
                    Pick::Resolution(w, h) => self.simple_edit(|l, s| l.set_resolution(s, i, w, h)),
                    Pick::Mode(id) => self.simple_edit(|l, s| l.set_mode(s, i, id)),
                    Pick::Scale(s) => self.simple_edit(|l, _| l.set_scale(i, s)),
                };
            }
            Action::Cancel => self.mode = UiMode::Normal,
            _ => {}
        }
    }

    // --- Profiles ---------------------------------------------------------------------------

    fn save_prompt_key(&mut self, action: Action) -> Vec<Effect> {
        let UiMode::SavePrompt(line) = &mut self.mode else {
            return Vec::new();
        };
        match action {
            Action::Back => {
                if line.pop().is_none() {
                    self.mode = UiMode::Normal;
                }
                Vec::new()
            }
            Action::Accept => {
                let name = line.trim().to_owned();
                if name.is_empty() {
                    self.say(Severity::Info, "Type a name for the profile.");
                    return Vec::new();
                }
                self.mode = UiMode::Normal;
                vec![Effect::SaveProfile(name)]
            }
            _ => {
                self.mode = UiMode::Normal;
                Vec::new()
            }
        }
    }

    /// Opens the picker on the profiles the session read.
    pub fn open_profiles(&mut self, mut items: Vec<ProfileItem>, dir: &str) {
        for item in &mut items {
            item.preview.numbers = self.layout.numbers.clone();
        }
        if items.is_empty() {
            self.say(Severity::Info, format!("There are no profiles in {dir}."));
            return;
        }
        let selected = self
            .profile
            .as_ref()
            .and_then(|p| items.iter().position(|it| it.name == *p))
            .unwrap_or(0);
        self.status = None;
        self.mode = UiMode::Profiles(ProfilePicker { items, selected });
    }

    /// Opens a profile: first the remap dialog when some of its outputs are not connected.
    pub fn open_profile(&mut self, item: ProfileItem) {
        if item.unmatched.is_empty() {
            self.load_profile(&item, &Vec::new());
            return;
        }
        let rows = item.profile.default_remap(&self.snap);
        let free = item.profile.free_outputs(&self.snap);
        self.status = None;
        self.mode = UiMode::Remap(Box::new(RemapDialog {
            item,
            rows,
            selected: 0,
            free,
        }));
    }

    /// Makes the profile's layout the pending one, as one undo step.
    pub fn load_profile(&mut self, item: &ProfileItem, remap: &Remap) {
        self.mode = UiMode::Normal;
        let (mut layout, notes) = item.profile.layout(&self.snap, remap);
        // A refresh may have numbered the displays differently from the snapshot's order.
        layout.numbers = self.layout.numbers.clone();
        self.profile = Some(item.name.clone());
        if layout == self.layout {
            let text = format!("{} is the layout you have already.", item.name);
            self.say(Severity::Info, text);
            return;
        }
        let before = std::mem::replace(&mut self.layout, layout);
        self.history.record(before, &self.layout);
        self.viewport.refit();
        self.revalidate();
        let pending = self.pending().len();
        let mut text = format!(
            "Opened {}: {pending} pending change{}. ",
            item.name,
            if pending == 1 { "" } else { "s" }
        );
        let apply = self
            .keymap
            .key_for(Context::Normal, Action::Apply)
            .unwrap_or_default();
        text.push_str(&format!("{apply} applies them."));
        if notes.is_empty() {
            self.say(Severity::Info, text);
        } else {
            self.say(Severity::Warning, format!("{text} {}", notes.join(" ")));
        }
    }

    /// A save would change an existing file: ask first, showing the diff.
    pub fn confirm_overwrite(&mut self, plan: SavePlan) {
        self.mode = UiMode::Overwrite(plan);
    }

    /// The profile was written.
    pub fn saved(&mut self, name: String, shown: &str) {
        self.profile = Some(name);
        self.say(Severity::Info, format!("Saved {shown}."));
    }

    fn profiles_key(&mut self, action: Action) {
        let UiMode::Profiles(picker) = &mut self.mode else {
            return;
        };
        match action {
            Action::Move(Dir::Down) => {
                picker.selected = (picker.selected + 1).min(picker.items.len().saturating_sub(1));
            }
            Action::Move(Dir::Up) => picker.selected = picker.selected.saturating_sub(1),
            Action::Accept => {
                let item = picker.items[picker.selected].clone();
                self.mode = UiMode::Normal;
                self.open_profile(item);
            }
            Action::Cancel => self.mode = UiMode::Normal,
            _ => {}
        }
    }

    /// The outputs row `k` of the remap dialog may go to: nowhere, or a free output no other
    /// row took.
    pub fn remap_choices(dialog: &RemapDialog, k: usize) -> Vec<Option<usize>> {
        let taken: Vec<usize> = dialog
            .rows
            .iter()
            .enumerate()
            .filter(|&(j, _)| j != k)
            .filter_map(|(_, (_, to))| *to)
            .collect();
        std::iter::once(None)
            .chain(
                dialog
                    .free
                    .iter()
                    .filter(|i| !taken.contains(i))
                    .map(|&i| Some(i)),
            )
            .collect()
    }

    fn remap_key(&mut self, action: Action) {
        let UiMode::Remap(dialog) = &mut self.mode else {
            return;
        };
        match action {
            Action::Move(Dir::Down) => {
                dialog.selected = (dialog.selected + 1).min(dialog.rows.len().saturating_sub(1));
            }
            Action::Move(Dir::Up) => dialog.selected = dialog.selected.saturating_sub(1),
            Action::Move(dir @ (Dir::Left | Dir::Right)) => {
                let k = dialog.selected;
                let choices = Self::remap_choices(dialog, k);
                let at = choices
                    .iter()
                    .position(|c| *c == dialog.rows[k].1)
                    .unwrap_or(0);
                let n = choices.len();
                let next = if dir == Dir::Right {
                    (at + 1) % n
                } else {
                    (at + n - 1) % n
                };
                dialog.rows[k].1 = choices[next];
            }
            Action::Accept => {
                let (item, rows) = (dialog.item.clone(), dialog.rows.clone());
                self.load_profile(&item, &rows);
            }
            Action::Cancel => self.mode = UiMode::Normal,
            _ => {}
        }
    }

    // --- Command line -----------------------------------------------------------------------

    fn command_key(&mut self, action: Action) -> Vec<Effect> {
        let UiMode::Command(line) = &mut self.mode else {
            return Vec::new();
        };
        match action {
            Action::Back => {
                if line.pop().is_none() {
                    self.mode = UiMode::Normal;
                }
                Vec::new()
            }
            Action::Accept => {
                let line = std::mem::take(line);
                self.mode = UiMode::Normal;
                if line.trim().is_empty() {
                    return Vec::new();
                }
                self.run_command(&line)
            }
            Action::Complete => {
                let (completed, candidates) = cmdline::complete(line, &self.layout);
                *line = completed;
                match candidates.len() {
                    0 => self.status = None,
                    1 => {}
                    _ => self.say(Severity::Info, candidates.join("  ")),
                }
                Vec::new()
            }
            Action::Cancel => {
                self.mode = UiMode::Normal;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Runs one command-line command against the focused display (or the one it names).
    pub fn run_command(&mut self, line: &str) -> Vec<Effect> {
        let cmd = match cmdline::parse(line) {
            Ok(cmd) => cmd,
            Err(err) => {
                self.say(Severity::Warning, err);
                return Vec::new();
            }
        };
        let f = self.focus;
        match cmd {
            Cmd::Pos(x, y) => {
                self.simple_edit(|l, _| l.move_to(f, x, y));
            }
            Cmd::Move(dx, dy) => {
                self.simple_edit(|l, _| l.move_by(f, dx, dy));
            }
            Cmd::Mode { w, h, rate: None } => {
                self.simple_edit(|l, s| l.set_resolution(s, f, w, h));
            }
            Cmd::Mode {
                w,
                h,
                rate: Some(r),
            } => match nearest_rate(&self.snap, f, w, h, r) {
                Some(id) => {
                    self.simple_edit(|l, s| l.set_mode(s, f, id));
                }
                None => {
                    let text = format!("{} has no {w}x{h} mode.", self.layout.label(f));
                    self.say(Severity::Warning, text);
                }
            },
            Cmd::Rate(r) => {
                let size = self.layout.outputs[f].mode.as_ref().map(Mode::size);
                match size.and_then(|s| nearest_rate(&self.snap, f, s.w, s.h, r)) {
                    Some(id) => {
                        self.simple_edit(|l, s| l.set_mode(s, f, id));
                    }
                    None => {
                        let text = format!("{} has no mode to change.", self.layout.label(f));
                        self.say(Severity::Warning, text);
                    }
                }
            }
            Cmd::Rotate(r) => {
                self.simple_edit(|l, _| l.set_rotation(f, r));
            }
            Cmd::Reflect(r) => {
                self.simple_edit(|l, _| l.set_reflection(f, r));
            }
            Cmd::Scale { percent: true, .. } if self.layout.caps.kind == Kind::X11 => {
                self.say(
                    Severity::Warning,
                    "On X11 a scale is a factor, such as :scale 1.5; percentages are for Wayland.",
                );
            }
            Cmd::Scale { factor, .. } => {
                self.simple_edit(|l, _| l.set_scale(f, factor));
            }
            Cmd::Stick {
                side: Side::Same, ..
            } if !self.layout.caps.mirror => self.missing(Cap::Mirror),
            Cmd::Stick {
                child,
                side,
                parent,
                align,
            } => {
                let (Some(c), Some(p)) = (self.named(Some(child)), self.named(Some(parent))) else {
                    return Vec::new();
                };
                let align = align.unwrap_or(Align::default_for(side));
                if self.simple_edit(|l, s| l.stick(s, c, p, side, align)) && self.status.is_none() {
                    let text = format!(
                        "Stuck {} {}.",
                        self.layout.label(c),
                        self.layout.link_text(c)
                    );
                    self.say(Severity::Info, text);
                }
            }
            Cmd::Unstick(t) => {
                if let Some(i) = self.named(t) {
                    self.simple_edit(|l, _| l.unstick(i));
                }
            }
            Cmd::Primary(_) if !self.layout.caps.primary => self.missing(Cap::Primary),
            Cmd::Primary(t) => {
                if let Some(i) = self.named(t) {
                    self.simple_edit(|l, _| l.set_primary(i));
                }
            }
            Cmd::On(t) => self.switch(t, true),
            Cmd::Off(t) => self.switch(t, false),
            Cmd::Quit { force } => return self.quit(force),
            Cmd::Save(_) | Cmd::Open(_) if !self.has_profiles() => {}
            Cmd::Save(Some(name)) => return vec![Effect::SaveProfile(name)],
            Cmd::Save(None) => match self.profile.clone() {
                Some(name) => return vec![Effect::SaveProfile(name)],
                None => self.mode = UiMode::SavePrompt(String::new()),
            },
            Cmd::Open(Some(name)) => return vec![Effect::OpenProfile(name)],
            Cmd::Open(None) => return vec![Effect::ListProfiles],
            Cmd::Apply => return self.open_apply(),
        }
        Vec::new()
    }

    /// The output a command names, or the focused one; reports a bad name.
    fn named(&mut self, token: Option<String>) -> Option<usize> {
        let Some(token) = token else {
            return Some(self.focus);
        };
        match cmdline::resolve_output(&self.layout, &token) {
            Ok(i) => Some(i),
            Err(err) => {
                self.say(Severity::Warning, capitalise(&err));
                None
            }
        }
    }

    /// `:on` and `:off`.
    fn switch(&mut self, token: Option<String>, on: bool) {
        let Some(i) = self.named(token) else { return };
        if self.layout.is_enabled(i) == on {
            let state = if on { "on" } else { "off" };
            let text = format!("{} is already {state}.", self.layout.label(i));
            self.say(Severity::Info, text);
        } else {
            self.toggle(i);
        }
    }

    /// Turns display `i` on or off. A Wayland head that has never been on starts at 100 %,
    /// whatever the compositor used before, so the status says how to pick a scale.
    fn toggle(&mut self, i: usize) {
        let first_time = self.layout.outputs[i].mode.is_none();
        if self.simple_edit(|l, s| l.toggle(s, i))
            && first_time
            && self.layout.caps.kind == Kind::Wayland
        {
            let key = self
                .keymap
                .key_for(Context::Normal, Action::ScalePicker)
                .unwrap_or_default();
            let mut text = format!(
                "{} starts at 100%: {key} picks a scale.",
                self.layout.label(i)
            );
            if let Some(status) = self.status.take() {
                text = format!("{} {text}", status.text);
            }
            self.say(Severity::Info, text);
        }
    }

    /// Whether this display server has profiles yet; says why not when it has none.
    fn has_profiles(&mut self) -> bool {
        if self.layout.caps.kind == Kind::Wayland {
            self.say(Severity::Info, NO_WAYLAND_PROFILES);
            return false;
        }
        true
    }
}

/// Outputs plugged in or unplugged between two readings, by name, and whether the configuration
/// of any other output changed.
struct Plugs {
    /// Connected now and not before, or connected to a different display (another EDID).
    connected: Vec<String>,
    /// Connected before and not now, or gone from the list.
    disconnected: Vec<String>,
    /// An output whose connection did not change was turned on or off, moved, or changed mode.
    outside: bool,
}

impl Plugs {
    fn between(old: &Snapshot, new: &Snapshot) -> Self {
        fn find<'a>(snap: &'a Snapshot, name: &str) -> Option<&'a Output> {
            snap.find(name).map(|i| &snap.outputs[i])
        }
        let connected = new
            .outputs
            .iter()
            .filter(|o| {
                o.is_connected()
                    && find(old, &o.name).is_none_or(|p| {
                        !p.is_connected() || p.edid != o.edid || p.identity != o.identity
                    })
            })
            .map(|o| o.name.clone())
            .collect();
        let disconnected = old
            .outputs
            .iter()
            .filter(|o| o.is_connected() && find(new, &o.name).is_none_or(|n| !n.is_connected()))
            .map(|o| o.name.clone())
            .collect();
        let outside = new.outputs.iter().any(|o| {
            find(old, &o.name).is_some_and(|p| {
                p.is_connected() == o.is_connected()
                    && (p.active != o.active || p.primary != o.primary)
            })
        });
        Plugs {
            connected,
            disconnected,
            outside,
        }
    }
}

/// Whether a `(w, h)` size stays whole through `f`.
fn whole((w, h): (i32, i32), f: impl Fn(f64) -> f64) -> bool {
    [w, h].iter().all(|&v| f(f64::from(v)).fract().abs() < 1e-9)
}

/// `A`, `A and B`, `A, B and C`.
fn listing(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The mode of output `i` at `w`x`h` whose refresh is nearest to `rate`, as xrandr picks it.
fn nearest_rate(snap: &Snapshot, i: usize, w: i32, h: i32, rate: f64) -> Option<ModeId> {
    snap.outputs[i]
        .rates(w, h)
        .into_iter()
        .min_by(|a, b| {
            (a.refresh - rate)
                .abs()
                .total_cmp(&(b.refresh - rate).abs())
        })
        .map(|m| m.id)
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = chars
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    out.push_str(chars.as_str());
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}
