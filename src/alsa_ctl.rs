use libc::{c_char, c_int, c_long, c_uint, c_void, pollfd};
use serde::Serialize;
use std::ffi::{CStr, CString};
use std::fs;
use std::path::Path;

const SND_CTL_ELEM_TYPE_BOOLEAN: c_int = 1;
const SND_CTL_ELEM_TYPE_INTEGER: c_int = 2;
const SND_CTL_ELEM_TYPE_ENUMERATED: c_int = 3;

const SND_CTL_ELEM_IFACE_CARD: c_int = 0;
const SND_CTL_ELEM_IFACE_HWDEP: c_int = 1;
const SND_CTL_ELEM_IFACE_MIXER: c_int = 2;
const SND_CTL_ELEM_IFACE_PCM: c_int = 3;

const SND_CTL_EVENT_MASK_REMOVE: c_uint = !0;
const SND_CTL_EVENT_MASK_ADD: c_uint = 1 << 2;

#[link(name = "asound")]
unsafe extern "C" {
    fn snd_ctl_open(ctlp: *mut *mut c_void, name: *const c_char, mode: c_int) -> c_int;
    fn snd_ctl_close(ctl: *mut c_void) -> c_int;
    fn snd_ctl_subscribe_events(ctl: *mut c_void, subscribe: c_int) -> c_int;
    fn snd_ctl_poll_descriptors_count(ctl: *mut c_void) -> c_int;
    fn snd_ctl_poll_descriptors(ctl: *mut c_void, pfds: *mut pollfd, space: c_uint) -> c_int;
    fn snd_ctl_read(ctl: *mut c_void, event: *mut c_void) -> c_int;

    fn snd_ctl_elem_list_malloc(ptr: *mut *mut c_void) -> c_int;
    fn snd_ctl_elem_list_free(obj: *mut c_void);
    fn snd_ctl_elem_list_alloc_space(obj: *mut c_void, entries: c_uint) -> c_int;
    fn snd_ctl_elem_list_free_space(obj: *mut c_void);
    fn snd_ctl_elem_list(ctl: *mut c_void, list: *mut c_void) -> c_int;
    fn snd_ctl_elem_list_get_count(obj: *const c_void) -> c_uint;
    fn snd_ctl_elem_list_get_numid(obj: *const c_void, idx: c_uint) -> c_uint;

    fn snd_ctl_elem_info_malloc(ptr: *mut *mut c_void) -> c_int;
    fn snd_ctl_elem_info_free(obj: *mut c_void);
    fn snd_ctl_elem_info_set_numid(obj: *mut c_void, val: c_uint);
    fn snd_ctl_elem_info(ctl: *mut c_void, info: *mut c_void) -> c_int;
    fn snd_ctl_elem_info_get_type(obj: *const c_void) -> c_int;
    fn snd_ctl_elem_info_get_interface(obj: *const c_void) -> c_int;
    fn snd_ctl_elem_info_get_name(obj: *const c_void) -> *const c_char;
    fn snd_ctl_elem_info_get_count(obj: *const c_void) -> c_uint;
    fn snd_ctl_elem_info_get_min(obj: *const c_void) -> c_long;
    fn snd_ctl_elem_info_get_max(obj: *const c_void) -> c_long;
    fn snd_ctl_elem_info_get_items(obj: *const c_void) -> c_uint;
    fn snd_ctl_elem_info_set_item(obj: *mut c_void, val: c_uint);
    fn snd_ctl_elem_info_get_item_name(obj: *const c_void) -> *const c_char;
    fn snd_ctl_elem_info_is_writable(obj: *const c_void) -> c_int;
    fn snd_ctl_elem_info_is_readable(obj: *const c_void) -> c_int;
    fn snd_ctl_elem_info_is_tlv_readable(obj: *const c_void) -> c_int;
    fn snd_ctl_elem_info_get_id(obj: *const c_void, ptr: *mut c_void);

    fn snd_ctl_elem_id_malloc(ptr: *mut *mut c_void) -> c_int;
    fn snd_ctl_elem_id_free(obj: *mut c_void);
    fn snd_ctl_get_dB_range(
        ctl: *mut c_void,
        id: *const c_void,
        min: *mut c_long,
        max: *mut c_long,
    ) -> c_int;

    fn snd_ctl_elem_value_malloc(ptr: *mut *mut c_void) -> c_int;
    fn snd_ctl_elem_value_free(obj: *mut c_void);
    fn snd_ctl_elem_value_set_numid(obj: *mut c_void, val: c_uint);
    fn snd_ctl_elem_read(ctl: *mut c_void, data: *mut c_void) -> c_int;
    fn snd_ctl_elem_write(ctl: *mut c_void, data: *mut c_void) -> c_int;
    fn snd_ctl_elem_value_get_boolean(obj: *const c_void, idx: c_uint) -> c_int;
    fn snd_ctl_elem_value_set_boolean(obj: *mut c_void, idx: c_uint, val: c_long);
    fn snd_ctl_elem_value_get_integer(obj: *const c_void, idx: c_uint) -> c_long;
    fn snd_ctl_elem_value_set_integer(obj: *mut c_void, idx: c_uint, val: c_long);
    fn snd_ctl_elem_value_get_enumerated(obj: *const c_void, idx: c_uint) -> c_uint;
    fn snd_ctl_elem_value_set_enumerated(obj: *mut c_void, idx: c_uint, val: c_uint);

    fn snd_ctl_event_malloc(ptr: *mut *mut c_void) -> c_int;
    fn snd_ctl_event_free(obj: *mut c_void);
    fn snd_ctl_event_elem_get_numid(obj: *const c_void) -> c_uint;
    fn snd_ctl_event_elem_get_mask(obj: *const c_void) -> c_uint;
}

