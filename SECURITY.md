# Security Policy

## Reporting a vulnerability

Report security issues privately using GitHub's **Security** tab →
**Report a vulnerability**. Do not open a public issue.

Include what you ran, what happened, your kernel / Omarchy / ALSA versions, and
minimal reproduction steps (scrub any personal paths or serial numbers).

## What counts as a security issue here

- Memory safety bugs (out-of-bounds access, use-after-free, invalid FFI pointer
  handling) in the `libasound` bindings (`src/alsa_ctl.rs`).
- Command injection or unexpected process execution from crafted ALSA control
  names, PipeWire port names, or JSON daemon messages.
- Rich-text / markup injection or untrusted resource loading in `Panel.qml`
  (which runs inside Omarchy's unsandboxed Quickshell process).

Broken routing, missing controls for a specific Scarlett model, or layout bugs
are normal bugs — please use the bug report template for those.

## What to expect

- One person maintains this project in spare time; replies may be slow.
- There is no bug bounty.
- Fixes are credited to the reporter if desired.
- Only the latest `main` branch is supported.
