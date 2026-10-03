//! [`WlrBackend`]: a `zwlr_output_manager_v1` client, the way outlay talks to sway, Hyprland,
//! niri, river and the other wlroots-style compositors. One connection lives as long as the
//! backend, behind a mutex; there is no thread.
//!
//! What the compositor reports arrives as head and mode events, made atomic by the manager's
//! `done`: a snapshot is taken only from the state as of the last `done`. An apply builds one
//! configuration that names every head, as the protocol requires, and sends a mode only when it
//! changes, so a virtual mode (a custom one, which wlroots advertises as a mode object of its
//! own) is never sent back.

use std::collections::HashMap;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use wayland_client::backend::ObjectId;
use wayland_client::globals::{BindError, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_output::Transform;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child,
};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, AdaptiveSyncState, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

use super::capture::{self, HeadReport, ModeReport, mode_millihertz};
use super::{transform_from_value, transform_value};
use crate::backend::{ApplyOutcome, Backend, Plan, PlanForm, Planned, Verdict};
use crate::model::Snapshot;
use crate::model::geometry::{Point, Size};

/// How long an apply waits for the state it caused, once the compositor said it succeeded.
const SETTLE: Duration = Duration::from_secs(1);

/// The same text `Layout::mismatches` uses, so both failures read alike.
const OUTPUTS_CHANGED: &str = "The outputs changed while applying.";

/// Why connecting gave no backend.
#[derive(Debug)]
pub enum ConnectError {
    /// Nothing answers on the socket.
    NoCompositor(String),
    /// The compositor answers but has no `zwlr_output_manager_v1`.
    NoOutputManagement,
    /// It has one, older than version 2.
    TooOld(u32),
}

/// A mode as the compositor last described it. A virtual mode's size changes in place.
#[derive(Clone, Debug)]
struct ModeState {
    proxy: ZwlrOutputModeV1,
    width: i32,
    height: i32,
    refresh: i32,
    preferred: bool,
}

#[derive(Clone, Debug)]
struct HeadState {
    proxy: ZwlrOutputHeadV1,
    name: String,
    description: Option<String>,
    make: Option<String>,
    model: Option<String>,
    serial: Option<String>,
    physical: Option<Size>,
    /// Mode ids in the order they were announced.
    modes: Vec<ObjectId>,
    enabled: bool,
    current: Option<ObjectId>,
    position: Point,
    transform: u32,
    scale: f64,
    adaptive_sync: Option<bool>,
}

/// The answer to one configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Default)]
struct State {
    heads: Vec<HeadState>,
    modes: HashMap<ObjectId, ModeState>,
    /// The serial of the last `done`, and how many there have been.
    serial: Option<u32>,
    dones: u64,
    /// The heads as of the last `done`.
    committed: Vec<HeadState>,
    committed_modes: HashMap<ObjectId, ModeState>,
    /// Answers to configurations, by configuration.
    answers: HashMap<ObjectId, Answer>,
    /// The manager is gone; nothing more will come.
    finished: bool,
}

impl State {
    fn head(&mut self, id: &ObjectId) -> Option<&mut HeadState> {
        self.heads.iter_mut().find(|h| h.proxy.id() == *id)
    }
}

struct Inner {
    queue: EventQueue<State>,
    state: State,
    manager: ZwlrOutputManagerV1,
}

/// The compositor, through `zwlr_output_manager_v1`.
pub struct WlrBackend {
    inner: Mutex<Inner>,
    /// A second handle on the connection's socket, to hang up with.
    socket: UnixStream,
    /// Which backend [`LIVE`] belongs to.
    id: usize,
}

/// The socket of the backend connected last, for [`hang_up`], with that backend's id. It is a
/// second handle, so the backend's drop takes it out: it would keep the connection open.
static LIVE: Mutex<Option<(usize, UnixStream)>> = Mutex::new(None);

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// Ends the connection of the backend connected last, if it still exists, so the compositor
/// drops this client and everything it holds. The panic hook calls it before `revert.sh`, whose
/// `outlay restore` is another client: up to wlroots 0.20.2, a client that turns a custom-mode
/// head back on aborts the compositor while another client still holds that head's old virtual
/// mode. Only shuts the socket down, never closes it, and never waits for a lock.
pub fn hang_up() {
    if let Ok(live) = LIVE.try_lock()
        && let Some((_, socket)) = live.as_ref()
    {
        let _ = socket.shutdown(Shutdown::Both);
    }
}