#[derive(Debug, Clone, Serialize)]
pub struct CardEntry {
    pub index: u32,
    pub alsa_id: String,
    pub name: String,
    pub usb_id: String,
    pub generation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlKind {
    Bool,
    Int,
    Enum,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlsaControl {
    pub numid: u32,
    pub name: String,
    pub iface: String,
    pub kind: ControlKind,
    pub count: u32,
    pub min: i64,
    pub max: i64,
    pub db_min: Option<f64>,
    pub db_max: Option<f64>,
    pub items: Vec<String>,
    pub values: Vec<i64>,
    pub rw: bool,
}

impl AlsaControl {
    pub fn val(&self) -> i64 {
        self.values.first().copied().unwrap_or(0)
    }

    pub fn bool_val(&self) -> bool {
        self.val() != 0
    }

    pub fn enum_label(&self) -> &str {
        let idx = self.val().max(0) as usize;
        self.items.get(idx).map(|s| s.as_str()).unwrap_or("")
    }

    pub fn raw_to_db(&self, raw: i64) -> Option<f64> {
        let (lo, hi) = (self.db_min?, self.db_max?);
        if self.max == self.min {
            return Some(lo);
        }
        Some(lo + (raw - self.min) as f64 * (hi - lo) / (self.max - self.min) as f64)
    }

    pub fn current_db(&self) -> Option<f64> {
        self.raw_to_db(self.val())
    }

    pub fn db_to_raw(&self, db: f64) -> Option<i64> {
        let (lo, hi) = (self.db_min?, self.db_max?);
        if (hi - lo).abs() < f64::EPSILON {
            return Some(self.min);
        }
        let raw = self.min as f64 + (db - lo) * (self.max - self.min) as f64 / (hi - lo);
        Some((raw.round() as i64).clamp(self.min, self.max))
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StreamRateInfo {
    pub active_rate: Option<u32>,
    pub playback_running: bool,
    pub capture_running: bool,
    pub supported_rates: Vec<u32>,
}

/// Kernel 7.2 `mixer_quirks.c` / `mixer_scarlett2.c` Focusrite USB PID table.
pub fn focusrite_generation(usb_id: &str) -> &'static str {
    let pid = usb_id.split(':').nth(1).unwrap_or("").to_ascii_lowercase();
    match pid.as_str() {
        "8010" => "Forte",
        "8012" | "8002" | "8004" | "8014" | "800c" => "Gen 1",
        "8201" | "8203" | "8204" => "Gen 2",
        "8210" | "8211" | "8212" | "8213" | "8214" | "8215" => "Gen 3",
        "8216" | "8217" => "Vocaster",
        "8218" | "8219" | "821a" => "Gen 4",
        "821b" | "821c" | "821d" => "Gen 4 (FCP)",
        "821e" => "ISA (FCP)",
        "8206" | "8207" | "8208" => "Clarett USB",
        "820a" | "820b" | "820c" => "Clarett+",
        _ => "Focusrite USB",
    }
}

pub fn discover_focusrite_cards() -> Vec<CardEntry> {
    let mut cards = Vec::new();
    let proc_cards = fs::read_to_string("/proc/asound/cards").unwrap_or_default();

    let Ok(entries) = fs::read_dir("/proc/asound") else {
        return cards;
    };
    let mut indices = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if let Some(rest) = s.strip_prefix("card")
            && let Ok(idx) = rest.parse::<u32>()
        {
            indices.push(idx);
        }
    }
    indices.sort_unstable();

    for idx in indices {
        let base = format!("/proc/asound/card{idx}");
        let Ok(usbid_raw) = fs::read_to_string(format!("{base}/usbid")) else {
            continue;
        };
        let usb_id = usbid_raw.trim().to_ascii_lowercase();
        if !usb_id.starts_with("1235:") {
            continue;
        }
        let alsa_id = fs::read_to_string(format!("{base}/id"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let name = parse_card_name(&proc_cards, idx).unwrap_or_else(|| "Focusrite Scarlett".into());
        let generation = focusrite_generation(&usb_id).to_string();
        cards.push(CardEntry {
            index: idx,
            alsa_id,
            name,
            usb_id,
            generation,
        });
    }
    cards
}

fn parse_card_name(proc_cards: &str, card_idx: u32) -> Option<String> {
    let prefix = format!("{card_idx} [");
    for line in proc_cards.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(&prefix)
            && let Some((_, rest)) = trimmed.split_once(" - ")
        {
            return Some(rest.trim().to_string());
        }
    }
    None
}

pub fn read_stream_rate(card_idx: u32) -> StreamRateInfo {
    let mut info = StreamRateInfo::default();
    let path = format!("/proc/asound/card{card_idx}/stream0");
    let Ok(text) = fs::read_to_string(&path) else {
        return info;
    };

    let mut section = "";
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "Playback:" {
            section = "playback";
        } else if trimmed == "Capture:" {
            section = "capture";
        } else if let Some(st) = trimmed.strip_prefix("Status:") {
            let running = st.trim() == "Running";
            match section {
                "playback" => info.playback_running = running,
                "capture" => info.capture_running = running,
                _ => {}
            }
        } else if let Some(rest) = trimmed.strip_prefix("Momentary freq =") {
            if info.active_rate.is_none()
                && let Some(hz_str) = rest.split_whitespace().next()
                && let Ok(hz) = hz_str.parse::<u32>()
            {
                info.active_rate = Some(hz);
            }
        } else if let Some(rest) = trimmed.strip_prefix("Rates:")
            && info.supported_rates.is_empty()
        {
            for part in rest.split(',') {
                if let Ok(r) = part.trim().parse::<u32>() {
                    info.supported_rates.push(r);
                }
            }
        }
    }
    info
}

pub struct AlsaCard {
    pub card_index: u32,
    handle: *mut c_void,
    info: *mut c_void,
    value: *mut c_void,
    elem_id: *mut c_void,
    event: *mut c_void,
}

impl AlsaCard {
    pub fn open(card_index: u32) -> Result<Self, String> {
        let dev = CString::new(format!("hw:{card_index}")).map_err(|e| e.to_string())?;
        let mut handle: *mut c_void = std::ptr::null_mut();
        let rc = unsafe { snd_ctl_open(&mut handle, dev.as_ptr(), 0) };
        if rc < 0 || handle.is_null() {
            return Err(format!("snd_ctl_open(hw:{card_index}) failed: {rc}"));
        }

        let mut info = std::ptr::null_mut();
        let mut value = std::ptr::null_mut();
        let mut elem_id = std::ptr::null_mut();
        let mut event = std::ptr::null_mut();
        unsafe {
            snd_ctl_elem_info_malloc(&mut info);
            snd_ctl_elem_value_malloc(&mut value);
            snd_ctl_elem_id_malloc(&mut elem_id);
            snd_ctl_event_malloc(&mut event);
            let _ = snd_ctl_subscribe_events(handle, 1);
        }

        Ok(Self {
            card_index,
            handle,
            info,
            value,
            elem_id,
            event,
        })
    }

    pub fn exists(&self) -> bool {
        Path::new(&format!("/proc/asound/card{}", self.card_index)).exists()
    }

    pub fn poll_fds(&self) -> Vec<pollfd> {
        unsafe {
            let count = snd_ctl_poll_descriptors_count(self.handle);
            if count <= 0 {
                return Vec::new();
            }
            let mut fds = vec![
                pollfd {
                    fd: -1,
                    events: 0,
                    revents: 0,
                };
                count as usize
            ];
            let filled = snd_ctl_poll_descriptors(self.handle, fds.as_mut_ptr(), count as c_uint);
            if filled > 0 {
                fds.truncate(filled as usize);
                fds
            } else {
                Vec::new()
            }
        }
    }

    /// Drain pending ALSA control events. Returns (changed_numids, structural_change).
    pub fn drain_events(&self) -> Result<(Vec<u32>, bool), String> {
        let mut changed = Vec::new();
        let mut structural = false;
        loop {
            let mut fds = self.poll_fds();
            if fds.is_empty() {
                break;
            }
            let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 0) };
            if ready <= 0 {
                break;
            }
            let rc = unsafe { snd_ctl_read(self.handle, self.event) };
            if rc == 0 || rc == -libc::EAGAIN {
                break;
            }
            if rc < 0 {
                return Err(format!("ALSA device disconnected ({rc})"));
            }
            let numid = unsafe { snd_ctl_event_elem_get_numid(self.event) };
            let mask = unsafe { snd_ctl_event_elem_get_mask(self.event) };
            if mask == SND_CTL_EVENT_MASK_REMOVE || (mask & SND_CTL_EVENT_MASK_ADD) != 0 {
                structural = true;
            }
            changed.push(numid);
        }
        Ok((changed, structural))
    }

