use crate::alsa_ctl::{
    AlsaCard, CardEntry, ControlKind, discover_focusrite_cards, read_stream_rate,
};
use crate::model::{RouteGroup, ScarlettModel, meter_to_dbfs};
use crate::pipewire;
use serde::Serialize;
use std::io::{self, BufRead, Write};
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

#[derive(Serialize)]
struct SnapshotPayload<'a> {
    #[serde(rename = "type")]
    msg_type: &'static str,
    connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    card: Option<&'a CardEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    firmware: Option<i64>,
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
struct ToggleSummary {
    numid: u32,
    label: String,
    kind: ControlKind,
    value: i64,
    text: String,
    active: bool,
    items: Vec<String>,
    rw: bool,
}

#[derive(Serialize)]
struct InputSummary {
    index: u32,
    label: String,
    toggles: Vec<ToggleSummary>,
}

#[derive(Serialize)]
struct OutputSummary {
    index: u32,
    label: String,
    volume_numid: Option<u32>,
    db: Option<f64>,
    mute_numid: Option<u32>,
    muted: bool,
    route_numid: Option<u32>,
    route: String,
    route_summary: String,
    route_items: Vec<String>,
    route_val: i64,
}

#[derive(Serialize)]
struct RouteSummary {
    numid: u32,
    group: String,
    label: String,
    current: String,
    summary: String,
    items: Vec<String>,
    val: i64,
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
}

fn toggle_from(model: &ScarlettModel, numid: u32, label: &str) -> Option<ToggleSummary> {
    let c = model.controls.get(&numid)?;
    let (active, text) = match c.kind {
        ControlKind::Bool => (c.bool_val(), if c.bool_val() { "On" } else { "Off" }.into()),
        ControlKind::Enum => (c.val() > 0, c.enum_label().to_string()),
        ControlKind::Int => (
            c.val() > c.min,
            c.current_db()
                .map(|db| format!("{db:.1} dB"))
                .unwrap_or_else(|| c.val().to_string()),
        ),
    };
    Some(ToggleSummary {
        numid,
        label: label.into(),
        kind: c.kind,
        value: c.val(),
        text,
        active,
        items: c.items.clone(),
        rw: c.rw,
    })
}

