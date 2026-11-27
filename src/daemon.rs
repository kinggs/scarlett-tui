use crate::alsa_ctl::{
    AlsaCard, CardEntry, ControlKind, discover_focusrite_cards, read_stream_rate,
};
use crate::model::{ClipLatch, RouteGroup, ScarlettModel, meter_to_dbfs_with_max};
use crate::pipewire;
use serde::Serialize;
use std::io::{self, Write};
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const MAX_STDIN_LINE_BYTES: usize = 8192;
const PW_REFRESH_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Serialize)]
struct SnapshotPayload<'a> {
    #[serde(rename = "type")]
    msg_type: &'static str,
    connected: bool,
    mixer_available: bool,
    generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    card: Option<&'a CardEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    firmware: Option<i64>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    firmware_versions: &'a [(String, i64)],
    sync_status: String,
    locked: bool,
    active_rate: Option<u32>,
    playback_running: bool,
    capture_running: bool,
    inputs: Vec<InputSummary>,
    phantom: Vec<ToggleSummary>,
    outputs: Vec<OutputSummary>,
    routes: Vec<RouteSummary>,
    has_matrix: bool,
    matrix_size: String,
    pw_scarlett_links: usize,
    meter_labels: Vec<MeterLabel>,
}

#[derive(Serialize)]
struct ErrorPayload<'a> {
    #[serde(rename = "type")]
    msg_type: &'static str,
    message: &'a str,
}

#[derive(Serialize)]
struct ToggleSummary {
    numid: u32,
    label: String,
    kind: ControlKind,
    value: Option<i64>,
    text: String,
    active: bool,
    items: Vec<String>,
    rw: bool,
}

#[derive(Serialize)]
struct InputSummary {
    index: u32,
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    gain_numid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gain_db: Option<f64>,
    toggles: Vec<ToggleSummary>,
}

#[derive(Serialize)]
struct OutputSummary {
    index: u32,
    label: String,
    volume_numid: Option<u32>,
    volume_rw: bool,
    db: Option<f64>,
    mute_numid: Option<u32>,
    mute_rw: bool,
    muted: bool,
    route_numid: Option<u32>,
    route_rw: bool,
    route: String,
    route_summary: String,
    route_items: Vec<String>,
    route_val: Option<i64>,
}

#[derive(Serialize)]
struct RouteSummary {
    numid: u32,
    group: String,
    label: String,
    current: String,
    summary: String,
    items: Vec<String>,
    val: Option<i64>,
    rw: bool,
}

#[derive(Serialize)]
struct MeterLabel {
    index: usize,
    label: String,
    source: String,
    group: String,
}

#[derive(Serialize)]
struct MeterPayload {
    #[serde(rename = "type")]
    msg_type: &'static str,
    raw: Vec<i64>,
    dbfs: Vec<f64>,
    /// Per-channel clip latch (≥ −1 dBFS), held until `clip clear`.
    clip: Vec<bool>,
    clip_any: bool,
}

/// Incremental, bounded byte accumulator for non-blocking line-oriented stdin.
/// Preserves partial lines across `WouldBlock` reads and handles multiple
/// commands per chunk cleanly.
struct LineAccumulator {
    buf: Vec<u8>,
    discarding_oversize: bool,
}

impl LineAccumulator {
    fn new() -> Self {
        Self {
            buf: Vec::with_capacity(256),
            discarding_oversize: false,
        }
    }

    fn push_bytes(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &b in chunk {
            if b == b'\n' {
                if self.discarding_oversize {
                    self.discarding_oversize = false;
                    self.buf.clear();
                } else {
                    if self.buf.last() == Some(&b'\r') {
                        self.buf.pop();
                    }
                    let s = String::from_utf8_lossy(&self.buf).into_owned();
                    self.buf.clear();
                    lines.push(s);
                }
            } else if !self.discarding_oversize {
                if self.buf.len() >= MAX_STDIN_LINE_BYTES {
                    self.buf.clear();
                    self.discarding_oversize = true;
                } else {
                    self.buf.push(b);
                }
            }
        }
        lines
    }

