## What and why

<!-- What does this change and why? Link the issue if there is one. -->

Closes #

## Hardware tested

<!-- Which Focusrite model, USB ID, firmware, and kernel did you test on?
     Or "None — unit tests only (⚠)". Never include serial numbers or home paths. -->

## How it was checked

- [ ] `make test` passed
- [ ] `make lint` passed (`rustfmt`, `clippy -D warnings`, `scripts/check-plugin.sh`)
- [ ] `make build` succeeded

## Checklist

- [ ] No hardcoded ALSA `numid`s or fixed channel/mix counts introduced.
- [ ] Control device only (`snd_ctl_*`); no ALSA PCM open.
- [ ] If `Panel.qml` changed, every `Text` sets `textFormat: Text.PlainText` and animations use `Style.duration(...)`.
- [ ] `CREDITS.md` updated if any new dependency or reference was added.
- [ ] If a coding agent assisted, the parts it wrote are noted below and I ran the checks myself.
- [ ] Anything unverified is listed below, marked ⚠.

## Unverified

<!-- ⚠ items, or "None". -->
