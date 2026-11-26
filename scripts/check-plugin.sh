#!/usr/bin/env bash
# Validate the Omarchy bar plugin manifest and QML UI-safety rules.
set -euo pipefail

python3 - <<'PY'
import json, re, sys
from pathlib import Path

root = Path(".")
manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))

assert manifest.get("schemaVersion") == 1, "schemaVersion must be 1"
pid = manifest.get("id", "")
assert pid and not pid.startswith("omarchy."), f"invalid plugin id: {pid!r}"
assert manifest.get("entryPoints", {}).get("barWidget") == "Panel.qml"
assert manifest.get("license") == "MIT"

qml = (root / "Panel.qml").read_text(encoding="utf-8")

# Every Text { ... } element must set textFormat: Text.PlainText (Omarchy plugin UI safety).
pos = 0
text_blocks = 0
while True:
    m = re.search(r"\bText\s*\{", qml[pos:])
    if not m:
        break
    text_blocks += 1
    start = pos + m.end()
    depth = 1
    i = start
    while i < len(qml) and depth > 0:
        if qml[i] == "{":
            depth += 1
        elif qml[i] == "}":
            depth -= 1
        i += 1
    body = qml[start:i]
    if "textFormat: Text.PlainText" not in body:
        line_no = qml[: start].count("\n") + 1
        sys.exit(f"Panel.qml:{line_no}: Text element missing 'textFormat: Text.PlainText'")
    pos = i

assert text_blocks > 0, "expected Text elements in Panel.qml"

# Motion rule: no bare duration: <digits> (must wrap in Style.duration(...)).
bad_dur = re.findall(r"\bduration\s*:\s*\d+", qml)
assert not bad_dur, f"Panel.qml uses bare animation duration(s): {bad_dur}"

# No privilege escalation in plugin QML.
for forbidden in ("sudo", "pkexec"):
    assert forbidden not in qml, f"Panel.qml must not reference {forbidden}"

print(f"plugin check ok ({text_blocks} Text elements verified PlainText, motion & privilege checks clean)")
PY

if command -v omarchy >/dev/null 2>&1; then
  omarchy plugin validate .
  echo "omarchy plugin validate ok"
fi
