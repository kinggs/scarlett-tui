use crate::alsa_ctl::{
    AlsaCard, CardEntry, ControlKind, discover_focusrite_cards, read_stream_rate,
};
use crate::model::{RouteGroup, ScarlettModel, meter_to_dbfs_with_max};
use crate::pipewire::{self, PwLink};
use crossterm::{
    cursor::Show,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, Tabs,
    },
};
use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview = 0,
    IoDevice = 1,
    Routing = 2,
    Matrix = 3,
    PipeWire = 4,
}

impl Tab {
    const ALL: [Tab; 5] = [
        Tab::Overview,
        Tab::IoDevice,
        Tab::Routing,
        Tab::Matrix,
        Tab::PipeWire,
    ];

    fn title(self) -> &'static str {
        match self {
            Tab::Overview => "1 Overview & Meters",
            Tab::IoDevice => "2 I/O & Device",
            Tab::Routing => "3 Routing",
            Tab::Matrix => "4 Mix Matrix",
            Tab::PipeWire => "5 PipeWire Links",
        }
    }
}

struct IoItem {
    section: &'static str,
    label: String,
    numid: u32,
}

struct RoutePicker {
    numid: u32,
    card_index: u32,
    generation: u64,
    title: String,
    items: Vec<String>,
    selected: usize,
}

/// RAII guard that restores raw mode, alternate screen, and cursor even on panic.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen, Show);
    }
}

pub struct App {
    cards: Vec<CardEntry>,
    card_override: Option<u32>,
    current_card_index: Option<u32>,
    generation: u64,
    alsa: Option<AlsaCard>,
    model: Option<ScarlettModel>,
    tab: Tab,

    // Meters (raw + peak hold)
    meter_raw: Vec<i64>,
    meter_peak: Vec<f64>,
    meter_peak_time: Vec<Instant>,

    // Per-tab cursors & remembered matrix levels for safe unmute
    io_cursor: usize,
    route_cursor: usize,
    route_picker: Option<RoutePicker>,
    matrix_row: usize,
    matrix_col: usize,
    matrix_prev_levels: HashMap<(char, u32), i64>,
    pw_links: Vec<PwLink>,
    pw_show_all: bool,
    pw_cursor: usize,

    status_msg: String,
    status_at: Instant,
    last_rate_check: Instant,
    last_poll_fallback: Instant,
    should_quit: bool,
}

impl App {
    fn new(card_override: Option<u32>) -> Self {
        let cards = discover_focusrite_cards();
        let current_card_index = match card_override {
            Some(idx) => cards.iter().find(|c| c.index == idx).map(|c| c.index),
            None => cards.first().map(|c| c.index),
        };
        let mut app = Self {
            cards,
            card_override,
            current_card_index,
            generation: 0,
            alsa: None,
            model: None,
            tab: Tab::Overview,
            meter_raw: Vec::new(),
            meter_peak: Vec::new(),
            meter_peak_time: Vec::new(),
            io_cursor: 0,
            route_cursor: 0,
            route_picker: None,
            matrix_row: 0,
            matrix_col: 0,
            matrix_prev_levels: HashMap::new(),
            pw_links: Vec::new(),
            pw_show_all: false,
            pw_cursor: 0,
            status_msg: String::new(),
            status_at: Instant::now(),
            last_rate_check: Instant::now(),
            last_poll_fallback: Instant::now(),
            should_quit: false,
        };
        app.connect_current();
        app.refresh_pipewire();
        app
    }

    /// An app with no ALSA handle, no card scan and no `pw-link` call, for render tests.
    #[cfg(test)]
    fn for_test(model: Option<ScarlettModel>) -> Self {
        let n = model.as_ref().map_or(0, |m| m.meter_channels.len());
        Self {
            cards: Vec::new(),
            card_override: None,
            current_card_index: model.as_ref().map(|m| m.card.index),
            generation: 1,
            alsa: None,
            model,
            tab: Tab::Overview,
            meter_raw: vec![0; n],
            meter_peak: vec![-96.0; n],
            meter_peak_time: vec![Instant::now(); n],
            io_cursor: 0,
            route_cursor: 0,
            route_picker: None,
            matrix_row: 0,
            matrix_col: 0,
            matrix_prev_levels: HashMap::new(),
            pw_links: Vec::new(),
            pw_show_all: false,
            pw_cursor: 0,
            status_msg: String::new(),
            status_at: Instant::now(),
            last_rate_check: Instant::now(),
            last_poll_fallback: Instant::now(),
            should_quit: false,
        }
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = msg.into();
        self.status_at = Instant::now();
    }

    /// Clamp every cursor and invalidate any open modal after a model change so
    /// switching to a smaller interface can never index out of bounds.
    fn clamp_cursors(&mut self) {
        self.route_picker = None;
        let io_len = self.io_items().len();
        self.io_cursor = if io_len == 0 {
            0
        } else {
            self.io_cursor.min(io_len - 1)
        };
        if let Some(ref m) = self.model {
            self.route_cursor = if m.routes.is_empty() {
                0
            } else {
                self.route_cursor.min(m.routes.len() - 1)
            };
            self.matrix_row = if m.matrix.inputs.is_empty() {
                0
            } else {
                self.matrix_row.min(m.matrix.inputs.len() - 1)
            };
            self.matrix_col = if m.matrix.mixes.is_empty() {
                0
            } else {
                self.matrix_col.min(m.matrix.mixes.len() - 1)
            };
        } else {
            self.route_cursor = 0;
            self.matrix_row = 0;
            self.matrix_col = 0;
        }
        let pw_len = self.visible_pw_links().len();
        self.pw_cursor = if pw_len == 0 {
            0
        } else {
            self.pw_cursor.min(pw_len - 1)
        };
    }

    fn connect_current(&mut self) {
        self.alsa = None;
        self.model = None;
        self.route_picker = None;
        self.matrix_prev_levels.clear();
        self.meter_raw.clear();
        self.meter_peak.clear();
        self.meter_peak_time.clear();
        self.generation = self.generation.wrapping_add(1);

        let entry = self
            .current_card_index
            .and_then(|idx| self.cards.iter().find(|c| c.index == idx))
            .cloned();

        let Some(entry) = entry else {
            if let Some(req) = self.card_override {
                if self.cards.is_empty() {
                    self.set_status(format!(
                        "Requested card {req} not found (no Focusrite USB interfaces detected)"
                    ));
                } else {
                    let avail: Vec<String> = self
                        .cards
                        .iter()
                        .map(|c| format!("{} ({})", c.index, c.name))
                        .collect();
                    self.set_status(format!(
                        "Requested card {req} not found (available: {})",
                        avail.join(", ")
                    ));
                }
            } else {
                self.set_status("No Focusrite Scarlett USB interface found");
            }
            self.clamp_cursors();
            return;
        };

        match AlsaCard::open(entry.index) {
            Ok(alsa) => match alsa.enumerate() {
                Ok(ctrls) => {
                    let stream = read_stream_rate(entry.index);
                    let model = ScarlettModel::build(entry, stream, ctrls);
                    let n = model.meter_channels.len();
                    self.meter_raw = vec![0; n];
                    self.meter_peak = vec![-96.0; n];
                    self.meter_peak_time = vec![Instant::now(); n];
                    if !alsa.events_subscribed {
                        self.set_status("ALSA event subscription unavailable; polling every 2s");
                    } else if !model.has_mixer_controls() {
                        self.set_status(
                            "Card opened, but mixer controls are unavailable (for Gen 4 16i16/18i16/18i20, check fcp-server)",
                        );
                    }
                    self.alsa = Some(alsa);
                    self.model = Some(model);
                    self.clamp_cursors();
                }
                Err(e) => {
                    self.set_status(format!("ALSA enumerate error: {e}"));
                    self.clamp_cursors();
                }
            },
            Err(e) => {
                self.set_status(format!("ALSA open error: {e}"));
                self.clamp_cursors();
            }
        }
    }

