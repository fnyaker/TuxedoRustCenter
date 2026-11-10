import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import "Model.js" as Model

// TUXEDO Control Center — bar widget. All data and actions go through
// `tcc-cli` (non-privileged); this file holds presentation only.
Panel {
  id: root
  moduleName: "tuxedo.tcc-stats"
  ipcTarget: "tuxedo.tcc-stats"

  Commons.Translations { id: strings; directory: Qt.resolvedUrl("i18n") }

  property var stats: null
  property var profileList: []

  readonly property bool online: !!(stats && stats.daemon)
  readonly property string profileName: online && stats.profile ? String(stats.profile.name) : ""

  function refresh() {
    if (statsProc.running) return
    // Sensors (CPU package power) are only collected while the panel is open.
    statsProc.command = root.opened
      ? ["tcc-cli", "--json", "stats"]
      : ["tcc-cli", "--json", "stats", "--no-sensors"]
    statsProc.running = true
  }

  function refreshProfiles() {
    if (!listProc.running) listProc.running = true
  }

  function useProfile(id) {
    if (actionProc.running) return
    actionProc.command = ["tcc-cli", "profile", "use", String(id)]
    actionProc.running = true
  }

  function clearOverride() {
    if (actionProc.running) return
    actionProc.command = ["tcc-cli", "profile", "clear"]
    actionProc.running = true
  }

  Component.onCompleted: refresh()
  onOpenedChanged: if (opened) { refresh(); refreshProfiles() }

  Timer {
    interval: root.opened ? 2000 : 5000
    running: true
    repeat: true
    onTriggered: root.refresh()
  }

  Process {
    id: statsProc
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var parsed = Model.parseJson(text)
        // Keep the last good reading across a transient failure.
        if (parsed) root.stats = parsed
      }
    }
  }

  Process {
    id: listProc
    command: ["tcc-cli", "--json", "profile", "list"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.profileList = Model.profiles(Model.parseJson(text))
    }
  }

  Process {
    id: actionProc
    onExited: { root.refresh(); root.refreshProfiles() }
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    // Pas de glyphe : la police des icônes de la barre (omarchy.ttf) ne couvre
    // pas les glyphes Nerd Font ; le libellé suffit.
    text: ""
    labelText: Model.barLabel(root.stats)
    slotSize: Style.bar.iconSlot * (labelText !== "" && !vertical ? 3 : 1)
    tooltipText: ""
    onPressed: function(b) { root.toggle() }
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: panel.loadedContent
    contentWidth: panel.fittedContentWidth(Style.space(340))
    contentHeight: panel.fittedContentHeight(panel.loadedContentHeight)

    content: PanelKeyCatcher {
      anchors.fill: parent
      implicitHeight: column.implicitHeight
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }

      Column {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        spacing: Style.space(12)

        PlainText {
          width: parent.width
          visible: !root.online
          text: strings.t("offline")
          opacity: 0.7
        }

        Column {
          width: parent.width
          visible: root.online
          spacing: Style.space(12)

          PlainText {
            width: parent.width
            text: root.profileName
            font.pixelSize: Style.font.title
            font.bold: true
            elide: Text.ElideRight
          }
          PlainText {
            width: parent.width
            opacity: 0.6
            text: {
              if (!root.online) return ""
              var src = root.stats.powerSource === "ac" ? strings.t("ac") : strings.t("onBattery")
              return root.stats.manualOverride ? src + " · " + strings.t("manual") : src
            }
          }

          Column {
            width: parent.width
            spacing: Style.spacing.labelGap
            InfoPair { label: strings.t("cpu"); value: Model.number(root.online ? root.stats.cpu.usagePercent : null, 0, "%") }
            InfoPair { label: strings.t("cpuPower"); value: Model.number(root.online ? root.stats.cpu.packagePowerW : null, 1, "W") }
            InfoPair { label: strings.t("cpuTemp"); value: Model.number(root.online ? root.stats.cpu.temperatureC : null, 0, "°C") }
            InfoPair {
              visible: root.online && root.stats.battery.present === true
              label: strings.t("battery")
              value: root.online && root.stats.battery.present ? Model.number(root.stats.battery.percent, 0, "%") : "—"
            }
            InfoPair {
              visible: root.online && root.stats.battery.present === true
              label: strings.t("batteryPower")
              value: Model.number(root.online && root.stats.battery.present ? root.stats.battery.powerW : null, 1, "W")
            }
          }

          PanelSeparator { foreground: root.bar.foreground }

          PanelSectionHeader {
            text: strings.t("profile")
            foreground: root.bar.foreground
            fontFamily: root.bar.fontFamily
          }

          Flow {
            width: parent.width
            spacing: Style.space(6)

            Repeater {
              model: root.profileList
              Button {
                required property var modelData
                text: modelData.name
                fontSize: Style.font.bodySmall
                foreground: root.bar.foreground
                fontFamily: root.bar.fontFamily
                bordered: true
                active: modelData.active
                onClicked: root.useProfile(modelData.id)
              }
            }

            Button {
              visible: root.online && root.stats.manualOverride === true
              text: strings.t("auto")
              fontSize: Style.font.bodySmall
              foreground: root.bar.foreground
              fontFamily: root.bar.fontFamily
              bordered: true
              onClicked: root.clearOverride()
            }
          }
        }
      }
    }
  }

  component PlainText: Text {
    textFormat: Text.PlainText
    color: root.bar.foreground
    font.family: root.bar.fontFamily
    font.pixelSize: Style.font.bodySmall
    wrapMode: Text.WordWrap
  }

  component InfoPair: Row {
    property string label: ""
    property string value: ""
    width: parent.width
    spacing: Style.space(8)

    PlainText { text: label; opacity: 0.6 }
    Item { width: Math.max(0, parent.width - parent.children[0].implicitWidth - parent.children[2].implicitWidth - parent.spacing * 2); height: 1 }
    PlainText { text: value; wrapMode: Text.NoWrap }
  }
}