    pub fn enumerate(&self) -> Result<Vec<AlsaControl>, String> {
        let mut list: *mut c_void = std::ptr::null_mut();
        unsafe {
            if snd_ctl_elem_list_malloc(&mut list) < 0 {
                return Err("snd_ctl_elem_list_malloc failed".into());
            }
        }
        let numids = unsafe {
            let rc = snd_ctl_elem_list(self.handle, list);
            if rc < 0 {
                snd_ctl_elem_list_free(list);
                return Err(format!("snd_ctl_elem_list failed: {rc}"));
            }
            let count = snd_ctl_elem_list_get_count(list);
            if count > 0 && snd_ctl_elem_list_alloc_space(list, count) >= 0 {
                let _ = snd_ctl_elem_list(self.handle, list);
                let ids: Vec<u32> = (0..count)
                    .map(|i| snd_ctl_elem_list_get_numid(list, i))
                    .collect();
                snd_ctl_elem_list_free_space(list);
                snd_ctl_elem_list_free(list);
                ids
            } else {
                snd_ctl_elem_list_free(list);
                Vec::new()
            }
        };

        let mut controls = Vec::with_capacity(numids.len());
        for numid in numids {
            if let Some(ctrl) = self.describe_and_read(numid) {
                controls.push(ctrl);
            }
        }
        Ok(controls)
    }

