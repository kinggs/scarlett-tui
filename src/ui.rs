use crate::alsa_ctl::{
    AlsaCard, AlsaControl, CardEntry, ControlKind, discover_focusrite_cards, read_stream_rate,
};
use crate::keys::{self, Action, Scope};
use crate::model::{
    CLIP_DBFS, ClipLatch, HOT_DBFS, RouteGroup, ScarlettModel, UnityJump, meter_to_dbfs_with_max,
    unity_jump,
};
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
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, Tabs,
    },
};
use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

    fn scope(self) -> Scope {
        match self {
            Tab::Overview => Scope::Overview,
            Tab::IoDevice => Scope::Io,
            Tab::Routing => Scope::Routing,
            Tab::Matrix => Scope::Matrix,
            Tab::PipeWire => Scope::PipeWire,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Tab::Overview => "1 Overview",
            Tab::IoDevice => "2 I/O",
            Tab::Routing => "3 Device routing",
            Tab::Matrix => "4 Matrix",
            Tab::PipeWire => "5 PipeWire graph",
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
    /// Item index under the cursor.
    selected: usize,
    /// Item index the hardware is set to now (marked ●).
    current: usize,
    filter: String,
    filtering: bool,
}

impl RoutePicker {
    /// Item indices matching the filter, grouped by source type, then in device order.
    fn visible(&self) -> Vec<usize> {
        let needle = self.filter.to_lowercase();
        let mut v: Vec<usize> = (0..self.items.len())
            .filter(|&i| needle.is_empty() || self.items[i].to_lowercase().contains(&needle))
            .collect();
        v.sort_by_key(|&i| (source_group(&self.items[i]).0, i));
        v
    }

    fn step(&mut self, delta: isize) {
        let v = self.visible();
        if v.is_empty() {
            return;
        }
        let pos = v.iter().position(|&i| i == self.selected);
        let next = match pos {
            Some(p) => (p as isize + delta).clamp(0, v.len() as isize - 1) as usize,
            None => 0,
        };
        self.selected = v[next];
    }

    /// Keep the cursor on a visible item after the filter changes.
    fn snap(&mut self) {
        let v = self.visible();
        if !v.contains(&self.selected)
            && let Some(&first) = v.first()
        {
            self.selected = first;
        }
    }
}

/// Source type of a routing source name: sort key and heading.
fn source_group(name: &str) -> (u8, &'static str) {
    if name.starts_with("Mix ") {
        (0, "Mixes")
    } else if name.starts_with("PCM ") {
        (1, "From computer (PCM)")
    } else if name.starts_with("Analogue") || name.starts_with("Line") {
        (2, "Analogue")
    } else if name.contains("S/PDIF") {
        (3, "S/PDIF")
    } else if name.starts_with("ADAT") {
        (4, "ADAT")
    } else if name == "Off" {
        (6, "Off")
    } else {
        (5, "Other")
    }
}

/// Fixed source colours (docs/DESIGN.md rule 5): cyan mix, green PCM, bold
/// analogue and other, dim off. The words carry the type too.
fn source_style(name: &str) -> Style {
    if name == "Off" {
        secondary()
    } else if name.starts_with("Mix ") {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if name.starts_with("PCM ") {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    }
}

/// A safety action waiting for its confirming key press (docs/DESIGN.md).
#[derive(Clone, Debug, PartialEq)]
enum ArmedAction {
    /// Jump a level to 0 dB; confirmed by pressing `0` again within 2 s.
    Unity {
        numid: u32,
        raw: i64,
        label: String,
        rise_db: Option<f64>,
    },
    /// Enable 48 V phantom power; confirmed by Space or Enter within 3 s.
    Phantom { numid: u32, label: String },
}

impl ArmedAction {
    fn window(&self) -> Duration {
        match self {
            ArmedAction::Unity { .. } => Duration::from_secs(2),
            ArmedAction::Phantom { .. } => Duration::from_secs(3),
        }
    }

    fn confirmed_by(&self, code: KeyCode) -> bool {
        match self {
            ArmedAction::Unity { .. } => code == KeyCode::Char('0'),
            // Only Space confirms 48V (docs/DESIGN.md "48V keys").
            ArmedAction::Phantom { .. } => code == KeyCode::Char(' '),
        }
    }

    fn prompt(&self) -> String {
        match self {
            ArmedAction::Unity {
                label,
                rise_db: Some(rise),
                ..
            } => format!("Press 0 again: +{rise:.0} dB to 0.0 dB on {label}"),
            ArmedAction::Unity { label, .. } => {
                format!("Press 0 again: −∞ to 0.0 dB on {label}")
            }
            ArmedAction::Phantom { label, .. } => {
                format!("48V for {label}. Check connected mics, then Space to enable.")
            }
        }
    }

    fn confirm_hint(&self) -> &'static str {
        match self {
            ArmedAction::Unity { .. } => " 0 confirm · Esc cancel",
            ArmedAction::Phantom { .. } => " Space confirm · Esc cancel",
        }
    }
}

struct Armed {
    action: ArmedAction,
    at: Instant,
    generation: u64,
}

/// How the TUI draws (docs/DESIGN.md "Colour-free modes").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderOpts {
    /// Drop every colour; keep bold, dim, reversed, glyphs and words.
    pub no_color: bool,
    /// Replace Unicode glyphs and box drawing with ASCII.
    pub ascii: bool,
}

impl RenderOpts {
    /// Colour is off for `--no-color`, a non-empty `NO_COLOR` (no-color.org) or `TERM=dumb`.
    pub fn from_env(
        no_color_flag: bool,
        ascii: bool,
        no_color_env: Option<&str>,
        term: Option<&str>,
    ) -> Self {
        let no_color =
            no_color_flag || no_color_env.is_some_and(|v| !v.is_empty()) || term == Some("dumb");
        Self { no_color, ascii }
    }
}

/// ASCII stand-in for one drawn glyph; `None` keeps the symbol.
fn ascii_for(sym: &str) -> Option<char> {
    let mut chars = sym.chars();
    let c = chars.next()?;
    if chars.next().is_none() && c.is_ascii() {
        return None;
    }
    Some(match c {
        '█' => '#',
        '░' => '-',
        '│' | '┃' | '║' | '▏' => '|',
        '─' | '━' | '═' => '-',
        '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼' | '╭' | '╮' | '╯' | '╰' => {
            '+'
        }
        '●' => '*',
        '○' => 'o',
        '■' => 'x',
        '□' => '.',
        '▲' => '!',
        '✕' | '×' => 'x',
        '⊘' => '/',
        '·' | '…' => '.',
        '⟲' | '∞' => '~',
        '◂' | '←' => '<',
        '▸' | '→' => '>',
        '↓' => 'v',
        '↑' => '^',
        '−' | '–' | '—' => '-',
        '±' => '+',
        _ => '?',
    })
}

/// Apply colour-free and ASCII modes to a finished frame.
fn apply_render_opts(buf: &mut ratatui::buffer::Buffer, opts: RenderOpts) {
    if !opts.no_color && !opts.ascii {
        return;
    }
    for cell in buf.content.iter_mut() {
        if opts.no_color {
            cell.fg = Color::Reset;
            cell.bg = Color::Reset;
        }
        if opts.ascii
            && let Some(c) = ascii_for(cell.symbol())
        {
            cell.set_char(c);
        }
    }
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
    clips: ClipLatch,

    // Per-tab cursors & remembered matrix levels for safe unmute
    io_cursor: usize,
    route_cursor: usize,
    route_picker: Option<RoutePicker>,
    armed: Option<Armed>,
    show_help: bool,
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
    opts: RenderOpts,
}

