import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Io
import qs.Ui
import qs.Commons
import qs.Commons as Commons

Panel {
  id: root
  moduleName: "io.github.kinggs.scarlett-tui"
  ipcTarget: "io.github.kinggs.scarlett-tui"

  property bool daemonOnline: false
  property bool connected: false
  property bool mixerAvailable: false
  property int stateGeneration: 0
  property string lastError: ""
  property var card: ({})
  property var firmware: null
  property string syncStatus: "Disconnected"
  property bool locked: false
  property var activeRate: null
  property var inputs: []
  property var phantom: []
  property var outputs: []
  property bool hasMatrix: false
  property string matrixSize: ""
  property int pwScarlettLinks: 0
  property var meterLabels: []
  property var meterDbfs: []
  property int expandedOutput: -1
  // Clip latch from the daemon (meters frames while open, "clip" messages while closed).
  property bool clipAny: false
  property var clipFlags: []
  // numid of the 48V switch waiting for its confirming click, or -1.
  property int armedPhantom: -1

  // Keyboard cursor: one row cursor across inputs, 48V and outputs; Left/Right
  // moves across an input's chips or an expanded output's sources.
  property int navCursor: -1
  property int chipCursor: 0
  property int routeCursor: -1
  readonly property var navRows: {
    var rows = []
    for (var i = 0; i < inputs.length; i++)
      if ((inputs[i].toggles || []).length > 0) rows.push({ kind: "input", index: i })
    for (var p = 0; p < phantom.length; p++) rows.push({ kind: "phantom", index: p })
    for (var o = 0; o < outputs.length; o++) rows.push({ kind: "output", index: o })
    return rows
  }

  readonly property color fg: bar ? bar.foreground : Commons.Color.foreground
  readonly property color barFg: bar ? bar.barForeground : Commons.Color.foreground
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  readonly property string titleText: {
    if (!daemonOnline) return "Scarlett"
    if (!connected) return "Scarlett"
    var name = String(card.name || "Focusrite Scarlett")
    return name.replace(/^Focusrite\s+/i, "")
  }

  readonly property string subtitleText: {
    if (!daemonOnline) return "BACKEND UNAVAILABLE · RESTARTING"
    if (!connected) return "NOT CONNECTED"
    if (!mixerAvailable) return "MIXER UNAVAILABLE · CHECK DRIVER / FCP-SERVER"
    // Status pills: sync · rate · generation. Firmware lives in the tooltip.
    var parts = [locked ? "● Locked" : "▲ " + String(syncStatus || "Unlocked")]
    if (activeRate) {
      var khz = Number(activeRate) / 1000
      parts.push((Math.round(khz) === khz ? khz.toString() : khz.toFixed(1)) + " kHz")
    }
    if (card.generation) parts.push(String(card.generation))
    return parts.join(" · ")
  }

  readonly property string firmwareText: (firmware !== null && firmware !== undefined) ? " · FW " + String(firmware) : ""

  // Bar button state (docs/DESIGN.md states table, bar-states board).
  readonly property string barState: !daemonOnline ? "backend"
    : !connected ? "disconnected"
    : !mixerAvailable ? "nomixer"
    : "ok"

  readonly property string phantomText: {
    var on = []
    for (var i = 0; i < phantom.length; i++) {
      var lbl = String(phantom[i].label || "")
      if (lbl.indexOf("Remember") < 0 && phantom[i].active) on.push(lbl)
    }
    return on.join(", ")
  }

  readonly property string clippedText: {
    var names = []
    for (var i = 0; i < meterLabels.length; i++) {
      var m = meterLabels[i]
      if (m && clipFlags[m.index]) names.push(String(m.label || ("Ch " + (m.index + 1))))
    }
    if (names.length === 0 && clipAny) return "a channel"
    return names.join(", ")
  }

  readonly property string barTooltip: {
    var mouse = "Left: panel · Right: TUI · Scroll: nothing"
    if (barState === "backend") return "Scarlett backend restarting\n" + mouse
    if (barState === "disconnected") return "No Scarlett connected\nPlug in to restore · " + mouse
    if (barState === "nomixer") return "Scarlett found, no mixer controls\nCheck fcp-server or kernel ≥ 6.7"
    var lines = []
    if (clipAny) lines.push("Clipped: " + clippedText + " · open panel to clear")
    if (anyPhantomOn) lines.push("48V on " + phantomText)
    lines.push(titleText + " · " + subtitleText + firmwareText)
    lines.push(mouse)
    return lines.join("\n")
  }

  function clearClips() {
    send("clip clear")
    clipAny = false
    clipFlags = []
  }

  readonly property bool anyPhantomOn: {
    for (var i = 0; i < phantom.length; i++) {
      var lbl = String(phantom[i].label || "")
      if (lbl.indexOf("Remember") < 0 && phantom[i].active) return true
    }
    return false
  }

  // Pick up to 6 key meter channels (Outputs + active Inputs, or neutral fallback meters).
  readonly property var visibleMeters: {
    var out = []
    for (var i = 0; i < meterLabels.length; i++) {
      var m = meterLabels[i]
      if (!m || m.source === "Off") continue
      if (m.group === "Analogue Outputs" || m.group === "PC Capture (Rec)") {
        out.push(m)
      }
      if (out.length >= 12) break
    }
    if (out.length === 0) {
      for (var j = 0; j < meterLabels.length && out.length < 12; j++) {
        var fm = meterLabels[j]
        if (fm && fm.group === "Meters") out.push(fm)
      }
    }
    return out
  }

  // One block per channel (docs/DESIGN.md "Layout: Panel"): a channel's meter
  // sits in its own row. A meter is matched by name, never by position, and a
  // row with no match simply has no meter.
  function outputMeter(out) {
    var label = String((out && out.label) || "")
    for (var i = 0; i < meterLabels.length; i++) {
      var m = meterLabels[i]
      if (m && m.group === "Analogue Outputs" && String(m.label || "") === label) return m.index
    }
    return -1
  }

  // An analogue input has no meter of its own, so use the capture or mixer
  // channel it feeds. The daemon names it (`meter`, unit-tested in Rust and
  // tolerant of other source spellings); older daemons fall back to matching
  // the source name "Analogue N" here.
  function inputMeter(inp) {
    if (inp && typeof inp.meter === "number" && inp.meter >= 0) return inp.meter
    var n = String((inp && inp.label) || "").match(/(\d+)\s*$/)
    if (!n) return -1
    var src = "Analogue " + Number(n[1])
    for (var i = 0; i < meterLabels.length; i++) {
      var m = meterLabels[i]
      if (m && String(m.source || "") === src
          && (m.group === "PC Capture (Rec)" || m.group === "Mixer Inputs")) return m.index
    }
    return -1
  }

  // True when the outputs carry their own meters. Otherwise (models whose
  // meters have neutral names) the separate LEVELS list is shown instead.
  readonly property bool metersInRows: {
    for (var i = 0; i < outputs.length; i++) if (outputMeter(outputs[i]) >= 0) return true
    return false
  }

  // The ? key shows the key list in the footer.
  property bool showKeys: false
  readonly property var keyHelp: [
    ["↑ ↓ ← →", "move (also h j k l)"],
    ["Space", "toggle · 48V takes two"],
    ["Enter", "open an output's sources · set"],
    ["m", "mute the output under the cursor"],
    ["C", "clear clip latches"],
    ["r / T", "refresh / open the TUI"],
    ["Esc", "collapse, then close"]
  ]

  // Stereo pairs: neighbours in the same group share one row with two bars.
  readonly property var meterPairs: {
    var v = visibleMeters
    var pairs = []
    var i = 0
    while (i < v.length) {
      var a = v[i]
      var b = (i + 1 < v.length && v[i + 1].group === a.group) ? v[i + 1] : null
      pairs.push({ label: pairLabel(a, b), a: a.index, b: b ? b.index : -1 })
      i += b ? 2 : 1
    }
    return pairs
  }

  function pairLabel(a, b) {
    var neutral = /^Meter \d+$/
    if (neutral.test(String(a.label || "")))
      return b ? ("Ch " + (a.index + 1) + "–" + (b.index + 1)) : ("Ch " + (a.index + 1))
    var s = String(a.label || "")
    var inParens = s.match(/\(([^)]*)\)/)
    if (inParens) s = inParens[1]
    return b ? s.replace(/\s+(L|R|Left|Right)$/, "") : s
  }

  // Peak hold per channel: 1.5 s, then falls (docs/DESIGN.md "Meter zones").
  property var meterPeak: []
  property var meterPeakAt: []

  function updatePeaks(dbfs) {
    var now = Date.now()
    var peaks = meterPeak.slice(0, dbfs.length)
    var at = meterPeakAt.slice(0, dbfs.length)
    for (var i = 0; i < dbfs.length; i++) {
      var db = Number(dbfs[i])
      if (peaks[i] === undefined || db >= peaks[i] || now - at[i] > 1500) {
        peaks[i] = db
        at[i] = now
      }
    }
    meterPeak = peaks
    meterPeakAt = at
  }

  function peakFraction(idx) {
    if (idx < 0 || idx >= meterPeak.length) return 0
    var db = Number(meterPeak[idx])
    if (isNaN(db) || db <= -60) return 0
    return Math.min(1, (db + 60) / 60)
  }

  function pairDbText(a, b) {
    if (b < 0) return meterDbText(a)
    var da = Number(meterDbfs[a]), dbb = Number(meterDbfs[b])
    return meterDbText((isNaN(dbb) || da >= dbb) ? a : b)
  }

  // Source chips grouped by type; over 24 sources, only PCM and Mix are shown.
  function sourceGroup(name) {
    if (name.indexOf("PCM ") === 0) return { order: 0, title: "PC playback" }
    if (name.indexOf("Mix ") === 0) return { order: 1, title: "Mixes" }
    if (name.indexOf("Analogue") === 0 || name.indexOf("Line") === 0) return { order: 2, title: "Analogue" }
    if (name.indexOf("S/PDIF") >= 0) return { order: 3, title: "S/PDIF" }
    if (name.indexOf("ADAT") === 0) return { order: 4, title: "ADAT" }
    if (name === "Off") return { order: 6, title: "Off" }
    return { order: 5, title: "Other" }
  }

  function routeGroups(out) {
    var items = (out && out.route_items) || []
    var capped = items.length > 24
    var groups = {}
    var hidden = 0
    for (var i = 0; i < items.length; i++) {
      var name = String(items[i])
      var g = sourceGroup(name)
      if (capped && g.order > 1) { hidden++; continue }
      if (!groups[g.order]) groups[g.order] = { order: g.order, title: g.title, items: [] }
      groups[g.order].items.push({ name: name, index: i })
    }
    var list = []
    for (var k in groups) list.push(groups[k])
    list.sort(function(x, y) { return x.order - y.order })
    return { groups: list, hidden: hidden }
  }

  function shownRouteIndices(out) {
    var idx = []
    var gs = routeGroups(out).groups
    for (var i = 0; i < gs.length; i++)
      for (var j = 0; j < gs[i].items.length; j++) idx.push(gs[i].items[j].index)
    return idx
  }

  // Input toggles in fixed columns (union of labels, in first-seen order).
  readonly property var inputColumns: {
    var cols = []
    for (var i = 0; i < inputs.length; i++) {
      var ts = inputs[i].toggles || []
      for (var j = 0; j < ts.length; j++) {
        var l = String(ts[j].label || "")
        if (cols.indexOf(l) < 0) cols.push(l)
      }
    }
    return cols
  }

  // docs/DESIGN.md "States". A control the device will not let us change is
  // read-only; one whose value could not be read is unknown and shows "—".
  readonly property string readOnlyHint: "Set on device"
  function isUnknown(t) { return !t || t.value === null || t.value === undefined || t.text === "?" }

  // A disabled qs.Ui control gets no hover, so its own tooltip never shows.
  // This sits over it (or over plain text) and shows `hint` instead.
  component Hint: MouseArea {
    id: hintArea
    property string hint: ""
    hoverEnabled: true
    acceptedButtons: Qt.NoButton
    PanelToolTip {
      visible: hintArea.containsMouse && hintArea.hint !== ""
      text: hintArea.hint
    }
  }

  function toggleIndex(input, label) {
    var ts = (input && input.toggles) || []
    for (var i = 0; i < ts.length; i++) if (String(ts[i].label || "") === label) return i
    return -1
  }

  function send(line) {
    if (daemon.running) daemon.write(line + "\n")
  }

  function setControl(numid, value) {
    send("set " + numid + " " + Math.round(Number(value)) + " " + root.stateGeneration)
  }

  function cycleToggle(t) {
    if (!t || !t.rw) return
    if (t.kind === "bool") {
      setControl(t.numid, t.active ? 0 : 1)
    } else if (t.kind === "enum" && t.items && t.items.length > 0) {
      var cur = (t.value === null || t.value === undefined) ? 0 : Number(t.value)
      var next = (cur + 1) % t.items.length
      setControl(t.numid, next)
    }
  }

  // 48V takes two clicks to enable (arm, then confirm within 3 s) and one to
  // disable. Kenny's ruling, docs/DESIGN.md "Rulings".
  function clickPhantom(t) {
    if (!t || !t.rw) return
    if (t.active) {
      armedPhantom = -1
      setControl(t.numid, 0)
    } else if (armedPhantom === t.numid) {
      armedPhantom = -1
      phantomArmTimer.stop()
      setControl(t.numid, 1)
    } else {
      armedPhantom = t.numid
      phantomArmTimer.restart()
    }
  }

  function navRow() {
    return (navCursor >= 0 && navCursor < navRows.length) ? navRows[navCursor] : null
  }

  function hasNav(kind, index) {
    var row = navRow()
    return !!row && row.kind === kind && row.index === index
  }

  function clampInt(v, lo, hi) { return Math.max(lo, Math.min(hi, v)) }

  function moveCursor(dx, dy) {
    if (navRows.length === 0) return
    if (dy !== 0) {
      navCursor = navCursor < 0 ? 0 : clampInt(navCursor + dy, 0, navRows.length - 1)
      chipCursor = 0
      routeCursor = -1
      var r = navRow()
      if (expandedOutput >= 0 && !(r.kind === "output" && r.index === expandedOutput)) expandedOutput = -1
      return
    }
    var row = navRow()
    if (!row) return
    if (row.kind === "input") {
      var n = (inputs[row.index].toggles || []).length
      chipCursor = clampInt(chipCursor + dx, 0, Math.max(0, n - 1))
    } else if (row.kind === "output" && expandedOutput === row.index) {
      var out = outputs[row.index]
      var items = out.route_items || []
      var shown = shownRouteIndices(out)
      if (shown.length === 0) return
      var from = shown.indexOf(routeCursor >= 0 ? routeCursor : Number(out.route_val || 0))
      routeCursor = shown[clampInt((from < 0 ? 0 : from) + dx, 0, shown.length - 1)]
    }
  }

  // Enter and Space: toggle the chip, arm/confirm 48V, expand an output, or set
  // the highlighted source of an expanded output.
  function activateCursor() {
    var row = navRow()
    if (!row) { if (navRows.length > 0) navCursor = 0; return }
    if (row.kind === "input") {
      var t = (inputs[row.index].toggles || [])[chipCursor]
      if (t) cycleToggle(t)
    } else if (row.kind === "phantom") {
      var ph = phantom[row.index]
      if (String(ph.label || "").indexOf("Remember") < 0) clickPhantom(ph)
      else cycleToggle(ph)
    } else if (row.kind === "output") {
      var out = outputs[row.index]
      var routable = out.route_numid !== null && out.route_numid !== undefined && out.route_rw
      if (expandedOutput === row.index && routeCursor >= 0 && routable) {
        setControl(out.route_numid, routeCursor)
        expandedOutput = -1
        routeCursor = -1
      } else if (routable) {
        expandedOutput = expandedOutput === row.index ? -1 : row.index
        routeCursor = -1
      }
    }
  }

  function muteCursor() {
    var row = navRow()
    if (!row || row.kind !== "output") return
    var out = outputs[row.index]
    if (out.mute_numid !== null && out.mute_numid !== undefined && out.mute_rw)
      setControl(out.mute_numid, out.muted ? 0 : 1)
  }

  function openTui() {
    Quickshell.execDetached([
      "omarchy-launch-or-focus-tui",
      "--app-id=TUI.float",
      "scarlett-tui"
    ])
    root.close()
  }

  function meterFraction(idx) {
    if (idx < 0 || idx >= meterDbfs.length) return 0
    var db = Number(meterDbfs[idx])
    if (isNaN(db) || db <= -60) return 0
    if (db >= 0) return 1
    return (db + 60) / 60
  }

  function meterDbText(idx) {
    if (idx < 0 || idx >= meterDbfs.length) return "−∞"
    var db = Number(meterDbfs[idx])
    if (isNaN(db) || db <= -60) return "−∞"
    return db.toFixed(0) + " dB"
  }

  // Caution tone for the −6 to −1 dBFS zone. Commons.Color has no warning role,
  // so blend accent halfway toward urgent; it follows whichever theme is active.
  readonly property color cautionColor: Qt.tint(Commons.Color.accent, Util.alpha(Commons.Color.urgent, 0.5))

  // Zones match docs/DESIGN.md "Meter zones" and the TUI.
  function meterColor(idx) {
    if (idx < 0 || idx >= meterDbfs.length) return Commons.Color.accent
    var db = Number(meterDbfs[idx])
    if (db >= -1) return Commons.Color.urgent
    if (db >= -6) return root.cautionColor
    return Commons.Color.accent
  }

  function handleMessage(line) {
    var msg
    try { msg = JSON.parse(line) } catch (e) { return }
    daemonOnline = true
    if (msg.type === "error") {
      lastError = String(msg.message || "Control update failed")
      errorTimer.restart()
    } else if (msg.type === "meters" || msg.type === "clip") {
      if (msg.type === "meters") {
        meterDbfs = msg.dbfs || []
        updatePeaks(meterDbfs)
      }
      if (msg.clip !== undefined) {
        clipFlags = msg.clip || []
        clipAny = !!msg.clip_any
      }
    } else if (msg.type === "state") {
      var wasConnected = connected
      connected = !!msg.connected
      mixerAvailable = msg.mixer_available !== false && connected
      stateGeneration = Number(msg.generation || 0)
      card = msg.card || {}
      firmware = msg.firmware !== undefined ? msg.firmware : null
      syncStatus = String(msg.sync_status || "Disconnected")
      locked = !!msg.locked
      activeRate = msg.active_rate !== undefined ? msg.active_rate : null
      inputs = msg.inputs || []
      phantom = msg.phantom || []
      outputs = msg.outputs || []
      hasMatrix = !!msg.has_matrix
      matrixSize = String(msg.matrix_size || "")
      pwScarlettLinks = Number(msg.pw_scarlett_links || 0)
      meterLabels = msg.meter_labels || []
      if (!connected || !mixerAvailable) {
        meterDbfs = []
        clipAny = false
        clipFlags = []
        expandedOutput = -1
      } else if (expandedOutput >= outputs.length) {
        expandedOutput = -1
      }
      if (!wasConnected && connected && root.opened) {
        send("meters on")
      }
    }
  }

  onOpenedChanged: {
    expandedOutput = -1
    armedPhantom = -1
    navCursor = -1
    routeCursor = -1
    showKeys = false
    if (opened) {
      send("refresh")
      send("meters on")
    } else {
      send("meters off")
      meterDbfs = []
    }
  }

  Process {
    id: daemon
    command: ["scarlett-tui", "--daemon"]
    running: true
    stdinEnabled: true
    stdout: SplitParser {
      onRead: function(data) { root.handleMessage(data) }
    }
    onRunningChanged: {
      if (running) {
        root.send("refresh")
        if (root.opened) root.send("meters on")
      }
    }
    onExited: {
      root.daemonOnline = false
      root.connected = false
      root.mixerAvailable = false
      root.stateGeneration = 0
      root.meterDbfs = []
      root.clipAny = false
      root.clipFlags = []
      root.expandedOutput = -1
      restartTimer.start()
    }
  }

  Timer {
    id: restartTimer
    interval: 3000
    onTriggered: daemon.running = true
  }

  Timer {
    id: phantomArmTimer
    interval: 3000
    onTriggered: root.armedPhantom = -1
  }

  Timer {
    id: errorTimer
    interval: 4000
    onTriggered: root.lastError = ""
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    // One glyph; state shown by shape (slash, badge, ring, dot), not only colour.
    text: (root.barState === "ok" || root.barState === "nomixer") ? "󰓃" : "󰓄"
    opacity: root.barState === "ok" ? 1.0 : (root.barState === "nomixer" ? 0.55 : 0.45)
    tooltipText: root.barTooltip
    onPressed: function(b) {
      if (b === Qt.RightButton || b === Qt.MiddleButton) root.openTui()
      else root.toggle()
    }
  }

  // Clip latched: an urgent ring round the bar icon.
  Rectangle {
    visible: root.barState === "ok" && root.clipAny
    anchors.centerIn: button
    width: Math.min(button.width, button.height) - Style.space(2)
    height: width
    radius: Style.space(5)
    color: "transparent"
    border.color: Commons.Color.urgent
    border.width: Math.max(1, Style.space(1.5))
  }

  // Backend restarting: a faint outline round the (slashed) glyph.
  Rectangle {
    visible: root.barState === "backend"
    anchors.centerIn: button
    width: Math.min(button.width, button.height) - Style.space(2)
    height: width
    radius: Style.space(5)
    color: "transparent"
    border.color: Util.alpha(root.barFg, 0.4)
    border.width: 1
  }

  // Mixer unavailable: a caution "!" beside the dimmed glyph.
  Text {
    visible: root.barState === "nomixer"
    textFormat: Text.PlainText
    text: "!"
    color: root.cautionColor
    font.family: root.fontFamily
    font.pixelSize: Style.font.caption
    font.bold: true
    anchors.top: button.top
    anchors.right: button.right
    anchors.rightMargin: Style.space(2)
  }

  // 48V live: a small urgent dot on the bar icon (docs/DESIGN.md "48V live").
  Rectangle {
    visible: root.connected && root.mixerAvailable && root.anyPhantomOn
    width: Style.space(6)
    height: width
    radius: width / 2
    color: Commons.Color.urgent
    anchors.top: button.top
    anchors.right: button.right
    anchors.topMargin: Style.space(3)
    anchors.rightMargin: Style.space(3)
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: panel.loadedContent
    contentWidth: panel.fittedContentWidth(Style.space(400))
    // Tall enough for a 4i4 without scrolling (620 cut off its last output).
    // The shell also stops the card at the screen edge, so small screens scroll.
    contentHeight: panel.fittedContentHeight(panel.loadedContentHeight, Style.space(760))

    content: PanelKeyCatcher {
      id: keyCatcher
      readonly property real footerHeight: panelFooter.visible ? panelFooter.implicitHeight : 0
      implicitHeight: panelColumn.implicitHeight + footerHeight
      anchors.fill: parent
      onCloseRequested: {
        if (root.expandedOutput >= 0) root.expandedOutput = -1
        else root.close()
      }
      onTabRequested: function(direction) { root.switchPanel(direction) }
      onMoveRequested: function(dx, dy) { root.moveCursor(dx, dy) }
      onActivateRequested: root.activateCursor()
      onTextKey: function(t) {
        if (t === "t" || t === "T") root.openTui()
        else if (t === "r" || t === "R") root.send("refresh")
        else if (t === "m" || t === "M") root.muteCursor()
        else if (t === "c" || t === "C") root.clearClips()
        else if (t === "?") root.showKeys = !root.showKeys
      }

      ScrollView {
        id: scrollArea
        anchors.fill: parent
        anchors.bottomMargin: keyCatcher.footerHeight
        clip: true
        ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
        ScrollBar.vertical.policy: panelColumn.implicitHeight > height ? ScrollBar.AsNeeded : ScrollBar.AlwaysOff

        Column {
          id: panelColumn
          width: scrollArea.availableWidth
          spacing: Style.space(12)

          // ---------- Hero header (inspired by omarchy.audio) ----------
          Item {
            width: parent.width
            implicitHeight: Math.max(heroIcon.implicitHeight, heroLabels.implicitHeight, tuiBtn.implicitHeight)

            Text {
              id: heroIcon
              textFormat: Text.PlainText
              text: (root.connected && root.mixerAvailable) ? "󰓃" : "󰓄"
              color: root.fg
              font.family: root.fontFamily
              font.pixelSize: Style.font.display
              opacity: (root.connected && root.mixerAvailable) ? 1.0 : 0.45
              anchors.left: parent.left
              anchors.verticalCenter: parent.verticalCenter
            }

            Button {
              id: tuiBtn
              text: "TUI"
              bordered: true
              foreground: root.fg
              fontFamily: root.fontFamily
              fontSize: Style.font.caption
              tooltipText: "Open full scarlett-tui mixer & matrix (T)"
              anchors.right: parent.right
              anchors.verticalCenter: parent.verticalCenter
              onClicked: root.openTui()
            }

            Column {
              id: heroLabels
              anchors.left: heroIcon.right
              anchors.leftMargin: Style.space(12)
              anchors.right: tuiBtn.left
              anchors.rightMargin: Style.space(10)
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.space(2)

              Text {
                text: root.titleText
                textFormat: Text.PlainText
                color: root.fg
                font.family: root.fontFamily
                font.pixelSize: Style.font.title
                font.bold: true
                elide: Text.ElideRight
                width: parent.width
              }

              Text {
                text: root.subtitleText
                textFormat: Text.PlainText
                color: (root.connected && root.mixerAvailable && root.locked) ? root.fg : root.cautionColor
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                font.bold: true
                elide: Text.ElideRight
                width: parent.width
              }
            }
          }

          // ---------- Clip alert (latched until cleared) ----------
          Item {
            visible: root.connected && root.mixerAvailable && root.clipAny
            width: parent.width
            implicitHeight: clipClear.implicitHeight

            Text {
              textFormat: Text.PlainText
              text: "CLIP  " + root.clippedText
              color: Commons.Color.urgent
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              font.bold: true
              elide: Text.ElideRight
              anchors.left: parent.left
              anchors.right: clipClear.left
              anchors.rightMargin: Style.space(8)
              anchors.verticalCenter: parent.verticalCenter
            }

            Button {
              id: clipClear
              text: "Clear"
              bordered: true
              foreground: root.fg
              fontFamily: root.fontFamily
              fontSize: Style.font.caption
              verticalPadding: Style.space(2)
              tooltipText: "Clear the clip latch"
              anchors.right: parent.right
              anchors.verticalCenter: parent.verticalCenter
              onClicked: root.clearClips()
            }
          }

          // ---------- Inline error banner (write / stale-state rejections) ----------
          Text {
            visible: root.lastError.length > 0
            width: parent.width
            textFormat: Text.PlainText
            text: root.lastError
            color: Commons.Color.urgent
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
            wrapMode: Text.WordWrap
          }

          // ---------- Live Hardware Meters ----------
          // Shown only when the meters cannot be matched to output rows.
          PanelSeparator {
            visible: root.connected && root.mixerAvailable && !root.metersInRows && root.visibleMeters.length > 0
            foreground: root.fg
          }

          Column {
            visible: root.connected && root.mixerAvailable && !root.metersInRows && root.visibleMeters.length > 0
            width: parent.width
            spacing: Style.space(5)

            PanelSectionHeader {
              text: "LEVELS"
              foreground: root.fg
              fontFamily: root.fontFamily
            }

            component MeterTrack: Rectangle {
              property int idx: -1
              height: Style.space(3)
              radius: height / 2
              color: Style.normalFillFor(root.fg, Commons.Color.accent)

              Rectangle {
                width: parent.width * root.meterFraction(parent.idx)
                height: parent.height
                radius: parent.radius
                color: root.meterColor(parent.idx)
                Behavior on width {
                  NumberAnimation { duration: Style.duration(45); easing.type: Easing.OutCubic }
                }
              }

              // Peak-hold tick.
              Rectangle {
                visible: root.peakFraction(parent.idx) > 0
                x: Math.max(0, parent.width * root.peakFraction(parent.idx) - width)
                width: Math.max(1, Style.space(2))
                height: parent.height + Style.space(2)
                anchors.verticalCenter: parent.verticalCenter
                color: root.fg
              }
            }

            Repeater {
              model: root.meterPairs

              Row {
                required property var modelData
                width: panelColumn.width
                spacing: Style.space(8)

                Text {
                  width: Style.space(92)
                  textFormat: Text.PlainText
                  text: String(modelData.label || "")
                  color: root.fg
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  elide: Text.ElideRight
                  anchors.verticalCenter: parent.verticalCenter
                }

                Column {
                  width: Math.max(Style.space(80), panelColumn.width - Style.space(176))
                  spacing: Style.space(2)
                  anchors.verticalCenter: parent.verticalCenter

                  MeterTrack { width: parent.width; idx: modelData.a }
                  MeterTrack { width: parent.width; idx: modelData.b; visible: modelData.b >= 0 }
                }

                Text {
                  width: Style.space(42)
                  horizontalAlignment: Text.AlignRight
                  textFormat: Text.PlainText
                  text: root.pairDbText(modelData.a, modelData.b)
                  color: Util.alpha(root.fg, 0.66)
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  anchors.verticalCenter: parent.verticalCenter
                }

                // Latched clip dot.
                Rectangle {
                  width: Style.space(8)
                  height: width
                  radius: width / 2
                  color: (root.clipFlags[modelData.a] || (modelData.b >= 0 && root.clipFlags[modelData.b]))
                    ? Commons.Color.urgent : "transparent"
                  anchors.verticalCenter: parent.verticalCenter
                }
              }
            }
          }

          // ---------- Inputs & 48V ----------
          PanelSeparator {
            visible: root.connected && root.mixerAvailable && (root.inputs.length > 0 || root.phantom.length > 0)
            foreground: root.fg
          }

          Column {
            visible: root.connected && root.mixerAvailable && (root.inputs.length > 0 || root.phantom.length > 0)
            width: parent.width
            spacing: Style.space(8)

            PanelSectionHeader {
              text: "INPUTS"
              foreground: root.fg
              fontFamily: root.fontFamily
            }

            Row {
              visible: root.inputColumns.length > 0
              width: panelColumn.width
              spacing: Style.space(6)

              Item { width: Style.space(92); height: 1 }

              Repeater {
                model: root.inputColumns

                Text {
                  required property var modelData
                  width: (panelColumn.width - Style.space(92) - Style.space(6) * root.inputColumns.length) / Math.max(1, root.inputColumns.length)
                  textFormat: Text.PlainText
                  text: String(modelData).toLowerCase()
                  color: Util.alpha(root.fg, 0.66)
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  elide: Text.ElideRight
                }
              }
            }

            Repeater {
              model: root.inputs

              Column {
                id: inBlock
                required property var modelData
                required property int index
                readonly property int meterIdx: root.metersInRows ? root.inputMeter(modelData) : -1
                width: panelColumn.width
                spacing: Style.space(3)

              Row {
                id: inRow
                readonly property var modelData: inBlock.modelData
                readonly property int index: inBlock.index
                width: panelColumn.width
                spacing: Style.space(6)

                Text {
                  width: Style.space(92)
                  textFormat: Text.PlainText
                  text: String(inRow.modelData.label || "")
                  color: root.fg
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.bodySmall
                  elide: Text.ElideRight
                  anchors.verticalCenter: parent.verticalCenter
                }

                Repeater {
                  model: root.inputColumns

                  // One fixed column per control type; blank where this input lacks it.
                  Item {
                    id: cell
                    required property var modelData
                    readonly property int tIndex: root.toggleIndex(inRow.modelData, String(modelData))
                    readonly property var toggle: tIndex >= 0 ? inRow.modelData.toggles[tIndex] : null
                    width: (panelColumn.width - Style.space(92) - Style.space(6) * root.inputColumns.length) / Math.max(1, root.inputColumns.length)
                    implicitHeight: chip.implicitHeight

                    Button {
                      id: chip
                      visible: cell.toggle !== null
                      width: parent.width
                      hasCursor: root.hasNav("input", inRow.index) && root.chipCursor === cell.tIndex
                      text: {
                        var t = cell.toggle
                        if (!t) return ""
                        if (root.isUnknown(t)) return String(t.label || "") + " —"
                        if (t.kind === "enum" && t.active && t.label === "AIR") return "AIR · " + String(t.text || "")
                        if (t.kind === "enum") return String(t.text || t.label || "")
                        return String(t.label || "")
                      }
                      bordered: true
                      selected: !!(cell.toggle && cell.toggle.active)
                      enabled: !!(cell.toggle && cell.toggle.rw)
                      opacity: (cell.toggle && cell.toggle.rw) ? 1.0 : 0.45
                      foreground: root.fg
                      fontFamily: root.fontFamily
                      fontSize: Style.font.caption
                      verticalPadding: Style.space(3)
                      tooltipText: cell.toggle ? String(cell.toggle.label || "") + ": " + String(cell.toggle.text || "") : ""
                      onClicked: root.cycleToggle(cell.toggle)
                    }

                    Hint {
                      anchors.fill: chip
                      visible: chip.visible && (!chip.enabled || root.isUnknown(cell.toggle))
                      hint: cell.toggle
                        ? String(cell.toggle.label || "") + ": "
                          + (root.isUnknown(cell.toggle) ? "value could not be read" : root.readOnlyHint)
                        : ""
                    }
                  }
                }
              }

              // The input's own meter, under its chips.
              Row {
                visible: inBlock.meterIdx >= 0
                width: panelColumn.width
                spacing: Style.space(8)

                Item { width: Style.space(92) - Style.space(2); height: 1 }

                MeterTrack {
                  width: panelColumn.width - Style.space(92) - Style.space(72)
                  idx: inBlock.meterIdx
                  anchors.verticalCenter: parent.verticalCenter
                }

                Text {
                  width: Style.space(42)
                  horizontalAlignment: Text.AlignRight
                  textFormat: Text.PlainText
                  text: root.meterDbText(inBlock.meterIdx)
                  color: Util.alpha(root.fg, 0.66)
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  anchors.verticalCenter: parent.verticalCenter
                }

                Rectangle {
                  width: Style.space(8)
                  height: width
                  radius: width / 2
                  color: root.clipFlags[inBlock.meterIdx] ? Commons.Color.urgent : "transparent"
                  anchors.verticalCenter: parent.verticalCenter
                }
              }
              }
            }

            Repeater {
              model: root.phantom

              Item {
                id: phRow
                required property var modelData
                required property int index
                // "Remember 48V" is a persistence setting, not phantom power itself.
                readonly property bool isSwitch: String(modelData.label || "").indexOf("Remember") < 0
                readonly property bool live: isSwitch && !!modelData.active
                readonly property bool armed: isSwitch && root.armedPhantom === modelData.numid
                readonly property bool unknown: root.isUnknown(modelData)
                width: panelColumn.width
                implicitHeight: Math.max(phLabel.implicitHeight, phSwitch.implicitHeight, phButton.implicitHeight)

                Text {
                  id: phLabel
                  textFormat: Text.PlainText
                  text: String(modelData.label || "")
                  color: phRow.live ? Commons.Color.urgent : root.fg
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.bodySmall
                  font.bold: phRow.live
                  anchors.left: parent.left
                  anchors.verticalCenter: parent.verticalCenter
                }

                Button {
                  id: phButton
                  visible: phRow.isSwitch
                  hasCursor: root.hasNav("phantom", phRow.index)
                  text: phRow.unknown ? "48V —"
                    : (phRow.armed ? "Click again to enable 48V" : (phRow.live ? "48V LIVE" : "48V OFF"))
                  bordered: true
                  selected: phRow.live || phRow.armed
                  accent: phRow.armed ? root.cautionColor : Commons.Color.urgent
                  enabled: !!modelData.rw
                  opacity: modelData.rw ? 1.0 : 0.45
                  foreground: root.fg
                  fontFamily: root.fontFamily
                  fontSize: Style.font.caption
                  verticalPadding: Style.space(3)
                  tooltipText: phRow.live
                    ? "48V is on. Click to turn it off."
                    : "Click, then click again within 3 s, to enable 48V"
                  anchors.right: parent.right
                  anchors.verticalCenter: parent.verticalCenter
                  onClicked: root.clickPhantom(modelData)
                }

                ToggleSwitch {
                  id: phSwitch
                  visible: !phRow.isSwitch
                  hasCursor: root.hasNav("phantom", phRow.index)
                  checked: !!modelData.active
                  enabled: !!modelData.rw
                  opacity: modelData.rw ? 1.0 : 0.45
                  foreground: root.fg
                  anchors.right: parent.right
                  anchors.verticalCenter: parent.verticalCenter
                  onToggled: root.cycleToggle(modelData)
                }

                Hint {
                  anchors.fill: phButton.visible ? phButton : phSwitch
                  visible: !modelData.rw || phRow.unknown
                  hint: phRow.unknown ? "48V state could not be read" : root.readOnlyHint
                }
              }
            }
          }

          // ---------- Outputs & Routing ----------
          PanelSeparator {
            visible: root.connected && root.mixerAvailable && root.outputs.length > 0
            foreground: root.fg
          }

          Column {
            visible: root.connected && root.mixerAvailable && root.outputs.length > 0
            width: parent.width
            spacing: Style.space(6)

            Item {
              width: parent.width
              implicitHeight: outHdr.implicitHeight

              PanelSectionHeader {
                id: outHdr
                text: "OUTPUTS"
                foreground: root.fg
                fontFamily: root.fontFamily
                anchors.left: parent.left
              }

              Text {
                textFormat: Text.PlainText
                text: root.hasMatrix ? ("MATRIX " + root.matrixSize) : ""
                color: Qt.darker(root.fg, 1.45)
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                font.bold: true
                anchors.right: parent.right
                anchors.verticalCenter: outHdr.verticalCenter
              }
            }

            Repeater {
              model: root.outputs

              Column {
                required property var modelData
                required property int index
                width: panelColumn.width
                spacing: Style.space(4)

                CursorSurface {
                  width: parent.width
                  height: outRow.implicitHeight + Style.space(10)
                  foreground: root.fg
                  hasCursor: root.hasNav("output", index) || root.expandedOutput === index

                  Item {
                    id: outRow
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.leftMargin: Style.space(8)
                    anchors.rightMargin: Style.space(8)
                    implicitHeight: outCol.implicitHeight

                    Column {
                      id: outCol
                      anchors.left: parent.left
                      anchors.right: muteBtn.left
                      anchors.rightMargin: Style.space(8)
                      spacing: Style.space(1)

                      Text {
                        textFormat: Text.PlainText
                        text: String(modelData.label || "")
                        color: root.fg
                        font.family: root.fontFamily
                        font.pixelSize: Style.font.bodySmall
                        font.bold: true
                        elide: Text.ElideRight
                        width: parent.width
                      }

                      Text {
                        textFormat: Text.PlainText
                        // "?" is a source the device did not report: unknown.
                        text: (modelData.route_summary && modelData.route_summary !== "?") ? String(modelData.route_summary) : "—"
                        color: Qt.darker(root.fg, 1.35)
                        font.family: root.fontFamily
                        font.pixelSize: Style.font.caption
                        elide: Text.ElideRight
                        width: parent.width
                      }

                      // The output's own meter: one block per channel.
                      Row {
                        id: outMeter
                        readonly property int idx: root.outputMeter(modelData)
                        visible: idx >= 0
                        width: parent.width
                        spacing: Style.space(8)
                        topPadding: Style.space(3)

                        MeterTrack {
                          width: parent.width - Style.space(66)
                          idx: outMeter.idx
                          anchors.verticalCenter: parent.verticalCenter
                        }

                        Text {
                          width: Style.space(42)
                          horizontalAlignment: Text.AlignRight
                          textFormat: Text.PlainText
                          text: root.meterDbText(outMeter.idx)
                          color: Util.alpha(root.fg, 0.66)
                          font.family: root.fontFamily
                          font.pixelSize: Style.font.caption
                          anchors.verticalCenter: parent.verticalCenter
                        }

                        Rectangle {
                          width: Style.space(8)
                          height: width
                          radius: width / 2
                          color: root.clipFlags[outMeter.idx] ? Commons.Color.urgent : "transparent"
                          anchors.verticalCenter: parent.verticalCenter
                        }
                      }
                    }

                    Button {
                      id: muteBtn
                      visible: modelData.mute_numid !== null && modelData.mute_numid !== undefined
                      enabled: !!modelData.mute_rw
                      opacity: modelData.mute_rw ? 1.0 : 0.45
                      // Always "MUTE": filled urgent when muted, a quiet outline when live.
                      text: modelData.mute_unknown ? "MUTE —" : "MUTE"
                      bordered: true
                      selected: !!modelData.muted
                      accent: Commons.Color.urgent
                      tooltipText: modelData.muted ? "Muted. Click to unmute." : "Click to mute"

                      foreground: root.fg
                      fontFamily: root.fontFamily
                      fontSize: Style.font.caption
                      verticalPadding: Style.space(2)
                      anchors.right: parent.right
                      anchors.verticalCenter: parent.verticalCenter
                      onClicked: {
                        if (modelData.mute_rw) {
                          root.setControl(modelData.mute_numid, modelData.muted ? 0 : 1)
                        }
                      }
                    }

                    Hint {
                      anchors.fill: muteBtn
                      visible: muteBtn.visible && (!muteBtn.enabled || !!modelData.mute_unknown)
                      hint: modelData.mute_unknown ? "Mute state could not be read" : root.readOnlyHint
                    }
                  }

                  MouseArea {
                    anchors.fill: parent
                    anchors.rightMargin: muteBtn.visible ? muteBtn.width + Style.space(12) : 0
                    cursorShape: (modelData.route_numid !== null && modelData.route_numid !== undefined && modelData.route_rw)
                      ? Qt.PointingHandCursor
                      : Qt.ArrowCursor
                    onClicked: {
                      if (modelData.route_numid !== null && modelData.route_numid !== undefined && modelData.route_rw) {
                        root.expandedOutput = root.expandedOutput === index ? -1 : index
                      }
                    }
                  }
                }

                // Inline source chips, grouped by type, when this output row is expanded
                Column {
                  id: srcCol
                  readonly property var grouped: root.routeGroups(modelData)
                  readonly property var outRow: modelData
                  visible: root.expandedOutput === index && modelData.route_items && modelData.route_items.length > 0
                  width: parent.width
                  spacing: Style.space(4)

                  Repeater {
                    model: srcCol.grouped.groups

                    Column {
                      required property var modelData
                      width: srcCol.width
                      spacing: Style.space(3)

                      Text {
                        textFormat: Text.PlainText
                        text: String(modelData.title)
                        color: Util.alpha(root.fg, 0.66)
                        font.family: root.fontFamily
                        font.pixelSize: Style.font.caption
                      }

                      Flow {
                        width: parent.width
                        spacing: Style.space(4)

                        Repeater {
                          model: modelData.items

                          Button {
                            required property var modelData
                            readonly property bool current: srcCol.outRow.route_val !== null && srcCol.outRow.route_val !== undefined
                              && Number(srcCol.outRow.route_val) === modelData.index
                            hasCursor: root.routeCursor === modelData.index
                            text: (current ? "● " : "") + modelData.name
                            bordered: true
                            selected: current
                            enabled: !!srcCol.outRow.route_rw
                            foreground: root.fg
                            fontFamily: root.fontFamily
                            fontSize: Style.font.caption
                            verticalPadding: Style.space(2)
                            onClicked: {
                              if (srcCol.outRow.route_numid !== null && srcCol.outRow.route_rw) {
                                root.setControl(srcCol.outRow.route_numid, modelData.index)
                                root.expandedOutput = -1
                              }
                            }
                          }
                        }
                      }
                    }
                  }

                  Text {
                    visible: srcCol.grouped.hidden > 0
                    textFormat: Text.PlainText
                    text: "Analogue, ADAT, S/PDIF: " + srcCol.grouped.hidden + " more in TUI"
                    color: Util.alpha(root.fg, 0.66)
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.caption
                  }
                }
              }
            }
          }

        }
      }

      // ---------- Footer: the key line; ? opens the key list ----------
      // Pinned under the scroll area, so the key line (and ?) is always in
      // view however tall the device's channel list is.
      Column {
        id: panelFooter
        visible: root.connected && root.mixerAvailable
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        spacing: Style.space(2)
        topPadding: Style.space(8)

        PanelSeparator {
          foreground: root.fg
        }

        Item { width: 1; height: Style.space(6) }

        Text {
          visible: !root.showKeys
          width: parent.width
          textFormat: Text.PlainText
          text: "↑↓ move · Space toggle · m mute · ? keys"
          color: Util.alpha(root.fg, 0.66)
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          elide: Text.ElideRight
        }

        Repeater {
          model: root.showKeys ? root.keyHelp : []

          Row {
            required property var modelData
            width: panelFooter.width
            spacing: Style.space(8)

            Text {
              width: Style.space(92) - Style.space(8)
              textFormat: Text.PlainText
              text: String(modelData[0])
              color: root.fg
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              font.bold: true
            }

            Text {
              width: parent.width - Style.space(92)
              textFormat: Text.PlainText
              text: String(modelData[1])
              color: Util.alpha(root.fg, 0.66)
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              elide: Text.ElideRight
            }
          }
        }

        Text {
          visible: root.showKeys
          width: parent.width
          textFormat: Text.PlainText
          text: "? closes this list"
          color: Util.alpha(root.fg, 0.66)
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
        }
      }
    }
  }
}