fn build_snapshot(model: &ScarlettModel) -> SnapshotPayload<'_> {
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
            if let Some(id) = inp.link_numid
                && let Some(t) = toggle_from(model, id, "LINK")
            {
                toggles.push(t);
            }
            InputSummary {
                index: inp.index,
                label: inp.label.clone(),
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
            let db = out
                .volume_numid
                .and_then(|id| model.controls.get(&id))
                .and_then(|c| c.current_db());
            let muted = out
                .mute_numid
                .and_then(|id| model.controls.get(&id))
                .map(|c| c.bool_val())
                .unwrap_or(false);
            let route_entry = model
                .routes
                .iter()
                .find(|r| r.group == RouteGroup::AnalogueOut && r.channel == out.index);
            let (route_numid, route, route_summary, route_items, route_val) = match route_entry {
                Some(r) => {
                    let c = model.controls.get(&r.numid);
                    (
                        Some(r.numid),
                        r.current.clone(),
                        r.summary.clone(),
                        c.map(|x| x.items.clone()).unwrap_or_default(),
                        c.map(|x| x.val()).unwrap_or(0),
                    )
                }
                None => (None, String::new(), String::new(), Vec::new(), 0),
            };
            OutputSummary {
                index: out.index,
                label: out.label.clone(),
                volume_numid: out.volume_numid,
                db,
                mute_numid: out.mute_numid,
                muted,
                route_numid,
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
                val: c.map(|x| x.val()).unwrap_or(0),
            }
        })
        .collect();

    let pw_scarlett_links = pipewire::list_links()
        .map(|ls| ls.iter().filter(|l| l.is_scarlett).count())
        .unwrap_or(0);

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

    SnapshotPayload {
        msg_type: "state",
        connected: true,
        card: Some(&model.card),
        firmware: model.firmware,
        sync_status: model.sync_status.clone(),
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

fn emit_json<T: Serialize>(val: &T) {
    if let Ok(line) = serde_json::to_string(val) {
        let mut out = io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

fn emit_disconnected() {
    emit_json(&serde_json::json!({
        "type": "state",
        "connected": false,
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
    }));
}

pub fn run_once_json(card_override: Option<u32>) -> i32 {
    let cards = discover_focusrite_cards();
    let selected = match card_override {
        Some(idx) => cards.into_iter().find(|c| c.index == idx),
        None => cards.into_iter().next(),
    };
    let Some(entry) = selected else {
        emit_disconnected();
        return 1;
    };
    let Ok(alsa) = AlsaCard::open(entry.index) else {
        emit_disconnected();
        return 1;
    };
    let Ok(ctrls) = alsa.enumerate() else {
        emit_disconnected();
        return 1;
    };
    let stream = read_stream_rate(entry.index);
    let model = ScarlettModel::build(entry, stream, ctrls);
    emit_json(&build_snapshot(&model));
    0
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

    let mut alsa_opt: Option<AlsaCard> = None;
    let mut model_opt: Option<ScarlettModel> = None;
    let mut meters_on = false;
    let mut last_meter = Instant::now();
    let mut last_scan = Instant::now() - Duration::from_secs(10);
    let mut stdin_buf = String::new();
    let mut stdin_reader = io::BufReader::new(io::stdin());

    loop {
        if alsa_opt.is_none() && last_scan.elapsed() >= Duration::from_secs(2) {
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
                let model = ScarlettModel::build(entry, stream, ctrls);
                emit_json(&build_snapshot(&model));
                alsa_opt = Some(alsa);
                model_opt = Some(model);
            } else {
                emit_disconnected();
            }
        }

        let mut pfds = vec![libc::pollfd {
            fd: stdin_fd,
            events: libc::POLLIN,
            revents: 0,
        }];
        if let Some(ref alsa) = alsa_opt {
            pfds.extend(alsa.poll_fds());
        }

        let timeout_ms = if meters_on && alsa_opt.is_some() {
            50
        } else {
            500
        };
        let _ = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, timeout_ms) };

        // Check stdin commands
        if (pfds[0].revents & (libc::POLLIN | libc::POLLHUP)) != 0 {
            loop {
                stdin_buf.clear();
                match stdin_reader.read_line(&mut stdin_buf) {
                    Ok(0) => return 0, // EOF: parent shell exited
                    Ok(_) => {
                        let line = stdin_buf.trim();
                        if line == "meters on" {
                            meters_on = true;
                        } else if line == "meters off" {
                            meters_on = false;
                        } else if line == "refresh" {
                            if let (Some(alsa), Some(model)) = (&alsa_opt, &mut model_opt)
                                && let Ok(ctrls) = alsa.enumerate()
                            {
                                let stream = read_stream_rate(model.card.index);
                                *model = ScarlettModel::build(model.card.clone(), stream, ctrls);
                                emit_json(&build_snapshot(model));
                            } else {
                                emit_disconnected();
                            }
                        } else if let Some(rest) = line.strip_prefix("set ") {
                            let mut parts = rest.split_whitespace();
                            if let (Some(id_s), Some(val_s)) = (parts.next(), parts.next())
                                && let (Ok(numid), Ok(val)) =
                                    (id_s.parse::<u32>(), val_s.parse::<i64>())
                                && let (Some(alsa), Some(model)) = (&alsa_opt, &mut model_opt)
                                && let Some(ctrl) = model.controls.get(&numid).cloned()
                                && alsa.write_single(&ctrl, val).is_ok()
                            {
                                if let Ok(vals) = alsa.read_values(numid, ctrl.kind, ctrl.count)
                                    && let Some(c) = model.controls.get_mut(&numid)
                                {
                                    c.values = vals;
                                }
                                model.recompute_derived();
                                emit_json(&build_snapshot(model));
                            }
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => return 0,
                }
            }
        }

        // Check ALSA events
        if let (Some(alsa), Some(model)) = (&alsa_opt, &mut model_opt) {
            if !alsa.exists() {
                alsa_opt = None;
                model_opt = None;
                emit_disconnected();
                continue;
            }
            match alsa.drain_events() {
                Ok((changed, structural)) => {
                    let meter_id = model.meter_numid;
                    let non_meter: Vec<u32> = changed
                        .into_iter()
                        .filter(|&id| Some(id) != meter_id)
                        .collect();
                    if structural {
                        if let Ok(ctrls) = alsa.enumerate() {
                            let stream = read_stream_rate(model.card.index);
                            *model = ScarlettModel::build(model.card.clone(), stream, ctrls);
                            emit_json(&build_snapshot(model));
                        }
                    } else if !non_meter.is_empty() {
                        for id in non_meter {
                            if let Some(c) = model.controls.get_mut(&id)
                                && let Ok(vals) = alsa.read_values(id, c.kind, c.count)
                            {
                                c.values = vals;
                            }
                        }
                        model.stream = read_stream_rate(model.card.index);
                        model.recompute_derived();
                        emit_json(&build_snapshot(model));
                    }
                }
                Err(_) => {
                    alsa_opt = None;
                    model_opt = None;
                    emit_disconnected();
                    continue;
                }
            }
        }

        // Stream hardware meters when requested (panel open)
        if meters_on
            && last_meter.elapsed() >= Duration::from_millis(50)
            && let (Some(alsa), Some(model)) = (&alsa_opt, &model_opt)
            && let Some(mid) = model.meter_numid
            && let Some(mc) = model.controls.get(&mid)
            && let Ok(raw) = alsa.read_values(mid, mc.kind, mc.count)
        {
            last_meter = Instant::now();
            let dbfs = raw.iter().map(|&v| meter_to_dbfs(v)).collect();
            emit_json(&MeterPayload {
                msg_type: "meters",
                raw,
                dbfs,
            });
        }
    }
}
