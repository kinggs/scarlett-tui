use crate::alsa_ctl::{
    AlsaCard, CardEntry, ControlKind, discover_focusrite_cards, read_stream_rate,
};
use crate::model::{RouteGroup, ScarlettModel, meter_to_dbfs};
use crate::pipewire::{self, PwLink};
use crossterm::{
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
    title: String,
    items: Vec<String>,
    selected: usize,
}

pub struct App {
    cards: Vec<CardEntry>,
    card_pos: usize,
    alsa: Option<AlsaCard>,
    model: Option<ScarlettModel>,
    tab: Tab,

    // Meters (raw + peak hold)
    meter_raw: Vec<i64>,
    meter_peak: Vec<f64>,
    meter_peak_time: Vec<Instant>,

    // Per-tab cursors
    io_cursor: usize,
    route_cursor: usize,
    route_picker: Option<RoutePicker>,
    matrix_row: usize,
    matrix_col: usize,
    pw_links: Vec<PwLink>,
    pw_show_all: bool,
    pw_cursor: usize,

    status_msg: String,
    status_at: Instant,
    last_rate_check: Instant,
    should_quit: bool,
}

impl App {
    fn new(card_override: Option<u32>) -> Self {
        let cards = discover_focusrite_cards();
        let card_pos = card_override
            .and_then(|idx| cards.iter().position(|c| c.index == idx))
            .unwrap_or(0);
        let mut app = Self {
            cards,
            card_pos,
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
            pw_links: Vec::new(),
            pw_show_all: false,
            pw_cursor: 0,
            status_msg: String::new(),
            status_at: Instant::now(),
            last_rate_check: Instant::now(),
            should_quit: false,
        };
        app.connect_current();
        app.refresh_pipewire();
        app
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = msg.into();
        self.status_at = Instant::now();
    }