    fn refresh_all(&mut self) {
        self.cards = discover_focusrite_cards();
        self.current_card_index = match self.card_override {
            Some(req) => self.cards.iter().find(|c| c.index == req).map(|c| c.index),
            None => self
                .current_card_index
                .filter(|idx| self.cards.iter().any(|c| c.index == *idx))
                .or_else(|| self.cards.first().map(|c| c.index)),
        };
        self.connect_current();
        self.refresh_pipewire();
        if self.model.is_some() {
            self.set_status("Refreshed ALSA & PipeWire state");
        }
    }

    fn refresh_pipewire(&mut self) {
        let card = self.model.as_ref().map(|m| &m.card);
        match pipewire::list_links_for_card(card) {
            Ok(links) => {
                self.pw_links = links;
                let count = self.visible_pw_links().len();
                if self.pw_cursor >= count {
                    self.pw_cursor = count.saturating_sub(1);
                }
            }
            Err(e) => self.set_status(format!("pw-link: {e}")),
        }
    }

    fn visible_pw_links(&self) -> Vec<&PwLink> {
        self.pw_links
            .iter()
            .filter(|l| self.pw_show_all || l.is_scarlett)
            .collect()
    }

    fn io_items(&self) -> Vec<IoItem> {
        let Some(ref m) = self.model else {
            return Vec::new();
        };
        let mut items = Vec::new();
        for inp in &m.inputs {
            if let Some(id) = inp.gain_numid {
                items.push(IoItem {
                    section: "Inputs",
                    label: format!("{} Gain", inp.label),
                    numid: id,
                });
            }
            if let Some(id) = inp.level_numid {
                items.push(IoItem {
                    section: "Inputs",
                    label: format!("{} Level (Line/Inst)", inp.label),
                    numid: id,
                });
            }
            if let Some(id) = inp.air_numid {
                items.push(IoItem {
                    section: "Inputs",
                    label: format!("{} Air", inp.label),
                    numid: id,
                });
            }
            if let Some(id) = inp.pad_numid {
                items.push(IoItem {
                    section: "Inputs",
                    label: format!("{} Pad", inp.label),
                    numid: id,
                });
            }
            if let Some(id) = inp.safe_numid {
                items.push(IoItem {
                    section: "Inputs",
                    label: format!("{} Clip Safe", inp.label),
                    numid: id,
                });
            }
            if let Some(id) = inp.autogain_numid {
                items.push(IoItem {
                    section: "Inputs",
                    label: format!("{} Autogain", inp.label),
                    numid: id,
                });
            }
            if let Some(id) = inp.link_numid {
                items.push(IoItem {
                    section: "Inputs",
                    label: format!("{} Link", inp.label),
                    numid: id,
                });
            }
        }
        for &id in &m.phantom_numids {
            if let Some(c) = m.controls.get(&id) {
                items.push(IoItem {
                    section: "Phantom Power",
                    label: c.name.replace(" Capture Switch", ""),
                    numid: id,
                });
            }
        }
        for out in &m.outputs {
            if let Some(id) = out.volume_numid {
                items.push(IoItem {
                    section: "Outputs",
                    label: format!("{} Volume", out.label),
                    numid: id,
                });
            }
            if let Some(id) = out.mute_numid {
                items.push(IoItem {
                    section: "Outputs",
                    label: format!("{} Mute", out.label),
                    numid: id,
                });
            }
        }
        if let Some(id) = m.direct_monitor_numid
            && let Some(c) = m.controls.get(&id)
        {
            items.push(IoItem {
                section: "Device",
                label: c.name.clone(),
                numid: id,
            });
        }
        for &id in &m.device_numids {
            if let Some(c) = m.controls.get(&id) {
                items.push(IoItem {
                    section: "Device",
                    label: c.name.clone(),
                    numid: id,
                });
            }
        }
        items
    }

    fn write_control(&mut self, numid: u32, new_val: i64) -> bool {
        let Some(ref alsa) = self.alsa else {
            return false;
        };
        let Some(ref mut model) = self.model else {
            return false;
        };
        let Some(ctrl) = model.controls.get(&numid).cloned() else {
            self.set_status(format!("Control #{numid} not found"));
            return false;
        };
        if let Err(e) = alsa.write_single(&ctrl, new_val) {
            self.set_status(format!("Write failed: {e}"));
            return false;
        }
        let readback = match model.controls.get_mut(&numid) {
            Some(c) => alsa.refresh_control_value(c),
            None => Err(format!("Control #{numid} disappeared")),
        };
        if let Err(e) = readback {
            self.set_status(format!("Readback failed: {e}"));
            return false;
        }
        model.recompute_derived();
        true
    }