impl WlrBackend {
    /// Connects to the compositor whose socket is at `path` and reads the outputs.
    pub fn connect(path: &Path) -> Result<Self, ConnectError> {
        let stream =
            UnixStream::connect(path).map_err(|err| ConnectError::NoCompositor(err.to_string()))?;
        let second = || {
            stream
                .try_clone()
                .map_err(|err| ConnectError::NoCompositor(err.to_string()))
        };
        let (socket, live) = (second()?, second()?);
        let conn = Connection::from_socket(stream)
            .map_err(|err| ConnectError::NoCompositor(err.to_string()))?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn)
            .map_err(|err| ConnectError::NoCompositor(err.to_string()))?;
        let qh = queue.handle();
        let manager = globals
            .bind::<ZwlrOutputManagerV1, _, _>(&qh, 2..=4, ())
            .map_err(|err| match err {
                BindError::UnsupportedVersion => {
                    let version = globals.contents().with_list(|list| {
                        list.iter()
                            .find(|g| g.interface == ZwlrOutputManagerV1::interface().name)
                            .map_or(0, |g| g.version)
                    });
                    ConnectError::TooOld(version)
                }
                BindError::NotPresent => ConnectError::NoOutputManagement,
            })?;
        let mut state = State::default();
        // The heads and their first `done` follow the bind.
        queue
            .roundtrip(&mut state)
            .map_err(|err| ConnectError::NoCompositor(err.to_string()))?;
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        *LIVE.lock().unwrap_or_else(|e| e.into_inner()) = Some((id, live));
        Ok(Self {
            inner: Mutex::new(Inner {
                queue,
                state,
                manager,
            }),
            socket,
            id,
        })
    }

    /// Ends this connection, as [`hang_up`] does for the one connected last.
    pub fn hang_up(&self) {
        let _ = self.socket.shutdown(Shutdown::Both);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Carries out or tests `plan` on the state as last read, without a fresh roundtrip first,
    /// so a test can make the serial stale. Not for anything else.
    #[doc(hidden)]
    pub fn configure_without_roundtrip(&self, plan: &Plan, test: bool) -> Result<Configured> {
        let mut inner = self.lock();
        inner.configure(plan, test, false)
    }
}

impl Drop for WlrBackend {
    fn drop(&mut self) {
        let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        if live.as_ref().is_some_and(|(id, _)| *id == self.id) {
            *live = None;
        }
    }
}

/// What a configuration came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Configured {
    pub answer: Result<(), String>,
    /// Warnings, such as heads a restore skipped.
    pub warnings: Vec<String>,
    /// How many times the compositor cancelled it for a stale serial.
    pub cancelled: u32,
}

impl Inner {
    fn lost(&self, err: impl std::fmt::Display) -> anyhow::Error {
        anyhow!("Lost the connection to the compositor: {err}")
    }

    /// A roundtrip: every event the compositor sent before it is handled.
    fn sync(&mut self) -> Result<()> {
        if self.state.finished {
            bail!("Lost the connection to the compositor: it stopped reporting outputs.");
        }
        let Inner { queue, state, .. } = self;
        queue.roundtrip(state).map_err(|err| self.lost(err))?;
        if self.state.finished {
            bail!("Lost the connection to the compositor: it stopped reporting outputs.");
        }
        Ok(())
    }

    fn snapshot(&self) -> Result<Snapshot> {
        let outputs = self
            .state
            .committed
            .iter()
            .map(|head| capture::output(report(head, &self.state.committed_modes)))
            .collect::<Result<Vec<_>, _>>()
            .context("could not read the compositor's outputs")?;
        Ok(capture::snapshot(outputs))
    }

    /// Dispatches until the configuration has its answer.
    fn answer(&mut self, config: &ZwlrOutputConfigurationV1) -> Result<Answer> {
        loop {
            if let Some(answer) = self.state.answers.remove(&config.id()) {
                return Ok(answer);
            }
            let Inner { queue, state, .. } = self;
            queue
                .blocking_dispatch(state)
                .map_err(|err| self.lost(err))?;
        }
    }

