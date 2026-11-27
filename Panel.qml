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
    var parts = []
    if (card.generation) parts.push(String(card.generation).toUpperCase())
    if (activeRate) {
      var khz = Number(activeRate) / 1000
      parts.push((Math.round(khz) === khz ? khz.toString() : khz.toFixed(1)) + " KHZ")
    }
    parts.push(locked ? "LOCKED" : String(syncStatus || "UNLOCKED").toUpperCase())
    if (firmware !== null && firmware !== undefined) parts.push("FW " + String(firmware))
    return parts.join(" · ")
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
      if (out.length >= 6) break
    }
    if (out.length === 0) {
      for (var j = 0; j < meterLabels.length && out.length < 6; j++) {
        var fm = meterLabels[j]
        if (fm && fm.group === "Meters") out.push(fm)
      }
    }
    return out
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
      var from = routeCursor >= 0 ? routeCursor : Number(out.route_val || 0)
      routeCursor = clampInt(from + dx, 0, Math.max(0, items.length - 1))
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

  // Caution tone for the −9 to −1 dBFS zone. Commons.Color has no warning role,
  // so blend accent halfway toward urgent; it follows whichever theme is active.
  readonly property color cautionColor: Qt.tint(Commons.Color.accent, Util.alpha(Commons.Color.urgent, 0.5))

  // Zones match docs/DESIGN.md "Meter zones" and the TUI.
  function meterColor(idx) {
    if (idx < 0 || idx >= meterDbfs.length) return Commons.Color.accent
    var db = Number(meterDbfs[idx])
    if (db >= -1) return Commons.Color.urgent
    if (db >= -9) return root.cautionColor
    return Commons.Color.accent
  }

  function handleMessage(line) {
    var msg
    try { msg = JSON.parse(line) } catch (e) { return }
    daemonOnline = true
    if (msg.type === "error") {
      lastError = String(msg.message || "Control update failed")
      errorTimer.restart()
    } else if (msg.type === "meters") {
      meterDbfs = msg.dbfs || []
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
    text: (root.connected && root.mixerAvailable) ? "󰓃" : "󰓄"
    opacity: (root.connected && root.mixerAvailable) ? 1.0 : 0.45
    tooltipText: !root.daemonOnline
      ? "Scarlett backend unavailable (restarting)"
      : root.connected
        ? root.titleText + (root.anyPhantomOn ? " · 48V LIVE" : "") + " · " + root.subtitleText
          + " (right-click: open TUI · scroll: no action)"
        : "Scarlett not connected"
    onPressed: function(b) {
      if (b === Qt.RightButton || b === Qt.MiddleButton) root.openTui()
      else root.toggle()
    }
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
    contentHeight: panel.fittedContentHeight(panel.loadedContentHeight, Style.space(620))

    content: PanelKeyCatcher {
      id: keyCatcher
      implicitHeight: panelColumn.implicitHeight
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
      }

      ScrollView {
        id: scrollArea
        anchors.fill: parent
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
                color: Qt.darker(root.fg, 1.4)
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                font.bold: true
                font.letterSpacing: 1.0
                elide: Text.ElideRight
                width: parent.width
              }
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
          PanelSeparator {
            visible: root.connected && root.mixerAvailable && root.visibleMeters.length > 0
            foreground: root.fg
          }

          Column {
            visible: root.connected && root.mixerAvailable && root.visibleMeters.length > 0
            width: parent.width
            spacing: Style.space(5)

            PanelSectionHeader {
              text: "LEVELS"
              foreground: root.fg
              fontFamily: root.fontFamily
            }

            Repeater {
              model: root.visibleMeters

              Row {
                required property var modelData
                width: panelColumn.width
                spacing: Style.space(8)

                Text {
                  width: Style.space(115)
                  textFormat: Text.PlainText
                  text: String(modelData.label || "")
                  color: Qt.darker(root.fg, 1.15)
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  elide: Text.ElideRight
                  anchors.verticalCenter: parent.verticalCenter
                }

                Rectangle {
                  width: Math.max(Style.space(80), panelColumn.width - Style.space(175))
                  height: Style.space(6)
                  radius: height / 2
                  color: Style.normalFillFor(root.fg, Commons.Color.accent)
                  anchors.verticalCenter: parent.verticalCenter

                  Rectangle {
                    width: parent.width * root.meterFraction(modelData.index)
                    height: parent.height
                    radius: parent.radius
                    color: root.meterColor(modelData.index)
                    Behavior on width {
                      NumberAnimation { duration: Style.duration(45); easing.type: Easing.OutCubic }
                    }
                  }
                }

                Text {
                  width: Style.space(42)
                  horizontalAlignment: Text.AlignRight
                  textFormat: Text.PlainText
                  text: root.meterDbText(modelData.index)
                  color: Qt.darker(root.fg, 1.35)
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
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

            Repeater {
              model: root.inputs

              Item {
                id: inRow
                required property var modelData
                required property int index
                width: panelColumn.width
                implicitHeight: Math.max(inLabel.implicitHeight, chipRow.implicitHeight)

                Text {
                  id: inLabel
                  textFormat: Text.PlainText
                  text: String(modelData.label || "")
                  color: root.fg
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.bodySmall
                  anchors.left: parent.left
                  anchors.verticalCenter: parent.verticalCenter
                }

                Row {
                  id: chipRow
                  spacing: Style.space(6)
                  anchors.right: parent.right
                  anchors.verticalCenter: parent.verticalCenter

                  Repeater {
                    model: modelData.toggles || []

                    Button {
                      required property var modelData
                      required property int index
                      hasCursor: root.hasNav("input", inRow.index) && root.chipCursor === index
                      text: {
                        if (modelData.kind === "enum" && modelData.active && modelData.label === "AIR")
                          return "AIR · " + String(modelData.text || "")
                        return String(modelData.label || "")
                      }
                      bordered: true
                      selected: !!modelData.active
                      enabled: !!modelData.rw
                      opacity: modelData.rw ? 1.0 : 0.45
                      foreground: root.fg
                      fontFamily: root.fontFamily
                      fontSize: Style.font.caption
                      verticalPadding: Style.space(3)
                      tooltipText: String(modelData.label || "") + ": " + String(modelData.text || "")
                      onClicked: root.cycleToggle(modelData)
                    }
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
                  text: phRow.armed ? "Click again to enable 48V" : (phRow.live ? "48V LIVE" : "48V OFF")
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
                text: "OUTPUT ROUTING"
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
                        text: modelData.route_summary ? String(modelData.route_summary) : "—"
                        color: Qt.darker(root.fg, 1.35)
                        font.family: root.fontFamily
                        font.pixelSize: Style.font.caption
                        elide: Text.ElideRight
                        width: parent.width
                      }
                    }

                    Button {
                      id: muteBtn
                      visible: modelData.mute_numid !== null && modelData.mute_numid !== undefined
                      enabled: !!modelData.mute_rw
                      opacity: modelData.mute_rw ? 1.0 : 0.45
                      // Always "MUTE": filled urgent when muted, a quiet outline when live.
                      text: "MUTE"
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

                // Inline route picker chips when this output row is expanded
                Flow {
                  visible: root.expandedOutput === index && modelData.route_items && modelData.route_items.length > 0
                  width: parent.width
                  spacing: Style.space(4)

                  Repeater {
                    model: modelData.route_items || []

                    Button {
                      required property var modelData
                      required property int index
                      readonly property var parentOut: root.outputs[root.expandedOutput]
                      hasCursor: root.routeCursor === index
                      text: String(modelData)
                      bordered: true
                      selected: parentOut && parentOut.route_val !== null && parentOut.route_val !== undefined
                        ? Number(parentOut.route_val) === index
                        : false
                      enabled: parentOut ? !!parentOut.route_rw : false
                      foreground: root.fg
                      fontFamily: root.fontFamily
                      fontSize: Style.font.caption
                      verticalPadding: Style.space(2)
                      onClicked: {
                        if (parentOut && parentOut.route_numid !== null && parentOut.route_rw) {
                          root.setControl(parentOut.route_numid, index)
                          root.expandedOutput = -1
                        }
                      }
                    }
                  }
                }
              }
            }
          }

          // ---------- Footer: Matrix + PipeWire links shortcut ----------
          PanelSeparator {
            visible: root.connected && root.mixerAvailable
            foreground: root.fg
          }

          Item {
            visible: root.connected && root.mixerAvailable
            width: parent.width
            implicitHeight: footerBtn.implicitHeight

            Text {
              textFormat: Text.PlainText
              text: root.pwScarlettLinks > 0
                ? (root.pwScarlettLinks + " active PipeWire links · press T for matrix & links")
                : "Press T for full Mix Matrix, Routing & PipeWire Links"
              color: Qt.darker(root.fg, 1.4)
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              elide: Text.ElideRight
              anchors.left: parent.left
              anchors.right: footerBtn.left
              anchors.rightMargin: Style.space(8)
              anchors.verticalCenter: parent.verticalCenter
            }

            Button {
              id: footerBtn
              text: "Open Matrix"
              bordered: true
              foreground: root.fg
              fontFamily: root.fontFamily
              fontSize: Style.font.caption
              verticalPadding: Style.space(3)
              anchors.right: parent.right
              anchors.verticalCenter: parent.verticalCenter
              onClicked: root.openTui()
            }
          }
        }
      }
    }
  }
}
