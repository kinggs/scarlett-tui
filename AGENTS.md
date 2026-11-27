# AGENTS.md

Read this before working in this repository. The rules in `CONTRIBUTING.md`
apply to people and agents alike; this file is the code map, commands, and
invariants to preserve.

## What this is

A Rust + Ratatui terminal UI for Focusrite Scarlett USB audio interfaces on
Linux, MIT-licensed, plus an optional bar panel (`Panel.qml`) for Omarchy
desktops. One binary (`scarlett-tui`) serves the terminal UI and an
event-driven `--daemon` JSON-lines interface.

**What runs where.** Say which one you mean in docs, issues and pull requests.

- The TUI and the JSON interface (`--json`, `--daemon`) are plain Linux: ALSA
  through `libasound`, optional PipeWire. Never make them depend on Omarchy.
- The panel is Omarchy-only. It is a Quickshell QML plugin, but it imports
  Omarchy's own `qs.Ui` and `qs.Commons`, so it is "the Omarchy bar panel",
  not "a Quickshell widget". `manifest.json`, `scripts/check-plugin.sh` and
  `make install-plugin` belong to it.
- Write "Omarchy" only for something that needs Omarchy. Terminal colours,
  the 95×30 size and the daemon are general; Omarchy is at most the example
  or the place a number was measured.

## The map

| Path | What it holds |
|---|---|
| `src/main.rs` | CLI parsing (`tui`, `--list`, `--json`, `--daemon`, `--card N`) |
| `src/alsa_ctl.rs` | Safe `libasound` (`snd_ctl_*`) FFI, `/proc/asound` Focusrite discovery (`1235:*`), `/proc/asound/cardN/stream0` rate reader, USB PID generation table |
| `src/model.rs` | Dynamic model builder (inputs, outputs, routes, `Mix A..Z` matrix, `scarlett2_ports` level-meter order, dB conversions) |
| `src/pipewire.rs` | Read-only `pw-link -l -I` parser and `pw-link -d` disconnect helper |
| `src/daemon.rs` | Event-driven JSON-lines daemon (`poll()` on stdin + ALSA control FDs + optional 20 Hz meter stream) |
| `src/ui.rs` | 5-tab Ratatui TUI using standard ANSI terminal colours |
| `manifest.json` | Omarchy plugin manifest (`id: io.github.kinggs.scarlett-tui`) at repo root for marketplace compatibility |
| `Panel.qml` | Omarchy bar widget + `KeyboardPanel` popup using `qs.Ui` and `qs.Commons` |
| `scripts/check-plugin.sh` | Static verification for `manifest.json`, `Panel.qml` `Text.PlainText` safety, `Style.duration(...)` motion, and `omarchy plugin validate` |
| `docs/DESIGN.md` | Design rules for both surfaces: rulings, semantic state table, glyphs, meter zones, keymap, layout |
| `design/tui-*.svg`, `design/tui-*.png` | README screenshots, rendered from the `model_demo` fixture by `make screenshots`. The SVGs are the checked source; the PNGs are what the README shows |
| `scripts/render-screenshots.sh` | Converts `design/tui-*.svg` to PNG for `make screenshots` |
| `docs/HARDWARE-TESTING.md` | Agent-first, read-only playbook for verifying new Scarlett / Clarett / Vocaster hardware and submitting fixture PRs |
| `Makefile` | `build`, `test`, `lint`, `check-toolchain`, `check-plugin`, `install`, `install-plugin`, `uninstall` |

## Invariants — do not break these

- **Never hardcode ALSA `numid`s or device channel counts.** `numid`s shift
  between models and firmware versions. Always discover controls by name, type,
  and range via `snd_ctl_elem_list` / `snd_ctl_elem_info`.
- **Never open ALSA PCM streams (`snd_pcm_open`).** Only open the ALSA control
  interface (`snd_ctl_open("hw:N")`). Opening PCM directly claims the device
  exclusively and breaks PipeWire playback.