impl App {
    fn new(card_override: Option<u32>, opts: RenderOpts) -> Self {
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
            clips: ClipLatch::default(),
            io_cursor: 0,
            route_cursor: 0,
            route_picker: None,
            armed: None,
            show_help: false,
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
            opts,
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
            clips: ClipLatch::default(),
            io_cursor: 0,
            route_cursor: 0,
            route_picker: None,
            armed: None,
            show_help: false,
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
            opts: RenderOpts::default(),
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
        self.armed = None;
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
        self.clips = ClipLatch::default();
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

    /// Take one frame of raw meter values: update peak hold and clip latches.
    fn ingest_meters(&mut self, raw: Vec<i64>, now: Instant) {
        let max = self.model.as_ref().map_or(4095, |m| m.meter_max);
        if self.meter_peak.len() != raw.len() {
            self.meter_peak = vec![-96.0; raw.len()];
            self.meter_peak_time = vec![now; raw.len()];
        }
        let dbfs: Vec<f64> = raw
            .iter()
            .map(|&v| meter_to_dbfs_with_max(v, max))
            .collect();
        for (i, &db) in dbfs.iter().enumerate() {
            if db >= self.meter_peak[i] || now.duration_since(self.meter_peak_time[i]) > PEAK_HOLD {
                self.meter_peak[i] = db;
                self.meter_peak_time[i] = now;
            }
        }
        self.clips.update(&dbfs);
        self.meter_raw = raw;
    }

    fn arm(&mut self, action: ArmedAction) {
        self.armed = Some(Armed {
            action,
            at: Instant::now(),
            generation: self.generation,
        });
    }

    fn fire_armed(&mut self, action: ArmedAction) {
        match action {
            ArmedAction::Unity {
                numid, raw, label, ..
            } => {
                if self.write_control(numid, raw) {
                    self.set_status(format!("{label} → 0.0 dB"));
                }
            }
            ArmedAction::Phantom { numid, label } => {
                if self.write_control(numid, 1) {
                    self.set_status(format!("48V on: {label}"));
                }
            }
        }
    }

    /// Drop an armed action whose confirm window has passed.
    fn expire_armed(&mut self, now: Instant) {
        let expired = self
            .armed
            .as_ref()
            .filter(|a| now.duration_since(a.at) >= a.action.window())
            .map(|a| match a.action {
                ArmedAction::Phantom { .. } => "48V not enabled (timed out)",
                ArmedAction::Unity { .. } => "Unity not applied (timed out)",
            });
        if let Some(msg) = expired {
            self.armed = None;
            self.set_status(msg);
        }
    }

    /// Set `ctrl` to `unity`, or arm a confirm when that is a rise of more than +6 dB.
    fn unity_or_arm(&mut self, ctrl: &AlsaControl, unity: i64, label: String) {
        let cur = ctrl.opt_val();
        let jump = if cur.is_some_and(|v| v >= unity) {
            UnityJump::Instant
        } else {
            unity_jump(ctrl.current_db(), cur.is_some_and(|v| v <= ctrl.min))
        };
        match jump {
            UnityJump::Instant => {
                if self.write_control(ctrl.numid, unity) {
                    self.set_status(format!("{label} → 0.0 dB"));
                }
            }
            UnityJump::Confirm { rise_db } => self.arm(ArmedAction::Unity {
                numid: ctrl.numid,
                raw: unity,
                label,
                rise_db,
            }),
        }
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
        self.expire_armed(Instant::now());

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
            self.ingest_meters(raw, Instant::now());
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

        // An armed safety action consumes its confirm key; Esc cancels it, and any
        // other key cancels it and is then handled normally.
        if let Some(armed) = self.armed.take() {
            let live =
                armed.generation == self.generation && armed.at.elapsed() < armed.action.window();
            if live && armed.action.confirmed_by(code) {
                self.fire_armed(armed.action);
                return;
            }
            if code == KeyCode::Esc {
                self.set_status("Cancelled");
                return;
            }
        }

        if self.show_help {
            if keys::lookup(Scope::Help, code) == Some(Action::Cancel) {
                self.show_help = false;
            }
            return;
        }

        // Route picker modal intercepts keys
        if let Some(mut picker) = self.route_picker.take() {
            // While filtering, typed characters edit the filter; Enter and the
            // arrow keys still go through the keymap.
            if picker.filtering {
                match code {
                    KeyCode::Char(c) => {
                        picker.filter.push(c);
                        picker.snap();
                        self.route_picker = Some(picker);
                        return;
                    }
                    KeyCode::Backspace => {
                        picker.filter.pop();
                        picker.snap();
                        self.route_picker = Some(picker);
                        return;
                    }
                    KeyCode::Esc => {
                        picker.filtering = false;
                        picker.filter.clear();
                        self.route_picker = Some(picker);
                        return;
                    }
                    _ => {}
                }
            }
            match keys::lookup(Scope::Picker, code) {
                Some(Action::Cancel) => {}
                Some(Action::Filter) => {
                    picker.filtering = true;
                    self.route_picker = Some(picker);
                }
                Some(Action::Up) => {
                    picker.step(-1);
                    self.route_picker = Some(picker);
                }
                Some(Action::Down) => {
                    picker.step(1);
                    self.route_picker = Some(picker);
                }
                Some(Action::Confirm) if !picker.visible().contains(&picker.selected) => {
                    self.set_status("No source matches the filter");
                    self.route_picker = Some(picker);
                }
                Some(Action::Confirm) => {
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

        let Some(action) = keys::lookup(self.tab.scope(), code) else {
            return;
        };
        match action {
            Action::Quit => {
                self.should_quit = true;
                return;
            }
            Action::GoTab(n) => {
                self.go_tab(Tab::ALL[n.min(Tab::ALL.len() - 1)]);
                return;
            }
            Action::NextTab => {
                self.go_tab(Tab::ALL[(self.tab as usize + 1) % Tab::ALL.len()]);
                return;
            }
            Action::PrevTab => {
                let prev = (self.tab as usize + Tab::ALL.len() - 1) % Tab::ALL.len();
                self.go_tab(Tab::ALL[prev]);
                return;
            }
            Action::Rescan => {
                self.refresh_all();
                return;
            }
            Action::Help => {
                self.show_help = true;
                return;
            }
            Action::ClearClips => {
                self.clips.clear();
                self.set_status("Clip latches cleared");
                return;
            }
            Action::SwitchCard if self.cards.len() > 1 && self.card_override.is_none() => {
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
            Tab::IoDevice => self.handle_io_key(action),
            Tab::Routing => self.handle_routing_key(action),
            Tab::Matrix => self.handle_matrix_key(action),
            Tab::PipeWire => self.handle_pw_key(action),
        }
    }

    fn go_tab(&mut self, tab: Tab) {
        self.tab = tab;
        if tab == Tab::PipeWire {
            self.refresh_pipewire();
        }
    }

    fn handle_io_key(&mut self, action: Action) {
        let step_db = if matches!(action, Action::IncBig | Action::DecBig) {
            6.0
        } else {
            1.0
        };
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
        let is_phantom = self
            .model
            .as_ref()
            .is_some_and(|m| m.is_phantom_switch(item.numid));

        match action {
            Action::Up => {
                self.io_cursor = self.io_cursor.saturating_sub(1);
            }
            Action::Down => {
                if self.io_cursor + 1 < items.len() {
                    self.io_cursor += 1;
                }
            }
            Action::Toggle => {
                if !ctrl.is_editable() {
                    self.set_status(format!("'{}' is read-only", ctrl.name));
                    return;
                }
                match ctrl.kind {
                    ControlKind::Bool => {
                        if let Some(b) = ctrl.opt_bool_val() {
                            if !b && is_phantom {
                                // 48V takes two steps to enable, one to disable.
                                self.arm(ArmedAction::Phantom {
                                    numid: ctrl.numid,
                                    label: item.label.clone(),
                                });
                            } else {
                                self.write_control(ctrl.numid, if b { 0 } else { 1 });
                            }
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
                        self.set_status("Use h/l (±1 dB), H/L (±6 dB), 0 for unity, m to mute");
                    }
                }
            }
            Action::Inc | Action::IncBig => {
                if !ctrl.is_editable() {
                    self.set_status(format!("'{}' is read-only", ctrl.name));
                    return;
                }
                match ctrl.kind {
                    ControlKind::Bool if is_phantom => {
                        if ctrl.opt_bool_val() == Some(false) {
                            self.arm(ArmedAction::Phantom {
                                numid: ctrl.numid,
                                label: item.label.clone(),
                            });
                        }
                    }
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
                            .and_then(|a| a.step_db(&ctrl, step_db))
                            .or_else(|| {
                                let cur = ctrl.opt_val()?;
                                let step = ((ctrl.max - ctrl.min) / 64).max(1) * step_db as i64;
                                Some((cur + step).min(ctrl.max))
                            });
                        if let Some(v) = next {
                            self.write_control(ctrl.numid, v);
                        }
                    }
                }
            }
            Action::Dec | Action::DecBig => {
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
                            .and_then(|a| a.step_db(&ctrl, -step_db))
                            .or_else(|| {
                                let cur = ctrl.opt_val()?;
                                let step = ((ctrl.max - ctrl.min) / 64).max(1) * step_db as i64;
                                Some((cur - step).max(ctrl.min))
                            });
                        if let Some(v) = next {
                            self.write_control(ctrl.numid, v);
                        }
                    }
                }
            }
            Action::Unity if ctrl.kind == ControlKind::Int => {
                if let Some(unity) = self
                    .model
                    .as_ref()
                    .and_then(|m| m.unity_raw_for(ctrl.numid))
                {
                    self.unity_or_arm(&ctrl, unity, item.label.clone());
                } else {
                    self.set_status(format!("No 0 dB reference for {}", item.label));
                }
            }
            Action::Mute if ctrl.kind == ControlKind::Int => {
                // On an output volume row, m toggles that output's mute switch.
                let mute = self.model.as_ref().and_then(|m| {
                    let out = m
                        .outputs
                        .iter()
                        .find(|o| o.volume_numid == Some(ctrl.numid))?;
                    let id = out.mute_numid?;
                    Some((id, m.controls.get(&id)?.opt_bool_val()?, out.label.clone()))
                });
                if let Some((id, muted, label)) = mute {
                    if self.write_control(id, if muted { 0 } else { 1 }) {
                        let word = if muted { "unmuted" } else { "muted" };
                        self.set_status(format!("{label} {word}"));
                    }
                } else {
                    self.write_control(ctrl.numid, ctrl.min);
                }
            }
            _ => {}
        }
    }

    fn handle_routing_key(&mut self, action: Action) {
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

        match action {
            Action::Up => {
                self.route_cursor = self.route_cursor.saturating_sub(1);
            }
            Action::Down => {
                if self.route_cursor + 1 < model.routes.len() {
                    self.route_cursor += 1;
                }
            }
            Action::Toggle => {
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
                    current: ctrl.val().max(0) as usize,
                    filter: String::new(),
                    filtering: false,
                });
            }
            Action::Inc => {
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
            Action::Dec => {
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
            Action::Off if self.write_control(numid, 0) => {
                self.set_status(format!("{title} → Off"));
            }
            _ => {}
        }
    }

    fn handle_matrix_key(&mut self, action: Action) {
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

        match action {
            Action::Up => self.matrix_row = self.matrix_row.saturating_sub(1),
            Action::Down => {
                if self.matrix_row + 1 < n_rows {
                    self.matrix_row += 1;
                }
            }
            Action::Left => self.matrix_col = self.matrix_col.saturating_sub(1),
            Action::Right => {
                if self.matrix_col + 1 < n_cols {
                    self.matrix_col += 1;
                }
            }
            Action::Inc => {
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
            Action::Dec => {
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
            Action::IncBig => {
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
            Action::DecBig => {
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
            Action::PageUp => self.matrix_row = self.matrix_row.saturating_sub(MATRIX_PAGE),
            Action::PageDown => self.matrix_row = (self.matrix_row + MATRIX_PAGE).min(n_rows - 1),
            Action::First => self.matrix_col = 0,
            Action::Last => self.matrix_col = n_cols - 1,
            Action::Unity => {
                if let Some(ref c) = cell_ctrl {
                    if let Some(unity) = unity_opt {
                        self.unity_or_arm(c, unity, format!("Mix {mix} In {input:02}"));
                    } else {
                        self.set_status(format!("No 0 dB reference for Mix {mix} In {input:02}"));
                    }
                }
            }
            Action::Mute => {
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
                        self.unity_or_arm(c, unity, format!("Mix {mix} In {input:02}"));
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

    fn handle_pw_key(&mut self, action: Action) {
        match action {
            Action::Up => {
                self.pw_cursor = self.pw_cursor.saturating_sub(1);
            }
            Action::Down => {
                let len = self.visible_pw_links().len();
                if self.pw_cursor + 1 < len {
                    self.pw_cursor += 1;
                }
            }
            Action::ShowAll => {
                self.pw_show_all = !self.pw_show_all;
                self.pw_cursor = 0;
            }
            Action::Disconnect => {
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
        self.render_frame(f);
        apply_render_opts(f.buffer_mut(), self.opts);
    }

    fn render_frame(&self, f: &mut Frame) {
        let area = f.area();
        if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
            render_too_small(f, area);
            return;
        }
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                // One header line and one tab line; the body's own border is
                // the rule under them (docs/DESIGN.md "Layout: Chrome").
                Constraint::Length(1), // Header
                Constraint::Length(1), // Tabs
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
        if self.show_help {
            self.render_help(f, area);
        }
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let line = if let Some(ref m) = self.model {
            let rate_str = m
                .stream
                .active_rate
                .map(|r| format!("{:.1} kHz", r as f64 / 1000.0))
                .unwrap_or_else(|| "Idle".into());
            // Locked is green ●; unlocked or mixer unavailable is caution ▲ (docs/DESIGN.md).
            let (sync_glyph, sync_color) = if m.has_mixer_controls() && m.is_locked() {
                ("●", Color::Green)
            } else {
                ("▲", Color::Yellow)
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

            // Priority order, left to right: sync · rate · 48V · model · USB/FW.
            // Segments that do not fit are dropped from the right; USB and FW
            // only show at 120+ columns (they are in the ? overlay's About block).
            let mut segs: Vec<Vec<Span<'static>>> = vec![
                vec![Span::styled(
                    format!(" {sync_glyph} {sync_label}"),
                    Style::default().fg(sync_color).add_modifier(Modifier::BOLD),
                )],
                vec![Span::styled(
                    format!("  {rate_str}"),
                    Style::default().add_modifier(Modifier::BOLD),
                )],
            ];
            if m.phantom_live() {
                segs.push(vec![
                    Span::raw("  "),
                    Span::styled(" 48V ", badge(Color::Red)),
                ]);
            }
            if self.clips.any() {
                segs.push(vec![
                    Span::raw("  "),
                    Span::styled(" CLIP ", badge(Color::Red)),
                ]);
            }
            segs.push(vec![
                Span::raw("  "),
                Span::styled(
                    m.card.name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" · {}", m.card.generation), secondary()),
            ]);
            if area.width >= 120 {
                segs.push(vec![Span::styled(
                    format!("  USB {}  {fw}", m.card.usb_id),
                    secondary(),
                )]);
            }
            let pw = Span::styled(
                format!("PW {pw_count} "),
                if pw_count > 0 {
                    Style::default().fg(Color::Magenta)
                } else {
                    secondary()
                },
            );
            fit_header(segs, pw, area.width as usize)
        } else if let Some(req) = self.card_override {
            Line::from(Span::styled(
                format!(" ✕ Focusrite card {req} not connected · r rescan"),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::from(Span::styled(
                " ✕ No Focusrite Scarlett connected · r rescan",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ))
        };

        f.render_widget(Paragraph::new(line), area);
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

        // Under 120 columns the meters stack full-width above the summary, so
        // bars stay long enough to read −6 vs −12 (docs/DESIGN.md "Layout").
        let cols = if area.width < 120 {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
                .split(area)
        };

        // Left: Live Hardware Level Meters
        // Label (24) + bar + value (9) + CLIP badge (7) + borders (2).
        let bar_width = cols[0].width.saturating_sub(44).max(8) as usize;
        let mut meter_lines = vec![scale_row(bar_width)];
        let mut is_channel = vec![false];
        let mut last_group: Option<RouteGroup> = None;

        for mc in &m.meter_channels {
            // Hide unrouted, silent channels, but never one holding a clip latch.
            if mc.source == "Off"
                && self.meter_raw.get(mc.meter_index).copied().unwrap_or(0) == 0
                && !self.clips.get(mc.meter_index)
            {
                continue;
            }
            if last_group != Some(mc.group) {
                if last_group.is_some() {
                    meter_lines.push(Line::from(""));
                    is_channel.push(false);
                }
                meter_lines.push(Line::from(Span::styled(
                    format!(" {}", mc.group.title()),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
                is_channel.push(false);
                last_group = Some(mc.group);
            }

            let raw = self.meter_raw.get(mc.meter_index).copied().unwrap_or(0);
            let db = meter_to_dbfs_with_max(raw, m.meter_max);
            let peak = self
                .meter_peak
                .get(mc.meter_index)
                .copied()
                .unwrap_or(-96.0);
            let db_text = if db <= -60.0 {
                "  -∞ dB".to_string()
            } else {
                format!("{db:+5.1} dB")
            };
            let raw_label = if mc.source.is_empty() {
                mc.label.clone()
            } else {
                format!("{} [{}]", mc.label, mc.source)
            };
            let label = truncate(&raw_label, 22);

            let mut spans = vec![Span::raw(format!(" {label:<22} "))];
            spans.extend(meter_bar(db, peak, bar_width));
            spans.push(Span::raw(format!(" {db_text}")));
            if self.clips.get(mc.meter_index) {
                spans.push(Span::raw(" "));
                spans.push(Span::styled("CLIP", badge(Color::Red)));
            }
            meter_lines.push(Line::from(spans));
            is_channel.push(true);
        }

        if meter_lines.len() == 1 {
            meter_lines = vec![Line::from(Span::styled(
                " No hardware level meter exposed by this model.",
                secondary(),
            ))];
            is_channel = vec![false];
        }

        // Never cut rows silently: show what fits, then "↓ N more".
        let inner_h = cols[0].height.saturating_sub(2) as usize;
        if meter_lines.len() > inner_h && inner_h > 1 {
            let hidden = is_channel[inner_h - 1..].iter().filter(|&&c| c).count();
            meter_lines.truncate(inner_h - 1);
            meter_lines.push(Line::from(Span::styled(
                format!(" ↓ {hidden} more (widen or heighten the window)"),
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
                    Some(true) if m.is_phantom_switch(pid) => (" 48V ", badge(Color::Red)),
                    Some(true) => ("ON", Style::default().fg(Color::Green)),
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
                format!(" PipeWire graph links ({})", pw_scarlett.len()),
                Style::default()
                    .fg(Color::Magenta)
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
                        Some(true) => ("ON".into(), Style::default().fg(Color::Green)),
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
                let armed_here = matches!(
                    self.armed.as_ref().map(|a| &a.action),
                    Some(ArmedAction::Phantom { numid, .. }) if *numid == item.numid
                );
                let on = c.opt_bool_val() == Some(true);
                let value_cell = if armed_here {
                    Cell::from(Line::from(vec![
                        Span::styled(" ARM 48V? ", badge(Color::Yellow)),
                        Span::styled(" Space again within 3 s · Esc cancels", secondary()),
                    ]))
                } else if on && m.is_phantom_switch(item.numid) {
                    Cell::from(Line::from(vec![
                        Span::styled(" 48V ", badge(Color::Red)),
                        Span::styled(
                            " live",
                            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                        ),
                    ]))
                } else if on && item.label.ends_with(" Mute") {
                    Cell::from(Span::styled(" MUTED ", badge(Color::Red)))
                } else {
                    Cell::from(val_str).style(val_style)
                };
                Row::new(vec![
                    Cell::from(item.section).style(secondary()),
                    Cell::from(item.label.clone()),
                    value_cell,
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
            Row::new(["Section", "Control", "Value"]).style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" I/O & Device ")
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
                let src_style = source_style(&r.current);
                let src_text = if r.current == "Off" {
                    "· off".to_string()
                } else {
                    r.current.clone()
                };
                Row::new(vec![
                    Cell::from(r.group.title()).style(secondary()),
                    Cell::from(r.label.clone()),
                    Cell::from(src_text).style(src_style),
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
                .title(" Device routing ")
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

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Mix matrix ")
            .border_style(decoration())
            .title_style(title_style());
        let inner = block.inner(area);
        f.render_widget(block, area);
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),    // table (2 header rows + cells)
                Constraint::Length(1), // ↓ N more
                Constraint::Length(1), // N ▸ more mixes
            ])
            .split(inner);

        let n_rows = m.matrix.inputs.len();
        let n_cols = m.matrix.mixes.len();
        let cur_row = self.matrix_row.min(n_rows - 1);
        let cur_col = self.matrix_col.min(n_cols - 1);
        let visible_rows = parts[0].height.saturating_sub(2).max(1) as usize;
        let visible_cols =
            (parts[0].width.saturating_sub(MATRIX_LABEL_W + 5) / MATRIX_COL_W).max(1) as usize;
        let row_start = window_start(cur_row, n_rows, visible_rows);
        let col_start = window_start(cur_col, n_cols, visible_cols);
        let row_end = (row_start + visible_rows).min(n_rows);
        let col_end = (col_start + visible_cols).min(n_cols);
        let shown_mixes = &m.matrix.mixes[col_start..col_end];
        let shown_inputs = &m.matrix.inputs[row_start..row_end];
        let hidden_left = col_start;
        let hidden_right = n_cols - col_end;
        let cyan_bold = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);

        // Header: mix letters, then the output each mix feeds (crosshair in cyan).
        let corner_top = if hidden_left > 0 {
            format!(" mixer input  ◂ {hidden_left}")
        } else {
            " mixer input  →  mix".to_string()
        };
        let mut header_cells = vec![Cell::from(Text::from(vec![
            Line::from(Span::styled(corner_top, secondary())),
            Line::from(Span::styled("        feeds output", secondary())),
        ]))];
        for (i, &mix) in shown_mixes.iter().enumerate() {
            let feeds = mix_output(m, mix);
            let on_cursor = col_start + i == cur_col;
            let letter_style = if on_cursor {
                cyan_bold.add_modifier(Modifier::UNDERLINED)
            } else if feeds.is_some() {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                secondary()
            };
            let feeds_style = if on_cursor {
                Style::default().fg(Color::Cyan)
            } else {
                secondary()
            };
            header_cells.push(Cell::from(Text::from(vec![
                Line::from(Span::styled(format!("{mix:^6}"), letter_style)),
                Line::from(Span::styled(
                    format!("{:^6}", feeds.unwrap_or_else(|| "—".into())),
                    feeds_style,
                )),
            ])));
        }
        if hidden_right > 0 {
            header_cells.push(Cell::from(Span::styled(
                format!("{hidden_right} ▸"),
                cyan_bold,
            )));
        }

        let rows: Vec<Row> = shown_inputs
            .iter()
            .enumerate()
            .map(|(local_r, &in_idx)| {
                let r_idx = row_start + local_r;
                let src = m.mixer_input_source(in_idx);
                let off = src == "Off";
                let label = format!(
                    " In {in_idx:02}  {}",
                    truncate(if off { "off" } else { src }, 13)
                );
                let label_style = if r_idx == cur_row {
                    cyan_bold
                } else if off {
                    secondary()
                } else {
                    Style::default()
                };
                let mut cells = vec![Cell::from(label).style(label_style)];
                for (local_c, &mix) in shown_mixes.iter().enumerate() {
                    let selected = r_idx == cur_row && col_start + local_c == cur_col;
                    let ctrl = m
                        .matrix
                        .cells
                        .get(&(mix, in_idx))
                        .and_then(|cid| m.controls.get(cid));
                    let (txt, mut style) = matrix_cell(ctrl);
                    if selected {
                        style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
                    }
                    cells.push(Cell::from(txt).style(style));
                }
                Row::new(cells)
            })
            .collect();

        let mut widths = vec![Constraint::Length(MATRIX_LABEL_W)];
        widths.extend(
            shown_mixes
                .iter()
                .map(|_| Constraint::Length(MATRIX_COL_W - 1)),
        );
        widths.push(Constraint::Length(4));
        let table = Table::new(rows, widths)
            .header(Row::new(header_cells).height(2))
            .column_spacing(1);
        f.render_widget(table, parts[0]);

        let hidden_below = n_rows - row_end;
        if hidden_below > 0 {
            let first = m.matrix.inputs[row_end];
            let last = m.matrix.inputs[n_rows - 1];
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!(" ↓ {hidden_below} more   In {first:02}–{last:02}   PgDn"),
                    secondary(),
                ))),
                parts[1],
            );
        }

        // The cell legend lives in the ? overlay; this row only counts the
        // mixes hidden to the right.
        if hidden_right > 0 {
            let more = vec![
                Span::styled(format!(" {hidden_right} ▸"), cyan_bold),
                Span::styled(" more mixes", secondary()),
            ];
            f.render_widget(Paragraph::new(Line::from(more)), parts[2]);
        }
    }

    fn render_pipewire(&self, f: &mut Frame, area: Rect) {
        let links = self.visible_pw_links();
        let title = if self.pw_show_all {
            " PipeWire graph · all links · software, not stored on the interface "
        } else {
            " PipeWire graph · Scarlett links · software, not stored on the interface "
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
                // Software links are magenta, never the device-routing cyan (DESIGN.md rule 5).
                let (tag, tag_style) = if is_loopback {
                    (
                        "⟲ loopback",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    )
                } else if l.is_scarlett {
                    ("device", Style::default().fg(Color::Magenta))
                } else {
                    ("other", secondary())
                };
                Row::new(vec![
                    Cell::from(format!("#{}", l.id)).style(secondary()),
                    Cell::from(tag).style(tag_style),
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
        let visible = picker.visible();
        let mut rows: Vec<ListItem> = Vec::new();
        let mut cursor_row = None;
        let mut last_group = None;
        for &i in &visible {
            let name = &picker.items[i];
            let (group, heading) = source_group(name);
            if last_group != Some(group) {
                rows.push(ListItem::new(Line::from(Span::styled(
                    format!(" {heading}"),
                    secondary(),
                ))));
                last_group = Some(group);
            }
            let current = i == picker.current;
            let mut extra = String::new();
            if let Some(mix) = name.strip_prefix("Mix ").and_then(|l| l.chars().next())
                && let Some(ref m) = self.model
            {
                extra = match mix_output(m, mix) {
                    Some(o) => format!("  → {o}"),
                    None => "  unused".into(),
                };
            }
            if current {
                extra.push_str(" (current)");
            }
            rows.push(ListItem::new(Line::from(vec![
                Span::raw(if current { " ● " } else { "   " }),
                Span::styled(name.clone(), source_style(name)),
                Span::styled(extra, secondary()),
            ])));
            if i == picker.selected {
                cursor_row = Some(rows.len() - 1);
            }
        }

        let popup_w = 52.min(area.width.saturating_sub(4));
        let popup_h = (rows.len() as u16 + 4).min(area.height.saturating_sub(4));
        let rect = Rect::new(
            area.x + area.width.saturating_sub(popup_w) / 2,
            area.y + area.height.saturating_sub(popup_h) / 2,
            popup_w,
            popup_h,
        );
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" Source for {} ", picker.title))
            .title_style(title_style().add_modifier(Modifier::BOLD))
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner);

        let filter_line = if picker.filtering || !picker.filter.is_empty() {
            Line::from(vec![
                Span::styled(" / ", secondary()),
                Span::styled(
                    picker.filter.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(if picker.filtering { "▏" } else { "" }, Style::default()),
            ])
        } else {
            Line::from(Span::styled(" / to filter", secondary()))
        };
        f.render_widget(Paragraph::new(filter_line), parts[0]);

        let list = List::new(rows)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD));
        let mut state = ListState::default();
        state.select(cursor_row);
        f.render_stateful_widget(list, parts[1], &mut state);

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(
                    " {} of {} match · Enter set · Esc cancel",
                    visible.len(),
                    picker.items.len()
                ),
                secondary(),
            ))),
            parts[2],
        );
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        if let Some(ref a) = self.armed {
            let left = a.action.window().saturating_sub(a.at.elapsed());
            let prompt = Line::from(vec![
                Span::styled(" ARMED ", badge(Color::Yellow)),
                Span::raw(format!(" {} ", a.action.prompt())),
                Span::styled(format!("{:.1} s", left.as_secs_f32()), secondary()),
            ]);
            let hint = Line::from(Span::styled(a.action.confirm_hint(), secondary()));
            f.render_widget(Paragraph::new(vec![prompt, hint]), area);
            return;
        }
        let width = area.width as usize;
        let status =
            if !self.status_msg.is_empty() && self.status_at.elapsed() < Duration::from_secs(5) {
                self.status_msg.as_str()
            } else {
                ""
            };
        let scope = if self.show_help {
            Scope::Help
        } else if self.route_picker.is_some() {
            Scope::Picker
        } else {
            self.tab.scope()
        };
        let rows = vec![
            status_line(status, &self.selection_context(), width),
            hint_line(&keys::hints(scope), scope != Scope::Help, width),
        ];
        f.render_widget(Paragraph::new(rows), area);
    }

    /// What the cursor is on, for the right of footer row 1.
    fn selection_context(&self) -> String {
        let Some(ref m) = self.model else {
            return String::new();
        };
        match self.tab {
            Tab::Overview => String::new(),
            Tab::IoDevice => self
                .io_items()
                .get(self.io_cursor)
                .map(|i| i.label.clone())
                .unwrap_or_default(),
            Tab::Routing => m
                .routes
                .get(self.route_cursor)
                .map(|r| format!("{} ← {}", r.label, r.current))
                .unwrap_or_default(),
            Tab::Matrix => {
                match (
                    m.matrix.mixes.get(self.matrix_col),
                    m.matrix.inputs.get(self.matrix_row),
                ) {
                    (Some(&mix), Some(&input)) => {
                        let feeds = mix_output(m, mix)
                            .map(|o| format!(" (feeds {o})"))
                            .unwrap_or_default();
                        let level = m
                            .matrix
                            .cells
                            .get(&(mix, input))
                            .and_then(|id| m.controls.get(id))
                            .map(|c| match matrix_cell(Some(c)).0.trim() {
                                "·" => "  −∞".to_string(),
                                t => format!("  {t} dB"),
                            })
                            .unwrap_or_default();
                        format!(
                            "In {input:02} {} → Mix {mix}{feeds}{level}",
                            m.mixer_input_source(input)
                        )
                    }
                    _ => String::new(),
                }
            }
            Tab::PipeWire => self
                .visible_pw_links()
                .get(self.pw_cursor)
                .map(|l| format!("link #{}", l.id))
                .unwrap_or_default(),
        }
    }

    fn render_help(&self, f: &mut Frame, area: Rect) {
        let scope = self.tab.scope();
        let entry = |b: &keys::Binding| {
            Line::from(vec![
                Span::styled(
                    format!(" {:<12}", b.label),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(b.help),
            ])
        };
        let heading = |t: &str| {
            Line::from(Span::styled(
                format!(" {t}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
        };

        let mut left = vec![heading("This tab")];
        let own: Vec<Line> = keys::bindings(scope).map(entry).collect();
        if own.is_empty() {
            left.push(Line::from(Span::styled(
                " No keys of its own; see Everywhere",
                secondary(),
            )));
        } else {
            left.extend(own);
        }
        if scope == Scope::Matrix {
            left.push(Line::from(""));
            left.push(heading("Cells"));
            left.push(Line::from(vec![
                Span::styled(" ·", decoration()),
                Span::raw(" −∞ (off)   "),
                Span::styled("0.0", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" unity   "),
                Span::styled(
                    "+3.0",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" boost"),
            ]));
            left.push(Line::from(" —  mix not routed to an output"));
        }
        left.push(Line::from(""));
        left.push(heading("Safety"));
        left.extend(keys::bindings(Scope::Safety).map(entry));

        let mut right = vec![heading("Everywhere")];
        right.extend(keys::bindings(Scope::Global).map(entry));
        right.push(Line::from(""));
        right.push(heading("About"));
        let about = |k: &str, v: String| {
            Line::from(vec![
                Span::styled(format!(" {k:<7}"), secondary()),
                Span::raw(v),
            ])
        };
        if let Some(ref m) = self.model {
            right.push(about(
                "Model",
                format!("{} {}", m.card.name, m.card.generation),
            ));
            right.push(about("USB", m.card.usb_id.clone()));
            right.push(about(
                "FW",
                m.firmware
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".into()),
            ));
            right.push(about(
                "Card",
                format!("hw:{} · {} found", m.card.index, self.cards.len().max(1)),
            ));
        } else {
            right.push(Line::from(Span::styled(" No device", secondary())));
        }

        let body_h = left.len().max(right.len()) as u16;
        let w = 86.min(area.width.saturating_sub(2));
        let h = (body_h + 4).min(area.height.saturating_sub(2));
        let rect = Rect::new(
            area.x + area.width.saturating_sub(w) / 2,
            area.y + area.height.saturating_sub(h) / 2,
            w,
            h,
        );
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title_style(title_style().add_modifier(Modifier::BOLD))
            .title(format!(" Keys · {} ", self.tab.title()));
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(parts[0]);
        f.render_widget(Paragraph::new(left), cols[0]);
        f.render_widget(Paragraph::new(right), cols[1]);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " Generated from the keymap the app uses, so this list cannot drift.",
                secondary(),
            ))),
            parts[1],
        );
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

/// Alert badge: reversed colour so the word survives light themes (docs/DESIGN.md rule 3).
fn badge(color: Color) -> Style {
    Style::default()
        .fg(color)
        .add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

/// Block titles are readable text: default foreground, so they do not inherit
/// the DarkGray border colour.
fn title_style() -> Style {
    Style::default().fg(Color::Reset)
}

/// Footer row 1: transient status on the left, selection context on the right.
fn status_line(status: &str, context: &str, width: usize) -> Line<'static> {
    let ctx = truncate(context, width / 2);
    let room = width.saturating_sub(ctx.chars().count() + 2);
    let left = truncate(status, room.saturating_sub(1));
    let pad = width.saturating_sub(left.chars().count() + 1 + ctx.chars().count() + 1);
    Line::from(vec![
        Span::raw(format!(" {left}")),
        Span::raw(" ".repeat(pad)),
        Span::styled(format!("{ctx} "), secondary()),
    ])
}

/// Footer row 2: as many hints as fit, highest value first, then `? help`.
fn hint_line(hints: &[(&str, &str)], with_help: bool, width: usize) -> Line<'static> {
    let help_len = if with_help { " ? help".len() } else { 0 };
    let mut spans = Vec::new();
    let mut used = 0;
    for (key, word) in hints {
        let len = 1 + key.chars().count() + 1 + word.chars().count() + 1;
        if used + len + help_len > width {
            break;
        }
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            key.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {word} "), secondary()));
        used += len;
    }
    if with_help {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            "?",
            Style::default().add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" help", secondary()));
    }
    Line::from(spans)
}

/// How long the peak-hold tick holds before it falls (docs/DESIGN.md).
const PEAK_HOLD: Duration = Duration::from_millis(1500);

/// Meter range drawn by the bars, in dBFS.
const METER_FLOOR_DB: f64 = -60.0;

fn zone_color(db: f64) -> Color {
    if db >= CLIP_DBFS {
        Color::Red
    } else if db >= HOT_DBFS {
        Color::Yellow
    } else {
        Color::Green
    }
}

/// Column (0-based) of `db` in a bar `width` cells wide.
fn meter_col(db: f64, width: usize) -> usize {
    let frac = ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0);
    ((frac * width as f64).round() as usize).min(width)
}

/// A meter bar: filled cells coloured by zone, empty track in DarkGray, and a
/// peak-hold tick `│` inside the bar.
fn meter_bar(db: f64, peak: f64, width: usize) -> Vec<Span<'static>> {
    let filled = meter_col(db, width);
    let tick = (peak > METER_FLOOR_DB).then(|| meter_col(peak, width).saturating_sub(1));
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut push = |text: char, style: Style| match spans.last_mut() {
        Some(last) if last.style == style => last.content.to_mut().push(text),
        _ => spans.push(Span::styled(text.to_string(), style)),
    };
    for i in 0..width {
        let cell_db = METER_FLOOR_DB + (i as f64 + 0.5) / width as f64 * -METER_FLOOR_DB;
        if i < filled {
            push('█', Style::default().fg(zone_color(cell_db)));
        } else if tick == Some(i) {
            push('│', Style::default().fg(zone_color(cell_db)));
        } else {
            push('░', decoration());
        }
    }
    spans
}

/// The dB scale above the meters, aligned to `meter_bar` columns.
fn scale_row(width: usize) -> Line<'static> {
    let mut row = vec![' '; width];
    for mark in [-48.0, -24.0, -12.0, -6.0, -3.0, 0.0] {
        let text = if mark == 0.0 {
            "0".to_string()
        } else {
            format!("{mark:.0}")
        };
        let end = meter_col(mark, width);
        let start = end.saturating_sub(text.chars().count());
        if start == 0 && mark != 0.0 && end < text.len() {
            continue;
        }
        // Skip a label that would overwrite its right-hand neighbour.
        if row[start..end].iter().any(|&c| c != ' ') {
            continue;
        }
        for (slot, ch) in row[start..end].iter_mut().zip(text.chars()) {
            *slot = ch;
        }
    }
    Line::from(vec![
        Span::raw(" ".repeat(24)),
        Span::styled(row.into_iter().collect::<String>(), secondary()),
    ])
}

/// Smallest terminal the layout supports (Kenny's ruling: design for 95×30,
/// show a message under 80×24).
const MIN_WIDTH: u16 = 80;
const MIN_HEIGHT: u16 = 24;

fn render_too_small(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled(
            "Terminal too small",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "{}×{} now; scarlett-tui needs at least {MIN_WIDTH}×{MIN_HEIGHT}.",
            area.width, area.height
        )),
        Line::from(Span::styled(
            "Enlarge the window, or press q to quit.",
            secondary(),
        )),
    ];
    let h = (lines.len() as u16).min(area.height);
    let y = area.y + area.height.saturating_sub(h) / 2;
    let rect = Rect::new(area.x, y, area.width, h);
    f.render_widget(
        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        rect,
    );
}

/// Join header segments in priority order, dropping whatever does not fit,
/// and right-align `right` (the PipeWire link count) when there is room.
fn fit_header(segs: Vec<Vec<Span<'static>>>, right: Span<'static>, width: usize) -> Line<'static> {
    let span_w = |s: &Span| s.content.chars().count();
    let right_w = span_w(&right);
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for seg in segs {
        let w: usize = seg.iter().map(span_w).sum();
        if used + w > width {
            break;
        }
        used += w;
        out.extend(seg);
    }
    if used + 2 + right_w <= width {
        out.push(Span::raw(" ".repeat(width - used - right_w)));
        out.push(right);
    }
    Line::from(out)
}

/// Matrix column widths: the row label, and each mix column including its gap.
const MATRIX_LABEL_W: u16 = 22;
const MATRIX_COL_W: u16 = 7;

/// The output a mix feeds, shortened for a 6-cell column header; `+N` when it
/// feeds several. `None` when no analogue output takes the mix.
fn mix_output(m: &ScarlettModel, mix: char) -> Option<String> {
    let want = format!("Mix {mix}");
    let outs: Vec<&str> = m
        .routes
        .iter()
        .filter(|r| r.group == RouteGroup::AnalogueOut && r.current == want)
        .map(|r| r.label.as_str())
        .collect();
    let first = short_output_name(outs.first()?);
    Some(if outs.len() > 1 {
        truncate(&format!("{first}+{}", outs.len() - 1), 6)
    } else {
        first
    })
}

/// "Line 01 (Monitor L)" → "Mon L"; "Headphones 1 L" → "Phn1 L"; at most 6 characters.
fn short_output_name(label: &str) -> String {
    let name = match (label.find('('), label.rfind(')')) {
        (Some(a), Some(b)) if b > a + 1 => &label[a + 1..b],
        _ => label,
    };
    let short = name
        .replace("Monitor", "Mon")
        .replace("Headphones ", "Phn")
        .replace("Phones ", "Phn")
        .replace("Analogue Output ", "Out ")
        .replace("Line ", "Ln ");
    truncate(short.trim(), 6)
}

/// Text and style for one matrix cell: blank when the cell does not exist,
/// `·` (decoration) for −∞, bold for unity, yellow bold with `+` for boost.
fn matrix_cell(ctrl: Option<&AlsaControl>) -> (String, Style) {
    let Some(c) = ctrl else {
        return (String::new(), Style::default());
    };
    match c.opt_val() {
        None => (
            format!("{:>5}", "?"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Some(v) if v <= c.min => (format!("{:>5}", "·"), decoration()),
        Some(v) => match c.current_db() {
            Some(db) if db > 0.25 => (
                format!("{db:+5.1}"),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Some(db) if db.abs() < 0.25 => (
                format!("{:>5}", "0.0"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Some(db) => (format!("{db:5.1}").replace('-', "−"), Style::default()),
            None => (format!("{v:5}"), Style::default()),
        },
    }
}

/// Rows moved by PgUp / PgDn in the matrix.
const MATRIX_PAGE: usize = 10;

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

pub fn run_tui(card_override: Option<u32>, opts: RenderOpts) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    if let Err(e) = execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(e);
    }
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(card_override, opts);

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
        let y = 29;
        let row: String = (0..95).map(|x| buf[(x, y)].symbol().to_string()).collect();
        let byte = row
            .find("q quit")
            .unwrap_or_else(|| panic!("footer row was {row:?}"));
        let x = row[..byte].chars().count() as u16;
        // The key is bold default foreground; its word is dimmed default foreground.
        let key = &buf[(x, y)];
        assert_eq!(key.fg, Color::Reset);
        assert!(key.modifier.contains(Modifier::BOLD));
        let word = &buf[(x + 2, y)];
        assert_eq!(word.fg, Color::Reset);
        assert!(word.modifier.contains(Modifier::DIM));
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect()
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(code, KeyModifiers::NONE);
    }

    fn armed_action(app: &App) -> Option<&ArmedAction> {
        app.armed.as_ref().map(|a| &a.action)
    }

    fn io_app_on_phantom() -> App {
        let mut app = App::for_test(Some(model_4i4()));
        app.tab = Tab::IoDevice;
        app.io_cursor = app
            .io_items()
            .iter()
            .position(|i| i.section == "Phantom Power")
            .expect("fixture has a phantom row");
        app
    }

    #[test]
    fn phantom_enable_needs_two_presses_and_esc_cancels() {
        let mut app = io_app_on_phantom();
        press(&mut app, KeyCode::Char(' '));
        assert!(matches!(
            armed_action(&app),
            Some(ArmedAction::Phantom { numid: 20, .. })
        ));
        // The first press never writes.
        let ctrl = &app.model.as_ref().unwrap().controls[&20];
        assert_eq!(ctrl.opt_bool_val(), Some(false));
        let buf = render_to_buffer(&app, 95, 30);
        assert!(row_text(&buf, 28).contains("ARMED"));
        assert!(row_text(&buf, 29).contains("Space confirm"));

        press(&mut app, KeyCode::Esc);
        assert!(app.armed.is_none());

        // Moving away cancels too.
        press(&mut app, KeyCode::Char('l'));
        assert!(app.armed.is_some(), "l arms like Space");
        press(&mut app, KeyCode::Char('k'));
        assert!(app.armed.is_none());

        // A second Space inside the window consumes the arm (and writes, given ALSA).
        app.io_cursor += 1;
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Char(' '));
        assert!(app.armed.is_none());
    }

    #[test]
    fn only_space_confirms_an_armed_48v() {
        let mut app = io_app_on_phantom();
        press(&mut app, KeyCode::Char(' '));
        let first = app.armed.as_ref().unwrap().at;
        // Enter cancels the arm and is then handled as a new toggle: it never
        // confirms, so 48V is still only armed.
        press(&mut app, KeyCode::Enter);
        let again = app.armed.as_ref().expect("Enter must not confirm 48V");
        assert!(again.at >= first);
        press(&mut app, KeyCode::Char(' '));
        assert!(app.armed.is_none(), "Space confirms");
    }

    #[test]
    fn matrix_space_and_enter_do_not_touch_a_cell() {
        let mut app = App::for_test(Some(model_4i4()));
        app.tab = Tab::Matrix;
        for code in [KeyCode::Char(' '), KeyCode::Enter, KeyCode::Char('x')] {
            press(&mut app, code);
            assert!(app.matrix_prev_levels.is_empty(), "{code:?} muted a cell");
            assert!(app.status_msg.is_empty(), "{code:?} did something");
        }
    }

    #[test]
    fn routing_picker_opens_on_enter_only() {
        let mut app = App::for_test(Some(model_4i4()));
        app.tab = Tab::Routing;
        press(&mut app, KeyCode::Char(' '));
        assert!(app.route_picker.is_none());
        press(&mut app, KeyCode::Enter);
        assert!(app.route_picker.is_some());
        // Space does not set the source; Esc cancels.
        press(&mut app, KeyCode::Char(' '));
        assert!(app.route_picker.is_some());
        press(&mut app, KeyCode::Esc);
        assert!(app.route_picker.is_none());
    }

    #[test]
    fn phantom_arm_times_out_after_three_seconds() {
        let mut app = io_app_on_phantom();
        press(&mut app, KeyCode::Enter);
        app.expire_armed(Instant::now() + Duration::from_millis(2500));
        assert!(app.armed.is_some(), "still armed inside 3 s");
        app.expire_armed(Instant::now() + Duration::from_secs(3));
        assert!(app.armed.is_none());
        assert!(app.status_msg.contains("timed out"));
    }

    #[test]
    fn phantom_disable_is_one_step() {
        let mut app = io_app_on_phantom();
        app.model
            .as_mut()
            .unwrap()
            .controls
            .get_mut(&20)
            .unwrap()
            .values = vec![1];
        press(&mut app, KeyCode::Char(' '));
        assert!(app.armed.is_none(), "turning 48V off must not arm");
    }

    #[test]
    fn unity_from_silence_needs_a_second_press() {
        let mut app = App::for_test(Some(model_4i4()));
        app.tab = Tab::Matrix;
        press(&mut app, KeyCode::Char('0'));
        assert!(matches!(
            armed_action(&app),
            Some(ArmedAction::Unity {
                numid: 40,
                rise_db: None,
                ..
            })
        ));
        let buf = render_to_buffer(&app, 95, 30);
        assert!(row_text(&buf, 28).contains("Press 0 again: −∞ to 0.0 dB"));
        press(&mut app, KeyCode::Char('0'));
        assert!(app.armed.is_none());
    }

    #[test]
    fn unity_on_low_gain_states_the_jump() {
        let mut app = App::for_test(Some(model_4i4()));
        app.tab = Tab::IoDevice;
        app.io_cursor = 0; // Input 1 Gain, raw 40 = −60 dB in the fixture
        press(&mut app, KeyCode::Char('0'));
        match armed_action(&app) {
            Some(ArmedAction::Unity {
                rise_db: Some(r), ..
            }) => assert!((*r - 60.0).abs() < 0.5),
            other => panic!("expected a unity confirm, got {other:?}"),
        }
    }

    #[test]
    fn header_shows_48v_badge_only_while_live() {
        let mut app = App::for_test(Some(model_4i4()));
        let buf = render_to_buffer(&app, 95, 30);
        assert!(!row_text(&buf, 0).contains("48V"));

        app.model
            .as_mut()
            .unwrap()
            .controls
            .get_mut(&20)
            .unwrap()
            .values = vec![1];
        let buf = render_to_buffer(&app, 95, 30);
        let row = row_text(&buf, 0);
        let x = row[..row.find("48V").expect("header badge")]
            .chars()
            .count() as u16;
        let cell = &buf[(x, 0)];
        assert_eq!(cell.fg, Color::Red);
        assert!(cell.modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn footer_ends_with_help_at_every_width() {
        let mut app = App::for_test(Some(model_4i4()));
        for tab in Tab::ALL {
            app.tab = tab;
            for w in [80, 95, 160] {
                let buf = render_to_buffer(&app, w, 30);
                let hints = row_text(&buf, 29);
                assert!(
                    hints.trim_end().ends_with("? help"),
                    "{w} cols, {}: {hints:?}",
                    tab.title()
                );
            }
        }
        // At 95 columns the matrix footer leads with its own keys.
        app.tab = Tab::Matrix;
        let hints = row_text(&render_to_buffer(&app, 95, 30), 29);
        assert!(hints.starts_with(" h j k l move"), "{hints:?}");
    }

    #[test]
    fn footer_row_one_shows_selection_context() {
        let mut app = App::for_test(Some(model_4i4()));
        app.tab = Tab::Matrix;
        let row = row_text(&render_to_buffer(&app, 95, 30), 28);
        assert!(
            row.trim_end().ends_with("In 01 PCM 1 → Mix A  −∞"),
            "{row:?}"
        );
    }

    #[test]
    fn help_overlay_lists_tab_keys_from_the_keymap() {
        let mut app = App::for_test(Some(model_4i4()));
        app.tab = Tab::Matrix;
        press(&mut app, KeyCode::Char('?'));
        assert!(app.show_help);
        let buf = render_to_buffer(&app, 95, 30);
        let screen: String = (0..30)
            .map(|y| row_text(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        for b in keys::bindings(Scope::Matrix).chain(keys::bindings(Scope::Global)) {
            assert!(screen.contains(b.help), "overlay is missing {:?}", b.help);
        }
        assert!(screen.contains("1235:8212"), "About block shows the USB id");
        // Keys other than ?/Esc/q do nothing while help is open.
        press(&mut app, KeyCode::Char('2'));
        assert_eq!(app.tab, Tab::Matrix);
        press(&mut app, KeyCode::Esc);
        assert!(!app.show_help);
    }

    /// docs/DESIGN.md "Layout: Chrome": the body gets 26 of 30 rows.
    #[test]
    fn chrome_is_one_header_line_and_one_tab_line() {
        let app = App::for_test(Some(model_4i4()));
        let buf = render_to_buffer(&app, 95, 30);
        assert!(row_text(&buf, 0).contains("● Locked"), "header on row 0");
        assert!(!row_text(&buf, 0).contains("scarlett-tui"), "no header box");
        assert!(row_text(&buf, 1).contains("1 Overview"), "tabs on row 1");
        assert!(row_text(&buf, 2).starts_with('┌'), "body starts on row 2");
        assert!(row_text(&buf, 27).starts_with('└'), "body ends on row 27");
    }

    #[test]
    fn footer_keeps_tab_keys_and_drops_global_noise() {
        let mut app = App::for_test(Some(model_4i4()));
        app.tab = Tab::Matrix;
        let hints = row_text(&render_to_buffer(&app, 95, 30), 29);
        assert!(
            hints.contains("unity") && hints.contains("quit"),
            "{hints:?}"
        );
        assert!(
            !hints.contains("tabs") && !hints.contains("rescan"),
            "{hints:?}"
        );
        assert!(hints.trim_end().ends_with("? help"), "{hints:?}");
    }

    #[test]
    fn matrix_legend_is_in_the_help_overlay() {
        let mut app = App::for_test(Some(model_4i4()));
        app.tab = Tab::Matrix;
        let tab = screen_text(&render_to_buffer(&app, 95, 30));
        assert!(!tab.contains("mix not routed"), "legend left the tab");
        press(&mut app, KeyCode::Char('?'));
        let help = screen_text(&render_to_buffer(&app, 95, 30));
        assert!(help.contains("−∞ (off)") && help.contains("mix not routed"));
    }

    #[test]
    fn block_titles_hold_names_only() {
        let mut app = App::for_test(Some(model_4i4()));
        for tab in Tab::ALL {
            app.tab = tab;
            let buf = render_to_buffer(&app, 95, 30);
            let body = row_text(&buf, 2);
            assert!(
                !body.contains("·  ") && !body.contains("toggle"),
                "{body:?}"
            );
        }
    }

    #[test]
    fn io_shift_l_steps_six_db_through_the_keymap() {
        assert_eq!(
            keys::lookup(Scope::Io, KeyCode::Char('L')),
            Some(Action::IncBig)
        );
        let mut app = App::for_test(Some(model_4i4()));
        app.tab = Tab::IoDevice;
        // Without ALSA the write fails, but the key must reach the I/O handler
        // and not be swallowed as a global key.
        press(&mut app, KeyCode::Char('L'));
        assert_eq!(app.tab, Tab::IoDevice);
    }

    #[test]
    fn too_small_message_below_80_by_24_only() {
        let app = App::for_test(Some(model_4i4()));
        for (w, h) in [(79, 24), (80, 23), (40, 10)] {
            let buf = render_to_buffer(&app, w, h);
            let screen: String = (0..h).map(|y| row_text(&buf, y)).collect();
            assert!(screen.contains("Terminal too small"), "{w}x{h}");
        }
        let buf = render_to_buffer(&app, 80, 24);
        let screen: String = (0..24).map(|y| row_text(&buf, y)).collect();
        assert!(!screen.contains("Terminal too small"));
    }

    #[test]
    fn header_keeps_priority_items_and_drops_usb_when_narrow() {
        let mut app = App::for_test(Some(model_4i4()));
        app.model
            .as_mut()
            .unwrap()
            .controls
            .get_mut(&20)
            .unwrap()
            .values = vec![1];
        let narrow = row_text(&render_to_buffer(&app, 95, 30), 0);
        let (sync, rate, v48, model) = (
            narrow.find("● Locked").expect("sync"),
            narrow.find("Idle").expect("rate"),
            narrow.find("48V").expect("48V badge"),
            narrow.find("Scarlett 4i4").expect("model"),
        );
        assert!(sync < rate && rate < v48 && v48 < model, "{narrow:?}");
        assert!(!narrow.contains("USB 1235"), "{narrow:?}");
        assert!(
            narrow
                .trim_end()
                .trim_end_matches('│')
                .trim_end()
                .ends_with("PW 0")
        );
        let wide = row_text(&render_to_buffer(&app, 160, 36), 0);
        assert!(wide.contains("USB 1235:8212"), "{wide:?}");
    }

    #[test]
    fn fit_header_drops_from_the_right() {
        let segs = vec![
            vec![Span::raw("aaaa")],
            vec![Span::raw("bbbb")],
            vec![Span::raw("cccc")],
        ];
        let line = fit_header(segs, Span::raw("PW"), 9);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, "aaaabbbb");
    }

    #[test]
    fn overview_stacks_meters_above_summary_under_120_columns() {
        let app = App::for_test(Some(model_4i4()));
        let find_row = |buf: &Buffer, h: u16, needle: &str| {
            (0..h).find(|&y| row_text(buf, y).contains(needle))
        };
        let buf = render_to_buffer(&app, 95, 30);
        let meters = find_row(&buf, 30, "Live Hardware Meters").unwrap();
        let summary = find_row(&buf, 30, "Status & Signal Path").unwrap();
        assert!(meters < summary, "stacked at 95 columns");
        let buf = render_to_buffer(&app, 160, 36);
        let meters = find_row(&buf, 36, "Live Hardware Meters").unwrap();
        let summary = find_row(&buf, 36, "Status & Signal Path").unwrap();
        assert_eq!(meters, summary, "side by side at 160 columns");
    }

    #[test]
    fn clip_latches_shows_badges_and_clears_with_shift_c() {
        let mut app = App::for_test(Some(model_4i4()));
        let now = Instant::now();
        app.ingest_meters(vec![4095, 0], now);
        app.ingest_meters(vec![0, 0], now);
        assert!(app.clips.get(0), "latch holds after the level drops");
        let buf = render_to_buffer(&app, 95, 30);
        assert!(row_text(&buf, 0).contains("CLIP"), "header badge");
        let screen: String = (0..30).map(|y| row_text(&buf, y)).collect();
        assert_eq!(screen.matches("CLIP").count(), 2, "header + one meter row");

        app.tab = Tab::Matrix; // C works from any tab
        press(&mut app, KeyCode::Char('C'));
        assert!(!app.clips.any());
        let buf = render_to_buffer(&app, 95, 30);
        assert!(!row_text(&buf, 1).contains("CLIP"));
    }

    #[test]
    fn peak_tick_holds_inside_the_bar() {
        let spans = meter_bar(-96.0, -6.0, 40);
        let bar: String = spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(bar.chars().count(), 40);
        let tick = bar.chars().position(|c| c == '│').expect("tick drawn");
        assert_eq!(tick, meter_col(-6.0, 40) - 1);
        assert!(!bar.contains('█'));

        // Peak hold: falls after 1.5 s, not before.
        let mut app = App::for_test(Some(model_4i4()));
        let t0 = Instant::now();
        app.ingest_meters(vec![2048, 0], t0); // about −6 dBFS
        app.ingest_meters(vec![0, 0], t0 + Duration::from_millis(1400));
        assert!(app.meter_peak[0] > -7.0);
        app.ingest_meters(vec![0, 0], t0 + Duration::from_millis(1600));
        assert!(app.meter_peak[0] < -60.0);
    }

    #[test]
    fn meter_bar_colours_by_zone_and_scale_lines_up() {
        let spans = meter_bar(0.0, -96.0, 60);
        let colours: Vec<Color> = spans.iter().filter_map(|s| s.style.fg).collect();
        assert_eq!(colours, vec![Color::Green, Color::Yellow, Color::Red]);
        let scale: String = scale_row(60).spans[1].content.to_string();
        for mark in ["-48", "-24", "-12", "-6", "-3", "0"] {
            assert!(scale.contains(mark), "{scale:?}");
        }
        assert!(scale.trim_end().ends_with('0'));
    }

    #[test]
    fn overflowing_meters_say_how_many_are_hidden() {
        let mut model = model_4i4();
        for i in 2..40 {
            model.meter_channels.push(crate::model::MeterChannel {
                meter_index: i,
                label: format!("Meter {:02}", i + 1),
                source: String::new(),
                group: RouteGroup::Unknown,
            });
        }
        let mut app = App::for_test(Some(model));
        app.meter_raw = vec![100; 40];
        let buf = render_to_buffer(&app, 95, 30);
        let screen: String = (0..30).map(|y| row_text(&buf, y)).collect();
        assert!(
            screen.contains("↓ ") && screen.contains(" more"),
            "{screen}"
        );
    }

    #[test]
    fn short_output_names_fit_a_column() {
        assert_eq!(short_output_name("Line 01 (Monitor L)"), "Mon L");
        assert_eq!(short_output_name("Headphones 1 L"), "Phn1 L");
        assert_eq!(short_output_name("Analogue Output 07"), "Out 07");
        assert!(
            short_output_name("Some Very Long Output Name")
                .chars()
                .count()
                <= 6
        );
    }

    #[test]
    fn matrix_cells_use_dot_blank_and_signed_boost() {
        use crate::model::fixtures::mk_ctrl;
        assert_eq!(matrix_cell(None).0, "");
        let mut c = mk_ctrl(
            1,
            "Mix A Input 01 Playback Volume",
            ControlKind::Int,
            &[],
            0,
        );
        let (txt, style) = matrix_cell(Some(&c));
        assert_eq!(txt.trim(), "·");
        assert_eq!(style.fg, Some(Color::DarkGray));
        c.values = vec![c.max]; // +12 dB in the fixture range
        let (txt, style) = matrix_cell(Some(&c));
        assert!(txt.trim().starts_with('+'), "{txt:?}");
        assert!(style.add_modifier.contains(Modifier::BOLD));
        c.value_valid = false;
        assert_eq!(matrix_cell(Some(&c)).0.trim(), "?");
    }

    #[test]
    fn matrix_header_names_outputs_and_counts_overflow() {
        let mut model = model_4i4();
        // Route Analogue Output 01 to Mix A, and add mixes B..P with no cells.
        model.controls.get_mut(&22).unwrap().values = vec![3];
        model.recompute_derived();
        model.matrix.mixes = ('A'..='P').collect();
        let mut app = App::for_test(Some(model));
        app.tab = Tab::Matrix;
        let buf = render_to_buffer(&app, 95, 30);
        let screen: Vec<String> = (0..30).map(|y| row_text(&buf, y)).collect();
        let all = screen.join("\n");
        let feeds = mix_output(app.model.as_ref().unwrap(), 'A').expect("Mix A feeds an output");
        assert!(
            screen
                .iter()
                .any(|r| r.contains("feeds output") && r.contains(&feeds)),
            "{all}"
        );
        assert!(all.contains(" ▸"), "right overflow count shown");
        assert!(!all.contains("−∞ (off)"), "the legend is in the overlay");
        assert!(!all.contains("-∞"), "no ASCII -∞ in cells");
        // Only one real cell exists; absent cells stay blank (no em dash in the body).
        let body_row = screen.iter().find(|r| r.contains("In 01")).unwrap();
        assert_eq!(body_row.matches('·').count(), 1, "{body_row:?}");
        // Moving right past the window shows a left overflow count.
        for _ in 0..15 {
            press(&mut app, KeyCode::Char('l'));
        }
        let buf = render_to_buffer(&app, 95, 30);
        let all: String = (0..30).map(|y| row_text(&buf, y)).collect();
        assert!(all.contains("◂ "), "left overflow count shown");
    }

    fn screen_text(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| row_text(buf, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn tabs_name_device_routing_and_pipewire_graph() {
        let app = App::for_test(Some(model_4i4()));
        let tabs = row_text(&render_to_buffer(&app, 95, 30), 1);
        assert!(
            tabs.contains("3 Device routing") && tabs.contains("5 PipeWire graph"),
            "{tabs:?}"
        );
    }

    #[test]
    fn route_picker_marks_current_groups_and_filters() {
        let mut app = App::for_test(Some(model_4i4()));
        app.tab = Tab::Routing;
        press(&mut app, KeyCode::Enter);
        let picker = app.route_picker.as_ref().expect("picker opens");
        let current = picker.current;
        let screen = screen_text(&render_to_buffer(&app, 95, 30));
        let cur_name = &picker.items[current];
        assert!(screen.contains(&format!("● {cur_name}")), "{screen}");
        assert!(screen.contains("Mixes") && screen.contains("From computer (PCM)"));
        assert!(screen.contains("6 of 6 match"));

        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('p'));
        press(&mut app, KeyCode::Char('c'));
        let picker = app.route_picker.as_ref().unwrap();
        assert_eq!(
            picker.items[picker.selected], "PCM 1",
            "cursor snaps to a match"
        );
        let screen = screen_text(&render_to_buffer(&app, 95, 30));
        assert!(screen.contains("2 of 6 match"), "{screen}");
        // j is typed into the filter, not used to move.
        press(&mut app, KeyCode::Char('j'));
        assert!(app.route_picker.as_ref().unwrap().visible().is_empty());
        press(&mut app, KeyCode::Enter);
        assert!(
            app.route_picker.is_some(),
            "Enter with no match keeps the picker"
        );
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Down);
        let picker = app.route_picker.as_ref().unwrap();
        assert_eq!(picker.items[picker.selected], "PCM 2");
        // Esc clears the filter first, then closes.
        press(&mut app, KeyCode::Esc);
        assert!(app.route_picker.as_ref().unwrap().filter.is_empty());
        press(&mut app, KeyCode::Esc);
        assert!(app.route_picker.is_none());
    }

    #[test]
    fn pipewire_loopback_rows_are_magenta_with_a_glyph() {
        let mut app = App::for_test(Some(model_4i4()));
        app.pw_links = vec![PwLink {
            id: 42,
            output_port: "alsa_input.usb-Focusrite:capture_FL".into(),
            input_port: "alsa_output.usb-Focusrite:playback_FL".into(),
            is_scarlett: true,
        }];
        app.tab = Tab::PipeWire;
        let buf = render_to_buffer(&app, 95, 30);
        let y = (0..30)
            .find(|&y| row_text(&buf, y).contains("⟲ loopback"))
            .expect("loopback tag");
        let row = row_text(&buf, y);
        let x = row[..row.find('⟲').unwrap()].chars().count() as u16;
        assert_eq!(buf[(x, y)].fg, Color::Magenta);
        assert!(screen_text(&buf).contains("not stored on the interface"));
    }

    #[test]
    fn no_color_comes_from_flag_env_or_dumb_terminal() {
        assert!(!RenderOpts::from_env(false, false, None, Some("xterm-256color")).no_color);
        assert!(RenderOpts::from_env(true, false, None, None).no_color);
        assert!(RenderOpts::from_env(false, false, Some("1"), None).no_color);
        assert!(
            !RenderOpts::from_env(false, false, Some(""), None).no_color,
            "empty NO_COLOR is unset"
        );
        assert!(RenderOpts::from_env(false, false, None, Some("dumb")).no_color);
        assert!(RenderOpts::from_env(false, true, None, None).ascii);
    }

    #[test]
    fn no_color_mode_keeps_badges_by_modifier() {
        let mut app = App::for_test(Some(model_4i4()));
        app.opts.no_color = true;
        app.model
            .as_mut()
            .unwrap()
            .controls
            .get_mut(&20)
            .unwrap()
            .values = vec![1];
        for tab in Tab::ALL {
            app.tab = tab;
            let buf = render_to_buffer(&app, 95, 30);
            assert!(
                buf.content()
                    .iter()
                    .all(|c| c.fg == Color::Reset && c.bg == Color::Reset),
                "colour left on {}",
                tab.title()
            );
        }
        app.tab = Tab::Overview;
        let buf = render_to_buffer(&app, 95, 30);
        let row = row_text(&buf, 0);
        let x = row[..row.find("48V").unwrap()].chars().count() as u16;
        assert!(
            buf[(x, 0)].modifier.contains(Modifier::REVERSED),
            "48V badge still reversed"
        );
    }

    #[test]
    fn ascii_mode_draws_only_ascii() {
        let mut app = App::for_test(Some(model_4i4()));
        app.opts.ascii = true;
        app.ingest_meters(vec![2048, 4095], Instant::now());
        for tab in Tab::ALL {
            app.tab = tab;
            for (w, h) in [(95, 30), (160, 36), (60, 20)] {
                let buf = render_to_buffer(&app, w, h);
                for cell in buf.content() {
                    assert!(
                        cell.symbol().is_ascii(),
                        "{:?} on {} at {w}x{h}",
                        cell.symbol(),
                        tab.title()
                    );
                }
            }
        }
        app.tab = Tab::Overview;
        let screen: String = (0..30)
            .map(|y| row_text(&render_to_buffer(&app, 95, 30), y))
            .collect();
        assert!(
            screen.contains('#') && screen.contains("CLIP"),
            "meter fill and clip word survive"
        );
    }

    /// Tokyo Night, as Omarchy ships it (`themes/tokyo-night/colors.toml`), for
    /// README screenshots only. The TUI itself never uses RGB.
    fn svg_color(c: Color, fg: bool) -> &'static str {
        match c {
            Color::Reset => {
                if fg {
                    "#a9b1d6"
                } else {
                    "#1a1b26"
                }
            }
            Color::Red | Color::LightRed => "#f7768e",
            Color::Green | Color::LightGreen => "#9ece6a",
            Color::Yellow | Color::LightYellow => "#e0af68",
            Color::Blue | Color::LightBlue => "#7aa2f7",
            Color::Magenta | Color::LightMagenta => "#ad8ee6",
            Color::Cyan | Color::LightCyan => "#449dab",
            Color::DarkGray => "#414868",
            Color::Gray | Color::White => "#c0caf5",
            _ => "#a9b1d6",
        }
    }

    /// Paint a rendered buffer as an SVG terminal: one text run per style span,
    /// stretched to the cell grid so columns line up.
    fn buffer_to_svg(buf: &Buffer) -> String {
        let (cw, ch, fs) = (8.4_f32, 18.0_f32, 14.0_f32);
        let (w, h) = (buf.area.width, buf.area.height);
        let pad = 12.0;
        let mut out = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\">\n\
             <rect width=\"100%\" height=\"100%\" rx=\"8\" fill=\"#1a1b26\"/>\n\
             <g font-family=\"JetBrainsMono Nerd Font, monospace\" font-size=\"{fs}\">\n",
            w as f32 * cw + 2.0 * pad,
            h as f32 * ch + 2.0 * pad
        );
        let esc = |t: &str| {
            t.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        };
        for y in 0..h {
            let mut x = 0;
            while x < w {
                let c0 = &buf[(x, y)];
                let key = (c0.fg, c0.bg, c0.modifier);
                let start = x;
                let mut text = String::new();
                while x < w && {
                    let c = &buf[(x, y)];
                    (c.fg, c.bg, c.modifier) == key
                } {
                    text.push_str(buf[(x, y)].symbol());
                    x += 1;
                }
                let n = (x - start) as f32;
                let rev = key.2.contains(Modifier::REVERSED);
                let (fg, bg) = if rev {
                    (svg_color(key.1, false), svg_color(key.0, true))
                } else {
                    (svg_color(key.0, true), svg_color(key.1, false))
                };
                let px = pad + start as f32 * cw;
                let py = pad + y as f32 * ch;
                if rev || key.1 != Color::Reset {
                    out += &format!(
                        "<rect x=\"{px}\" y=\"{py}\" width=\"{}\" height=\"{ch}\" fill=\"{bg}\"/>\n",
                        n * cw
                    );
                }
                if text.trim().is_empty() {
                    continue;
                }
                let weight = if key.2.contains(Modifier::BOLD) {
                    " font-weight=\"bold\""
                } else {
                    ""
                };
                let opacity = if key.2.contains(Modifier::DIM) {
                    " fill-opacity=\"0.62\""
                } else {
                    ""
                };
                let deco = if key.2.contains(Modifier::UNDERLINED) {
                    " text-decoration=\"underline\""
                } else {
                    ""
                };
                out += &format!(
                    "<text x=\"{px}\" y=\"{}\" fill=\"{fg}\"{weight}{opacity}{deco} textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\" xml:space=\"preserve\">{}</text>\n",
                    py + ch * 0.75,
                    n * cw,
                    esc(&text)
                );
            }
        }
        out + "</g>\n</svg>\n"
    }

    /// Writes `design/tui-*.svg` from the real TUI against an attached Scarlett.
    /// Opens the ALSA control interface read-only (no writes, no PCM).
    /// Run with: cargo test --locked -- --ignored capture_readme_screenshots
    #[test]
    #[ignore = "needs a Scarlett attached; writes design/tui-*.svg"]
    fn capture_readme_screenshots() {
        let mut app = App::new(None, RenderOpts::default());
        assert!(app.model.is_some(), "no Scarlett attached");
        app.status_msg.clear();
        for _ in 0..12 {
            app.tick();
            std::thread::sleep(Duration::from_millis(60));
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("design");
        std::fs::create_dir_all(&dir).unwrap();
        let shot = |app: &App, name: &str, w: u16, h: u16| {
            let svg = buffer_to_svg(&render_to_buffer(app, w, h));
            std::fs::write(dir.join(format!("tui-{name}.svg")), svg).unwrap();
        };
        shot(&app, "overview", 95, 30);
        shot(&app, "overview-wide", 160, 36);
        app.tab = Tab::IoDevice;
        shot(&app, "io", 95, 30);
        app.tab = Tab::Matrix;
        shot(&app, "matrix", 95, 30);
        app.show_help = true;
        shot(&app, "help", 95, 30);
    }

    #[test]
    fn window_start_never_exceeds_bounds_after_shrink() {
        assert_eq!(window_start(25, 4, 10), 0);
        assert_eq!(window_start(25, 0, 10), 0);
        assert_eq!(window_start(12, 20, 5), 8);
        assert_eq!(window_start(19, 20, 5), 15);
    }
}
