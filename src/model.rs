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
    pub meter_channels: Vec<MeterChannel>,
}

impl ScarlettModel {
    pub fn build(card: CardEntry, stream: StreamRateInfo, list: Vec<AlsaControl>) -> Self {
        let mut controls = BTreeMap::new();
        for c in list {
            controls.insert(c.numid, c);
        }

        let mut firmware = None;
        let mut sync_status = String::from("Unknown");
        let mut meter_numid = None;

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
            if name == "Firmware Version" {
                firmware = Some(ctrl.val());
                continue;
            }
            if name == "Sync Status" {
                sync_status = ctrl.enum_label().to_string();
                continue;
            }
            if name == "Level Meter" && ctrl.kind == ControlKind::Int {
                meter_numid = Some(ctrl.numid);
                continue;
            }

            // Input channel controls: "Line In N ..."
            if let Some(rest) = name.strip_prefix("Line In ") {
                if name.contains("Phantom Power") {
                    phantom_numids.push(ctrl.numid);
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
                    if tail.starts_with("Gain") {
                        entry.gain_numid = Some(ctrl.numid);
                        continue;
                    } else if tail.starts_with("Level") {
                        entry.level_numid = Some(ctrl.numid);
                        continue;
                    } else if tail.starts_with("Air") {
                        entry.air_numid = Some(ctrl.numid);
                        continue;
                    } else if tail.starts_with("Pad") {
                        entry.pad_numid = Some(ctrl.numid);
                        continue;
                    } else if tail.starts_with("Safe") {
                        entry.safe_numid = Some(ctrl.numid);
                        continue;
                    } else if tail.starts_with("Autogain") && ctrl.rw {
                        entry.autogain_numid = Some(ctrl.numid);
                        continue;
                    } else if tail.starts_with("Link") {
                        entry.link_numid = Some(ctrl.numid);
                        continue;
                    }
                }
            }

            if name == "Phantom Power Persistence Capture Switch" {
                phantom_numids.push(ctrl.numid);
                continue;
            }

            if name.starts_with("Direct Monitor") && ctrl.rw {
                direct_monitor_numid = Some(ctrl.numid);
                continue;
            }

            // Output volumes & mutes: "Line 01 (Monitor L) Playback Volume" / "Line 01 Mute Playback Switch"
            if let Some(rest) = name.strip_prefix("Line ")
                && !name.starts_with("Line In ")
                && let Some((num_str, tail)) = rest.split_once(' ')
                && let Ok(ch) = num_str.parse::<u32>()
            {
                let entry = output_map.entry(ch).or_insert_with(|| OutputChannel {
                    index: ch,
                    label: format!("Line {ch:02}"),
                    volume_numid: None,
                    mute_numid: None,
                });
                if tail.contains("Playback Volume") {
                    if let Some(start) = tail.find('(')
                        && let Some(end) = tail[start + 1..].find(')')
                    {
                        let descr = &tail[start + 1..start + 1 + end];
                        entry.label = format!("Line {ch:02} ({descr})");
                    }
                    entry.volume_numid = Some(ctrl.numid);
                    continue;
                } else if tail.contains("Mute") {
                    entry.mute_numid = Some(ctrl.numid);
                    continue;
                }
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

            // Other writable device switches/enums/ints
            if ctrl.rw
                && matches!(
                    name,
                    "Standalone Switch"
                        | "MSD Mode Switch"
                        | "Front Panel Brightness"
                        | "Front Panel Sleep Time"
                        | "Speaker Switching Playback Enum"
                        | "Talkback Playback Switch"
                        | "Dim Playback Switch"
                        | "Mute Playback Switch"
                        | "Master Playback Volume"
                        | "PCM Input Switch"
                )
            {
                device_numids.push(ctrl.numid);
            }
        }

        // Sort routes in scarlett2_ports order: AnalogueOut -> SpdifOut -> AdatOut -> MixerIn -> DspIn -> PcmCapture
        routes.sort_by_key(|r| (r.group, r.channel));

        // Enrich AnalogueOut labels from OutputChannel hardware descriptions (Monitor L, Headphones L, etc.)
        for r in &mut routes {
            if r.group == RouteGroup::AnalogueOut
                && let Some(out) = output_map.get(&r.channel)
            {
                r.label = out.label.clone();
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
            meter_channels: Vec::new(),
        };
        model.recompute_derived();
        model
    }

    pub fn recompute_derived(&mut self) {
        // Refresh sync_status
        for c in self.controls.values() {
            if c.name == "Sync Status" {
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
                        && c.val() > c.min
                    {
                        let src = mixer_in_sources
                            .get(&in_idx)
                            .map(|s| s.as_str())
                            .unwrap_or("Off");
                        if src != "Off" {
                            let db = c.current_db().unwrap_or(0.0);
                            if db.abs() < 0.25 {
                                active_parts.push(src.to_string());
                            } else {
                                active_parts.push(format!("{src} ({db:+.1} dB)"));
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

        // Map Level Meter slots in scarlett2_ports destination order.
        let meter_count = self
            .meter_numid
            .and_then(|id| self.controls.get(&id))
            .map(|c| c.count as usize)
            .unwrap_or(0);

        self.meter_channels.clear();
        if meter_count > 0 {
            if self.routes.len() == meter_count {
                for (i, r) in self.routes.iter().enumerate() {
                    self.meter_channels.push(MeterChannel {
                        meter_index: i,
                        label: r.label.clone(),
                        source: r.current.clone(),
                        group: r.group,
                    });
                }
            } else {
                // Fallback for Solo/2i2 or devices whose meter count differs from routes
                let mut i = 0;
                for r in &self.routes {
                    if i >= meter_count {
                        break;
                    }
                    self.meter_channels.push(MeterChannel {
                        meter_index: i,
                        label: r.label.clone(),
                        source: r.current.clone(),
                        group: r.group,
                    });
                    i += 1;
                }
                while i < meter_count {
                    self.meter_channels.push(MeterChannel {
                        meter_index: i,
                        label: format!("Meter {:02}", i + 1),
                        source: String::new(),
                        group: RouteGroup::AnalogueOut,
                    });
                    i += 1;
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

    pub fn unity_raw_for(&self, numid: u32) -> i64 {
        let Some(c) = self.controls.get(&numid) else {
            return 0;
        };
        c.db_to_raw(0.0).unwrap_or(c.max)
    }

    pub fn is_locked(&self) -> bool {
        self.sync_status.eq_ignore_ascii_case("Locked")
    }
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

/// Convert 12-bit Scarlett hardware meter value (0..4095) to dBFS.
pub fn meter_to_dbfs(raw: i64) -> f64 {
    if raw <= 0 {
        return -96.0;
    }
    (20.0 * ((raw as f64) / 4095.0).log10()).clamp(-96.0, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_ctrl(numid: u32, name: &str, kind: ControlKind, items: &[&str], val: i64) -> AlsaControl {
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
            items: items.iter().map(|s| s.to_string()).collect(),
            values: vec![val],
            rw: true,
        }
    }

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
        assert_eq!(model.inputs.len(), 1);
        assert_eq!(model.outputs.len(), 1);
        assert_eq!(model.outputs[0].label, "Line 01 (Monitor L)");
        assert_eq!(model.matrix.mixes, vec!['A']);
        assert_eq!(model.matrix.inputs, vec![1]);
        assert_eq!(model.unity_raw_for(40), 160);
        assert_eq!(model.routes[0].summary, "Mix A ← PCM 1");
        assert!(model.is_locked());
    }

    #[test]
    fn handles_gen4_air_enum_and_gain() {
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
                "Direct Monitor Playback Enum",
                ControlKind::Enum,
                &["Off", "Mono", "Stereo"],
                1,
            ),
        ];
        let model = ScarlettModel::build(card, StreamRateInfo::default(), list);
        assert_eq!(model.inputs.len(), 1);
        assert!(model.inputs[0].gain_numid.is_some());
        assert!(model.inputs[0].air_numid.is_some());
        assert!(model.inputs[0].safe_numid.is_some());
        assert!(model.direct_monitor_numid.is_some());
    }

    #[test]
    fn handles_large_fcp_names_and_digital_outputs() {
        let card = CardEntry {
            index: 2,
            alsa_id: "FCP".into(),
            name: "Scarlett 18i20 4th Gen".into(),
            usb_id: "1235:821d".into(),
            generation: "Gen 4 (FCP)".into(),
        };
        let items = &["Off", "Analogue 1", "PCM 1", "Mix A"];
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
            ), // -6.0 dB
            mk_ctrl(6, "PCM 1 Capture Enum", ControlKind::Enum, items, 1),
        ];
        let model = ScarlettModel::build(card, StreamRateInfo::default(), list);
        assert_eq!(model.routes.len(), 5);
        assert_eq!(model.routes[0].group, RouteGroup::AnalogueOut);
        assert_eq!(model.routes[0].summary, "Mix A ← Analogue 1 (-6.0 dB)");
        assert_eq!(model.routes[1].group, RouteGroup::SpdifOut);
        assert_eq!(model.routes[2].group, RouteGroup::AdatOut);
        assert_eq!(model.routes[3].group, RouteGroup::MixerIn);
        assert_eq!(model.routes[4].group, RouteGroup::PcmCapture);
    }

    #[test]
    fn meter_and_db_conversions_match_hardware_scale() {
        assert!((meter_to_dbfs(0) - (-96.0)).abs() < 0.01);
        assert!((meter_to_dbfs(4095) - 0.0).abs() < 0.01);
        assert!((meter_to_dbfs(2048) - (-6.02)).abs() < 0.05);

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
