#!/usr/bin/env bash
# Convert design/tui-*.svg to the PNGs the README shows. Called by
# `make screenshots` after the demo fixture has been re-rendered to SVG.
# The SVGs name the JetBrainsMono Nerd Font; without it the PNGs fall back to
# another monospace font and will not match the committed ones.
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v rsvg-convert >/dev/null 2>&1; then
  echo "rsvg-convert not found: the SVGs are updated, the PNGs are not." >&2
  exit 1
fi

count=0
for svg in design/tui-*.svg; do
  png="design/$(basename "$svg" .svg).png"
  rsvg-convert -z 2 "$svg" -o "$png"
  count=$((count + 1))
done
echo "Regenerated $count screenshots in design/. Review and commit them."
