# Credits

Everything `scarlett-tui` builds on or learned from, with licences and what is
actually in this repository. Our own code is MIT ([LICENSE](LICENSE)).

## Hardware driver & protocol knowledge

| Source | Author | Licence | What is in this repository |
|---|---|---|---|
| Linux kernel `sound/usb/mixer_scarlett2.c` & `mixer_quirks.c` | Geoffrey D. Bennett & ALSA contributors | GPL-2.0 | **No source code.** Read for USB PID tables, control naming conventions, and `scarlett2_ports` meter ordering (`v7.2.15`) |
| [alsa-scarlett-gui](https://github.com/geoffreybennett/alsa-scarlett-gui) | Geoffrey D. Bennett | GPL-3.0 | **Nothing.** Reference GTK4 GUI by the kernel driver's author; inspiration for grouping controls and matrix editing |
| [alsa-lib (`libasound`)](https://www.alsa-project.org/) | The ALSA Project | LGPL-2.1 | Dynamically linked at runtime via the public C `snd_ctl_*` ABI (`build.rs`) |

## Rust crate dependencies (in `Cargo.toml`)

| Crate | Licence | Role |
|---|---|---|
| [ratatui](https://github.com/ratatui/ratatui) | MIT | Terminal UI layout and widgets |
| [crossterm](https://github.com/crossterm-rs/crossterm) | MIT | Terminal raw mode and keyboard events |
| [libc](https://github.com/rust-lang/libc) | MIT / Apache-2.0 | POSIX `poll` / `fcntl` and C FFI types |
| [serde](https://github.com/serde-rs/serde) & [serde_json](https://github.com/serde-rs/serde_json) | MIT / Apache-2.0 | JSON snapshot and daemon protocol |

## Desktop ecosystem & inspiration

| Source | Author | Licence | What is in this repository |
|---|---|---|---|
| [Omarchy](https://omarchy.org) (`omarchy.audio` & `qs.Ui`) | Omacom / Basecamp | MIT | **No packaged code.** `Panel.qml` imports the host's `qs.Ui` and `qs.Commons` components and takes layout inspiration from the built-in `omarchy.audio` panel |
| [Quickshell](https://quickshell.org) | Quickshell contributors | LGPL-3.0 | Runtime host for `Panel.qml` on Omarchy desktops |
| [PipeWire](https://pipewire.org) (`pw-link`) | PipeWire contributors | MIT | Optional external CLI called read-only for Tab 5 (and `pw-link -d` on explicit user disconnect) |

## Trademarks

Focusrite, Scarlett, Clarett, and Vocaster are trademarks of Focusrite Audio
Engineering Ltd. This project is independent and is not affiliated with or
endorsed by Focusrite.

## If we missed a credit

Open an issue if something is missing attribution or has an inaccurate licence
note and we will fix it promptly.