    fn tick(&mut self) {
        // Drain hardware events (physical button presses / plugin changes / access changes)
        let mut disconnected = false;
        let mut structural_rebuild = false;
        if let (Some(alsa), Some(model)) = (&self.alsa, &mut self.model) {
            if !alsa.exists() {
                disconnected = true;
            } else {
                match alsa.drain_events() {
                    Ok(events) => {
                        let mid = model.meter_numid;
                        if events.structural {
                            if let Ok(ctrls) = alsa.enumerate() {
                                let stream = read_stream_rate(model.card.index);
                                *model = ScarlettModel::build(model.card.clone(), stream, ctrls);
                                structural_rebuild = true;
                            }
                        } else {
                            let mut dirty = false;
                            for id in events.meta_changed {
                                if Some(id) == mid {
                                    continue;
                                }
                                if let Some(updated) = alsa.describe_and_read(id) {
                                    model.controls.insert(id, updated);
                                    dirty = true;
                                }
                            }
                            for id in events.value_changed {
                                if Some(id) == mid {
                                    continue;
                                }
                                if let Some(c) = model.controls.get_mut(&id) {
                                    let _ = alsa.refresh_control_value(c);
                                    dirty = true;
                                }
                            }
                            if dirty {
                                model.recompute_derived();
                            }
                        }
                    }
                    Err(_) => {
                        disconnected = true;
                    }
                }
            }
        }

        if disconnected {
            self.alsa = None;
            self.model = None;
            self.generation = self.generation.wrapping_add(1);
            self.clamp_cursors();
            self.set_status("Scarlett disconnected — press r to rescan");
            return;
        }
        if structural_rebuild {
            self.generation = self.generation.wrapping_add(1);
            self.clamp_cursors();
        }

        // Polling fallback when event subscription failed or waiting for FCP mixer controls.
        if let (Some(alsa), Some(model)) = (&self.alsa, &mut self.model)
            && (!alsa.events_subscribed || !model.has_mixer_controls())
            && self.last_poll_fallback.elapsed() >= Duration::from_secs(2)
        {
            self.last_poll_fallback = Instant::now();
            if let Ok(ctrls) = alsa.enumerate() {
                let prev_len = model.controls.len();
                let prev_avail = model.has_mixer_controls();
                let stream = read_stream_rate(model.card.index);
                let new_model = ScarlettModel::build(model.card.clone(), stream, ctrls);
                let rebuilt = new_model.controls.len() != prev_len
                    || new_model.has_mixer_controls() != prev_avail;
                *model = new_model;
                if rebuilt {
                    self.generation = self.generation.wrapping_add(1);
                    self.clamp_cursors();
                }
            }
        }

        // Poll live hardware meters on Overview tab
        if self.tab == Tab::Overview
            && let (Some(alsa), Some(model)) = (&self.alsa, &self.model)
            && let Some(mid) = model.meter_numid
            && let Some(mc) = model.controls.get(&mid)
            && let Ok(raw) = alsa.read_values(mid, mc.kind, mc.count)
        {
            let now = Instant::now();
            if self.meter_peak.len() != raw.len() {
                self.meter_peak = vec![-96.0; raw.len()];
                self.meter_peak_time = vec![now; raw.len()];
            }
            for (i, &v) in raw.iter().enumerate() {
                let db = meter_to_dbfs_with_max(v, model.meter_max);
                if db >= self.meter_peak[i]
                    || now.duration_since(self.meter_peak_time[i]) > Duration::from_millis(1200)
                {
                    self.meter_peak[i] = db;
                    self.meter_peak_time[i] = now;
                }
            }
            self.meter_raw = raw;
        }

        // Periodically refresh active stream sample rate
        if self.last_rate_check.elapsed() >= Duration::from_secs(1) {
            self.last_rate_check = Instant::now();
            if let Some(ref mut model) = self.model {
                model.stream = read_stream_rate(model.card.index);
            }
        }
    }