- **Keep `libasound` FFI opaque-pointer-only.** Allocate every `snd_ctl_*_t` via
  `snd_ctl_*_malloc` and free in `Drop`. Never mirror private ALSA struct
  layouts in Rust.
- **No GPL/LGPL source copied.** Link `libasound` dynamically at runtime. Read
  upstream kernel source (`sound/usb/mixer_scarlett2.c`) for protocol facts and
  cite it in `CREDITS.md`, never copy C code from it.
- **Omarchy plugin UI safety (`Panel.qml`):**
  - Every `Text { ... }` element MUST set `textFormat: Text.PlainText` (enforced
    by `scripts/check-plugin.sh`).
  - Never use bare `duration: <ms>` in QML animations; wrap every duration in
    `Style.duration(<ms>)` (enforced by `scripts/check-plugin.sh`).
  - Never call `sudo` or `pkexec` from the plugin or binary.
  - Keep `manifest.json` and `Panel.qml` at the repository root so the repo
    remains valid for the Omarchy Plugin Marketplace.
- **ANSI palette colours only in `src/ui.rs`.** Avoid hardcoded RGB hex colours
  in the TUI so it follows whichever terminal theme is active.
- **DarkGray is decoration only.** Use `Color::DarkGray` for borders, the empty
  meter track and `·`, never for text a user must read; secondary text is
  `Color::Reset` + `Modifier::DIM`. Themes often make ANSI bright-black very
  faint: Omarchy's terminal themes map it to `muted`, measured at 1.9:1
  contrast ([docs/DESIGN.md](docs/DESIGN.md)).
- **No state by colour alone.** Every state pairs its colour with a glyph, a
  word, a modifier or a badge, because yellow and magenta are 2.3:1 in light
  themes. The state table is in [docs/DESIGN.md](docs/DESIGN.md), which wins
  over any drawing.
- **Stable Rust only.** `rust-toolchain.toml` and `mise.toml` must name the same
  stable version (`make check-toolchain`).
- **Screenshots follow the code.** After any change that alters what the TUI
  draws, run `make screenshots` and commit `design/` in the same pull request.
  `make test` fails when `design/tui-*.svg` are stale, because a README that
  shows an old UI misleads users. To show a new state, change the `model_demo`
  fixture in `src/model.rs` or `demo_app` in `src/ui.rs`, never edit the
  images. The demo must stay a plausible real device: real control names, no
  invented features.
- **Privacy & cleanliness:** Never commit device serial numbers, `/home/<user>`
  paths, or machine hostnames in source, fixtures, or documentation.
- **Facts vs. unverified claims:** Mark anything not verified on real hardware
  with `⚠`.

## Commands

Run from the repository root (`mise.toml` pins Rust automatically inside this
folder):

| Command | What it does |
|---|---|
| `mise install` | Install the pinned Rust toolchain once |
| `make test` | Check toolchain pin, check plugin & QML safety, run `cargo test --locked` |
| `make lint` | `cargo fmt --check`, `cargo clippy -- -D warnings`, `scripts/check-plugin.sh` |
| `make screenshots` | Re-render the README screenshots from the demo fixture (needs `rsvg-convert` and the JetBrainsMono Nerd Font for the PNGs) |
| `make build` | Build stripped release binary (`target/release/scarlett-tui`) |
| `make install` | Install binary to `~/.local/bin/scarlett-tui` and create `Scarlett TUI.desktop` |
| `make install-plugin` | Run `make install`, symlink repo into `~/.config/omarchy/plugins/io.github.kinggs.scarlett-tui`, and enable |
| `make uninstall` | Disable plugin and remove symlink, binary, and desktop file |

## Pull request flow

1. Branch from `main`. Never commit directly to `main`.
2. Add or update unit tests in `src/model.rs`, `src/pipewire.rs`, or
   `src/daemon.rs` for any parsing, routing, or model change.
3. Run `make test` and `make lint` before pushing.
4. If the TUI changed visibly, run `make screenshots` and commit `design/`.
5. Fill in `.github/PULL_REQUEST_TEMPLATE.md`, noting which hardware model (if
   any) you verified on and marking anything untested with `⚠`.