    /// Waits up to `limit` for a `done` beyond the `dones`-th.
    fn settle(&mut self, dones: u64, limit: Duration) -> Result<()> {
        let deadline = Instant::now() + limit;
        while self.state.dones <= dones {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(());
            }
            let Inner { queue, state, .. } = self;
            queue
                .dispatch_pending(state)
                .map_err(|err| self.lost(err))?;
            if self.state.dones > dones {
                break;
            }
            self.queue.flush().map_err(|err| self.lost(err))?;
            let Some(guard) = self.queue.prepare_read() else {
                continue;
            };
            let timeout = Timespec {
                tv_sec: left.as_secs() as i64,
                tv_nsec: i64::from(left.subsec_nanos()),
            };
            let ready = {
                let fd = guard.connection_fd();
                let mut fds = [PollFd::new(&fd, PollFlags::IN)];
                poll(&mut fds, Some(&timeout)).unwrap_or(0)
            };
            if ready > 0 {
                guard.read().map_err(|err| self.lost(err))?;
            }
        }
        let Inner { queue, state, .. } = self;
        queue
            .dispatch_pending(state)
            .map_err(|err| self.lost(err))?;
        Ok(())
    }

    /// Builds, sends and answers one configuration for `plan`. Every head is in it: a planned
    /// head as planned, any other as it is. With `fresh`, a roundtrip first gets the latest
    /// serial. A cancelled configuration (a stale serial) is tried once more after a roundtrip,
    /// if the heads are the same.
    fn configure(&mut self, plan: &Plan, test: bool, fresh: bool) -> Result<Configured> {
        if fresh {
            self.sync()?;
        }
        let mut warnings = Vec::new();
        let mut cancelled = 0;
        loop {
            let names: Vec<String> = self.state.heads.iter().map(|h| h.name.clone()).collect();
            let missing: Vec<&Planned> = plan
                .outputs
                .iter()
                .filter(|p| !names.contains(&p.name))
                .collect();
            if !missing.is_empty() {
                if plan.form == PlanForm::Apply {
                    return Ok(Configured {
                        answer: Err(OUTPUTS_CHANGED.to_owned()),
                        warnings,
                        cancelled,
                    });
                }
                for p in &missing {
                    let text = format!("{} is gone; it is left out.", p.name);
                    if !warnings.contains(&text) {
                        warnings.push(text);
                    }
                }
            }
            let Some(serial) = self.state.serial else {
                bail!("The compositor has not reported its outputs yet.");
            };
            let changes = self.changes(plan);
            let qh = self.queue.handle();
            let config = self.manager.create_configuration(serial, &qh, ());
            for head in &self.state.heads {
                self.add_head(&config, head, plan.find(&head.name), &qh);
            }
            if test {
                config.test();
            } else {
                config.apply();
            }
            let dones = self.state.dones;
            let answer = self.answer(&config);
            config.destroy();
            match answer? {
                Answer::Succeeded => {
                    // The new state may come after `succeeded` (it comes before it on sway).
                    if !test && changes {
                        self.settle(dones, SETTLE)?;
                    }
                    return Ok(Configured {
                        answer: Ok(()),
                        warnings,
                        cancelled,
                    });
                }
                Answer::Failed => {
                    let what = if test {
                        "The compositor rejects this layout."
                    } else {
                        "The compositor could not apply this layout."
                    };
                    return Ok(Configured {
                        answer: Err(what.to_owned()),
                        warnings,
                        cancelled,
                    });
                }
                Answer::Cancelled => {
                    cancelled += 1;
                    self.sync()?;
                    let now: Vec<String> =
                        self.state.heads.iter().map(|h| h.name.clone()).collect();
                    if cancelled > 1 || now != names {
                        return Ok(Configured {
                            answer: Err(OUTPUTS_CHANGED.to_owned()),
                            warnings,
                            cancelled,
                        });
                    }
                }
            }
        }
    }

    /// Whether `plan` changes anything about the heads as they are.
    fn changes(&self, plan: &Plan) -> bool {
        self.state.heads.iter().any(|head| {
            let Some(planned) = plan.find(&head.name) else {
                return false;
            };
            match (&planned.on, head.enabled) {
                (None, enabled) => enabled,
                (Some(_), false) => true,
                (Some(on), true) => {
                    let current = head
                        .current
                        .as_ref()
                        .and_then(|id| self.state.modes.get(id))
                        .map(mode_key);
                    current != Some(planned_key(&on.mode))
                        || on.pos != head.position
                        || transform_value(on.rotation, on.reflection) != head.transform
                        || (on.scaling.factor().unwrap_or(1.0) - head.scale).abs() > 1e-6
                }
            }
        })
    }

    fn add_head(
        &self,
        config: &ZwlrOutputConfigurationV1,
        head: &HeadState,
        planned: Option<&Planned>,
        qh: &QueueHandle<State>,
    ) {
        let on = match planned {
            Some(Planned { on: None, .. }) => None,
            Some(Planned { on: Some(on), .. }) => Some(on),
            // Not in the plan: as it is. `enable_head` copies the head's state.
            None if head.enabled => {
                config.enable_head(&head.proxy, qh, ());
                return;
            }
            None => None,
        };
        let Some(on) = on else {
            config.disable_head(&head.proxy);
            return;
        };
        let ch = config.enable_head(&head.proxy, qh, ());
        let want = planned_key(&on.mode);
        let current = head
            .current
            .as_ref()
            .and_then(|id| self.state.modes.get(id))
            .map(mode_key);
        // A mode is sent only when it changes: re-sending a virtual mode can be fatal.
        if current != Some(want) {
            let real = head
                .modes
                .iter()
                .filter_map(|id| self.state.modes.get(id))
                .find(|m| mode_key(m) == want);
            match real {
                Some(m) => ch.set_mode(&m.proxy),
                None => ch.set_custom_mode(want.0, want.1, want.2),
            }
        }
        ch.set_position(on.pos.x, on.pos.y);
        let transform = transform_value(on.rotation, on.reflection);
        ch.set_transform(Transform::try_from(transform).unwrap_or(Transform::Normal));
        ch.set_scale(on.scaling.factor().unwrap_or(1.0));
    }
}

