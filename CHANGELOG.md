# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Standalone 5-tab Rust + Ratatui TUI (`scarlett-tui`):
  - Live 12-bit hardware `Level Meter` view with peak hold and dBFS readouts
    mapped in `scarlett2_ports` destination order.
  - Dynamic I/O & device controls (`Line/Inst`, Gen 3 boolean `Air` & Gen 4
    enum `Air`, `Pad`, `48V` phantom power, `Remember 48V`, `Gain`, `Clip Safe`,
    `Link`, `Direct Monitor`, `Line 01..NN` volume/mute, `Standalone`).
  - Full hardware routing patchbay (`Analogue Outputs`, `S/PDIF`, `ADAT`,
    `Mixer Inputs`, `DSP Inputs`, `PC Capture`) with resolved `Mix A..Z` signal
    chains.
  - Dynamic 2D `Mix Matrix` (`Mix A..Z × In 01..NN`) in dB with unity snap and
    mute toggles.
  - Live PipeWire links tab (`pw-link -l -I`) highlighting Scarlett links and
    software loopbacks, with disconnect support.
- Event-driven `--json` and `--daemon` JSON-lines modes backed by `libasound`
  `snd_ctl_*` FFI and `poll()`.
- Root-level Omarchy bar plugin (`io.github.kinggs.scarlett-tui`, `manifest.json`
  and `Panel.qml`) inspired by `omarchy.audio`, with live meters while open,
  quick input/48V/routing controls, and floating `TUI.float` launcher (`T` /
  right-click).
- Hardware-verified on Focusrite Scarlett 4i4 (3rd Gen) (`1235:8212`, firmware
  `1605`, Linux `7.2.15`).
