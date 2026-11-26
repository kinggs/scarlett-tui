# Hardware Testing Playbook (Agent-First)

This document is written so either a person or a coding agent can verify
`scarlett-tui` against any Focusrite Scarlett, Clarett, or Vocaster interface in
under two minutes, without modifying system audio configuration.

## Safety guarantees

- Read-only discovery (`--list` and `--json`) opens only the ALSA control
  interface (`snd_ctl_open("hw:N")`), never PCM streams. It cannot disrupt
  PipeWire or JACK playback.
- `scarlett-tui --json` is a diagnostic snapshot: it omits dedicated serial
  number and local filesystem path fields, but it does include ALSA card IDs,
  device names, and enum strings. **Review the JSON before posting publicly**
  and scrub anything you do not want to share.
- Never run `sudo` or `pkexec`. Never paste raw `/proc/asound` or `lsusb -v`
  serial lines into issues or pull requests.
- For **Scarlett Gen 4 `16i16` / `18i16` / `18i20`**, ensure `fcp-server` is
  running before testing so ALSA mixer controls are exposed.

## Step 1 — Run the read-only verification commands

```sh
# 1. Confirm the kernel driver exposed Focusrite mixer controls
scarlett-tui --list

# 2. Capture the structured diagnostic snapshot (review before sharing)
scarlett-tui --json > /tmp/scarlett-snapshot.json

# 3. Verify the JSON parses and inspect detected counts
python3 - <<'PY'
import json
d = json.load(open("/tmp/scarlett-snapshot.json"))
assert d.get("connected") is True, "No Focusrite card detected"
print("Card:      ", d.get("card"))
print("Mixer OK:  ", d.get("mixer_available"))
print("Firmware:  ", d.get("firmware"), "Versions:", d.get("firmware_versions", []))
print("Sync/Rate: ", d.get("sync_status"), d.get("active_rate"))
print("Inputs:    ", len(d.get("inputs", [])), [(i["label"], i.get("gain_db")) for i in d.get("inputs", [])])
print("Phantom:   ", [p["label"] for p in d.get("phantom", [])])
print("Outputs:   ", len(d.get("outputs", [])), [o["label"] for o in d.get("outputs", [])])
print("Routes:    ", len(d.get("routes", [])))
print("Matrix:    ", d.get("matrix_size") or "none")
print("Meters:    ", len(d.get("meter_labels", [])))
PY

# 4. Capture read-only ALSA control names, types, counts, and ranges
CARD=$(scarlett-tui --list | awk '{print $2; exit}')
amixer -c "$CARD" contents | grep -E '^(numid=|  ; type=)'
```

## Step 2 — Check what is verified vs. missing

Compare the `amixer` control metadata against `/tmp/scarlett-snapshot.json`:

1. **Card & generation:** Does `card.generation` match your hardware generation,
   and is `mixer_available` true?
2. **Inputs (`inputs` & `phantom`):** Are all `Line In N` controls (`Gain`,
   `Level`, `Air`, `Pad`, `Safe`, `Autogain`, `Link`, `Phantom Power`) present?
3. **Outputs (`outputs`):** Are all analogue / speaker / headphone outputs
   present and labelled sensibly?
4. **Routing (`routes`):** Do all `Analogue`, `S/PDIF`, `ADAT`, `Mixer`, `DSP`,
   and `PCM Capture` enums appear?
5. **Mix matrix (`matrix_size`):** If your model has `Mix A..Z`, does the
   dimension match?
6. **Meters (`meter_labels`):** Does `len(meter_labels)` match `Level Meter`
   count, and do bars move on Tab 1 when audio plays or you speak into an input?
7. **Interactive write test (opt-in only, human-supervised):** Only if the user
   explicitly opts in, toggle one safe control (e.g. `Air` or `Inst`) in
   `scarlett-tui` and confirm the physical front-panel LED changes in sync (and
   toggling the physical button updates the TUI immediately).

## Step 3 — Report results or submit a fixture PR

- **Report a verified model or a gap:** Open a
  [Device support](https://github.com/kinggs/scarlett-tui/issues/new?template=device-support.yml)
  issue (or via `gh issue create --template device-support.yml`) with a reviewed
  excerpt of `/tmp/scarlett-snapshot.json` and what worked / did not.
- **Add a regression test directly (agent-friendly PR):**
  1. Add a unit test in `src/model.rs` synthesizing your model's ALSA control
     names, types, and ranges (do **not** copy GPL `.state` files from other
     projects).
  2. Update the hardware support table in `README.md` (only remove `⚠` for
     behaviour verified on physical hardware).
  3. Run `make test` and `make lint`.
  4. Open a PR using `.github/PULL_REQUEST_TEMPLATE.md`.

## Notes for coding agents

- Do not assume a model works from its USB ID alone. Distinguish **control-name
  audited** (checked against documented kernel `scarlett2`/`fcp` naming) from
  **hardware-verified** (tested live on physical hardware with write readback).
- **Scarlett Gen 1** (`mixer_scarlett.c`, USB `1235:8002/8004/800c/8012/8014`)
  uses a completely different ALSA control schema (`Master 1L ...`,
  `Matrix 01 Input ...`) and is currently out of scope unless a contributor
  adds a dedicated Gen 1 mapper.