    fn finish(&mut self) -> Option<String> {
        if self.discarding_oversize || self.buf.is_empty() {
            self.buf.clear();
            self.discarding_oversize = false;
            return None;
        }
        if self.buf.last() == Some(&b'\r') {
            self.buf.pop();
        }
        let s = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        Some(s)
    }
}

struct PwShared {
    card: Option<CardEntry>,
    count: usize,
    dirty: bool,
    trigger: bool,
    stop: bool,
}

/// Background worker that queries `pw-link` with a bounded timeout so daemon
/// snapshot generation never blocks ALSA events, commands, or meter frames.
struct PwMonitor {
    shared: Arc<(Mutex<PwShared>, Condvar)>,
    handle: Option<thread::JoinHandle<()>>,
}

impl PwMonitor {
    fn spawn() -> Self {
        let shared = Arc::new((
            Mutex::new(PwShared {
                card: None,
                count: 0,
                dirty: false,
                trigger: false,
                stop: false,
            }),
            Condvar::new(),
        ));
        let worker_shared = Arc::clone(&shared);
        let handle = thread::spawn(move || {
            loop {
                let card = {
                    let (lock, cv) = &*worker_shared;
                    let mut st = lock.lock().unwrap_or_else(|e| e.into_inner());
                    if !st.trigger && !st.stop {
                        let (guard, _) = cv
                            .wait_timeout(st, PW_REFRESH_INTERVAL)
                            .unwrap_or_else(|e| e.into_inner());
                        st = guard;
                    }
                    if st.stop {
                        break;
                    }
                    st.trigger = false;
                    st.card.clone()
                };

                let new_count = match card {
                    Some(ref c) => pipewire::list_links_for_card(Some(c))
                        .map(|ls| ls.iter().filter(|l| l.is_scarlett).count())
                        .unwrap_or(0),
                    None => 0,
                };

                let (lock, _) = &*worker_shared;
                let mut st = lock.lock().unwrap_or_else(|e| e.into_inner());
                if st.count != new_count {
                    st.count = new_count;
                    st.dirty = true;
                }
            }
        });
        Self {
            shared,
            handle: Some(handle),
        }
    }

    fn set_card(&self, card: Option<CardEntry>) {
        let (lock, cv) = &*self.shared;
        let mut st = lock.lock().unwrap_or_else(|e| e.into_inner());
        st.card = card;
        st.trigger = true;
        cv.notify_one();
    }

    fn request_refresh(&self) {
        let (lock, cv) = &*self.shared;
        let mut st = lock.lock().unwrap_or_else(|e| e.into_inner());
        st.trigger = true;
        cv.notify_one();
    }

    fn count(&self) -> usize {
        let (lock, _) = &*self.shared;
        lock.lock().unwrap_or_else(|e| e.into_inner()).count
    }

    fn take_dirty(&self) -> bool {
        let (lock, _) = &*self.shared;
        let mut st = lock.lock().unwrap_or_else(|e| e.into_inner());
        let d = st.dirty;
        st.dirty = false;
        d
    }
}

