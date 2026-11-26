import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Io
import qs.Ui
import qs.Commons

Panel {
  id: root
  moduleName: "io.github.kinggs.scarlett-tui"
  ipcTarget: "io.github.kinggs.scarlett-tui"

  property bool connected: false
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

  readonly property color fg: bar ? bar.foreground : Color.foreground
  readonly property color barFg: bar ? bar.barForeground : Color.foreground
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  readonly property string titleText: {
    if (!connected) return "Scarlett"
    var name = String(card.name || "Focusrite Scarlett")
    return name.replace(/^Focusrite\s+/i, "")
  }

  readonly property string subtitleText: {
    if (!connected) return "NOT CONNECTED"
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

  // Pick up to 6 key meter channels (Outputs + active Inputs) for the compact panel view.
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
    return out
  }

  function send(line) {
    if (daemon.running) daemon.write(line + "\n")
  }

  function setControl(numid, value) {
    send("set " + numid + " " + Math.round(Number(value)))
  }

  function cycleToggle(t) {
    if (!t || !t.rw) return
    if (t.kind === "bool") {
      setControl(t.numid, t.active ? 0 : 1)
    } else if (t.kind === "enum" && t.items && t.items.length > 0) {
      var next = (Number(t.value) + 1) % t.items.length
      setControl(t.numid, next)
    }
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

  function meterColor(idx) {
    if (idx < 0 || idx >= meterDbfs.length) return Color.accent
    var db = Number(meterDbfs[idx])
    if (db >= -1) return Color.urgent
    if (db >= -9) return "#e0af68"
    return Color.accent
  }

  function handleMessage(line) {
    var msg
    try { msg = JSON.parse(line) } catch (e) { return }
    if (msg.type === "meters") {
      meterDbfs = msg.dbfs || []
    } else if (msg.type === "state") {
      connected = !!msg.connected
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
      if (!connected) meterDbfs = []
    }
  }

  onOpenedChanged: {
    expandedOutput = -1
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
    onExited: {
      root.connected = false
      restartTimer.start()
    }
  }

  Timer {
    id: restartTimer
    interval: 3000
    onTriggered: daemon.running = true
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.connected ? "󰓃" : "󰓄"
    opacity: root.connected ? 1.0 : 0.45
    tooltipText: root.connected
      ? root.titleText + " · " + root.subtitleText + " (right-click: open TUI)"
      : "Scarlett not connected"
    onPressed: function(b) {
      if (b === Qt.RightButton || b === Qt.MiddleButton) root.openTui()
      else root.toggle()
    }
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
      onTextKey: function(t) {
        if (t === "t" || t === "T") root.openTui()
        else if (t === "r" || t === "R") root.send("refresh")
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
              text: root.connected ? "󰓃" : "󰓄"
              color: root.fg
              font.family: root.fontFamily
              font.pixelSize: Style.font.display
              opacity: root.connected ? 1.0 : 0.45
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

          // ---------- Live Hardware Meters ----------
          PanelSeparator {
            visible: root.connected && root.visibleMeters.length > 0
            foreground: root.fg
          }

          Column {
            visible: root.connected && root.visibleMeters.length > 0
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
                  color: Style.normalFillFor(root.fg, Color.accent)
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
            visible: root.connected && (root.inputs.length > 0 || root.phantom.length > 0)
            foreground: root.fg
          }

          Column {
            visible: root.connected && (root.inputs.length > 0 || root.phantom.length > 0)
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
                required property var modelData
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
                      text: {
                        if (modelData.kind === "enum" && modelData.active && modelData.label === "AIR")
                          return "AIR · " + String(modelData.text || "")
                        return String(modelData.label || "")
                      }
                      bordered: true
                      selected: !!modelData.active
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
                required property var modelData
                width: panelColumn.width
                implicitHeight: Math.max(phLabel.implicitHeight, phSwitch.implicitHeight)

                Text {
                  id: phLabel
                  textFormat: Text.PlainText
                  text: String(modelData.label || "")
                  color: root.fg
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.bodySmall
                  anchors.left: parent.left
                  anchors.verticalCenter: parent.verticalCenter
                }

                ToggleSwitch {
                  id: phSwitch
                  checked: !!modelData.active
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
            visible: root.connected && root.outputs.length > 0
            foreground: root.fg
          }

          Column {
            visible: root.connected && root.outputs.length > 0
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
                  hasCursor: root.expandedOutput === index

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
                      text: modelData.muted ? "MUTED" : "ON"
                      bordered: true
                      selected: !modelData.muted
                      foreground: root.fg
                      fontFamily: root.fontFamily
                      fontSize: Style.font.caption
                      verticalPadding: Style.space(2)
                      anchors.right: parent.right
                      anchors.verticalCenter: parent.verticalCenter
                      onClicked: root.setControl(modelData.mute_numid, modelData.muted ? 0 : 1)
                    }
                  }

                  MouseArea {
                    anchors.fill: parent
                    anchors.rightMargin: muteBtn.visible ? muteBtn.width + Style.space(12) : 0
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                      if (modelData.route_numid !== null && modelData.route_numid !== undefined) {
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
                      text: String(modelData)
                      bordered: true
                      selected: parentOut ? Number(parentOut.route_val) === index : false
                      foreground: root.fg
                      fontFamily: root.fontFamily
                      fontSize: Style.font.caption
                      verticalPadding: Style.space(2)
                      onClicked: {
                        if (parentOut && parentOut.route_numid !== null) {
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
            visible: root.connected
            foreground: root.fg
          }

          Item {
            visible: root.connected
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
