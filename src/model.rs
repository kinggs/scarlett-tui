use crate::alsa_ctl::{AlsaControl, CardEntry, ControlKind, StreamRateInfo};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Serialize)]
pub struct InputChannel {
    pub index: u32,
    pub label: String,
    pub gain_numid: Option<u32>,
    pub level_numid: Option<u32>,
    pub air_numid: Option<u32>,
    pub pad_numid: Option<u32>,
    pub safe_numid: Option<u32>,
    pub autogain_numid: Option<u32>,
    pub link_numid: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutputChannel {
    pub index: u32,
    pub label: String,
    pub volume_numid: Option<u32>,
    pub mute_numid: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteGroup {
    AnalogueOut,
    SpdifOut,
    AdatOut,
    MixerIn,
    DspIn,
    PcmCapture,
    Unknown,
}

impl RouteGroup {
    pub fn title(self) -> &'static str {
        match self {
            Self::AnalogueOut => "Analogue Outputs",
            Self::SpdifOut => "S/PDIF Outputs",
            Self::AdatOut => "ADAT Outputs",
            Self::MixerIn => "Mixer Inputs",
            Self::DspIn => "DSP Inputs",
            Self::PcmCapture => "PC Capture (Rec)",
            Self::Unknown => "Meters",
        }
    }

    pub fn short_prefix(self) -> &'static str {
        match self {
            Self::AnalogueOut => "Out",
            Self::SpdifOut => "S/PDIF",
            Self::AdatOut => "ADAT",
            Self::MixerIn => "Mix In",
            Self::DspIn => "DSP In",
            Self::PcmCapture => "Rec",
            Self::Unknown => "Meter",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteEntry {
    pub group: RouteGroup,
    pub channel: u32,
    pub label: String,
    pub numid: u32,
    pub current: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MixMatrix {
    pub mixes: Vec<char>,
    pub inputs: Vec<u32>,
    /// Keyed by (mix_letter, input_index) -> numid
    pub cells: HashMap<(char, u32), u32>,
}

impl MixMatrix {
    pub fn is_empty(&self) -> bool {
        self.mixes.is_empty() || self.inputs.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MeterChannel {
    pub meter_index: usize,
    pub label: String,
    pub source: String,
    pub group: RouteGroup,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScarlettModel {
    pub card: CardEntry,
    pub stream: StreamRateInfo,
    pub firmware: Option<i64>,
    pub firmware_versions: Vec<(String, i64)>,
    pub sync_status: String,
    pub controls: BTreeMap<u32, AlsaControl>,
    pub inputs: Vec<InputChannel>,
    pub outputs: Vec<OutputChannel>,
    pub phantom_numids: Vec<u32>,
    pub direct_monitor_numid: Option<u32>,
    pub device_numids: Vec<u32>,
    pub routes: Vec<RouteEntry>,
    pub matrix: MixMatrix,
    pub meter_numid: Option<u32>,
    pub meter_max: i64,
    pub meter_channels: Vec<MeterChannel>,
}

impl ScarlettModel {
    pub fn build(card: CardEntry, stream: StreamRateInfo, list: Vec<AlsaControl>) -> Self {
        let mut controls = BTreeMap::new();
        for c in list {
            controls.insert(c.numid, c);
        }

        let mut firmware = None;
        let mut firmware_versions = Vec::new();
        let mut sync_status = String::from("Unknown");
        let mut meter_numid = None;
        let mut meter_max: i64 = 4095;

        let mut input_map: BTreeMap<u32, InputChannel> = BTreeMap::new();
        let mut output_map: BTreeMap<u32, OutputChannel> = BTreeMap::new();
        let mut phantom_numids = Vec::new();
        let mut direct_monitor_numid = None;
        let mut device_numids = Vec::new();
        let mut routes = Vec::new();
        let mut mix_letters = BTreeMap::<char, ()>::new();
        let mut mix_inputs = BTreeMap::<u32, ()>::new();
        let mut mix_cells = HashMap::<(char, u32), u32>::new();

        for ctrl in controls.values() {
            let name = ctrl.name.as_str();
            if (name == "Firmware Version" || name.ends_with(" Firmware Version"))
                && ctrl.kind == ControlKind::Int
            {
                if let Some(v) = ctrl.opt_val() {
                    firmware_versions.push((ctrl.name.clone(), v));
                    if name == "Firmware Version"
                        || (firmware.is_none() && name == "App Firmware Version")
                        || firmware.is_none()
                    {
                        firmware = Some(v);
                    }
                }
                continue;
            }
            if name == "Sync Status" && ctrl.kind == ControlKind::Enum {
                sync_status = ctrl.enum_label().to_string();
                continue;
            }
            if name == "Level Meter" && ctrl.kind == ControlKind::Int {
                meter_numid = Some(ctrl.numid);
                if ctrl.max > 0 {
                    meter_max = ctrl.max;
                }
                continue;
            }

            // Input channel controls: "Line In N ..." or paired "Line In 1-2 ..."
            if let Some(rest) = name.strip_prefix("Line In ") {
                if name.contains("Phantom Power") && ctrl.kind == ControlKind::Bool {
                    phantom_numids.push(ctrl.numid);
                    continue;
                }
                if name.contains("Link")
                    && rest.contains('-')
                    && ctrl.kind == ControlKind::Bool
                    && ctrl.rw
                {
                    device_numids.push(ctrl.numid);
                    continue;
                }
                if let Some((num_str, tail)) = rest.split_once(' ')
                    && let Ok(ch) = num_str.parse::<u32>()
                {
                    let entry = input_map.entry(ch).or_insert_with(|| InputChannel {
                        index: ch,
                        label: format!("Input {ch}"),
                        gain_numid: None,
                        level_numid: None,
                        air_numid: None,
                        pad_numid: None,
                        safe_numid: None,
                        autogain_numid: None,
                        link_numid: None,
                    });
                    if tail.starts_with("Gain") && ctrl.kind == ControlKind::Int {
                        entry.gain_numid = Some(ctrl.numid);
                        continue;
                    } else if tail.starts_with("Level")
                        && matches!(ctrl.kind, ControlKind::Enum | ControlKind::Bool)
                    {
                        entry.level_numid = Some(ctrl.numid);
                        continue;
                    } else if tail.starts_with("Air")
                        && matches!(ctrl.kind, ControlKind::Bool | ControlKind::Enum)
                    {
                        entry.air_numid = Some(ctrl.numid);
                        continue;
                    } else if tail.starts_with("Pad") && ctrl.kind == ControlKind::Bool {
                        entry.pad_numid = Some(ctrl.numid);
                        continue;
                    } else if tail.starts_with("Safe") && ctrl.kind == ControlKind::Bool {
                        entry.safe_numid = Some(ctrl.numid);
                        continue;
                    } else if tail.starts_with("Autogain")
                        && ctrl.kind == ControlKind::Bool
                        && ctrl.rw
                    {
                        entry.autogain_numid = Some(ctrl.numid);
                        continue;
                    } else if tail.starts_with("Link") && ctrl.kind == ControlKind::Bool {
                        entry.link_numid = Some(ctrl.numid);
                        continue;
                    }
                }
            }

            if name == "Phantom Power Persistence Capture Switch" && ctrl.kind == ControlKind::Bool
            {
                phantom_numids.push(ctrl.numid);
                continue;
            }

            // Direct Monitor switches/enums & Solo/2i2 monitor mix faders
            if name.starts_with("Direct Monitor") && ctrl.rw {
                if matches!(ctrl.kind, ControlKind::Bool | ControlKind::Enum)
                    && direct_monitor_numid.is_none()
                    && !name.contains("Mix")
                {
                    direct_monitor_numid = Some(ctrl.numid);
                } else {
                    device_numids.push(ctrl.numid);
                }
                continue;
            }
            if (name.starts_with("Monitor Mix")
                || name.starts_with("Monitor 1 Mix")
                || name.starts_with("Monitor 2 Mix"))
                && ctrl.rw
            {
                device_numids.push(ctrl.numid);
                continue;
            }

            // Output volumes, mutes, and HW/SW volume selectors:
            //   scarlett2: "Line 01 (Monitor L) Playback Volume" / "Line 01 Mute Playback Switch"
            //   Clarett / Scarlett HW/SW: "Line 01 Volume Control Playback Enum"
            //   FCP Gen 4: "Analogue 1 Playback Volume" / "Analogue 1 Mute Playback Switch"
            if let Some(rest) = name.strip_prefix("Line ")
                && !name.starts_with("Line In ")
                && let Some((num_str, tail)) = rest.split_once(' ')
                && let Ok(ch) = num_str.parse::<u32>()
            {
                if tail.contains("Volume Control") && ctrl.kind == ControlKind::Enum && ctrl.rw {
                    device_numids.push(ctrl.numid);
                    continue;
                }
                if tail.contains("Playback Volume") && ctrl.kind == ControlKind::Int {
                    let entry = output_map.entry(ch).or_insert_with(|| OutputChannel {
                        index: ch,
                        label: format!("Line {ch:02}"),
                        volume_numid: None,
                        mute_numid: None,
                    });
                    if let Some(start) = tail.find('(')
                        && let Some(end) = tail[start + 1..].find(')')
                    {
                        let descr = &tail[start + 1..start + 1 + end];
                        entry.label = format!("Line {ch:02} ({descr})");
                    }
                    entry.volume_numid = Some(ctrl.numid);
                    continue;
                } else if tail.contains("Mute") && ctrl.kind == ControlKind::Bool {
                    let entry = output_map.entry(ch).or_insert_with(|| OutputChannel {
                        index: ch,
                        label: format!("Line {ch:02}"),
                        volume_numid: None,
                        mute_numid: None,
                    });
                    entry.mute_numid = Some(ctrl.numid);
                    continue;
                }
            } else if let Some(rest) = name.strip_prefix("Analogue ")
                && let Some((num_str, tail)) = rest.split_once(' ')
                && let Ok(ch) = num_str.parse::<u32>()
            {
                if tail.contains("Volume Control") && ctrl.kind == ControlKind::Enum && ctrl.rw {
                    device_numids.push(ctrl.numid);
                    continue;
                }
                if tail == "Playback Volume" && ctrl.kind == ControlKind::Int {
                    let entry = output_map.entry(ch).or_insert_with(|| OutputChannel {
                        index: ch,
                        label: format!("Analogue {ch:02}"),
                        volume_numid: None,
                        mute_numid: None,
                    });
                    entry.volume_numid = Some(ctrl.numid);
                    continue;
                } else if tail.contains("Mute") && ctrl.kind == ControlKind::Bool {
                    let entry = output_map.entry(ch).or_insert_with(|| OutputChannel {
                        index: ch,
                        label: format!("Analogue {ch:02}"),
                        volume_numid: None,
                        mute_numid: None,
                    });
                    entry.mute_numid = Some(ctrl.numid);
                    continue;
                }
            }

            // Vocaster named outputs: Speaker & Headphones N mutes/volumes
            if let Some((out_idx, out_label, is_vol)) = parse_named_output(name, ctrl.kind) {
                let entry = output_map.entry(out_idx).or_insert_with(|| OutputChannel {
                    index: out_idx,
                    label: out_label,
                    volume_numid: None,
                    mute_numid: None,
                });
                if is_vol {
                    entry.volume_numid = Some(ctrl.numid);
                } else {
                    entry.mute_numid = Some(ctrl.numid);
                }
                continue;
            }

            // Mix Matrix cells: "Mix A Input 01 Playback Volume"
            if let Some((mix_ch, in_ch)) = parse_mix_cell(name)
                && ctrl.kind == ControlKind::Int
            {
                mix_letters.insert(mix_ch, ());
                mix_inputs.insert(in_ch, ());
                mix_cells.insert((mix_ch, in_ch), ctrl.numid);
                continue;
            }

            // Routing enums
            if ctrl.kind == ControlKind::Enum
                && let Some((group, ch)) = parse_route_enum(name)
            {
                routes.push(RouteEntry {
                    group,
                    channel: ch,
                    label: format!("{} {ch:02}", group.short_prefix()),
                    numid: ctrl.numid,
                    current: ctrl.enum_label().to_string(),
                    summary: String::new(),
                });
                continue;
            }

            // Other writable device switches/enums/ints (including HW/SW volume selectors)
            if ctrl.rw
                && (name.ends_with("Volume Control Playback Enum")
                    || matches!(
                        name,
                        "Standalone Switch"
                            | "MSD Mode Switch"
                            | "Front Panel Brightness"
                            | "Front Panel Sleep Time"
                            | "Speaker Switching Playback Enum"
                            | "Speaker Switching Alt Playback Switch"
                            | "Talkback Playback Switch"
                            | "Talkback Enable Playback Switch"
                            | "Talk Playback Switch"
                            | "Dim Playback Switch"
                            | "Mute Playback Switch"
                            | "Master Playback Volume"
                            | "Master HW Playback Volume"
                            | "PCM Input Switch"
                            | "PCM Input Capture Switch"
                            | "Digital I/O Mode Capture Enum"
                            | "Clock Source Clock Source"
                            | "Host Mute Capture Switch"
                            | "Guest Mute Capture Switch"
                            | "Auto Gain Capture Switch"
                            | "Enhance Capture Switch"
                    ))
            {
                device_numids.push(ctrl.numid);
            }
        }

        // Sort routes in scarlett2_ports order: AnalogueOut -> SpdifOut -> AdatOut -> MixerIn -> DspIn -> PcmCapture
        routes.sort_by_key(|r| (r.group, r.channel));

        // Enrich AnalogueOut labels from OutputChannel hardware descriptions,
        // and ensure every AnalogueOut route has a corresponding OutputChannel
        // entry even on models without per-channel hardware faders.
        for r in &mut routes {
            if r.group == RouteGroup::AnalogueOut {
                if let Some(out) = output_map.get(&r.channel) {
                    r.label = out.label.clone();
                } else {
                    output_map.insert(
                        r.channel,
                        OutputChannel {
                            index: r.channel,
                            label: r.label.clone(),
                            volume_numid: None,
                            mute_numid: None,
                        },
                    );
                }
            }
        }

        let matrix = MixMatrix {
            mixes: mix_letters.into_keys().collect(),
            inputs: mix_inputs.into_keys().collect(),
            cells: mix_cells,
        };

        let mut model = Self {
            card,
            stream,
            firmware,
            firmware_versions,
            sync_status,
            controls,
            inputs: input_map.into_values().collect(),
            outputs: output_map.into_values().collect(),
            phantom_numids,
            direct_monitor_numid,
            device_numids,
            routes,
            matrix,
            meter_numid,
            meter_max,
            meter_channels: Vec::new(),
        };
        model.recompute_derived();
        model
    }

    /// True if at least one Focusrite mixer/router/I/O/meter control was discovered.
    /// Large Gen 4 FCP interfaces can appear as USB ALSA cards before `fcp-server`
    /// creates their mixer controls; this distinguishes that state.
    pub fn has_mixer_controls(&self) -> bool {
        !self.inputs.is_empty()
            || !self.outputs.is_empty()
            || !self.phantom_numids.is_empty()
            || self.direct_monitor_numid.is_some()
            || !self.device_numids.is_empty()
            || !self.routes.is_empty()
            || !self.matrix.is_empty()
            || self.meter_numid.is_some()
    }

    pub fn recompute_derived(&mut self) {
        // Refresh sync_status
        for c in self.controls.values() {
            if c.name == "Sync Status" && c.kind == ControlKind::Enum {
                self.sync_status = c.enum_label().to_string();
            }
        }

        // Map mixer input index -> source label
        let mut mixer_in_sources: HashMap<u32, String> = HashMap::new();
        for r in &mut self.routes {
            if let Some(c) = self.controls.get(&r.numid) {
                r.current = c.enum_label().to_string();
            }
            if r.group == RouteGroup::MixerIn {
                mixer_in_sources.insert(r.channel, r.current.clone());
            }
        }

        // Compute human-readable resolved summaries for routes (resolving Mix A..Z contents!)
        for r in &mut self.routes {
            if let Some(letter) = r
                .current
                .strip_prefix("Mix ")
                .and_then(|s| s.chars().next())
            {
                let mut active_parts = Vec::new();
                for &in_idx in &self.matrix.inputs {
                    if let Some(&cid) = self.matrix.cells.get(&(letter, in_idx))
                        && let Some(c) = self.controls.get(&cid)
                        && let Some(v) = c.opt_val()
                        && v > c.min
                    {
                        let src = mixer_in_sources
                            .get(&in_idx)
                            .map(|s| s.as_str())
                            .unwrap_or("Off");
                        if src != "Off" {
                            if let Some(db) = c.current_db() {
                                if db.abs() < 0.25 {
                                    active_parts.push(src.to_string());
                                } else {
                                    active_parts.push(format!("{src} ({db:+.1} dB)"));
                                }
                            } else {
                                active_parts.push(src.to_string());
                            }
                        }
                    }
                }
                if active_parts.is_empty() {
                    r.summary = format!("{} [silent]", r.current);
                } else {
                    r.summary = format!("{} ← {}", r.current, active_parts.join(" + "));
                }
            } else {
                r.summary = r.current.clone();
            }
        }

        // Map Level Meter slots only when count matches routes AND the driver
        // generation uses the documented `scarlett2_ports` destination order.
        // Otherwise use neutral `Meter NN` labels under `RouteGroup::Unknown`.
        let meter_count = self
            .meter_numid
            .and_then(|id| self.controls.get(&id))
            .map(|c| c.count as usize)
            .unwrap_or(0);

        self.meter_channels.clear();
        if meter_count > 0 {
            if self.routes.len() == meter_count && is_scarlett2_port_ordered(&self.card.generation)
            {
                for (i, r) in self.routes.iter().enumerate() {
                    self.meter_channels.push(MeterChannel {
                        meter_index: i,
                        label: r.label.clone(),
                        source: r.current.clone(),
                        group: r.group,
                    });
                }
            } else {
                for i in 0..meter_count {
                    self.meter_channels.push(MeterChannel {
                        meter_index: i,
                        label: format!("Meter {:02}", i + 1),
                        source: String::new(),
                        group: RouteGroup::Unknown,
                    });
                }
            }
        }
    }

    pub fn mixer_input_source(&self, input_idx: u32) -> &str {
        self.routes
            .iter()
            .find(|r| r.group == RouteGroup::MixerIn && r.channel == input_idx)
            .map(|r| r.current.as_str())
            .unwrap_or("Off")
    }

    pub fn unity_raw_for(&self, numid: u32) -> Option<i64> {
        self.controls.get(&numid)?.resolved_unity_raw()
    }

    pub fn is_locked(&self) -> bool {
        self.sync_status.eq_ignore_ascii_case("Locked")
    }

    /// True for a control that switches 48 V phantom power on inputs, not the
    /// "Phantom Power Persistence" setting that only remembers it.
    pub fn is_phantom_switch(&self, numid: u32) -> bool {
        self.phantom_numids.contains(&numid)
            && self
                .controls
                .get(&numid)
                .is_some_and(|c| !c.name.contains("Persistence"))
    }

    /// True while any input has 48 V phantom power on.
    pub fn phantom_live(&self) -> bool {
        self.phantom_numids.iter().any(|&id| {
            self.is_phantom_switch(id)
                && self
                    .controls
                    .get(&id)
                    .and_then(|c| c.opt_bool_val())
                    .unwrap_or(false)
        })
    }

    /// True while 48 V is on for input `input`. A switch named "Line In 1-2
    /// Phantom Power …" covers inputs 1 and 2; one with no input number in its
    /// name covers every input.
    pub fn input_phantom_on(&self, input: u32) -> bool {
        self.phantom_numids.iter().any(|&id| {
            let Some(c) = self.controls.get(&id) else {
                return false;
            };
            self.is_phantom_switch(id)
                && c.opt_bool_val().unwrap_or(false)
                && phantom_covers(&c.name, input)
        })
    }

    /// The input states the Overview shows next to a meter fed by `input`.
    pub fn input_tags(&self, input: u32) -> InputTags {
        let ctl = |id: Option<u32>| id.and_then(|id| self.controls.get(&id));
        let inp = self.inputs.iter().find(|i| i.index == input);
        // Level is "Line"/"Inst" as an enum, or a switch that is on for Inst.
        let inst = ctl(inp.and_then(|i| i.level_numid)).is_some_and(|c| match c.kind {
            ControlKind::Enum => c.enum_label().eq_ignore_ascii_case("Inst"),
            _ => c.bool_val(),
        });
        // Air is a switch (Gen 3) or an enum whose first item is "Off" (Gen 4).
        let air = ctl(inp.and_then(|i| i.air_numid)).is_some_and(|c| match c.kind {
            ControlKind::Enum => c.opt_val().is_some_and(|v| v > 0),
            _ => c.bool_val(),
        });
        InputTags {
            phantom: self.input_phantom_on(input),
            inst,
            air,
        }
    }

    /// The meter that shows input `input`'s level. An analogue input has no
    /// meter of its own, so this is the first PC capture channel it feeds,
    /// else the first mixer input it feeds. `None` when nothing it feeds has a
    /// meter, or when the meters carry no source names (`Meter NN`).
    pub fn input_meter(&self, input: u32) -> Option<usize> {
        let fed_by = |group: RouteGroup| {
            self.meter_channels
                .iter()
                .find(|m| m.group == group && analogue_input_number(&m.source) == Some(input))
                .map(|m| m.meter_index)
        };
        fed_by(RouteGroup::PcmCapture).or_else(|| fed_by(RouteGroup::MixerIn))
    }
}

/// Input states shown beside a meter: 48 V, instrument level, Air.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputTags {
    pub phantom: bool,
    pub inst: bool,
    pub air: bool,
}

/// The analogue input a route source names: "Analogue 3" → 3. The scarlett2
/// driver names them "Analogue N"; "Analog N", "Analogue In N", "Mic N" and
/// "Line In N" are accepted too ⚠ (other spellings are guesses for models not
/// yet tested). Digital sources (S/PDIF, ADAT, PCM, Mix) never match.
pub fn analogue_input_number(source: &str) -> Option<u32> {
    let s = source.trim();
    [
        "Analogue In ",
        "Analogue ",
        "Analog In ",
        "Analog ",
        "Line In ",
        "Mic ",
    ]
    .iter()
    .find_map(|p| s.strip_prefix(p))
    .and_then(|n| n.trim().parse().ok())
}

/// True when a phantom switch's name covers input `input` ("Line In 1-2 …",
/// "Line In 3 …", or no number for all inputs).
fn phantom_covers(name: &str, input: u32) -> bool {
    let Some(range) = name
        .strip_prefix("Line In ")
        .and_then(|r| r.split_whitespace().next())
    else {
        return true;
    };
    let mut ends = range.split(['-', '–']).map(|n| n.parse::<u32>());
    match (ends.next(), ends.next()) {
        (Some(Ok(a)), Some(Ok(b))) => (a..=b).contains(&input),
        (Some(Ok(a)), None) => a == input,
        _ => true,
    }
}

/// Meter zones shared by the TUI and the panel (docs/DESIGN.md "Meter zones").
/// Levels at or above `HOT_DBFS` are hot (caution); at or above `CLIP_DBFS` they clip.
/// −6 dBFS is where Focusrite's 3rd Gen Gain Halo turns amber. The halo turns
/// red only at 0 dBFS; the latch fires 1 dB earlier on purpose.
pub const HOT_DBFS: f64 = -6.0;
pub const CLIP_DBFS: f64 = -1.0;

/// Per-channel clip latch: set when a channel reaches `CLIP_DBFS`, held until cleared.
/// The TUI and the daemon both use it, so the panel and the TUI agree on what clipped.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClipLatch {
    latched: Vec<bool>,
}

impl ClipLatch {
    /// Latch any channel at or above the clip threshold. Returns true if a new
    /// channel latched on this frame.
    pub fn update(&mut self, dbfs: &[f64]) -> bool {
        if self.latched.len() != dbfs.len() {
            self.latched.resize(dbfs.len(), false);
        }
        let mut newly = false;
        for (slot, &db) in self.latched.iter_mut().zip(dbfs) {
            if db >= CLIP_DBFS && !*slot {
                *slot = true;
                newly = true;
            }
        }
        newly
    }

    pub fn clear(&mut self) {
        self.latched.iter_mut().for_each(|c| *c = false);
    }

    pub fn get(&self, channel: usize) -> bool {
        self.latched.get(channel).copied().unwrap_or(false)
    }

    pub fn any(&self) -> bool {
        self.latched.iter().any(|&c| c)
    }

    pub fn as_slice(&self) -> &[bool] {
        &self.latched
    }
}

/// A unity (0 dB) jump larger than this needs a second key press (docs/DESIGN.md).
pub const UNITY_CONFIRM_RISE_DB: f64 = 6.0;

/// What pressing the unity key should do for a level at `current_db`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnityJump {
    /// At, below or within +6 dB of unity: write immediately.
    Instant,
    /// Ask for a second press. `rise_db` is `None` when the level is silent
    /// (−∞) or unreadable, so the size of the jump is unknown.
    Confirm { rise_db: Option<f64> },
}

/// Decide whether jumping a level to 0 dB needs confirmation. `at_min` is true
/// when the control sits at its minimum raw value, which the hardware treats as −∞.
pub fn unity_jump(current_db: Option<f64>, at_min: bool) -> UnityJump {
    if at_min {
        return UnityJump::Confirm { rise_db: None };
    }
    match current_db {
        None => UnityJump::Confirm { rise_db: None },
        Some(db) if -db > UNITY_CONFIRM_RISE_DB => UnityJump::Confirm { rise_db: Some(-db) },
        Some(_) => UnityJump::Instant,
    }
}

fn is_scarlett2_port_ordered(generation: &str) -> bool {
    matches!(
        generation,
        "Gen 2" | "Gen 3" | "Gen 4" | "Clarett USB" | "Clarett+" | "Vocaster"
    )
}

fn parse_named_output(name: &str, kind: ControlKind) -> Option<(u32, String, bool)> {
    // Use synthetic indices >= 100 so named Vocaster outputs never collide with
    // numbered Line 01..NN / Analogue 01..NN channels.
    if name == "Speaker Mute Playback Switch" && kind == ControlKind::Bool {
        return Some((100, "Speaker".into(), false));
    }
    if name == "Speaker Playback Volume" && kind == ControlKind::Int {
        return Some((100, "Speaker".into(), true));
    }
    if name == "Headphones Mute Playback Switch" && kind == ControlKind::Bool {
        return Some((101, "Headphones".into(), false));
    }
    if name == "Headphones Playback Volume" && kind == ControlKind::Int {
        return Some((101, "Headphones".into(), true));
    }
    if let Some(rest) = name.strip_prefix("Headphones ")
        && let Some((num_str, tail)) = rest.split_once(' ')
        && let Ok(n) = num_str.parse::<u32>()
    {
        if tail == "Mute Playback Switch" && kind == ControlKind::Bool {
            return Some((100 + n, format!("Headphones {n}"), false));
        }
        if tail == "Playback Volume" && kind == ControlKind::Int {
            return Some((100 + n, format!("Headphones {n}"), true));
        }
    }
    None
}

fn parse_mix_cell(name: &str) -> Option<(char, u32)> {
    // "Mix A Input 01 Playback Volume"
    let rest = name.strip_prefix("Mix ")?;
    let mut chars = rest.chars();
    let letter = chars.next()?;
    if !letter.is_ascii_uppercase() {
        return None;
    }
    let after = chars.as_str().strip_prefix(" Input ")?;
    let (num_str, tail) = after.split_once(' ')?;
    if !tail.starts_with("Playback Volume") {
        return None;
    }
    let input_num = num_str.parse::<u32>().ok()?;
    Some((letter, input_num))
}

fn parse_route_enum(name: &str) -> Option<(RouteGroup, u32)> {
    let patterns: &[(&str, &str, RouteGroup)] = &[
        (
            "Analogue Output ",
            " Playback Enum",
            RouteGroup::AnalogueOut,
        ),
        ("Analogue ", " Playback Enum", RouteGroup::AnalogueOut),
        ("S/PDIF Output ", " Playback Enum", RouteGroup::SpdifOut),
        ("S/PDIF ", " Playback Enum", RouteGroup::SpdifOut),
        ("ADAT Output ", " Playback Enum", RouteGroup::AdatOut),
        ("ADAT ", " Playback Enum", RouteGroup::AdatOut),
        ("Mixer Input ", " Capture Enum", RouteGroup::MixerIn),
        ("Mixer ", " Capture Enum", RouteGroup::MixerIn),
        ("DSP Input ", " Capture Enum", RouteGroup::DspIn),
        ("PCM ", " Capture Enum", RouteGroup::PcmCapture),
    ];
    for &(prefix, suffix, group) in patterns {
        if let Some(rest) = name.strip_prefix(prefix)
            && let Some(num_str) = rest.strip_suffix(suffix)
            && let Ok(ch) = num_str.trim().parse::<u32>()
        {
            return Some((group, ch));
        }
    }
    None
}

/// Convert hardware meter value (0..max) to dBFS using the control's full scale.
pub fn meter_to_dbfs_with_max(raw: i64, max: i64) -> f64 {
    if raw <= 0 || max <= 0 {
        return -96.0;
    }
    (20.0 * ((raw as f64) / (max as f64)).log10()).clamp(-96.0, 0.0)
}

/// Convert 12-bit Scarlett hardware meter value (0..4095) to dBFS.
#[cfg(test)]
pub fn meter_to_dbfs(raw: i64) -> f64 {
    meter_to_dbfs_with_max(raw, 4095)
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::fixtures::mk_ctrl;

    #[test]
    fn builds_4i4_gen3_model_dynamically() {
        let card = CardEntry {
            index: 1,
            alsa_id: "USB".into(),
            name: "Scarlett 4i4 USB".into(),
            usb_id: "1235:8212".into(),
            generation: "Gen 3".into(),
        };
        let items = &["Off", "Analogue 1", "Analogue 2", "Mix A", "PCM 1", "PCM 2"];
        let list = vec![
            mk_ctrl(
                6,
                "Line 01 (Monitor L) Playback Volume",
                ControlKind::Int,
                &[],
                127,
            ),
            mk_ctrl(7, "Line 01 Mute Playback Switch", ControlKind::Bool, &[], 0),
            mk_ctrl(
                14,
                "Line In 1 Level Capture Enum",
                ControlKind::Enum,
                &["Line", "Inst"],
                0,
            ),
            mk_ctrl(
                18,
                "Line In 1 Air Capture Switch",
                ControlKind::Bool,
                &[],
                1,
            ),
            mk_ctrl(
                20,
                "Line In 1-2 Phantom Power Capture Switch",
                ControlKind::Bool,
                &[],
                1,
            ),
            mk_ctrl(
                22,
                "Analogue Output 01 Playback Enum",
                ControlKind::Enum,
                items,
                3,
            ),
            mk_ctrl(
                26,
                "Mixer Input 01 Capture Enum",
                ControlKind::Enum,
                items,
                4,
            ),
            mk_ctrl(
                40,
                "Mix A Input 01 Playback Volume",
                ControlKind::Int,
                &[],
                160,
            ),
            mk_ctrl(
                89,
                "Sync Status",
                ControlKind::Enum,
                &["Unlocked", "Locked"],
                1,
            ),
        ];
        let model = ScarlettModel::build(card, StreamRateInfo::default(), list);
        assert!(model.has_mixer_controls());
        assert_eq!(model.inputs.len(), 1);
        assert_eq!(model.outputs.len(), 1);
        assert_eq!(model.outputs[0].label, "Line 01 (Monitor L)");
        assert_eq!(model.matrix.mixes, vec!['A']);
        assert_eq!(model.matrix.inputs, vec![1]);
        assert_eq!(model.unity_raw_for(40), Some(160));
        assert_eq!(model.routes[0].summary, "Mix A ← PCM 1");
        assert!(model.is_locked());
    }

    #[test]
    fn handles_gen4_air_autogain_and_monitor_mix() {
        let card = CardEntry {
            index: 0,
            alsa_id: "Gen".into(),
            name: "Scarlett 2i2 4th Gen".into(),
            usb_id: "1235:8219".into(),
            generation: "Gen 4".into(),
        };
        let list = vec![
            mk_ctrl(
                1,
                "Line In 1 Gain Capture Volume",
                ControlKind::Int,
                &[],
                40,
            ),
            mk_ctrl(
                2,
                "Line In 1 Air Capture Enum",
                ControlKind::Enum,
                &["Off", "Presence", "Presence + Drive"],
                2,
            ),
            mk_ctrl(
                3,
                "Line In 1 Safe Capture Switch",
                ControlKind::Bool,
                &[],
                1,
            ),
            mk_ctrl(
                4,
                "Line In 1 Autogain Capture Switch",
                ControlKind::Bool,
                &[],
                0,
            ),
            mk_ctrl(
                5,
                "Direct Monitor Playback Enum",
                ControlKind::Enum,
                &["Off", "Mono", "Stereo"],
                1,
            ),
            mk_ctrl(
                6,
                "Direct Monitor Mix Playback Volume",
                ControlKind::Int,
                &[],
                120,
            ),
        ];
        let model = ScarlettModel::build(card, StreamRateInfo::default(), list);
        assert_eq!(model.inputs.len(), 1);
        assert!(model.inputs[0].gain_numid.is_some());
        assert!(model.inputs[0].air_numid.is_some());
        assert!(model.inputs[0].safe_numid.is_some());
        assert_eq!(model.inputs[0].autogain_numid, Some(4));
        assert_eq!(model.direct_monitor_numid, Some(5));
        assert!(model.device_numids.contains(&6));
    }

    #[test]
    fn handles_large_fcp_names_multi_firmware_and_neutral_meters() {
        let card = CardEntry {
            index: 2,
            alsa_id: "FCP".into(),
            name: "Scarlett 18i20 4th Gen".into(),
            usb_id: "1235:821d".into(),
            generation: "Gen 4 (FCP)".into(),
        };
        let items = &["Off", "Analogue 1", "PCM 1", "Mix A"];
        let mut meter = mk_ctrl(12, "Level Meter", ControlKind::Int, &[], 0);
        meter.count = 5;
        meter.max = 65535;
        meter.rw = false;
        let list = vec![
            mk_ctrl(1, "Analogue 1 Playback Enum", ControlKind::Enum, items, 3),
            mk_ctrl(2, "S/PDIF 1 Playback Enum", ControlKind::Enum, items, 2),
            mk_ctrl(3, "ADAT 1 Playback Enum", ControlKind::Enum, items, 0),
            mk_ctrl(4, "Mixer 1 Capture Enum", ControlKind::Enum, items, 1),
            mk_ctrl(
                5,
                "Mix A Input 01 Playback Volume",
                ControlKind::Int,
                &[],
                148,
            ),
            mk_ctrl(6, "PCM 1 Capture Enum", ControlKind::Enum, items, 1),
            mk_ctrl(7, "Analogue 1 Playback Volume", ControlKind::Int, &[], 127),
            mk_ctrl(
                8,
                "Line In 1-2 Link Capture Switch",
                ControlKind::Bool,
                &[],
                1,
            ),
            mk_ctrl(9, "PCM Input Capture Switch", ControlKind::Bool, &[], 0),
            mk_ctrl(10, "App Firmware Version", ControlKind::Int, &[], 2304),
            mk_ctrl(11, "ESP Firmware Version", ControlKind::Int, &[], 512),
            meter,
        ];
        let model = ScarlettModel::build(card, StreamRateInfo::default(), list);
        assert_eq!(model.firmware, Some(2304));
        assert_eq!(model.firmware_versions.len(), 2);
        assert_eq!(model.routes.len(), 5);
        assert_eq!(model.outputs.len(), 1);
        assert_eq!(model.outputs[0].label, "Analogue 01");
        assert_eq!(model.device_numids.len(), 2);
        assert_eq!(model.meter_max, 65535);
        // FCP layout is not guaranteed to match scarlett2_ports order, so meters stay neutral.
        assert_eq!(model.meter_channels.len(), 5);
        assert_eq!(model.meter_channels[0].group, RouteGroup::Unknown);
        assert_eq!(model.meter_channels[0].label, "Meter 01");
    }

    #[test]
    fn handles_vocaster_clarett_and_faderless_analogue_routes() {
        let card = CardEntry {
            index: 3,
            alsa_id: "Vocaster".into(),
            name: "Vocaster Two USB".into(),
            usb_id: "1235:8217".into(),
            generation: "Vocaster".into(),
        };
        let items = &["Off", "PCM 1"];
        let list = vec![
            mk_ctrl(1, "Speaker Mute Playback Switch", ControlKind::Bool, &[], 0),
            mk_ctrl(
                2,
                "Headphones 1 Mute Playback Switch",
                ControlKind::Bool,
                &[],
                1,
            ),
            mk_ctrl(
                3,
                "Line 01 Volume Control Playback Enum",
                ControlKind::Enum,
                &["SW", "HW"],
                1,
            ),
            mk_ctrl(
                4,
                "Analogue Output 02 Playback Enum",
                ControlKind::Enum,
                items,
                1,
            ),
        ];
        let model = ScarlettModel::build(card, StreamRateInfo::default(), list);
        assert!(
            model
                .outputs
                .iter()
                .any(|o| o.label == "Speaker" && o.mute_numid == Some(1))
        );
        assert!(
            model
                .outputs
                .iter()
                .any(|o| o.label == "Headphones 1" && o.mute_numid == Some(2))
        );
        // Analogue Output 02 has no volume/mute, yet is still exposed in outputs.
        assert!(
            model
                .outputs
                .iter()
                .any(|o| o.index == 2 && o.volume_numid.is_none())
        );
        assert!(model.device_numids.contains(&3));
    }

    #[test]
    fn detects_missing_mixer_controls_before_fcp_server_starts() {
        let card = CardEntry {
            index: 2,
            alsa_id: "FCP".into(),
            name: "Scarlett 18i20 4th Gen".into(),
            usb_id: "1235:821d".into(),
            generation: "Gen 4 (FCP)".into(),
        };
        let model = ScarlettModel::build(card, StreamRateInfo::default(), Vec::new());
        assert!(!model.has_mixer_controls());
    }

    #[test]
    fn unity_raw_is_none_when_no_db_range() {
        let mut c = mk_ctrl(1, "Front Panel Brightness", ControlKind::Int, &[], 50);
        c.db_min = None;
        c.db_max = None;
        c.max = 100;
        assert_eq!(c.resolved_unity_raw(), None);
    }

    #[test]
    fn unread_control_does_not_fabricate_zero() {
        let mut c = mk_ctrl(
            20,
            "Line In 1-2 Phantom Power Capture Switch",
            ControlKind::Bool,
            &[],
            1,
        );
        c.values.clear();
        c.value_valid = false;
        assert_eq!(c.opt_val(), None);
        assert_eq!(c.opt_bool_val(), None);
        assert_eq!(c.enum_label(), "?");
    }

    #[test]
    fn meter_and_db_conversions_match_hardware_scale() {
        assert!((meter_to_dbfs(0) - (-96.0)).abs() < 0.01);
        assert!((meter_to_dbfs(4095) - 0.0).abs() < 0.01);
        assert!((meter_to_dbfs(2048) - (-6.02)).abs() < 0.05);
        assert!((meter_to_dbfs_with_max(65535, 65535) - 0.0).abs() < 0.01);

        let c = mk_ctrl(
            40,
            "Mix A Input 01 Playback Volume",
            ControlKind::Int,
            &[],
            160,
        );
        assert!((c.current_db().unwrap() - 0.0).abs() < 0.01);
        assert_eq!(c.db_to_raw(0.0), Some(160));
        assert_eq!(c.db_to_raw(-80.0), Some(0));
        assert_eq!(c.db_to_raw(12.0), Some(184));
        assert_eq!(c.db_to_raw(50.0), Some(184)); // clamped
    }

    #[test]
    fn unity_jump_confirms_only_large_rises() {
        assert_eq!(
            unity_jump(Some(-40.0), false),
            UnityJump::Confirm {
                rise_db: Some(40.0)
            }
        );
        assert_eq!(
            unity_jump(Some(-6.5), false),
            UnityJump::Confirm { rise_db: Some(6.5) }
        );
        assert_eq!(unity_jump(Some(-6.0), false), UnityJump::Instant);
        assert_eq!(unity_jump(Some(-1.0), false), UnityJump::Instant);
        // Downward jumps (gain above unity) are always instant.
        assert_eq!(unity_jump(Some(12.0), false), UnityJump::Instant);
        // Silent (minimum = −∞) or unreadable levels always confirm.
        assert_eq!(
            unity_jump(Some(-3.0), true),
            UnityJump::Confirm { rise_db: None }
        );
        assert_eq!(
            unity_jump(None, false),
            UnityJump::Confirm { rise_db: None }
        );
    }

    #[test]
    fn phantom_live_ignores_persistence_setting() {
        let mut model = super::fixtures::model_4i4();
        assert!(model.is_phantom_switch(20));
        assert!(!model.phantom_live());

        // Persistence on, 48V off: not live, and not a phantom switch.
        model.controls.insert(
            21,
            mk_ctrl(
                21,
                "Phantom Power Persistence Capture Switch",
                ControlKind::Bool,
                &[],
                1,
            ),
        );
        model.phantom_numids.push(21);
        assert!(!model.is_phantom_switch(21));
        assert!(!model.phantom_live());

        model.controls.get_mut(&20).unwrap().values = vec![1];
        assert!(model.phantom_live());
    }

    #[test]
    fn analogue_input_number_accepts_known_spellings_only() {
        use super::analogue_input_number as n;
        assert_eq!(n("Analogue 3"), Some(3));
        assert_eq!(n("Analog 2"), Some(2));
        assert_eq!(n("Analogue In 10"), Some(10));
        assert_eq!(n("Mic 1"), Some(1));
        assert_eq!(n("Line In 4"), Some(4));
        for other in [
            "PCM 1", "Mix A", "S/PDIF 1", "ADAT 3", "Off", "Analogue", "",
        ] {
            assert_eq!(n(other), None, "{other:?}");
        }
    }

    #[test]
    fn input_meter_prefers_capture_then_mixer_input() {
        let model = super::fixtures::model_demo();
        // Demo order: 4 outputs (0–3), 8 mixer inputs (4–11), 6 captures (12–17).
        // Analogue 1 feeds mixer input 1 and capture 1: capture wins.
        assert_eq!(model.input_meter(1), Some(12));
        assert_eq!(model.input_meter(2), Some(13));
        // Nothing carries Analogue 3.
        assert_eq!(model.input_meter(3), None);

        // With the capture routed elsewhere, fall back to the mixer input.
        let mut m = model.clone();
        for mc in m.meter_channels.iter_mut() {
            if mc.group == RouteGroup::PcmCapture {
                mc.source = "Mix A".into();
            }
        }
        assert_eq!(m.input_meter(1), Some(4));

        // Another driver spelling still matches.
        m.meter_channels[4].source = "Analog 1".into();
        assert_eq!(m.input_meter(1), Some(4));

        // Neutral `Meter NN` labels carry no source: no guess by position.
        for mc in m.meter_channels.iter_mut() {
            mc.source.clear();
            mc.group = RouteGroup::Unknown;
        }
        assert_eq!(m.input_meter(1), None);
    }

    #[test]
    fn input_tags_follow_level_air_and_phantom_range() {
        let mut model = super::fixtures::model_demo();
        assert_eq!(
            model.input_tags(1),
            super::InputTags {
                phantom: true,
                inst: true,
                air: true
            }
        );
        assert_eq!(
            model.input_tags(2),
            super::InputTags {
                phantom: true,
                inst: false,
                air: false
            }
        );
        // "Line In 1-2" does not cover input 3; persistence never counts.
        assert!(!model.input_phantom_on(3));
        model.controls.get_mut(&20).unwrap().values = vec![0];
        assert!(!model.input_phantom_on(1), "persistence on is not 48V live");
        assert_eq!(model.input_tags(9), super::InputTags::default());

        assert!(super::phantom_covers(
            "Line In 3 Phantom Power Capture Switch",
            3
        ));
        assert!(!super::phantom_covers(
            "Line In 3 Phantom Power Capture Switch",
            4
        ));
        assert!(super::phantom_covers("Phantom Power Capture Switch", 7));
    }

    #[test]
    fn clip_latch_holds_until_cleared() {
        let mut latch = ClipLatch::default();
        assert!(!latch.update(&[-1.1, -30.0]));
        assert!(!latch.any());
        assert!(latch.update(&[-1.0, -30.0]), "-1 dBFS latches");
        assert!(latch.get(0) && !latch.get(1));
        // Falling back below the threshold keeps the latch.
        assert!(!latch.update(&[-40.0, -30.0]));
        assert!(latch.get(0));
        latch.clear();
        assert!(!latch.any());
        // A channel-count change resizes without panicking.
        latch.update(&[0.0, -50.0, -50.0]);
        assert_eq!(latch.as_slice(), &[true, false, false]);
        assert!(!latch.get(7));
    }

    #[test]
    fn generation_table_covers_kernel_7_2_ids() {
        use crate::alsa_ctl::focusrite_generation;
        assert_eq!(focusrite_generation("1235:8212"), "Gen 3");
        assert_eq!(focusrite_generation("1235:8211"), "Gen 3");
        assert_eq!(focusrite_generation("1235:821a"), "Gen 4");
        assert_eq!(focusrite_generation("1235:821d"), "Gen 4 (FCP)");
        assert_eq!(focusrite_generation("1235:8203"), "Gen 2");
        assert_eq!(focusrite_generation("1235:8217"), "Vocaster");
        assert_eq!(focusrite_generation("1235:820a"), "Clarett+");
    }
}

/// Synthetic controls shared by unit tests in this crate (no hardware needed).
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub(crate) fn mk_ctrl(
        numid: u32,
        name: &str,
        kind: ControlKind,
        items: &[&str],
        val: i64,
    ) -> AlsaControl {
        AlsaControl {
            numid,
            name: name.into(),
            iface: "MIXER".into(),
            kind,
            count: 1,
            min: 0,
            max: match kind {
                ControlKind::Bool => 1,
                ControlKind::Enum => items.len().saturating_sub(1) as i64,
                ControlKind::Int => 184,
            },
            db_min: if kind == ControlKind::Int {
                Some(-80.0)
            } else {
                None
            },
            db_max: if kind == ControlKind::Int {
                Some(12.0)
            } else {
                None
            },
            current_db_tlv: None,
            unity_raw: None,
            tlv_readable: false,
            items: items.iter().map(|s| s.to_string()).collect(),
            values: vec![val],
            readable: true,
            value_valid: true,
            metadata_complete: true,
            rw: true,
        }
    }

    /// A Scarlett 4i4 Gen 3 shaped model: one input with Inst/Air, 48V off,
    /// one output with volume and mute, routing, a one-cell matrix, sync and
    /// a two-slot level meter.
    pub(crate) fn model_4i4() -> ScarlettModel {
        let card = CardEntry {
            index: 1,
            alsa_id: "USB".into(),
            name: "Scarlett 4i4 USB".into(),
            usb_id: "1235:8212".into(),
            generation: "Gen 3".into(),
        };
        let items = &["Off", "Analogue 1", "Analogue 2", "Mix A", "PCM 1", "PCM 2"];
        let mut meter = mk_ctrl(90, "Level Meter", ControlKind::Int, &[], 0);
        meter.count = 2;
        meter.max = 4095;
        meter.values = vec![0, 0];
        meter.rw = false;
        let list = vec![
            mk_ctrl(
                1,
                "Line In 1 Gain Capture Volume",
                ControlKind::Int,
                &[],
                40,
            ),
            mk_ctrl(
                6,
                "Line 01 (Monitor L) Playback Volume",
                ControlKind::Int,
                &[],
                127,
            ),
            mk_ctrl(7, "Line 01 Mute Playback Switch", ControlKind::Bool, &[], 0),
            mk_ctrl(
                14,
                "Line In 1 Level Capture Enum",
                ControlKind::Enum,
                &["Line", "Inst"],
                0,
            ),
            mk_ctrl(
                18,
                "Line In 1 Air Capture Switch",
                ControlKind::Bool,
                &[],
                0,
            ),
            mk_ctrl(
                20,
                "Line In 1-2 Phantom Power Capture Switch",
                ControlKind::Bool,
                &[],
                0,
            ),
            mk_ctrl(
                22,
                "Analogue Output 01 Playback Enum",
                ControlKind::Enum,
                items,
                0,
            ),
            mk_ctrl(
                26,
                "Mixer Input 01 Capture Enum",
                ControlKind::Enum,
                items,
                4,
            ),
            mk_ctrl(
                40,
                "Mix A Input 01 Playback Volume",
                ControlKind::Int,
                &[],
                0,
            ),
            mk_ctrl(
                89,
                "Sync Status",
                ControlKind::Enum,
                &["Unlocked", "Locked"],
                1,
            ),
            meter,
        ];
        ScarlettModel::build(card, StreamRateInfo::default(), list)
    }

    /// The README's demo: a whole Scarlett 4i4 Gen 3, with the control names
    /// the kernel driver gives a real one, in a state chosen to show the UI
    /// off (48V live, Inst and Air on, headphones muted, a worked mix). It is
    /// not a capture: `make screenshots` renders it, so the pictures can be
    /// regenerated without hardware and never write to a device.
    pub(crate) fn model_demo() -> ScarlettModel {
        let card = CardEntry {
            index: 1,
            alsa_id: "USB".into(),
            name: "Scarlett 4i4 USB".into(),
            usb_id: "1235:8212".into(),
            generation: "Gen 3".into(),
        };
        let sources = &[
            "Off",
            "Analogue 1",
            "Analogue 2",
            "Analogue 3",
            "Analogue 4",
            "Mix A",
            "Mix B",
            "Mix C",
            "Mix D",
            "Mix E",
            "Mix F",
            "PCM 1",
            "PCM 2",
            "PCM 3",
            "PCM 4",
        ];
        let src = |name: &str| sources.iter().position(|s| *s == name).unwrap() as i64;
        let mut list = vec![mk_ctrl(4, "Firmware Version", ControlKind::Int, &[], 1605)];

        // Outputs: monitors at unity, headphones at −12 dB and muted.
        for (i, (label, raw, muted)) in [
            ("Line 01 (Monitor L)", 127, 0),
            ("Line 02 (Monitor R)", 127, 0),
            ("Line 03 (Headphones L)", 115, 1),
            ("Line 04 (Headphones R)", 115, 1),
        ]
        .into_iter()
        .enumerate()
        {
            let n = i as u32;
            let mut vol = mk_ctrl(
                6 + 2 * n,
                &format!("{label} Playback Volume"),
                ControlKind::Int,
                &[],
                raw,
            );
            vol.max = 127;
            vol.db_min = Some(-127.0);
            vol.db_max = Some(0.0);
            list.push(vol);
            list.push(mk_ctrl(
                7 + 2 * n,
                &format!("Line {:02} Mute Playback Switch", n + 1),
                ControlKind::Bool,
                &[],
                muted,
            ));
        }

        // Inputs: 1 is an instrument with Air; 2 is a padded line source.
        for (n, level, air, pad) in [(1u32, 1, 1, 0), (2, 0, 0, 1)] {
            list.push(mk_ctrl(
                13 + n,
                &format!("Line In {n} Level Capture Enum"),
                ControlKind::Enum,
                &["Line", "Inst"],
                level,
            ));
            list.push(mk_ctrl(
                17 + n,
                &format!("Line In {n} Air Capture Switch"),
                ControlKind::Bool,
                &[],
                air,
            ));
            list.push(mk_ctrl(
                15 + n,
                &format!("Line In {n} Pad Capture Switch"),
                ControlKind::Bool,
                &[],
                pad,
            ));
        }
        list.push(mk_ctrl(
            20,
            "Line In 1-2 Phantom Power Capture Switch",
            ControlKind::Bool,
            &[],
            1,
        ));
        list.push(mk_ctrl(
            21,
            "Phantom Power Persistence Capture Switch",
            ControlKind::Bool,
            &[],
            1,
        ));

        // Routing: monitors hear mix A/B, headphones hear the cue mix C/D.
        for (i, from) in ["Mix A", "Mix B", "Mix C", "Mix D"].into_iter().enumerate() {
            list.push(mk_ctrl(
                22 + i as u32,
                &format!("Analogue Output {:02} Playback Enum", i + 1),
                ControlKind::Enum,
                sources,
                src(from),
            ));
        }
        let mixer_in = [
            "Analogue 1",
            "Analogue 2",
            "PCM 1",
            "PCM 2",
            "PCM 3",
            "PCM 4",
            "Off",
            "Off",
        ];
        for (i, from) in mixer_in.into_iter().enumerate() {
            list.push(mk_ctrl(
                26 + i as u32,
                &format!("Mixer Input {:02} Capture Enum", i + 1),
                ControlKind::Enum,
                sources,
                src(from),
            ));
        }
        let capture = ["Analogue 1", "Analogue 2", "Mix A", "Mix B", "Off", "Off"];
        for (i, from) in capture.into_iter().enumerate() {
            list.push(mk_ctrl(
                34 + i as u32,
                &format!("PCM {:02} Capture Enum", i + 1),
                ControlKind::Enum,
                sources,
                src(from),
            ));
        }

        // The mix matrix, in dB per mixer input (None is −∞). A and B are a
        // stereo monitor mix; C and D boost the instrument for a cue mix.
        let off = None;
        let matrix: [(char, [Option<f64>; 8]); 6] = [
            (
                'A',
                [
                    Some(0.0),
                    Some(-6.0),
                    Some(-10.0),
                    off,
                    Some(-12.0),
                    off,
                    off,
                    off,
                ],
            ),
            (
                'B',
                [
                    Some(0.0),
                    Some(-6.0),
                    off,
                    Some(-10.0),
                    off,
                    Some(-12.0),
                    off,
                    off,
                ],
            ),
            ('C', [Some(3.0), off, Some(-20.0), off, off, off, off, off]),
            ('D', [Some(3.0), off, off, Some(-20.0), off, off, off, off]),
            ('E', [off; 8]),
            ('F', [off; 8]),
        ];
        for (m, (mix, levels)) in matrix.into_iter().enumerate() {
            for (i, db) in levels.into_iter().enumerate() {
                let mut cell = mk_ctrl(
                    40 + (m * 8 + i) as u32,
                    &format!("Mix {mix} Input {:02} Playback Volume", i + 1),
                    ControlKind::Int,
                    &[],
                    0,
                );
                if let Some(db) = db {
                    cell.values = vec![cell.db_to_raw(db).unwrap()];
                }
                list.push(cell);
            }
        }

        list.push(mk_ctrl(
            89,
            "Sync Status",
            ControlKind::Enum,
            &["Unlocked", "Locked"],
            1,
        ));
        list.push(mk_ctrl(90, "Standalone Switch", ControlKind::Bool, &[], 1));
        // One meter per route: 4 outputs, 8 mixer inputs, 6 capture channels.
        let mut meter = mk_ctrl(88, "Level Meter", ControlKind::Int, &[], 0);
        meter.count = 18;
        meter.max = 4095;
        meter.values = vec![0; 18];
        meter.rw = false;
        list.push(meter);

        let stream = StreamRateInfo {
            active_rate: Some(48_000),
            ..StreamRateInfo::default()
        };
        ScarlettModel::build(card, stream, list)
    }
}