impl Drop for PwMonitor {
    fn drop(&mut self) {
        let (lock, cv) = &*self.shared;
        {
            let mut st = lock.lock().unwrap_or_else(|e| e.into_inner());
            st.stop = true;
            cv.notify_one();
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn toggle_from(model: &ScarlettModel, numid: u32, label: &str) -> Option<ToggleSummary> {
    let c = model.controls.get(&numid)?;
    let val = c.opt_val();
    let (active, text) = match c.kind {
        ControlKind::Bool => match c.opt_bool_val() {
            Some(true) => (true, "On".into()),
            Some(false) => (false, "Off".into()),
            None => (false, "?".into()),
        },
        ControlKind::Enum => (val.unwrap_or(0) > 0, c.enum_label().to_string()),
        ControlKind::Int => match val {
            Some(v) => (
                v > c.min,
                c.current_db()
                    .map(|db| format!("{db:.1} dB"))
                    .unwrap_or_else(|| v.to_string()),
            ),
            None => (false, "?".into()),
        },
    };
    Some(ToggleSummary {
        numid,
        label: label.into(),
        kind: c.kind,
        value: val,
        text,
        active,
        items: c.items.clone(),
        rw: c.is_editable(),
    })
}

fn build_snapshot<'a>(
    model: &'a ScarlettModel,
    generation: u64,
    pw_scarlett_links: usize,
) -> SnapshotPayload<'a> {
    let inputs = model
        .inputs
        .iter()
        .map(|inp| {
            let mut toggles = Vec::new();
            if let Some(id) = inp.level_numid
                && let Some(t) = toggle_from(model, id, "INST")
            {
                toggles.push(t);
            }
            if let Some(id) = inp.air_numid
                && let Some(t) = toggle_from(model, id, "AIR")
            {
                toggles.push(t);
            }
            if let Some(id) = inp.pad_numid
                && let Some(t) = toggle_from(model, id, "PAD")
            {
                toggles.push(t);
            }
            if let Some(id) = inp.safe_numid
                && let Some(t) = toggle_from(model, id, "SAFE")
            {
                toggles.push(t);
            }
            if let Some(id) = inp.autogain_numid
                && let Some(t) = toggle_from(model, id, "AUTO")
            {
                toggles.push(t);
            }
            if let Some(id) = inp.link_numid
                && let Some(t) = toggle_from(model, id, "LINK")
            {
                toggles.push(t);
            }
            let gain_db = inp
                .gain_numid
                .and_then(|id| model.controls.get(&id))
                .and_then(|c| c.current_db());
            InputSummary {
                index: inp.index,
                label: inp.label.clone(),
                gain_numid: inp.gain_numid,
                gain_db,
                toggles,
            }
        })
        .collect();

    let phantom = model
        .phantom_numids
        .iter()
        .filter_map(|&id| {
            let c = model.controls.get(&id)?;
            let short = if c.name.contains("Persistence") {
                "Remember 48V".to_string()
            } else if let Some(rest) = c.name.strip_prefix("Line In ")
                && let Some(ch_range) = rest.split_whitespace().next()
            {
                format!("48V (In {})", ch_range.replace('-', "–"))
            } else {
                "48V Phantom".to_string()
            };
            toggle_from(model, id, &short)
        })
        .collect();

    let outputs = model
        .outputs
        .iter()
        .map(|out| {
            let vol_ctrl = out.volume_numid.and_then(|id| model.controls.get(&id));
            let mute_ctrl = out.mute_numid.and_then(|id| model.controls.get(&id));
            let db = vol_ctrl.and_then(|c| c.current_db());
            let volume_rw = vol_ctrl.map(|c| c.is_editable()).unwrap_or(false);
            let muted = mute_ctrl.map(|c| c.bool_val()).unwrap_or(false);
            let mute_rw = mute_ctrl.map(|c| c.is_editable()).unwrap_or(false);
            let route_entry = model
                .routes
                .iter()
                .find(|r| r.group == RouteGroup::AnalogueOut && r.channel == out.index);
            let (route_numid, route_rw, route, route_summary, route_items, route_val) =
                match route_entry {
                    Some(r) => {
                        let c = model.controls.get(&r.numid);
                        (
                            Some(r.numid),
                            c.map(|x| x.is_editable()).unwrap_or(false),
                            r.current.clone(),
                            r.summary.clone(),
                            c.map(|x| x.items.clone()).unwrap_or_default(),
                            c.and_then(|x| x.opt_val()),
                        )
                    }
                    None => (None, false, String::new(), String::new(), Vec::new(), None),
                };
            OutputSummary {
                index: out.index,
                label: out.label.clone(),
                volume_numid: out.volume_numid,
                volume_rw,
                db,
                mute_numid: out.mute_numid,
                mute_rw,
                muted,
                route_numid,
                route_rw,
                route,
                route_summary,
                route_items,
                route_val,
            }
        })
        .collect();

    let routes = model
        .routes
        .iter()
        .map(|r| {
            let c = model.controls.get(&r.numid);
            RouteSummary {
                numid: r.numid,
                group: r.group.title().into(),
                label: r.label.clone(),
                current: r.current.clone(),
                summary: r.summary.clone(),
                items: c.map(|x| x.items.clone()).unwrap_or_default(),
                val: c.and_then(|x| x.opt_val()),
                rw: c.map(|x| x.is_editable()).unwrap_or(false),
            }
        })
        .collect();

    let meter_labels = model
        .meter_channels
        .iter()
        .map(|m| MeterLabel {
            index: m.meter_index,
            label: m.label.clone(),
            source: m.source.clone(),
            group: m.group.title().into(),
        })
        .collect();

    let mixer_available = model.has_mixer_controls();
    let sync_status = if !mixer_available && model.sync_status == "Unknown" {
        "Mixer unavailable".to_string()
    } else {
        model.sync_status.clone()
    };

    SnapshotPayload {
        msg_type: "state",
        connected: true,
        mixer_available,
        generation,
        card: Some(&model.card),
        firmware: model.firmware,
        firmware_versions: &model.firmware_versions,
        sync_status,
        locked: model.is_locked(),
        active_rate: model.stream.active_rate,
        playback_running: model.stream.playback_running,
        capture_running: model.stream.capture_running,
        inputs,
        phantom,
        outputs,
        routes,
        has_matrix: !model.matrix.is_empty(),
        matrix_size: if model.matrix.is_empty() {
            String::new()
        } else {
            format!("{}×{}", model.matrix.mixes.len(), model.matrix.inputs.len())
        },
        pw_scarlett_links,
        meter_labels,
    }
}

fn emit_json<T: Serialize>(val: &T) -> io::Result<()> {
    let line = serde_json::to_string(val).map_err(io::Error::other)?;
    let mut out = io::stdout().lock();
    writeln!(out, "{line}")?;
    out.flush()
}

fn emit_error(msg: &str) -> io::Result<()> {
    emit_json(&ErrorPayload {
        msg_type: "error",
        message: msg,
    })
}

fn emit_disconnected(generation: u64) -> io::Result<()> {
    emit_json(&serde_json::json!({
        "type": "state",
        "connected": false,
        "mixer_available": false,
        "generation": generation,
        "sync_status": "Disconnected",
        "locked": false,
        "inputs": [],
        "phantom": [],
        "outputs": [],
        "routes": [],
        "has_matrix": false,
        "matrix_size": "",
        "pw_scarlett_links": 0,
        "meter_labels": []
    }))
}

pub fn run_once_json(card_override: Option<u32>) -> i32 {
    let cards = discover_focusrite_cards();
    let selected = match card_override {
        Some(idx) => cards.into_iter().find(|c| c.index == idx),
        None => cards.into_iter().next(),
    };
    let Some(entry) = selected else {
        let _ = emit_disconnected(0);
        return 1;
    };
    let Ok(alsa) = AlsaCard::open(entry.index) else {
        let _ = emit_disconnected(0);
        return 1;
    };
    let Ok(ctrls) = alsa.enumerate() else {
        let _ = emit_disconnected(0);
        return 1;
    };
    let pw_count = pipewire::list_links_for_card(Some(&entry))
        .map(|ls| ls.iter().filter(|l| l.is_scarlett).count())
        .unwrap_or(0);
    let stream = read_stream_rate(entry.index);
    let model = ScarlettModel::build(entry, stream, ctrls);
    let _ = emit_json(&build_snapshot(&model, 1, pw_count));
    0
}

#[derive(Debug, PartialEq, Eq)]
enum DaemonCmd {
    MetersOn,
    MetersOff,
    Refresh,
    ClipClear,
    Set {
        numid: u32,
        val: i64,
        expected_gen: Option<u64>,
    },
    Unknown,
}

fn parse_command(line: &str) -> DaemonCmd {
    let trimmed = line.trim();
    if trimmed == "meters on" {
        return DaemonCmd::MetersOn;
    }
    if trimmed == "meters off" {
        return DaemonCmd::MetersOff;
    }
    if trimmed == "refresh" {
        return DaemonCmd::Refresh;
    }
    if trimmed == "clip clear" {
        return DaemonCmd::ClipClear;
    }
    if let Some(rest) = trimmed.strip_prefix("set ") {
        let mut parts = rest.split_whitespace();
        if let (Some(id_s), Some(val_s)) = (parts.next(), parts.next())
            && let (Ok(numid), Ok(val)) = (id_s.parse::<u32>(), val_s.parse::<i64>())
        {
            let expected_gen = match parts.next() {
                Some(gen_s) => match gen_s.parse::<u64>() {
                    Ok(g) => Some(g),
                    Err(_) => return DaemonCmd::Unknown,
                },
                None => None,
            };
            return DaemonCmd::Set {
                numid,
                val,
                expected_gen,
            };
        }
    }
    DaemonCmd::Unknown
}

fn read_stdin_nonblocking(fd: i32, acc: &mut LineAccumulator) -> (Vec<String>, bool) {
    let mut lines = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if n > 0 {
            lines.extend(acc.push_bytes(&chunk[..n as usize]));
        } else if n == 0 {
            if let Some(last) = acc.finish() {
                lines.push(last);
            }
            return (lines, true);
        } else {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if err.kind() == io::ErrorKind::WouldBlock {
                return (lines, false);
            }
            return (lines, true);
        }
    }
}

