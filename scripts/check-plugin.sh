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

raw_qml = (root / "Panel.qml").read_text(encoding="utf-8")

def strip_comments_and_strings(src: str) -> str:
    """Replace comments and string literal interiors with spaces while keeping line numbers."""
    out = []
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if c == "/" and nxt == "/":
            while i < n and src[i] != "\n":
                out.append(" ")
                i += 1
        elif c == "/" and nxt == "*":
            out.extend([" ", " "])
            i += 2
            while i < n:
                if src[i] == "*" and i + 1 < n and src[i + 1] == "/":
                    out.extend([" ", " "])
                    i += 2
                    break
                out.append("\n" if src[i] == "\n" else " ")
                i += 1
        elif c in ('"', "'"):
            quote = c
            out.append(c)
            i += 1
            while i < n:
                if src[i] == "\\" and i + 1 < n:
                    out.extend([" ", " "])
                    i += 2
                elif src[i] == quote:
                    out.append(quote)
                    i += 1
                    break
                else:
                    out.append("\n" if src[i] == "\n" else " ")
                    i += 1
        else:
            out.append(c)
            i += 1
    return "".join(out)

qml = strip_comments_and_strings(raw_qml)

# Every Text { ... } element must set textFormat: Text.PlainText as a direct property
# (not inside a nested block or comment).
pos = 0
text_blocks = 0
while True:
    m = re.search(r"\bText\s*\{", qml[pos:])
    if not m:
        break
    text_blocks += 1
    start = pos + m.end()
    depth = 1
    top_level_chars = []
    i = start
    while i < len(qml) and depth > 0:
        ch = qml[i]
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
        elif depth == 1:
            top_level_chars.append(ch)
        i += 1
    top_body = "".join(top_level_chars)
    if not re.search(r"\btextFormat\s*:\s*Text\.PlainText\b", top_body):
        line_no = qml[:start].count("\n") + 1
        sys.exit(f"Panel.qml:{line_no}: Text element missing direct 'textFormat: Text.PlainText'")
    pos = i

assert text_blocks > 0, "expected Text elements in Panel.qml"

# Motion rule: every `duration:` property assignment must wrap in Style.duration(...).
for m in re.finditer(r"\bduration\s*:\s*([^\n;\}]+)", qml):
    expr = m.group(1).strip()
    if not expr.startswith("Style.duration("):
        line_no = qml[: m.start()].count("\n") + 1
        sys.exit(f"Panel.qml:{line_no}: animation duration must use Style.duration(...), got: {expr!r}")

# Avoid QtQuick QQuickColor shadowing: qualify Omarchy's Color singleton as Commons.Color.
bare_color = re.findall(r"(?<!\.)\bColor\.(?:accent|urgent|foreground|background)\b", qml)
assert not bare_color, f"Panel.qml must qualify Omarchy Color singleton as Commons.Color: {bare_color}"

# No privilege escalation in plugin QML.
for forbidden in ("sudo", "pkexec"):
    assert forbidden not in raw_qml, f"Panel.qml must not reference {forbidden}"

print(f"plugin check ok ({text_blocks} Text elements verified PlainText, motion, Color qualification & privilege checks clean)")
PY

QMLLINT=""
if command -v qmllint >/dev/null 2>&1; then
  QMLLINT="$(command -v qmllint)"
elif [ -x /usr/lib/qt6/bin/qmllint ]; then
  QMLLINT="/usr/lib/qt6/bin/qmllint"
fi

if [ -n "$QMLLINT" ]; then
  # Check for syntax errors and QQuickColor/missing-property errors (external qs.* imports are not on standalone qmllint path).
  lint_out="$("$QMLLINT" Panel.qml 2>&1 || true)"
  if echo "$lint_out" | grep -E "missing-property|syntax|Error:" >/dev/null 2>&1; then
    echo "$lint_out" | grep -E "missing-property|syntax|Error:" >&2
    exit 1
  fi
  echo "qmllint check ok"
fi

if command -v omarchy >/dev/null 2>&1; then
  omarchy plugin validate .
  echo "omarchy plugin validate ok"
fi