    pub fn describe_and_read(&self, numid: u32) -> Option<AlsaControl> {
        unsafe {
            snd_ctl_elem_info_set_numid(self.info, numid);
            if snd_ctl_elem_info(self.handle, self.info) < 0 {
                return None;
            }
            let etype = snd_ctl_elem_info_get_type(self.info);
            let kind = match etype {
                SND_CTL_ELEM_TYPE_BOOLEAN => ControlKind::Bool,
                SND_CTL_ELEM_TYPE_INTEGER => ControlKind::Int,
                SND_CTL_ELEM_TYPE_ENUMERATED => ControlKind::Enum,
                _ => return None,
            };
            let name_ptr = snd_ctl_elem_info_get_name(self.info);
            if name_ptr.is_null() {
                return None;
            }
            let name = CStr::from_ptr(name_ptr).to_string_lossy().into_owned();
            let iface = match snd_ctl_elem_info_get_interface(self.info) {
                SND_CTL_ELEM_IFACE_CARD => "CARD",
                SND_CTL_ELEM_IFACE_HWDEP => "HWDEP",
                SND_CTL_ELEM_IFACE_MIXER => "MIXER",
                SND_CTL_ELEM_IFACE_PCM => "PCM",
                _ => "OTHER",
            }
            .to_string();
            let count = snd_ctl_elem_info_get_count(self.info);
            let rw = snd_ctl_elem_info_is_writable(self.info) != 0;
            let readable = snd_ctl_elem_info_is_readable(self.info) != 0;

            let (min, max, db_min, db_max, items) = match kind {
                ControlKind::Bool => (0, 1, None, None, Vec::new()),
                ControlKind::Int => {
                    let lo = snd_ctl_elem_info_get_min(self.info) as i64;
                    let hi = snd_ctl_elem_info_get_max(self.info) as i64;
                    let mut d_min = None;
                    let mut d_max = None;
                    if snd_ctl_elem_info_is_tlv_readable(self.info) != 0 {
                        snd_ctl_elem_info_get_id(self.info, self.elem_id);
                        let (mut l, mut h): (c_long, c_long) = (0, 0);
                        if snd_ctl_get_dB_range(self.handle, self.elem_id, &mut l, &mut h) >= 0 {
                            d_min = Some(l as f64 / 100.0);
                            d_max = Some(h as f64 / 100.0);
                        }
                    }
                    (lo, hi, d_min, d_max, Vec::new())
                }
                ControlKind::Enum => {
                    let n_items = snd_ctl_elem_info_get_items(self.info);
                    let mut list = Vec::with_capacity(n_items as usize);
                    for i in 0..n_items {
                        snd_ctl_elem_info_set_item(self.info, i);
                        if snd_ctl_elem_info(self.handle, self.info) >= 0 {
                            let item_ptr = snd_ctl_elem_info_get_item_name(self.info);
                            if !item_ptr.is_null() {
                                list.push(CStr::from_ptr(item_ptr).to_string_lossy().into_owned());
                            }
                        }
                    }
                    (0, (list.len().saturating_sub(1)) as i64, None, None, list)
                }
            };

            let values = if readable {
                self.read_values(numid, kind, count).unwrap_or_default()
            } else {
                Vec::new()
            };

            Some(AlsaControl {
                numid,
                name,
                iface,
                kind,
                count,
                min,
                max,
                db_min,
                db_max,
                items,
                values,
                rw,
            })
        }
    }