struct DaemonState {
    alsa: Option<AlsaCard>,
    model: Option<ScarlettModel>,
    generation: u64,
    meters_on: bool,
    clips: ClipLatch,
    pw: PwMonitor,
}

impl DaemonState {
    fn disconnect(&mut self) -> io::Result<()> {
        self.alsa = None;
        self.model = None;
        self.clips = ClipLatch::default();
        self.generation = self.generation.wrapping_add(1);
        self.pw.set_card(None);
        emit_disconnected(self.generation)
    }

    fn emit_current_snapshot(&self) -> io::Result<()> {
        match &self.model {
            Some(m) => emit_json(&build_snapshot(m, self.generation, self.pw.count())),
            None => emit_disconnected(self.generation),
        }
    }

    /// Drain pending ALSA events and apply value, metadata, or structural updates.
    /// Returns `Ok(true)` if an event changed state (and emitted a snapshot).
    fn process_alsa_events(&mut self) -> io::Result<bool> {
        let (Some(alsa), Some(model)) = (&self.alsa, &mut self.model) else {
            return Ok(false);
        };
        if !alsa.exists() {
            self.disconnect()?;
            return Ok(true);
        }
        let events = match alsa.drain_events() {
            Ok(ev) => ev,
            Err(_) => {
                self.disconnect()?;
                return Ok(true);
            }
        };

        let meter_id = model.meter_numid;
        if events.structural {
            if let Ok(ctrls) = alsa.enumerate() {
                let stream = read_stream_rate(model.card.index);
                *model = ScarlettModel::build(model.card.clone(), stream, ctrls);
                self.generation = self.generation.wrapping_add(1);
                self.emit_current_snapshot()?;
                return Ok(true);
            }
            return Ok(false);
        }

        let mut changed_any = false;
        for id in events.meta_changed {
            if Some(id) == meter_id {
                continue;
            }
            if let Some(updated) = alsa.describe_and_read(id) {
                model.controls.insert(id, updated);
                changed_any = true;
            }
        }
        for id in events.value_changed {
            if Some(id) == meter_id {
                continue;
            }
            if let Some(c) = model.controls.get_mut(&id) {
                let _ = alsa.refresh_control_value(c);
                changed_any = true;
            }
        }

        if changed_any {
            model.stream = read_stream_rate(model.card.index);
            model.recompute_derived();
            self.emit_current_snapshot()?;
        }
        Ok(changed_any)
    }

