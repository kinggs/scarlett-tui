# Contributing

Contributions are welcome from people and coding agents alike. The same rules
hold for both.

One person maintains this project in spare time. Replies may be slow. A small,
tested pull request that follows the rules below gets merged soonest.

## Quick workflow

1. Open or pick an issue (especially for new Focusrite model quirks or UI
   changes).
2. Fork the repository and branch from `main`.
3. Make small commits with plain-English messages (one intent per commit).
4. Run `make test` and `make lint` before pushing.
5. Open a pull request with the template filled in.
6. CI must pass. Pull requests are squash-merged.

## Rules for every change

- **Dynamic discovery over hardcoding.** Do not hardcode ALSA `numid`s or assume
  a fixed channel/mix count. Every Scarlett size (Solo through 18i20, Gen 2
  through Gen 4, Clarett, Vocaster) should degrade cleanly.
- **Control device only, never PCM.** Never call `snd_pcm_open`; touching PCM
  directly steals the interface from PipeWire.
- **Stable Rust only.** Keep `rust-toolchain.toml` and `mise.toml` in sync
  (`make check-toolchain`).
- **Keep `Panel.qml` safe and native.** Every `Text` element must set
  `textFormat: Text.PlainText`, every animation duration must use
  `Style.duration(...)`, and no `sudo`/`pkexec` calls are permitted.
  `make check-plugin` enforces this.
- **Hardware vs. unit-test honesty.** Clearly separate what was verified on
  physical hardware from what was tested only against synthetic fixtures. Mark
  anything unverified with `⚠`.
- **Keep private data out of issues and fixtures.** Do not paste your hardware
  serial number, username, or local home paths into fixtures or pull requests.
- **Licence hygiene.** This project is MIT. Do not copy GPL/LGPL code (such as
  kernel driver C code or `alsa-scarlett-gui` source) into this repository.
  Credit every reference in [CREDITS.md](CREDITS.md).

## Testing new Focusrite models

If you own a Scarlett, Clarett, or Vocaster model marked `⚠` in the README:

1. Run `scarlett-tui --json` (it omits serial numbers and personal paths).
2. Test toggling inputs, outputs, routing, and meters in `scarlett-tui`.
3. Open a **Device support** issue or a pull request adding a unit test fixture
   for your model in `src/model.rs`.

## AI-assisted contributions

Welcome — parts of this project were built with coding agents.

- Read [AGENTS.md](AGENTS.md) first.
- State in the pull request what was generated and what you tested yourself.
- Keep `Co-Authored-By` trailers on commits when an agent assisted.
- You are responsible for what you submit, however it was written.

## Conduct and security

- [Code of conduct](CODE_OF_CONDUCT.md)
- [Security policy](SECURITY.md): report vulnerabilities privately, not in a
  public issue.