    fn connect_current(&mut self) {
        self.alsa = None;
        self.model = None;
        self.meter_raw.clear();
        self.meter_peak.clear();
        self.meter_peak_time.clear();

        let Some(entry) = self.cards.get(self.card_pos).cloned() else {
            self.set_status("No Focusrite Scarlett USB interface found");
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
                    self.alsa = Some(alsa);
                    self.model = Some(model);
                }
                Err(e) => self.set_status(format!("ALSA enumerate error: {e}")),
            },
            Err(e) => self.set_status(format!("ALSA open error: {e}")),
        }
    }

    fn refresh_all(&mut self) {
        self.cards = discover_focusrite_cards();
        if self.card_pos >= self.cards.len() {
            self.card_pos = 0;
        }
        self.connect_current();
        self.refresh_pipewire();
        self.set_status("Refreshed ALSA & PipeWire state");
    }

    fn refresh_pipewire(&mut self) {
        match pipewire::list_links() {
            Ok(links) => {
                self.pw_links = links;
                let count = self.visible_pw_links().len();
                if self.pw_cursor >= count && count > 0 {
                    self.pw_cursor = count - 1;
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

    fn write_control(&mut self, numid: u32, new_val: i64) {
        let Some(ref alsa) = self.alsa else { return };
        let Some(ref mut model) = self.model else {
            return;
        };
        let Some(ctrl) = model.controls.get(&numid).cloned() else {
            return;
        };
        match alsa.write_single(&ctrl, new_val) {
            Ok(()) => {
                if let Ok(vals) = alsa.read_values(numid, ctrl.kind, ctrl.count)
                    && let Some(c) = model.controls.get_mut(&numid)
                {
                    c.values = vals;
                }
                model.recompute_derived();
            }
            Err(e) => self.set_status(format!("Write failed: {e}")),
        }
    }

    fn tick(&mut self) {
        // Drain hardware events (physical button presses / alsa-scarlett-gui / plugin changes)
        let mut disconnected = false;
        if let (Some(alsa), Some(model)) = (&self.alsa, &mut self.model) {
            if !alsa.exists() {
                disconnected = true;
            } else if let Ok((changed, structural)) = alsa.drain_events() {
                let mid = model.meter_numid;
                let non_meter: Vec<u32> =
                    changed.into_iter().filter(|&id| Some(id) != mid).collect();
                if structural {
                    if let Ok(ctrls) = alsa.enumerate() {
                        let stream = read_stream_rate(model.card.index);
                        *model = ScarlettModel::build(model.card.clone(), stream, ctrls);
                    }
                } else if !non_meter.is_empty() {
                    for id in non_meter {
                        if let Some(c) = model.controls.get_mut(&id)
                            && let Ok(vals) = alsa.read_values(id, c.kind, c.count)
                        {
                            c.values = vals;
                        }
                    }
                    model.recompute_derived();
                }
            }
        }
        if disconnected {
            self.alsa = None;
            self.model = None;
            self.set_status("Scarlett disconnected — press r to rescan");
            return;
        }

        // Poll live 12-bit hardware meters on Overview tab
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
                let db = meter_to_dbfs(v);
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
                    let numid = picker.numid;
                    let sel = picker.selected as i64;
                    let label = picker
                        .items
                        .get(picker.selected)
                        .cloned()
                        .unwrap_or_default();
                    let title = picker.title.clone();
                    self.write_control(numid, sel);
                    self.set_status(format!("{title} → {label}"));
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
            KeyCode::Char('c') if self.cards.len() > 1 => {
                self.card_pos = (self.card_pos + 1) % self.cards.len();
                self.connect_current();
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
            KeyCode::Enter | KeyCode::Char(' ') => match ctrl.kind {
                ControlKind::Bool => {
                    self.write_control(ctrl.numid, if ctrl.bool_val() { 0 } else { 1 })
                }
                ControlKind::Enum => {
                    let next = if ctrl.items.is_empty() {
                        0
                    } else {
                        (ctrl.val() + 1) % ctrl.items.len() as i64
                    };
                    self.write_control(ctrl.numid, next);
                }
                ControlKind::Int => {
                    let unity = self
                        .model
                        .as_ref()
                        .map(|m| m.unity_raw_for(ctrl.numid))
                        .unwrap_or(ctrl.max);
                    self.write_control(ctrl.numid, unity);
                }
            },
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('=') => {
                match ctrl.kind {
                    ControlKind::Bool => self.write_control(ctrl.numid, 1),
                    ControlKind::Enum => {
                        let next = (ctrl.val() + 1).min(ctrl.max);
                        self.write_control(ctrl.numid, next);
                    }
                    ControlKind::Int => {
                        let step = ((ctrl.max - ctrl.min) / 64).max(1);
                        self.write_control(ctrl.numid, (ctrl.val() + step).min(ctrl.max));
                    }
                }
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') => match ctrl.kind {
                ControlKind::Bool => self.write_control(ctrl.numid, 0),
                ControlKind::Enum => {
                    let prev = (ctrl.val() - 1).max(ctrl.min);
                    self.write_control(ctrl.numid, prev);
                }
                ControlKind::Int => {
                    let step = ((ctrl.max - ctrl.min) / 64).max(1);
                    self.write_control(ctrl.numid, (ctrl.val() - step).max(ctrl.min));
                }
            },
            KeyCode::Char('0') if ctrl.kind == ControlKind::Int => {
                let unity = self
                    .model
                    .as_ref()
                    .map(|m| m.unity_raw_for(ctrl.numid))
                    .unwrap_or(ctrl.max);
                self.write_control(ctrl.numid, unity);
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
                self.route_picker = Some(RoutePicker {
                    numid,
                    title,
                    items: ctrl.items.clone(),
                    selected: ctrl.val().max(0) as usize,
                });
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if !ctrl.items.is_empty() {
                    let next = (ctrl.val() + 1) % ctrl.items.len() as i64;
                    self.write_control(numid, next);
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if !ctrl.items.is_empty() {
                    let len = ctrl.items.len() as i64;
                    let prev = (ctrl.val() + len - 1) % len;
                    self.write_control(numid, prev);
                }
            }
            KeyCode::Char('x') | KeyCode::Char('0') => {
                // Set to "Off" (index 0)
                self.write_control(numid, 0);
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
                if let Some(id) = cell_id
                    && let Some(c) = model.controls.get(&id)
                {
                    let v = (c.val() + 2).min(c.max); // +1.0 dB (0.5 dB steps)
                    self.write_control(id, v);
                }
            }
            KeyCode::Char('-') | KeyCode::Char('[') => {
                if let Some(id) = cell_id
                    && let Some(c) = model.controls.get(&id)
                {
                    let v = (c.val() - 2).max(c.min); // -1.0 dB
                    self.write_control(id, v);
                }
            }
            KeyCode::Char('}') => {
                if let Some(id) = cell_id
                    && let Some(c) = model.controls.get(&id)
                {
                    self.write_control(id, (c.val() + 12).min(c.max)); // +6 dB
                }
            }
            KeyCode::Char('{') => {
                if let Some(id) = cell_id
                    && let Some(c) = model.controls.get(&id)
                {
                    self.write_control(id, (c.val() - 12).max(c.min)); // -6 dB
                }
            }
            KeyCode::Char('0') => {
                if let Some(id) = cell_id {
                    let unity = model.unity_raw_for(id);
                    self.write_control(id, unity);
                    self.set_status(format!("Mix {mix} In {input:02} → 0.0 dB"));
                }
            }
            KeyCode::Char('m') | KeyCode::Char('x') => {
                if let Some(id) = cell_id
                    && let Some(c) = model.controls.get(&id)
                {
                    let target = if c.val() > c.min {
                        c.min
                    } else {
                        model.unity_raw_for(id)
                    };
                    self.write_control(id, target);
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(id) = cell_id
                    && let Some(c) = model.controls.get(&id)
                {
                    let target = if c.val() > c.min {
                        c.min
                    } else {
                        model.unity_raw_for(id)
                    };
                    self.write_control(id, target);
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
            let sync_color = if m.is_locked() {
                Color::Green
            } else {
                Color::Yellow
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
                Span::styled(
                    format!("USB {}  ", m.card.usb_id),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(format!("{fw}  "), Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("● {}  ", m.sync_status),
                    Style::default().fg(sync_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{rate_str}  "), Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!("PW links: {pw_count}"),
                    Style::default().fg(if pw_count > 0 {
                        Color::Yellow
                    } else {
                        Color::DarkGray
                    }),
                ),
            ])
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
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        f.render_widget(header, area);
    }

    fn render_tabs(&self, f: &mut Frame, area: Rect) {
        let titles: Vec<Line> = Tab::ALL.iter().map(|t| Line::from(t.title())).collect();
        let tabs = Tabs::new(titles)
            .select(self.tab as usize)
            .style(Style::default().fg(Color::DarkGray))
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
            let db = meter_to_dbfs(raw);
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
            let label = truncate(&format!("{} [{}]", mc.label, mc.source), 22);

            meter_lines.push(Line::from(vec![
                Span::raw(format!(" {label:<22} ")),
                Span::styled("█".repeat(filled), Style::default().fg(bar_color)),
                Span::styled("░".repeat(empty), Style::default().fg(Color::DarkGray)),
                Span::raw(format!(" {db_text}")),
                Span::styled(peak_tag, Style::default().fg(Color::DarkGray)),
            ]));
        }

        if meter_lines.is_empty() {
            meter_lines.push(Line::from(Span::styled(
                " No hardware level meter exposed by this model.",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let meter_panel = Paragraph::new(meter_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Live Hardware Meters (12-bit ALSA) ")
                .border_style(Style::default().fg(Color::DarkGray)),
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
                    Style::default().fg(if lbl == "Inst" {
                        Color::Yellow
                    } else {
                        Color::DarkGray
                    }),
                ));
            }
            if let Some(id) = inp.air_numid
                && let Some(c) = m.controls.get(&id)
            {
                let on = c.val() > 0;
                let txt = if c.kind == ControlKind::Enum && on {
                    format!("[Air:{}]", c.enum_label())
                } else {
                    "[Air]".into()
                };
                badges.push(Span::raw(" "));
                badges.push(Span::styled(
                    txt,
                    Style::default().fg(if on { Color::Yellow } else { Color::DarkGray }),
                ));
            }
            if let Some(id) = inp.pad_numid
                && let Some(c) = m.controls.get(&id)
            {
                badges.push(Span::raw(" "));
                badges.push(Span::styled(
                    "[Pad]",
                    Style::default().fg(if c.bool_val() {
                        Color::Cyan
                    } else {
                        Color::DarkGray
                    }),
                ));
            }
            if let Some(id) = inp.gain_numid
                && let Some(c) = m.controls.get(&id)
            {
                badges.push(Span::raw(format!(
                    " {:.0} dB",
                    c.current_db().unwrap_or(c.val() as f64)
                )));
            }
            let mut spans = vec![Span::raw(format!("  {:<8} ", inp.label))];
            spans.extend(badges);
            info_lines.push(Line::from(spans));
        }

        for &pid in &m.phantom_numids {
            if let Some(c) = m.controls.get(&pid) {
                let on = c.bool_val();
                let short = c.name.replace(" Capture Switch", "");
                info_lines.push(Line::from(vec![
                    Span::raw(format!("  {short}: ")),
                    Span::styled(
                        if on { "ON" } else { "OFF" },
                        Style::default()
                            .fg(if on { Color::Red } else { Color::DarkGray })
                            .add_modifier(if on {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    ),
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
                Span::styled(out_extra, Style::default().fg(Color::DarkGray)),
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
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }

        let info_panel = Paragraph::new(info_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Status & Signal Path ")
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        f.render_widget(info_panel, cols[1]);
    }

    fn render_io(&self, f: &mut Frame, area: Rect) {
        let Some(ref m) = self.model else { return };
        let items = self.io_items();
        let rows: Vec<Row> = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let c = &m.controls[&item.numid];
                let (val_str, val_color) = match c.kind {
                    ControlKind::Bool => {
                        if c.bool_val() {
                            (
                                "ON".into(),
                                if item.label.contains("Phantom") || item.label.contains("Mute") {
                                    Color::Red
                                } else {
                                    Color::Green
                                },
                            )
                        } else {
                            ("OFF".into(), Color::DarkGray)
                        }
                    }
                    ControlKind::Enum => {
                        let lbl = c.enum_label().to_string();
                        let col = if c.val() > 0 {
                            Color::Yellow
                        } else {
                            Color::Reset
                        };
                        (lbl, col)
                    }
                    ControlKind::Int => {
                        let frac = if c.max > c.min {
                            (c.val() - c.min) as f64 / (c.max - c.min) as f64
                        } else {
                            0.0
                        };
                        let bars = (frac * 18.0).round() as usize;
                        let meter = format!("{}{}", "█".repeat(bars), "░".repeat(18 - bars));
                        let txt = c
                            .current_db()
                            .map(|db| format!("{meter} {db:+6.1} dB"))
                            .unwrap_or_else(|| format!("{meter} {}", c.val()));
                        (txt, Color::Cyan)
                    }
                };
                let style = if i == self.io_cursor {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from(item.section).style(Style::default().fg(Color::DarkGray)),
                    Cell::from(item.label.clone()),
                    Cell::from(val_str).style(Style::default().fg(val_color)),
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
                .title(" Inputs, Outputs & Device Switches (Space/Enter toggle · h/l adjust · 0 unity · m min) ")
                .border_style(Style::default().fg(Color::DarkGray)),
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

        let visible_rows = area.height.saturating_sub(4).max(1) as usize;
        let row_start = if self.route_cursor >= visible_rows {
            self.route_cursor + 1 - visible_rows
        } else {
            0
        };
        let shown_routes = &m.routes[row_start..(row_start + visible_rows).min(m.routes.len())];

        let rows: Vec<Row> = shown_routes
            .iter()
            .enumerate()
            .map(|(local_i, r)| {
                let i = row_start + local_i;
                let style = if i == self.route_cursor {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                let src_color = if r.current == "Off" {
                    Color::DarkGray
                } else if r.current.starts_with("Mix ") {
                    Color::Yellow
                } else if r.current.starts_with("PCM ") {
                    Color::Green
                } else {
                    Color::Cyan
                };
                Row::new(vec![
                    Cell::from(r.group.title()).style(Style::default().fg(Color::DarkGray)),
                    Cell::from(r.label.clone()),
                    Cell::from(r.current.clone())
                        .style(Style::default().fg(src_color).add_modifier(Modifier::BOLD)),
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
                .border_style(Style::default().fg(Color::DarkGray)),
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

        let visible_rows = area.height.saturating_sub(4).max(1) as usize;
        let visible_cols = (area.width.saturating_sub(24).max(9) as usize / 9).max(1);
        let row_start = if self.matrix_row >= visible_rows {
            self.matrix_row + 1 - visible_rows
        } else {
            0
        };
        let col_start = if self.matrix_col >= visible_cols {
            self.matrix_col + 1 - visible_cols
        } else {
            0
        };
        let shown_mixes =
            &m.matrix.mixes[col_start..(col_start + visible_cols).min(m.matrix.mixes.len())];
        let shown_inputs =
            &m.matrix.inputs[row_start..(row_start + visible_rows).min(m.matrix.inputs.len())];

        let mut header_cells = vec![
            Cell::from("Mixer Input [Source]").style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        for &mix in shown_mixes {
            // Show if this Mix is actively feeding an Analogue Output
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
                let mut cells =
                    vec![
                        Cell::from(row_label).style(Style::default().fg(if src == "Off" {
                            Color::DarkGray
                        } else {
                            Color::Reset
                        })),
                    ];

                for (local_c, &mix) in shown_mixes.iter().enumerate() {
                    let c_idx = col_start + local_c;
                    let selected = r_idx == self.matrix_row && c_idx == self.matrix_col;
                    let (txt, mut style) = if let Some(&cid) = m.matrix.cells.get(&(mix, in_idx))
                        && let Some(c) = m.controls.get(&cid)
                    {
                        if c.val() <= c.min {
                            ("  -∞  ".into(), Style::default().fg(Color::DarkGray))
                        } else if let Some(db) = c.current_db() {
                            let col = if db > 0.25 {
                                Color::Red
                            } else if db.abs() < 0.25 {
                                Color::Green
                            } else {
                                Color::Yellow
                            };
                            (format!("{db:+5.1}"), Style::default().fg(col))
                        } else {
                            (format!("{:5}", c.val()), Style::default())
                        }
                    } else {
                        ("  —   ".into(), Style::default().fg(Color::DarkGray))
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
                    .border_style(Style::default().fg(Color::DarkGray)),
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
                            .border_style(Style::default().fg(Color::DarkGray)),
                    );
            f.render_widget(p, area);
            return;
        }

        let rows: Vec<Row> = links
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let style = if i == self.pw_cursor {
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
                    "scarlett"
                } else {
                    "other"
                };
                Row::new(vec![
                    Cell::from(format!("#{}", l.id)).style(Style::default().fg(Color::DarkGray)),
                    Cell::from(tag).style(Style::default().fg(if is_loopback {
                        Color::Yellow
                    } else {
                        Color::Cyan
                    })),
                    Cell::from(pipewire::short_port(&l.output_port)),
                    Cell::from("→").style(Style::default().fg(Color::DarkGray)),
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
                .border_style(Style::default().fg(Color::DarkGray)),
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
        state.select(Some(picker.selected));
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
                let multi = if self.cards.len() > 1 {
                    " · c switch card"
                } else {
                    ""
                };
                Span::styled(
                    format!(
                        " 1-5 / Tab switch view · hjkl/arrows navigate · r refresh{multi} · q quit "
                    ),
                    Style::default().fg(Color::DarkGray),
                )
            };
        f.render_widget(Paragraph::new(Line::from(msg)), area);
    }
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
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(card_override);

    let res = (|| -> io::Result<()> {
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
    })();

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    res
}