/// Width, height and rate in mHz: how modes are compared.
type ModeKey = (i32, i32, i32);

fn mode_key(m: &ModeState) -> ModeKey {
    (m.width, m.height, m.refresh)
}

fn planned_key(mode: &crate::model::Mode) -> ModeKey {
    (mode.width, mode.height, mode_millihertz(mode) as i32)
}

fn report(head: &HeadState, modes: &HashMap<ObjectId, ModeState>) -> HeadReport {
    let (rotation, reflection) = transform_from_value(head.transform).unwrap_or_default();
    HeadReport {
        name: head.name.clone(),
        description: head.description.clone(),
        make: head.make.clone(),
        model: head.model.clone(),
        serial: head.serial.clone(),
        physical_size: head.physical,
        enabled: head.enabled,
        modes: head
            .modes
            .iter()
            .filter_map(|id| modes.get(id).map(|m| (id, m)))
            .map(|(id, m)| ModeReport {
                width: m.width,
                height: m.height,
                millihertz: m.refresh.max(0) as u32,
                preferred: m.preferred,
                current: head.current.as_ref() == Some(id),
                custom: None,
            })
            .collect(),
        position: head.position,
        rotation,
        reflection,
        scale: head.scale,
        adaptive_sync: head.adaptive_sync,
    }
}

impl Backend for WlrBackend {
    fn query(&self) -> Result<Snapshot> {
        let mut inner = self.lock();
        inner.sync()?;
        inner.snapshot()
    }

    fn test(&self, plan: &Plan) -> Result<Verdict> {
        let mut inner = self.lock();
        let done = inner.configure(plan, true, true)?;
        Ok(match done.answer {
            Ok(()) => Verdict::Accepted,
            Err(why) => Verdict::Rejected(why),
        })
    }

    fn apply(&self, plan: &Plan) -> Result<ApplyOutcome> {
        let mut inner = self.lock();
        let done = inner.configure(plan, false, true)?;
        let mut stderr: String = done.warnings.iter().map(|w| format!("{w}\n")).collect();
        if let Err(why) = &done.answer {
            stderr.push_str(why);
            stderr.push('\n');
        }
        Ok(ApplyOutcome {
            success: done.answer.is_ok(),
            stdout: String::new(),
            stderr,
        })
    }