    fn handle_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        if code == KeyCode::Char('c') && mods.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }

        // Route picker modal intercepts keys
        if let Some(mut picker) = self.route_picker.take() {
            match code {
                KeyCode::Esc | KeyCode::Char('q') => {}
                KeyCode::Up | KeyCode::Char('k') => {
                    picker.selected = picker.selected.saturating_sub(1);
                    self.route_picker = Some(picker);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if picker.selected + 1 < picker.items.len() {
                        picker.selected += 1;
                    }
                    self.route_picker = Some(picker);
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let valid_gen = picker.generation == self.generation
                        && self
                            .model
                            .as_ref()
                            .is_some_and(|m| m.card.index == picker.card_index);
                    if !valid_gen {
                        self.set_status("Route selection expired after device update");
                        return;
                    }
                    let numid = picker.numid;
                    let sel = picker.selected as i64;
                    let label = picker
                        .items
                        .get(picker.selected)
                        .cloned()
                        .unwrap_or_default();
                    let title = picker.title.clone();
                    if self.write_control(numid, sel) {
                        self.set_status(format!("{title} → {label}"));
                    }
                }
                _ => {
                    self.route_picker = Some(picker);
                }
            }
            return;
        }

        match code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('1') => {
                self.tab = Tab::Overview;
                return;
            }
            KeyCode::Char('2') => {
                self.tab = Tab::IoDevice;
                return;
            }
            KeyCode::Char('3') => {
                self.tab = Tab::Routing;
                return;
            }
            KeyCode::Char('4') => {
                self.tab = Tab::Matrix;
                return;
            }
            KeyCode::Char('5') => {
                self.tab = Tab::PipeWire;
                self.refresh_pipewire();
                return;
            }
            KeyCode::Tab => {
                let next = (self.tab as usize + 1) % Tab::ALL.len();
                self.tab = Tab::ALL[next];
                if self.tab == Tab::PipeWire {
                    self.refresh_pipewire();
                }
                return;
            }
            KeyCode::BackTab => {
                let prev = (self.tab as usize + Tab::ALL.len() - 1) % Tab::ALL.len();
                self.tab = Tab::ALL[prev];
                if self.tab == Tab::PipeWire {
                    self.refresh_pipewire();
                }
                return;
            }
            KeyCode::Char('r') => {
                self.refresh_all();
                return;
            }
            KeyCode::Char('c') if self.cards.len() > 1 && self.card_override.is_none() => {
                let cur_pos = self
                    .current_card_index
                    .and_then(|idx| self.cards.iter().position(|c| c.index == idx))
                    .unwrap_or(0);
                let next_pos = (cur_pos + 1) % self.cards.len();
                self.current_card_index = Some(self.cards[next_pos].index);
                self.connect_current();
                self.refresh_pipewire();
                if let Some(ref m) = self.model {
                    self.set_status(format!("Switched to {}", m.card.name));
                }
                return;
            }
            _ => {}
        }

        match self.tab {
            Tab::Overview => {}
            Tab::IoDevice => self.handle_io_key(code),
            Tab::Routing => self.handle_routing_key(code),
            Tab::Matrix => self.handle_matrix_key(code),
            Tab::PipeWire => self.handle_pw_key(code),
        }
    }

    fn handle_io_key(&mut self, code: KeyCode) {
        let items = self.io_items();
        if items.is_empty() {
            return;
        }
        if self.io_cursor >= items.len() {
            self.io_cursor = items.len() - 1;
        }
        let item = &items[self.io_cursor];
        let Some(ctrl) = self
            .model
            .as_ref()
            .and_then(|m| m.controls.get(&item.numid))
            .cloned()
        else {
            return;
        };

        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.io_cursor = self.io_cursor.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.io_cursor + 1 < items.len() {
                    self.io_cursor += 1;
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if !ctrl.is_editable() {
                    self.set_status(format!("'{}' is read-only", ctrl.name));
                    return;
                }
                match ctrl.kind {
                    ControlKind::Bool => {
                        if let Some(b) = ctrl.opt_bool_val() {
                            self.write_control(ctrl.numid, if b { 0 } else { 1 });
                        }
                    }
                    ControlKind::Enum => {
                        if !ctrl.items.is_empty()
                            && let Some(cur) = ctrl.opt_val()
                        {
                            let next = (cur + 1) % ctrl.items.len() as i64;
                            self.write_control(ctrl.numid, next);
                        }
                    }
                    ControlKind::Int => {
                        // Do not jump integer gain controls on Enter/Space — use h/l or 0.
                        self.set_status("Use h/l (+/-) to adjust, 0 for 0 dB unity, m for min");
                    }
                }
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('=') => {
                if !ctrl.is_editable() {
                    self.set_status(format!("'{}' is read-only", ctrl.name));
                    return;
                }
                match ctrl.kind {
                    ControlKind::Bool => {
                        self.write_control(ctrl.numid, 1);
                    }
                    ControlKind::Enum => {
                        if let Some(cur) = ctrl.opt_val() {
                            self.write_control(ctrl.numid, (cur + 1).min(ctrl.max));
                        }
                    }
                    ControlKind::Int => {
                        let next = self
                            .alsa
                            .as_ref()
                            .and_then(|a| a.step_db(&ctrl, 1.0))
                            .or_else(|| {
                                let cur = ctrl.opt_val()?;
                                let step = ((ctrl.max - ctrl.min) / 64).max(1);
                                Some((cur + step).min(ctrl.max))
                            });
                        if let Some(v) = next {
                            self.write_control(ctrl.numid, v);
                        }
                    }
                }
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') => {
                if !ctrl.is_editable() {
                    self.set_status(format!("'{}' is read-only", ctrl.name));
                    return;
                }
                match ctrl.kind {
                    ControlKind::Bool => {
                        self.write_control(ctrl.numid, 0);
                    }
                    ControlKind::Enum => {
                        if let Some(cur) = ctrl.opt_val() {
                            self.write_control(ctrl.numid, (cur - 1).max(ctrl.min));
                        }
                    }
                    ControlKind::Int => {
                        let next = self
                            .alsa
                            .as_ref()
                            .and_then(|a| a.step_db(&ctrl, -1.0))
                            .or_else(|| {
                                let cur = ctrl.opt_val()?;
                                let step = ((ctrl.max - ctrl.min) / 64).max(1);
                                Some((cur - step).max(ctrl.min))
                            });
                        if let Some(v) = next {
                            self.write_control(ctrl.numid, v);
                        }
                    }
                }
            }
            KeyCode::Char('0') if ctrl.kind == ControlKind::Int => {
                if let Some(unity) = self
                    .model
                    .as_ref()
                    .and_then(|m| m.unity_raw_for(ctrl.numid))
                {
                    if self.write_control(ctrl.numid, unity) {
                        self.set_status(format!("{} → 0.0 dB", item.label));
                    }
                } else {
                    self.set_status(format!("No 0 dB reference for {}", item.label));
                }
            }
            KeyCode::Char('m') if ctrl.kind == ControlKind::Int => {
                self.write_control(ctrl.numid, ctrl.min);
            }
            _ => {}
        }
    }

    fn handle_routing_key(&mut self, code: KeyCode) {
        let Some(ref model) = self.model else { return };
        if model.routes.is_empty() {
            return;
        }
        if self.route_cursor >= model.routes.len() {
            self.route_cursor = model.routes.len() - 1;
        }
        let route = &model.routes[self.route_cursor];
        let numid = route.numid;
        let title = route.label.clone();
        let card_index = model.card.index;
        let Some(ctrl) = model.controls.get(&numid).cloned() else {
            return;
        };

        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.route_cursor = self.route_cursor.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.route_cursor + 1 < model.routes.len() {
                    self.route_cursor += 1;
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if !ctrl.is_editable() {
                    self.set_status(format!("'{}' is read-only", ctrl.name));
                    return;
                }
                self.route_picker = Some(RoutePicker {
                    numid,
                    card_index,
                    generation: self.generation,
                    title,
                    items: ctrl.items.clone(),
                    selected: ctrl.val().max(0) as usize,
                });
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if !ctrl.is_editable() {
                    self.set_status(format!("'{}' is read-only", ctrl.name));
                    return;
                }
                if !ctrl.items.is_empty()
                    && let Some(cur) = ctrl.opt_val()
                {
                    let next = (cur + 1) % ctrl.items.len() as i64;
                    self.write_control(numid, next);
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if !ctrl.is_editable() {
                    self.set_status(format!("'{}' is read-only", ctrl.name));
                    return;
                }
                if !ctrl.items.is_empty()
                    && let Some(cur) = ctrl.opt_val()
                {
                    let len = ctrl.items.len() as i64;
                    let prev = (cur + len - 1) % len;
                    self.write_control(numid, prev);
                }
            }
            KeyCode::Char('x') | KeyCode::Char('0') if self.write_control(numid, 0) => {
                self.set_status(format!("{title} → Off"));
            }
            _ => {}
        }
    }

    fn handle_matrix_key(&mut self, code: KeyCode) {
        let Some(ref model) = self.model else { return };
        if model.matrix.is_empty() {
            return;
        }
        let n_rows = model.matrix.inputs.len();
        let n_cols = model.matrix.mixes.len();
        self.matrix_row = self.matrix_row.min(n_rows - 1);
        self.matrix_col = self.matrix_col.min(n_cols - 1);

        let mix = model.matrix.mixes[self.matrix_col];
        let input = model.matrix.inputs[self.matrix_row];
        let cell_id = model.matrix.cells.get(&(mix, input)).copied();
        let cell_ctrl = cell_id.and_then(|id| model.controls.get(&id)).cloned();
        let unity_opt = cell_id.and_then(|id| model.unity_raw_for(id));

        match code {
            KeyCode::Up | KeyCode::Char('k') => self.matrix_row = self.matrix_row.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                if self.matrix_row + 1 < n_rows {
                    self.matrix_row += 1;
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.matrix_col = self.matrix_col.saturating_sub(1)
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.matrix_col + 1 < n_cols {
                    self.matrix_col += 1;
                }
            }
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char(']') => {
                if let Some(ref c) = cell_ctrl
                    && let Some(v) = self
                        .alsa
                        .as_ref()
                        .and_then(|a| a.step_db(c, 1.0))
                        .or_else(|| c.opt_val().map(|cur| (cur + 1).min(c.max)))
                {
                    self.write_control(c.numid, v);
                }
            }
            KeyCode::Char('-') | KeyCode::Char('[') => {
                if let Some(ref c) = cell_ctrl
                    && let Some(v) = self
                        .alsa
                        .as_ref()
                        .and_then(|a| a.step_db(c, -1.0))
                        .or_else(|| c.opt_val().map(|cur| (cur - 1).max(c.min)))
                {
                    self.write_control(c.numid, v);
                }
            }
            KeyCode::Char('}') => {
                if let Some(ref c) = cell_ctrl
                    && let Some(v) = self
                        .alsa
                        .as_ref()
                        .and_then(|a| a.step_db(c, 6.0))
                        .or_else(|| c.opt_val().map(|cur| (cur + 6).min(c.max)))
                {
                    self.write_control(c.numid, v);
                }
            }
            KeyCode::Char('{') => {
                if let Some(ref c) = cell_ctrl
                    && let Some(v) = self
                        .alsa
                        .as_ref()
                        .and_then(|a| a.step_db(c, -6.0))
                        .or_else(|| c.opt_val().map(|cur| (cur - 6).max(c.min)))
                {
                    self.write_control(c.numid, v);
                }
            }
            KeyCode::Char('0') => {
                if let Some(id) = cell_id {
                    if let Some(unity) = unity_opt {
                        if self.write_control(id, unity) {
                            self.set_status(format!("Mix {mix} In {input:02} → 0.0 dB"));
                        }
                    } else {
                        self.set_status(format!("No 0 dB reference for Mix {mix} In {input:02}"));
                    }
                }
            }
            KeyCode::Char('m') | KeyCode::Char('x') | KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(ref c) = cell_ctrl
                    && let Some(cur) = c.opt_val()
                {
                    if cur > c.min {
                        // Save previous level so unmute restores it instead of jumping to 0 dB.
                        self.matrix_prev_levels.insert((mix, input), cur);
                        self.write_control(c.numid, c.min);
                    } else if let Some(prev) = self.matrix_prev_levels.get(&(mix, input)).copied() {
                        self.write_control(c.numid, prev.clamp(c.min, c.max));
                    } else if let Some(unity) = unity_opt {
                        self.write_control(c.numid, unity);
                    } else {
                        self.set_status(format!(
                            "No previous level or 0 dB reference for Mix {mix} In {input:02}"
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_pw_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.pw_cursor = self.pw_cursor.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.visible_pw_links().len();
                if self.pw_cursor + 1 < len {
                    self.pw_cursor += 1;
                }
            }
            KeyCode::Char('a') => {
                self.pw_show_all = !self.pw_show_all;
                self.pw_cursor = 0;
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let link_info = self.visible_pw_links().get(self.pw_cursor).map(|l| {
                    (
                        l.id,
                        pipewire::short_port(&l.output_port),
                        pipewire::short_port(&l.input_port),
                    )
                });
                if let Some((id, src, dst)) = link_info {
                    match pipewire::disconnect_link(id) {
                        Ok(()) => {
                            self.refresh_pipewire();
                            self.set_status(format!("Disconnected #{id}: {src} → {dst}"));
                        }
                        Err(e) => self.set_status(format!("Disconnect failed: {e}")),
                    }
                }
            }
            _ => {}
        }
    }

    fn render(&self, f: &mut Frame) {
        let area = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Length(2), // Tabs
                Constraint::Min(8),    // Body
                Constraint::Length(2), // Footer / key hints
            ])
            .split(area);

        self.render_header(f, chunks[0]);
        self.render_tabs(f, chunks[1]);

        match self.tab {
            Tab::Overview => self.render_overview(f, chunks[2]),
            Tab::IoDevice => self.render_io(f, chunks[2]),
            Tab::Routing => self.render_routing(f, chunks[2]),
            Tab::Matrix => self.render_matrix(f, chunks[2]),
            Tab::PipeWire => self.render_pipewire(f, chunks[2]),
        }

        self.render_footer(f, chunks[3]);

        if let Some(ref picker) = self.route_picker {
            self.render_route_picker(f, area, picker);
        }
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let line = if let Some(ref m) = self.model {
            let rate_str = m
                .stream
                .active_rate
                .map(|r| format!("{:.1} kHz", r as f64 / 1000.0))
                .unwrap_or_else(|| "Idle".into());
            let sync_color = if !m.has_mixer_controls() {
                Color::Red
            } else if m.is_locked() {
                Color::Green
            } else {
                Color::Yellow
            };
            let sync_label = if !m.has_mixer_controls() && m.sync_status == "Unknown" {
                "Mixer unavailable".to_string()
            } else {
                m.sync_status.clone()
            };
            let fw = m
                .firmware
                .map(|v| format!("FW {v}"))
                .unwrap_or_else(|| "FW ?".into());
            let pw_count = self.pw_links.iter().filter(|l| l.is_scarlett).count();
            Line::from(vec![
                Span::styled(
                    format!(" {} ", m.card.name),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("({})  ", m.card.generation),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(format!("USB {}  ", m.card.usb_id), secondary()),
                Span::styled(format!("{fw}  "), secondary()),
                Span::styled(
                    format!("● {sync_label}  "),
                    Style::default().fg(sync_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{rate_str}  "), Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!("PW links: {pw_count}"),
                    if pw_count > 0 {
                        Style::default().fg(Color::Yellow)
                    } else {
                        secondary()
                    },
                ),
            ])
        } else if let Some(req) = self.card_override {
            Line::from(Span::styled(
                format!(" Focusrite card {req} not connected (press r to rescan)"),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::from(Span::styled(
                " No Focusrite Scarlett connected (press r to rescan)",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ))
        };

        let header = Paragraph::new(line).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" scarlett-tui ")
                .border_style(decoration())
                .title_style(title_style()),
        );
        f.render_widget(header, area);
    }

    fn render_tabs(&self, f: &mut Frame, area: Rect) {
        let titles: Vec<Line> = Tab::ALL.iter().map(|t| Line::from(t.title())).collect();
        let tabs = Tabs::new(titles)
            .select(self.tab as usize)
            .style(secondary())
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )
            .divider(" │ ");
        f.render_widget(tabs, area);
    }

    fn render_overview(&self, f: &mut Frame, area: Rect) {
        let Some(ref m) = self.model else {
            let p = Paragraph::new("Connect a Focusrite Scarlett interface and press 'r'.")
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(p, area);
            return;
        };

        if !m.has_mixer_controls() {
            let p = Paragraph::new(
                "USB audio device opened, but no Focusrite mixer controls are exposed yet.\n\
                 - For Scarlett Gen 4 16i16 / 18i16 / 18i20 (FCP), ensure fcp-server is running.\n\
                 - For kernels older than 6.7, ensure snd_usb_audio device_setup=1 is set.\n\
                 - Scarlett Gen 1 (mixer_scarlett) is not supported.\n\
                 Press 'r' to rescan once mixer controls are available.",
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Mixer Unavailable "),
            );
            f.render_widget(p, area);
            return;
        }

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(area);

        // Left: Live Hardware Level Meters
        let bar_width = cols[0].width.saturating_sub(36).max(8) as usize;
        let mut meter_lines = Vec::new();
        let mut last_group: Option<RouteGroup> = None;

        for mc in &m.meter_channels {
            if mc.source == "Off" && self.meter_raw.get(mc.meter_index).copied().unwrap_or(0) == 0 {
                continue;
            }
            if last_group != Some(mc.group) {
                if last_group.is_some() {
                    meter_lines.push(Line::from(""));
                }
                meter_lines.push(Line::from(Span::styled(
                    format!(" {}", mc.group.title()),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
                last_group = Some(mc.group);
            }

            let raw = self.meter_raw.get(mc.meter_index).copied().unwrap_or(0);
            let db = meter_to_dbfs_with_max(raw, m.meter_max);
            let peak = self
                .meter_peak
                .get(mc.meter_index)
                .copied()
                .unwrap_or(-96.0);
            let frac = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
            let filled = (frac * bar_width as f64).round() as usize;
            let empty = bar_width.saturating_sub(filled);

            let bar_color = if db >= -1.0 {
                Color::Red
            } else if db >= -9.0 {
                Color::Yellow
            } else {
                Color::Green
            };

            let db_text = if db <= -60.0 {
                "  -∞ dB".to_string()
            } else {
                format!("{db:+5.1} dB")
            };
            let peak_tag = if peak > -60.0 {
                format!(" (pk {peak:+.0})")
            } else {
                String::new()
            };
            let raw_label = if mc.source.is_empty() {
                mc.label.clone()
            } else {
                format!("{} [{}]", mc.label, mc.source)
            };
            let label = truncate(&raw_label, 22);

            meter_lines.push(Line::from(vec![
                Span::raw(format!(" {label:<22} ")),
                Span::styled("█".repeat(filled), Style::default().fg(bar_color)),
                Span::styled("░".repeat(empty), decoration()),
                Span::raw(format!(" {db_text}")),
                Span::styled(peak_tag, secondary()),
            ]));
        }

        if meter_lines.is_empty() {
            meter_lines.push(Line::from(Span::styled(
                " No hardware level meter exposed by this model.",
                secondary(),
            )));
        }

        let meter_panel = Paragraph::new(meter_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Live Hardware Meters (ALSA) ")
                .border_style(decoration())
                .title_style(title_style()),
        );
        f.render_widget(meter_panel, cols[0]);

        // Right: Summary of Inputs + Outputs + Active Routing
        let mut info_lines = Vec::new();
        info_lines.push(Line::from(Span::styled(
            " Inputs",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        for inp in &m.inputs {
            let mut badges = Vec::new();
            if let Some(id) = inp.level_numid
                && let Some(c) = m.controls.get(&id)
            {
                let lbl = c.enum_label();
                badges.push(Span::styled(
                    format!("[{lbl}]"),
                    if lbl == "Inst" {
                        Style::default().fg(Color::Yellow)
                    } else {
                        secondary()
                    },
                ));
            }
            if let Some(id) = inp.air_numid
                && let Some(c) = m.controls.get(&id)
            {
                let on = c.opt_val().unwrap_or(0) > 0;
                let txt = if c.kind == ControlKind::Enum && on {
                    format!("[Air:{}]", c.enum_label())
                } else {
                    "[Air]".into()
                };
                badges.push(Span::raw(" "));
                badges.push(Span::styled(
                    txt,
                    if on {
                        Style::default().fg(Color::Yellow)
                    } else {
                        secondary()
                    },
                ));
            }
            if let Some(id) = inp.pad_numid
                && let Some(c) = m.controls.get(&id)
            {
                badges.push(Span::raw(" "));
                badges.push(Span::styled(
                    "[Pad]",
                    if c.bool_val() {
                        Style::default().fg(Color::Cyan)
                    } else {
                        secondary()
                    },
                ));
            }
            if let Some(id) = inp.gain_numid
                && let Some(c) = m.controls.get(&id)
            {
                let gtxt = match c.current_db() {
                    Some(db) => format!(" {db:.0} dB"),
                    None => match c.opt_val() {
                        Some(v) => format!(" {v}"),
                        None => " ?".into(),
                    },
                };
                badges.push(Span::raw(gtxt));
            }
            let mut spans = vec![Span::raw(format!("  {:<8} ", inp.label))];
            spans.extend(badges);
            info_lines.push(Line::from(spans));
        }

        for &pid in &m.phantom_numids {
            if let Some(c) = m.controls.get(&pid) {
                let short = c.name.replace(" Capture Switch", "");
                let (txt, style) = match c.opt_bool_val() {
                    Some(true) => (
                        "ON",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Some(false) => ("OFF", secondary()),
                    None => ("?", Style::default().fg(Color::Yellow)),
                };
                info_lines.push(Line::from(vec![
                    Span::raw(format!("  {short}: ")),
                    Span::styled(txt, style),
                ]));
            }
        }

        info_lines.push(Line::from(""));
        info_lines.push(Line::from(Span::styled(
            " Outputs & Routing",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        for r in m
            .routes
            .iter()
            .filter(|r| r.group == RouteGroup::AnalogueOut)
        {
            let out_extra = m
                .outputs
                .iter()
                .find(|o| o.index == r.channel)
                .map(|o| {
                    let muted = o
                        .mute_numid
                        .and_then(|id| m.controls.get(&id))
                        .map(|c| c.bool_val())
                        .unwrap_or(false);
                    if muted {
                        " [MUTE]".to_string()
                    } else {
                        o.volume_numid
                            .and_then(|id| m.controls.get(&id))
                            .and_then(|c| c.current_db())
                            .map(|db| format!(" {db:.0}dB"))
                            .unwrap_or_default()
                    }
                })
                .unwrap_or_default();
            info_lines.push(Line::from(vec![
                Span::raw(format!("  {:<21} ", truncate(&r.label, 21))),
                Span::styled(r.summary.clone(), Style::default().fg(Color::Green)),
                Span::styled(out_extra, secondary()),
            ]));
        }

        if !m.matrix.is_empty() {
            info_lines.push(Line::from(""));
            info_lines.push(Line::from(vec![
                Span::styled(" Mix Matrix: ", Style::default().fg(Color::Cyan)),
                Span::raw(format!(
                    "{} mixes × {} inputs (press 4 to edit)",
                    m.matrix.mixes.len(),
                    m.matrix.inputs.len()
                )),
            ]));
        }

        let pw_scarlett: Vec<&PwLink> = self.pw_links.iter().filter(|l| l.is_scarlett).collect();
        if !pw_scarlett.is_empty() {
            info_lines.push(Line::from(""));
            info_lines.push(Line::from(Span::styled(
                format!(" Active PipeWire Links ({})", pw_scarlett.len()),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
            for l in pw_scarlett.iter().take(6) {
                info_lines.push(Line::from(Span::styled(
                    format!(
                        "  {} → {}",
                        pipewire::short_port(&l.output_port),
                        pipewire::short_port(&l.input_port)
                    ),
                    secondary(),
                )));
            }
        }

        let info_panel = Paragraph::new(info_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Status & Signal Path ")
                .border_style(decoration())
                .title_style(title_style()),
        );
        f.render_widget(info_panel, cols[1]);
    }

    fn render_io(&self, f: &mut Frame, area: Rect) {
        let Some(ref m) = self.model else { return };
        let items = self.io_items();
        if items.is_empty() {
            let p = Paragraph::new("No I/O or device controls exposed.").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" I/O & Device "),
            );
            f.render_widget(p, area);
            return;
        }

        let cursor = self.io_cursor.min(items.len() - 1);
        let visible_rows = area.height.saturating_sub(4).max(1) as usize;
        let row_start = window_start(cursor, items.len(), visible_rows);
        let row_end = (row_start + visible_rows).min(items.len());

        let rows: Vec<Row> = items[row_start..row_end]
            .iter()
            .enumerate()
            .map(|(local_i, item)| {
                let i = row_start + local_i;
                let c = &m.controls[&item.numid];
                let (mut val_str, val_style) = match c.kind {
                    ControlKind::Bool => match c.opt_bool_val() {
                        Some(true) => (
                            "ON".into(),
                            Style::default().fg(
                                if item.label.contains("Phantom") || item.label.contains("Mute") {
                                    Color::Red
                                } else {
                                    Color::Green
                                },
                            ),
                        ),
                        Some(false) => ("OFF".into(), secondary()),
                        None => ("?".into(), Style::default().fg(Color::Yellow)),
                    },
                    ControlKind::Enum => {
                        let lbl = c.enum_label().to_string();
                        let col = if !c.value_valid || c.val() > 0 {
                            Color::Yellow
                        } else {
                            Color::Reset
                        };
                        (lbl, Style::default().fg(col))
                    }
                    ControlKind::Int => match c.opt_val() {
                        Some(v) => {
                            let frac = if c.max > c.min {
                                ((v - c.min) as f64 / (c.max - c.min) as f64).clamp(0.0, 1.0)
                            } else {
                                0.0
                            };
                            let bars = (frac * 18.0).round() as usize;
                            let meter = format!("{}{}", "█".repeat(bars), "░".repeat(18 - bars));
                            let txt = c
                                .current_db()
                                .map(|db| format!("{meter} {db:+6.1} dB"))
                                .unwrap_or_else(|| format!("{meter} {v}"));
                            (txt, Style::default().fg(Color::Cyan))
                        }
                        None => ("?".into(), Style::default().fg(Color::Yellow)),
                    },
                };
                if !c.is_editable() {
                    val_str.push_str(" [RO]");
                }
                let style = if i == cursor {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from(item.section).style(secondary()),
                    Cell::from(item.label.clone()),
                    Cell::from(val_str).style(val_style),
                ])
                .style(style)
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(15),
                Constraint::Length(34),
                Constraint::Min(24),
            ],
        )
        .header(
            Row::new(["Section", "Control", "Value"])
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Inputs, Outputs & Device Switches (Space toggle · h/l adjust · 0 unity · m min) ")
                .border_style(decoration())
                .title_style(title_style()),
        );
        f.render_widget(table, area);
    }

    fn render_routing(&self, f: &mut Frame, area: Rect) {
        let Some(ref m) = self.model else { return };
        if m.routes.is_empty() {
            let p = Paragraph::new("This Scarlett model does not expose a hardware routing matrix (use Tab 2 for Direct Monitor).")
                .block(Block::default().borders(Borders::ALL).title(" Routing "));
            f.render_widget(p, area);
            return;
        }

        let cursor = self.route_cursor.min(m.routes.len() - 1);
        let visible_rows = area.height.saturating_sub(4).max(1) as usize;
        let row_start = window_start(cursor, m.routes.len(), visible_rows);
        let row_end = (row_start + visible_rows).min(m.routes.len());
        let shown_routes = &m.routes[row_start..row_end];

        let rows: Vec<Row> = shown_routes
            .iter()
            .enumerate()
            .map(|(local_i, r)| {
                let i = row_start + local_i;
                let style = if i == cursor {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                let src_style = if r.current == "Off" {
                    secondary()
                } else if r.current.starts_with("Mix ") {
                    Style::default().fg(Color::Yellow)
                } else if r.current.starts_with("PCM ") {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Cyan)
                };
                Row::new(vec![
                    Cell::from(r.group.title()).style(secondary()),
                    Cell::from(r.label.clone()),
                    Cell::from(r.current.clone()).style(src_style.add_modifier(Modifier::BOLD)),
                    Cell::from(r.summary.clone()),
                ])
                .style(style)
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(18),
                Constraint::Length(24),
                Constraint::Length(14),
                Constraint::Min(24),
            ],
        )
        .header(
            Row::new(["Group", "Destination", "Source", "Resolved Signal"]).style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Hardware Routing Matrix (Enter pick source · h/l cycle · x Off) ")
                .border_style(decoration())
                .title_style(title_style()),
        );
        f.render_widget(table, area);
    }

    fn render_matrix(&self, f: &mut Frame, area: Rect) {
        let Some(ref m) = self.model else { return };
        if m.matrix.is_empty() {
            let p = Paragraph::new("This Scarlett model has no internal Mix A–F matrix.")
                .block(Block::default().borders(Borders::ALL).title(" Mix Matrix "));
            f.render_widget(p, area);
            return;
        }

        let cur_row = self.matrix_row.min(m.matrix.inputs.len() - 1);
        let cur_col = self.matrix_col.min(m.matrix.mixes.len() - 1);
        let visible_rows = area.height.saturating_sub(4).max(1) as usize;
        let visible_cols = (area.width.saturating_sub(24).max(9) as usize / 9).max(1);
        let row_start = window_start(cur_row, m.matrix.inputs.len(), visible_rows);
        let col_start = window_start(cur_col, m.matrix.mixes.len(), visible_cols);
        let row_end = (row_start + visible_rows).min(m.matrix.inputs.len());
        let col_end = (col_start + visible_cols).min(m.matrix.mixes.len());
        let shown_mixes = &m.matrix.mixes[col_start..col_end];
        let shown_inputs = &m.matrix.inputs[row_start..row_end];

        let mut header_cells = vec![
            Cell::from("Mixer Input [Source]").style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        for &mix in shown_mixes {
            let used = m
                .routes
                .iter()
                .any(|r| r.group == RouteGroup::AnalogueOut && r.current == format!("Mix {mix}"));
            let label = if used {
                format!("Mix {mix}*")
            } else {
                format!("Mix {mix}")
            };
            header_cells.push(
                Cell::from(label).style(
                    Style::default()
                        .fg(if used { Color::Yellow } else { Color::Cyan })
                        .add_modifier(Modifier::BOLD),
                ),
            );
        }

        let rows: Vec<Row> = shown_inputs
            .iter()
            .enumerate()
            .map(|(local_r, &in_idx)| {
                let r_idx = row_start + local_r;
                let src = m.mixer_input_source(in_idx);
                let row_label = format!("In {in_idx:02} [{src}]");
                let mut cells = vec![Cell::from(row_label).style(if src == "Off" {
                    secondary()
                } else {
                    Style::default()
                })];

                for (local_c, &mix) in shown_mixes.iter().enumerate() {
                    let c_idx = col_start + local_c;
                    let selected = r_idx == cur_row && c_idx == cur_col;
                    let (txt, mut style) = if let Some(&cid) = m.matrix.cells.get(&(mix, in_idx))
                        && let Some(c) = m.controls.get(&cid)
                    {
                        match c.opt_val() {
                            None => ("  ?   ".into(), Style::default().fg(Color::Yellow)),
                            Some(v) if v <= c.min => ("  -∞  ".into(), secondary()),
                            Some(v) => {
                                if let Some(db) = c.current_db() {
                                    let col = if db > 0.25 {
                                        Color::Red
                                    } else if db.abs() < 0.25 {
                                        Color::Green
                                    } else {
                                        Color::Yellow
                                    };
                                    (format!("{db:+5.1}"), Style::default().fg(col))
                                } else {
                                    (format!("{v:5}"), Style::default())
                                }
                            }
                        }
                    } else {
                        ("  —   ".into(), secondary())
                    };
                    if selected {
                        style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
                    }
                    cells.push(Cell::from(txt).style(style));
                }
                Row::new(cells)
            })
            .collect();

        let mut widths = vec![Constraint::Length(22)];
        for _ in shown_mixes {
            widths.push(Constraint::Length(9));
        }

        let table = Table::new(rows, widths)
            .header(Row::new(header_cells))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Mix Matrix (dB · * = routed to output · +/- ±1dB · {/} ±6dB · 0 unity · m/Space mute) ")
                    .border_style(decoration())
                .title_style(title_style()),
            );
        f.render_widget(table, area);
    }

    fn render_pipewire(&self, f: &mut Frame, area: Rect) {
        let links = self.visible_pw_links();
        let title = if self.pw_show_all {
            " PipeWire Links — ALL (a: Scarlett only · d: disconnect · r: refresh) "
        } else {
            " PipeWire Links — Scarlett (a: show all · d: disconnect · r: refresh) "
        };

        if links.is_empty() {
            let p =
                Paragraph::new("No matching PipeWire links. Press 'a' to show all graph links.")
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(title)
                            .border_style(decoration())
                            .title_style(title_style()),
                    );
            f.render_widget(p, area);
            return;
        }

        let cursor = self.pw_cursor.min(links.len() - 1);
        let visible_rows = area.height.saturating_sub(4).max(1) as usize;
        let row_start = window_start(cursor, links.len(), visible_rows);
        let row_end = (row_start + visible_rows).min(links.len());

        let rows: Vec<Row> = links[row_start..row_end]
            .iter()
            .enumerate()
            .map(|(local_i, l)| {
                let i = row_start + local_i;
                let style = if i == cursor {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                let is_loopback = l.output_port.contains("alsa_input.")
                    && l.input_port.contains("alsa_output.")
                    && l.is_scarlett;
                let tag = if is_loopback {
                    "loopback"
                } else if l.is_scarlett {
                    "focusrite"
                } else {
                    "other"
                };
                Row::new(vec![
                    Cell::from(format!("#{}", l.id)).style(secondary()),
                    Cell::from(tag).style(Style::default().fg(if is_loopback {
                        Color::Yellow
                    } else {
                        Color::Cyan
                    })),
                    Cell::from(pipewire::short_port(&l.output_port)),
                    Cell::from("→").style(secondary()),
                    Cell::from(pipewire::short_port(&l.input_port)),
                ])
                .style(style)
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(7),
                Constraint::Length(10),
                Constraint::Percentage(42),
                Constraint::Length(3),
                Constraint::Percentage(42),
            ],
        )
        .header(
            Row::new(["ID", "Type", "Output Port", "", "Input Port"]).style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(decoration())
                .title_style(title_style()),
        );
        f.render_widget(table, area);
    }

    fn render_route_picker(&self, f: &mut Frame, area: Rect, picker: &RoutePicker) {
        let popup_w = 38.min(area.width.saturating_sub(4));
        let popup_h = (picker.items.len() as u16 + 4).min(area.height.saturating_sub(4));
        let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
        let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
        let rect = Rect::new(x, y, popup_w, popup_h);

        f.render_widget(Clear, rect);
        let items: Vec<ListItem> = picker
            .items
            .iter()
            .map(|s| ListItem::new(s.as_str()))
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" Route: {} ", picker.title))
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD))
            .highlight_symbol("▸ ");
        let mut state = ListState::default();
        if !picker.items.is_empty() {
            state.select(Some(picker.selected.min(picker.items.len() - 1)));
        }
        f.render_stateful_widget(list, rect, &mut state);
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let msg =
            if !self.status_msg.is_empty() && self.status_at.elapsed() < Duration::from_secs(5) {
                Span::styled(
                    format!(" {} ", self.status_msg),
                    Style::default().fg(Color::Yellow),
                )
            } else {
                let multi = if self.cards.len() > 1 && self.card_override.is_none() {
                    " · c switch card"
                } else {
                    ""
                };
                Span::styled(
                    format!(
                        " 1-5 / Tab switch view · hjkl/arrows navigate · r refresh{multi} · q quit "
                    ),
                    secondary(),
                )
            };
        f.render_widget(Paragraph::new(Line::from(msg)), area);
    }
}

/// Secondary text: default foreground, dimmed. Never DarkGray, which Omarchy
/// maps to the theme's `muted` at 1.9:1 contrast (docs/DESIGN.md rule 1).
fn secondary() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// Decoration only: borders, the empty meter track and `·` (docs/DESIGN.md rule 2).
/// Never use this for words a user must read.
fn decoration() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// Block titles are readable text: default foreground, so they do not inherit
/// the DarkGray border colour.
fn title_style() -> Style {
    Style::default().fg(Color::Reset)
}

fn window_start(cursor: usize, total: usize, visible: usize) -> usize {
    if total == 0 || visible == 0 {
        return 0;
    }
    let clamped = cursor.min(total - 1);
    let start = if clamped >= visible {
        clamped + 1 - visible
    } else {
        0
    };
    start.min(total.saturating_sub(1))
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

pub fn run_tui(card_override: Option<u32>) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    if let Err(e) = execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(e);
    }
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(card_override);

    while !app.should_quit {
        app.tick();
        terminal.draw(|f| app.render(f))?;
        if event::poll(Duration::from_millis(50))?
            && let Event::Key(k) = event::read()?
            && k.kind == KeyEventKind::Press
        {
            app.handle_key(k.code, k.modifiers);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::model::fixtures::model_4i4;
    use ratatui::{backend::TestBackend, buffer::Buffer};

    /// Render one frame of `app` into a `w`×`h` buffer.
    pub(super) fn render_to_buffer(app: &App, w: u16, h: u16) -> Buffer {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("test backend");
        term.draw(|f| app.render(f)).expect("draw");
        term.backend().buffer().clone()
    }

    /// Glyphs DarkGray may draw: box-drawing borders, the empty meter track and `·`.
    fn is_decoration(sym: &str) -> bool {
        sym.chars()
            .all(|c| c == ' ' || c == '░' || c == '·' || ('\u{2500}'..='\u{257F}').contains(&c))
    }

    #[test]
    fn darkgray_never_draws_readable_text() {
        let sizes = [(95, 30), (160, 36), (80, 24)];
        for model in [Some(model_4i4()), None] {
            let mut app = App::for_test(model);
            app.status_msg.clear();
            for tab in Tab::ALL {
                app.tab = tab;
                for (w, h) in sizes {
                    let buf = render_to_buffer(&app, w, h);
                    for cell in buf.content() {
                        if cell.fg == Color::DarkGray {
                            assert!(
                                is_decoration(cell.symbol()),
                                "DarkGray text {:?} on tab {} at {w}x{h}",
                                cell.symbol(),
                                tab.title()
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn footer_hints_are_dim_default_foreground() {
        let app = App::for_test(Some(model_4i4()));
        let buf = render_to_buffer(&app, 95, 30);
        let y = 28;
        let row: String = (0..95).map(|x| buf[(x, y)].symbol().to_string()).collect();
        assert!(row.contains("q quit"), "footer row was {row:?}");
        let x = row.find('q').unwrap() as u16;
        let cell = &buf[(x, y)];
        assert_eq!(cell.fg, Color::Reset);
        assert!(cell.modifier.contains(Modifier::DIM));
    }

    #[test]
    fn window_start_never_exceeds_bounds_after_shrink() {
        assert_eq!(window_start(25, 4, 10), 0);
        assert_eq!(window_start(25, 0, 10), 0);
        assert_eq!(window_start(12, 20, 5), 8);
        assert_eq!(window_start(19, 20, 5), 15);
    }
}