    fn handle_command(&mut self, line: &str) -> io::Result<()> {
        match parse_command(line) {
            DaemonCmd::MetersOn => {
                self.meters_on = true;
            }
            DaemonCmd::MetersOff => {
                self.meters_on = false;
            }
            DaemonCmd::ClipClear => {
                self.clips.clear();
            }
            DaemonCmd::Refresh => {
                self.pw.request_refresh();
                if let (Some(alsa), Some(model)) = (&self.alsa, &mut self.model)
                    && let Ok(ctrls) = alsa.enumerate()
                {
                    let stream = read_stream_rate(model.card.index);
                    *model = ScarlettModel::build(model.card.clone(), stream, ctrls);
                    self.generation = self.generation.wrapping_add(1);
                    self.emit_current_snapshot()?;
                } else {
                    self.emit_current_snapshot()?;
                }
            }
            DaemonCmd::Set {
                numid,
                val,
                expected_gen,
            } => {
                // Process any pending ALSA structural/metadata events BEFORE writing.
                self.process_alsa_events()?;

                if let Some(g) = expected_gen
                    && g != self.generation
                {
                    emit_error("Stale control write rejected after state change")?;
                    self.emit_current_snapshot()?;
                    return Ok(());
                }

                let (Some(alsa), Some(model)) = (&self.alsa, &mut self.model) else {
                    emit_error("No Focusrite device connected")?;
                    return Ok(());
                };
                let Some(ctrl) = model.controls.get(&numid).cloned() else {
                    emit_error(&format!("Control #{numid} not found"))?;
                    self.emit_current_snapshot()?;
                    return Ok(());
                };
                if let Err(e) = alsa.write_single(&ctrl, val) {
                    emit_error(&e)?;
                    self.emit_current_snapshot()?;
                    return Ok(());
                }
                let readback = match model.controls.get_mut(&numid) {
                    Some(c) => alsa.refresh_control_value(c),
                    None => Err(format!("Control #{numid} vanished after write")),
                };
                if let Err(e) = readback {
                    emit_error(&format!("Write readback failed: {e}"))?;
                    self.emit_current_snapshot()?;
                    return Ok(());
                }
                model.recompute_derived();
                self.emit_current_snapshot()?;
            }
            DaemonCmd::Unknown => {}
        }
        Ok(())
    }
}

