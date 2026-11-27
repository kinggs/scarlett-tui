# scarlett-tui

[![CI](https://github.com/kinggs/scarlett-tui/actions/workflows/ci.yml/badge.svg)](https://github.com/kinggs/scarlett-tui/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A keyboard-first terminal mixer, routing matrix, live hardware level meter, and
[Omarchy](https://omarchy.org) bar plugin for **Focusrite Scarlett** USB audio
interfaces on Linux — written in Rust, talking straight to the mainline Linux
kernel's `scarlett2` / FCP ALSA driver through `libasound`.

One compiled binary powers both the standalone Ratatui TUI and the Omarchy bar
panel. No background daemon when unused, no Electron or GTK toolkit, no `sudo`,
and no hardcoded ALSA `numid`s.

<p align="center">
  <img src="design/tui-overview.png" alt="Overview tab at 95×30: a one-line header with Locked, 48.0 kHz and the model; every meter on a dB scale, grouped, each row naming the source that feeds it" width="49%">
  &nbsp;
  <img src="design/tui-matrix.png" alt="Mix matrix tab: mixes A to F across, each naming the output it feeds; quiet dots for −∞; the cursor cell reversed" width="49%">
</p>

<p align="center">
  <img src="design/tui-io.png" alt="I/O tab as a board: one row per channel with Line/Inst, Air and Pad across; 48V in its own section; each output's volume and mute on one row; device switches" width="49%">
  &nbsp;
  <img src="design/tui-help.png" alt="The ? overlay: this tab's keys, safety keys, global keys and an About block with model, USB id, firmware and card" width="49%">
</p>

<p align="center"><em>The TUI at the 95×30 size of Omarchy's floating window, on a Scarlett 4i4 Gen 3:
overview, mix matrix, I/O, and the <code>?</code> key overlay.</em></p>

The screenshots come from the real TUI against a connected interface, painted
in Omarchy's Tokyo Night palette (the TUI itself uses only the 16 ANSI colours,
so it follows your theme). To regenerate them with a Scarlett attached:
`cargo test --locked -- --ignored capture_readme_screenshots`, then
`rsvg-convert -z 2` each `design/tui-*.svg` to PNG. ⚠ The bar plugin panel is
not pictured yet.

## Why this exists

The mainline Linux kernel already exposes everything inside modern Focusrite
Scarlett interfaces — input switches, 48V phantom power, output volumes and
mutes, the internal routing patchbay, the `Mix A–F` mixer matrix, and real-time
12-bit hardware level meters.

The reference GUI (`alsa-scarlett-gui`) is great, and `alsamixer` works in a
pinch, but if you live in the terminal (and on an Omarchy desktop) there was no
dedicated TUI that shows the routing matrix, mix faders, live meters, and active
PipeWire software links together in one place.

## Status and hardware support

Controls, routing tables, mix matrix dimensions, and level-meter mappings are
discovered dynamically from `libasound` (`snd_ctl_*`), and were audited against
control naming from the mainline Linux `7.2` `scarlett2`/`fcp` driver across
Gen 2, Gen 3, Gen 4, Clarett+, and Vocaster models. However, **only the
maintainer's own Scarlett 4i4 (3rd Gen) has been verified on physical
hardware**.

| Model family | Linux driver | Status |
|---|---|---|
| **Scarlett 4i4 (3rd Gen)** (`1235:8212`, fw `1605`) | `scarlett2` | **Verified on physical hardware** — all 5 tabs, live 18-channel meters, 6×8 matrix, Omarchy bar plugin |
| **Scarlett Gen 2** (`6i6`, `18i8`, `18i20`) | `scarlett2` | ⚠ Control names audited; not yet verified on physical hardware |
| **Scarlett Gen 3** (`Solo`, `2i2`, `8i6`, `18i8`, `18i20`) | `scarlett2` | ⚠ Control names audited (`Direct Monitor` / matrix); not yet verified on physical hardware |
| **Scarlett Gen 4** (`Solo`, `2i2`, `4i4`) | `scarlett2` | ⚠ Digital `Gain`, 3-state `Air` enum, `Clip Safe`, `Link`, `PCM Input`, `Direct Monitor` covered by unit tests; not yet verified on physical hardware |
| **Scarlett Gen 4 large** (`16i16`, `18i16`, `18i20`) | `fcp` | ⚠ Short FCP routing names (`Analogue 1`, `Mixer 1`, `Analogue 1 Playback Volume`, scrolling matrix) covered by unit tests; not yet verified on physical hardware |
| **Clarett USB / Clarett+ / Vocaster** | `scarlett2` | ⚠ Control names audited (`1235:*`); not yet verified on physical hardware |
| **Scarlett Gen 1** (`6i6`, `8i6`, `18i6`, `18i8`, `18i20`) | `mixer_scarlett` | **Not supported** — uses an older, different ALSA control schema (`Master 1L`, `Matrix 01`) |

### Call to action: help verify your Focusrite model (humans & agents)

I only own a Scarlett 4i4 (3rd Gen), so every `⚠` row above needs real-hardware
testing. If you (or your coding agent) have another Scarlett, Clarett, or
Vocaster plugged into Linux, follow the two-minute, read-only, privacy-safe
playbook in **[docs/HARDWARE-TESTING.md](docs/HARDWARE-TESTING.md)**:

```sh
scarlett-tui --list
scarlett-tui --json    # diagnostic snapshot (omits dedicated serial/path fields; review before posting)
```

Open a [Device support](https://github.com/kinggs/scarlett-tui/issues/new?template=device-support.yml)
issue with the JSON output and what worked, or send a PR adding a unit test for
your model in `src/model.rs`. **Agents are first-class testers here** — point
your agent at `AGENTS.md` and `docs/HARDWARE-TESTING.md`.

### What this does today

- **Tab 1 — Overview & Meters:** real-time 12-bit hardware `Level Meter` bars
  with peak hold and dBFS readouts (mapped in `scarlett2_ports` destination
  order), input badges, output signal paths, clock sync, firmware, active sample
  rate, and active PipeWire Scarlett links.
- **Tab 2 — I/O & Device:** `Line/Inst`, `Air` (handles both Gen 3 boolean
  switches and Gen 4 3-state enums), `Pad`, `48V` phantom power, `Remember 48V`,
  digital `Gain` / `Clip Safe` / `Link` (where present), `Line 01..NN` hardware
  volumes & mutes, `Direct Monitor`, and `Standalone` switches.
- **Tab 3 — Routing:** full hardware router for `Analogue Outputs`, `S/PDIF`,
  `ADAT`, `Mixer Inputs`, `DSP Inputs`, and `PC Capture (Rec)`, showing resolved
  signal chains (e.g. `Mix A ← PCM 1 + Analogue 2 (-6.0 dB)`).
- **Tab 4 — Mix Matrix:** dynamic 2D dB grid (`Mix A..Z × In 01..NN`), with `*`
  marking mixes currently routed to an output.
- **Tab 5 — PipeWire Links:** live `pw-link` view filtered to Scarlett nodes
  first (toggle `a` for all nodes) so software loopbacks stand out beside
  hardware routing, with `d` to disconnect.
- **Omarchy bar plugin (`io.github.kinggs.scarlett-tui`):** compact bar widget
  inspired by Omarchy's built-in `omarchy.audio` panel — live dB meters while
  open, input chips, `48V` switches, clickable output routing rows, and one
  click / `T` / right-click to launch `scarlett-tui` in a floating terminal
  (`TUI.float`).

### What this does not do (yet)

- **No firmware flashing** (use Geoffrey Bennett's [`scarlett2`](https://github.com/geoffreybennett/scarlett2) CLI for that).
- **No preset save/load yet** inside the TUI (`alsactl store` / `restore` works alongside it).
- **No PipeWire sample-rate / quantum switching** (intentionally focused on the hardware interface and routing visibility; PipeWire clock stays under PipeWire/WirePlumber).

## What it is not

- **Not affiliated with or endorsed by Focusrite Audio Engineering Ltd.**
  Focusrite, Scarlett, Clarett, and Vocaster are trademarks of Focusrite Audio
  Engineering Ltd., used here only to identify supported hardware.
- **No proprietary firmware, drivers, or assets** are included in this repository.
- **No privilege escalation:** `scarlett-tui` never runs `sudo` or `pkexec`,
  never installs system packages, and never writes under `/etc` or
  `~/.config/pipewire`.

## How it fits together

```mermaid
flowchart LR
  hw["Focusrite Scarlett USB\n(kernel snd_usb_audio / scarlett2)"]
  pw["PipeWire graph\n(pw-link -l -I)"]

  subgraph bin["scarlett-tui (single Rust binary)"]
    alsa["alsa_ctl.rs\nlibasound snd_ctl_* FFI\n+ event poll()"]
    model["model.rs\ndynamic control, matrix\n& meter model"]
    tui["ui.rs\n5-tab Ratatui TUI\n(ANSI theme colours)"]
    daemon["daemon.rs\n--json / --daemon\nJSON-lines protocol"]
  end

  plugin["Panel.qml\nOmarchy bar plugin\n(Quickshell + qs.Ui)"]

  hw <--> alsa
  alsa --> model
  model --> tui
  model --> daemon
  pw --> tui
  pw --> daemon
  daemon <-->|"stdin/stdout JSON-lines"| plugin
  plugin -. "T / right-click\nomarchy-launch-or-focus-tui" .-> tui
```

## Requirements

- **Linux kernel 6.7+** (mainline `snd_usb_audio` enables the `scarlett2` mixer
  driver automatically without module options; older kernels require
  `options snd_usb_audio vid=0x1235 pid=0x<pid> device_setup=1`).
- **Large Scarlett Gen 4 (`16i16`, `18i16`, `18i20`):** require Linux `6.14+`
  FCP support and the userspace **`fcp-server`** daemon running so ALSA mixer
  controls exist. Without `fcp-server`, `scarlett-tui` detects the USB card and
  reports *Mixer unavailable* until controls appear.
- **Runtime:** `alsa-lib` (`libasound.so.2`). Optional: `pipewire` (`pw-link`)
  for Tab 5; Omarchy 4 + Quickshell if using the bar plugin.
- **Build:** Rust `1.100.0` (pinned in `rust-toolchain.toml`, `mise.toml`, and
  `Cargo.toml`), `libasound` linker library (`alsa-lib` / `libasound2-dev`),
  and a C linker.

## Install

### Standalone TUI (any Linux distro)

```sh
git clone https://github.com/kinggs/scarlett-tui.git
cd scarlett-tui
mise install      # optional: installs the pinned Rust 1.100.0
make install      # builds release binary -> ~/.local/bin/scarlett-tui + portable Terminal=true desktop entry
```

Run it in any terminal with:

```sh
scarlett-tui
```

The TUI uses standard ANSI terminal colours, so it inherits your terminal
(and Omarchy) theme automatically.

### With the Omarchy bar plugin

```sh
git clone https://github.com/kinggs/scarlett-tui.git
cd scarlett-tui
make install-plugin
```

This builds and installs `~/.local/bin/scarlett-tui`, updates the desktop entry
for `omarchy-launch-or-focus-tui`, validates `manifest.json` and `Panel.qml`,
links the repository into `~/.config/omarchy/plugins/io.github.kinggs.scarlett-tui`,
and enables the bar widget on the right section of the Omarchy bar.

*(If you added the repository via `omarchy plugin add https://github.com/kinggs/scarlett-tui.git`,
run `cd ~/.config/omarchy/plugins/io.github.kinggs.scarlett-tui && make install`
once to build the Rust binary. To remove a marketplace checkout, use
`omarchy plugin remove io.github.kinggs.scarlett-tui`.)*

### Remove

```sh
cd /path/to/scarlett-tui
make uninstall
```

Or manually:

```sh
omarchy plugin disable io.github.kinggs.scarlett-tui
rm -f ~/.config/omarchy/plugins/io.github.kinggs.scarlett-tui \
      ~/.local/bin/scarlett-tui \
      "$HOME/.local/share/applications/Scarlett TUI.desktop"
```

Hardware settings live on the Scarlett itself (and ALSA's `alsa-restore.service`
if enabled); uninstalling removes no system configs or saved audio state.

## Keybindings

A key means one thing on every tab; the full table is in
[docs/DESIGN.md](docs/DESIGN.md#keymap), and `?` shows it in the app.

| Key | Action |
|---|---|
| `1`–`5` / `Tab` / `Shift+Tab` | Switch tabs |
| `h`/`j`/`k`/`l` or arrows | Move the cursor; never changes a value |
| `-` / `+` | Level −/+ 1 dB, or previous / next choice |
| `{` / `}` | Level −/+ 6 dB |
| `0` | Unity (`0.0 dB`); press twice if the rise is over +6 dB |
| `m` | Mute an output or a matrix cell; again restores |
| `Space` | Toggle a switch; 48V needs `Space` twice |
| `Enter` | Open the route picker; set the source |
| `x` | Disconnect: route off, or remove a PipeWire link |
| `a` | Show all PipeWire links (Tab 5) |
| `C` | Clear clip latches |
| `c` | Switch card (when multiple Focusrite interfaces are connected) |
| `r` | Rescan ALSA & PipeWire state |
| `?` | Help overlay |
| `q` / `Ctrl+C` | Quit |

### CLI modes

```sh
scarlett-tui [--card N]           # Interactive TUI
scarlett-tui --no-color           # TUI without colour (also NO_COLOR=1 or TERM=dumb)
scarlett-tui --ascii              # TUI with ASCII glyphs and borders
scarlett-tui --list               # List detected Focusrite USB cards
scarlett-tui --json [--card N]    # Print current state snapshot as JSON
scarlett-tui --daemon [--card N]  # Event-driven JSON-lines daemon (used by Panel.qml)
```

## Build, test, and lint

```sh
make test    # toolchain pin check, plugin & QML UI-safety check, cargo test
make lint    # cargo fmt --check, cargo clippy -D warnings, plugin check
make build   # release binary (target/release/scarlett-tui)
```

## Why MIT

Anyone — including other Omarchy plugin authors, Linux audio maintainers, or
Focusrite — should be able to reuse any part of this without licensing friction.
All `libasound` calls use the public C ABI dynamically linked at runtime; no GPL
driver or GUI source code is copied into this repository.

## Contributing

Contributions from people and coding agents are welcome under the same rules.
Read [CONTRIBUTING.md](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md) before
starting.

## Licence

[MIT](LICENSE). Copyright (c) 2026 Kenny Inggs and the scarlett-tui
contributors. Third-party credits and licences are listed in
[CREDITS.md](CREDITS.md).