    pub fn read_values(
        &self,
        numid: u32,
        kind: ControlKind,
        count: u32,
    ) -> Result<Vec<i64>, String> {
        unsafe {
            snd_ctl_elem_value_set_numid(self.value, numid);
            let rc = snd_ctl_elem_read(self.handle, self.value);
            if rc < 0 {
                return Err(format!("snd_ctl_elem_read({numid}) failed: {rc}"));
            }
            let mut out = Vec::with_capacity(count as usize);
            for i in 0..count {
                let v = match kind {
                    ControlKind::Bool => snd_ctl_elem_value_get_boolean(self.value, i) as i64,
                    ControlKind::Int => snd_ctl_elem_value_get_integer(self.value, i) as i64,
                    ControlKind::Enum => snd_ctl_elem_value_get_enumerated(self.value, i) as i64,
                };
                out.push(v);
            }
            Ok(out)
        }
    }

    pub fn write_single(&self, ctrl: &AlsaControl, val: i64) -> Result<(), String> {
        if !ctrl.rw {
            return Err(format!("control '{}' is read-only", ctrl.name));
        }
        let clamped = val.clamp(ctrl.min, ctrl.max);
        unsafe {
            snd_ctl_elem_value_set_numid(self.value, ctrl.numid);
            let _ = snd_ctl_elem_read(self.handle, self.value);
            match ctrl.kind {
                ControlKind::Bool => {
                    snd_ctl_elem_value_set_boolean(self.value, 0, if clamped != 0 { 1 } else { 0 });
                }
                ControlKind::Int => {
                    snd_ctl_elem_value_set_integer(self.value, 0, clamped as c_long);
                }
                ControlKind::Enum => {
                    snd_ctl_elem_value_set_enumerated(self.value, 0, clamped as c_uint);
                }
            }
            let rc = snd_ctl_elem_write(self.handle, self.value);
            if rc < 0 {
                return Err(format!("snd_ctl_elem_write({}) failed: {rc}", ctrl.name));
            }
        }
        Ok(())
    }
}

impl Drop for AlsaCard {
    fn drop(&mut self) {
        unsafe {
            if !self.event.is_null() {
                snd_ctl_event_free(self.event);
            }
            if !self.elem_id.is_null() {
                snd_ctl_elem_id_free(self.elem_id);
            }
            if !self.value.is_null() {
                snd_ctl_elem_value_free(self.value);
            }
            if !self.info.is_null() {
                snd_ctl_elem_info_free(self.info);
            }
            if !self.handle.is_null() {
                snd_ctl_close(self.handle);
            }
        }
    }
}