pub fn run_daemon(card_override: Option<u32>) -> i32 {
    let stdin_fd = io::stdin().as_raw_fd();
    // Set stdin non-blocking so poll() drives both stdin and ALSA events cleanly.
    unsafe {
        let flags = libc::fcntl(stdin_fd, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(stdin_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }

    let mut state = DaemonState {
        alsa: None,
        model: None,
        generation: 0,
        meters_on: false,
        clips: ClipLatch::default(),
        pw: PwMonitor::spawn(),
    };
    let mut last_meter = Instant::now();
    let mut last_scan = Instant::now() - Duration::from_secs(10);
    let mut last_poll_fallback = Instant::now();
    let mut last_rate_poll = Instant::now();
    let mut acc = LineAccumulator::new();

    loop {
        // Scan for device when disconnected
        if state.alsa.is_none() && last_scan.elapsed() >= Duration::from_secs(2) {
            last_scan = Instant::now();
            let cards = discover_focusrite_cards();
            let selected = match card_override {
                Some(idx) => cards.into_iter().find(|c| c.index == idx),
                None => cards.into_iter().next(),
            };
            if let Some(entry) = selected
                && let Ok(alsa) = AlsaCard::open(entry.index)
                && let Ok(ctrls) = alsa.enumerate()
            {
                let stream = read_stream_rate(entry.index);
                state.pw.set_card(Some(entry.clone()));
                let model = ScarlettModel::build(entry, stream, ctrls);
                state.generation = state.generation.wrapping_add(1);
                state.alsa = Some(alsa);
                state.model = Some(model);
                if state.emit_current_snapshot().is_err() {
                    return 0;
                }
            } else if emit_disconnected(state.generation).is_err() {
                return 0;
            }
        }

        // If connected without event subscription or waiting for FCP mixer controls,
        // periodically re-enumerate so late-appearing controls are discovered.
        if let (Some(alsa), Some(model)) = (&state.alsa, &mut state.model)
            && (!alsa.events_subscribed || !model.has_mixer_controls())
            && last_poll_fallback.elapsed() >= Duration::from_secs(2)
        {
            last_poll_fallback = Instant::now();
            if !alsa.exists() {
                if state.disconnect().is_err() {
                    return 0;
                }
                continue;
            }
            if let Ok(ctrls) = alsa.enumerate() {
                let prev_count = model.controls.len();
                let prev_avail = model.has_mixer_controls();
                let stream = read_stream_rate(model.card.index);
                let new_model = ScarlettModel::build(model.card.clone(), stream, ctrls);
                let changed = new_model.controls.len() != prev_count
                    || new_model.has_mixer_controls() != prev_avail
                    || !alsa.events_subscribed;
                *model = new_model;
                if changed {
                    state.generation = state.generation.wrapping_add(1);
                    if state.emit_current_snapshot().is_err() {
                        return 0;
                    }
                }
            }
        }

        let mut pfds = vec![libc::pollfd {
            fd: stdin_fd,
            events: libc::POLLIN,
            revents: 0,
        }];
        if let Some(ref alsa) = state.alsa {
            pfds.extend(alsa.poll_fds());
        }

        let timeout_ms = if state.meters_on && state.alsa.is_some() {
            50
        } else {
            500
        };
        let _ = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, timeout_ms) };

        // 1. Always process pending ALSA events before executing stdin writes.
        if state.process_alsa_events().is_err() {
            return 0;
        }

        // 2. Process stdin commands with persistent line buffering across WouldBlock.
        if (pfds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR)) != 0 {
            let (lines, eof) = read_stdin_nonblocking(stdin_fd, &mut acc);
            for line in lines {
                if state.handle_command(&line).is_err() {
                    return 0;
                }
            }
            if eof {
                return 0;
            }
        }

        // 3. Emit updated state if PipeWire links or stream sample rate changed.
        let mut status_changed = state.pw.take_dirty();
        if last_rate_poll.elapsed() >= Duration::from_secs(2) {
            last_rate_poll = Instant::now();
            if let Some(ref mut model) = state.model {
                let new_stream = read_stream_rate(model.card.index);
                if new_stream.active_rate != model.stream.active_rate
                    || new_stream.playback_running != model.stream.playback_running
                    || new_stream.capture_running != model.stream.capture_running
                {
                    model.stream = new_stream;
                    status_changed = true;
                }
            }
        }
        if status_changed && state.model.is_some() && state.emit_current_snapshot().is_err() {
            return 0;
        }

        // 4. Stream hardware meters when requested (panel open)
        if state.meters_on
            && last_meter.elapsed() >= Duration::from_millis(50)
            && let (Some(alsa), Some(model)) = (&state.alsa, &state.model)
            && let Some(mid) = model.meter_numid
            && let Some(mc) = model.controls.get(&mid)
            && let Ok(raw) = alsa.read_values(mid, mc.kind, mc.count)
        {
            last_meter = Instant::now();
            let dbfs: Vec<f64> = raw
                .iter()
                .map(|&v| meter_to_dbfs_with_max(v, model.meter_max))
                .collect();
            state.clips.update(&dbfs);
            if emit_json(&MeterPayload {
                msg_type: "meters",
                raw,
                dbfs,
                clip: state.clips.as_slice().to_vec(),
                clip_any: state.clips.any(),
            })
            .is_err()
            {
                return 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_accumulator_handles_fragmented_commands() {
        let mut acc = LineAccumulator::new();
        assert!(acc.push_bytes(b"ref").is_empty());
        assert!(acc.push_bytes(b"re").is_empty());
        assert_eq!(acc.push_bytes(b"sh\n"), vec!["refresh"]);
    }

    #[test]
    fn line_accumulator_handles_multiple_commands_and_crlf() {
        let mut acc = LineAccumulator::new();
        let lines = acc.push_bytes(b"meters on\r\nset 22 11 3\nref");
        assert_eq!(lines, vec!["meters on", "set 22 11 3"]);
        assert_eq!(acc.finish(), Some("ref".into()));
    }

    #[test]
    fn line_accumulator_drops_oversize_lines_safely() {
        let mut acc = LineAccumulator::new();
        let big = vec![b'a'; MAX_STDIN_LINE_BYTES + 100];
        assert!(acc.push_bytes(&big).is_empty());
        assert!(acc.push_bytes(b"\n").is_empty());
        assert_eq!(acc.push_bytes(b"refresh\n"), vec!["refresh"]);
    }

    #[test]
    fn parses_set_command_with_and_without_generation() {
        assert_eq!(
            parse_command("set 22 11"),
            DaemonCmd::Set {
                numid: 22,
                val: 11,
                expected_gen: None
            }
        );
        assert_eq!(
            parse_command("set 22 11 42"),
            DaemonCmd::Set {
                numid: 22,
                val: 11,
                expected_gen: Some(42)
            }
        );
        assert_eq!(parse_command("set 22 11 bogus"), DaemonCmd::Unknown);
        assert_eq!(parse_command(" clip clear "), DaemonCmd::ClipClear);
        assert_eq!(parse_command("clip"), DaemonCmd::Unknown);
    }
}