    fn dump(&self) -> Result<String> {
        Ok(capture::write(&self.query()?))
    }

    fn is_live(&self) -> bool {
        true
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => state.heads.push(HeadState {
                proxy: head,
                name: String::new(),
                description: None,
                make: None,
                model: None,
                serial: None,
                physical: None,
                modes: Vec::new(),
                enabled: false,
                current: None,
                position: Point::default(),
                transform: 0,
                scale: 1.0,
                adaptive_sync: None,
            }),
            zwlr_output_manager_v1::Event::Done { serial } => {
                state.serial = Some(serial);
                state.dones += 1;
                state.committed = state.heads.clone();
                state.committed_modes = state.modes.clone();
            }
            zwlr_output_manager_v1::Event::Finished => state.finished = true,
            _ => {}
        }
    }

    event_created_child!(State, ZwlrOutputManagerV1, [
        zwlr_output_manager_v1::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_output_head_v1::Event;
        let id = proxy.id();
        if let Event::Finished = event {
            state.heads.retain(|h| h.proxy.id() != id);
            if proxy.version() >= 3 {
                proxy.release();
            }
            return;
        }
        if let Event::Mode { mode } = &event {
            state.modes.insert(
                mode.id(),
                ModeState {
                    proxy: mode.clone(),
                    width: 0,
                    height: 0,
                    refresh: 0,
                    preferred: false,
                },
            );
        }
        let Some(head) = state.head(&id) else {
            return;
        };
        match event {
            Event::Name { name } => head.name = name,
            Event::Description { description } => head.description = Some(description),
            Event::PhysicalSize { width, height } => head.physical = Some(Size::new(width, height)),
            Event::Mode { mode } => head.modes.push(mode.id()),
            Event::Enabled { enabled } => head.enabled = enabled != 0,
            Event::CurrentMode { mode } => head.current = Some(mode.id()),
            Event::Position { x, y } => head.position = Point::new(x, y),
            Event::Transform { transform } => {
                head.transform = match transform {
                    WEnum::Value(t) => t.into(),
                    WEnum::Unknown(raw) => raw,
                }
            }
            Event::Scale { scale } => head.scale = scale,
            Event::Make { make } => head.make = Some(make),
            Event::Model { model } => head.model = Some(model),
            Event::SerialNumber { serial_number } => head.serial = Some(serial_number),
            Event::AdaptiveSync { state } => {
                head.adaptive_sync = Some(matches!(state, WEnum::Value(AdaptiveSyncState::Enabled)))
            }
            _ => {}
        }
    }

    event_created_child!(State, ZwlrOutputHeadV1, [
        zwlr_output_head_v1::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputModeV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_output_mode_v1::Event;
        let id = proxy.id();
        if let Event::Finished = event {
            state.modes.remove(&id);
            for head in &mut state.heads {
                head.modes.retain(|m| *m != id);
            }
            if proxy.version() >= 3 {
                proxy.release();
            }
            return;
        }
        let Some(mode) = state.modes.get_mut(&id) else {
            return;
        };
        match event {
            Event::Size { width, height } => (mode.width, mode.height) = (width, height),
            Event::Refresh { refresh } => mode.refresh = refresh,
            Event::Preferred => mode.preferred = true,
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_output_configuration_v1::Event;
        let answer = match event {
            Event::Succeeded => Answer::Succeeded,
            Event::Failed => Answer::Failed,
            Event::Cancelled => Answer::Cancelled,
            _ => return,
        };
        state.answers.insert(proxy.id(), answer);
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputConfigurationHeadV1,
        _: <ZwlrOutputConfigurationHeadV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn hanging_up_ends_the_live_connection_without_closing_it() {
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        *LIVE.lock().unwrap() = Some((usize::MAX, ours.try_clone().unwrap()));
        hang_up();
        let mut buf = [0u8; 1];
        assert_eq!(
            theirs.read(&mut buf).unwrap(),
            0,
            "the other end sees the end"
        );
        assert!(
            ours.try_clone().is_ok(),
            "the descriptor is still open: its owner closes it"
        );
        *LIVE.lock().unwrap() = None;
        hang_up();
    }
}
