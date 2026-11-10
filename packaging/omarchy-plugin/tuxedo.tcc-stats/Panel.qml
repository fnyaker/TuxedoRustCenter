import QtQuick
import QtQuick.Effects
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

  // Icon only by default (the TUXEDO logo, tinted like the other bar icons).
  // `showLabel: true` in the widget's shell.json entry adds "CPU 12% · 8.4W".
  readonly property bool showLabel: setting("showLabel", false) === true

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    labelText: root.showLabel ? Model.barLabel(root.stats, strings.t("cpuShort")) : ""
    slotSize: Style.bar.iconSlot * (labelText !== "" && !vertical ? 3 : 1)
    tooltipText: ""
    iconComponent: Component {
      Item {
        Image {
          id: logo
          anchors.centerIn: parent
          width: button.iconSize
          height: width * 0.95
          source: Qt.resolvedUrl("logo.svg")
          sourceSize.width: width * 2
          sourceSize.height: height * 2
          fillMode: Image.PreserveAspectFit
          smooth: true
          layer.enabled: true
          layer.effect: MultiEffect {
            colorization: 1.0
            colorizationColor: root.bar ? root.bar.foreground : Commons.Color.foreground
          }
        }
      }
    }
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

          // Search box: filters the list below as you type.
          Rectangle {
            width: parent.width
            height: Style.space(30)
            radius: Style.cornerRadius
            color: "transparent"
            border.width: 1
            border.color: Qt.rgba(root.bar.foreground.r, root.bar.foreground.g, root.bar.foreground.b, 0.25)

            FilterInput {
              id: filter
              anchors.fill: parent
              anchors.leftMargin: Style.space(10)
              anchors.rightMargin: Style.space(10)
              verticalAlignment: TextInput.AlignVCenter
              color: root.bar.foreground
              font.family: root.bar.fontFamily
              font.pixelSize: Style.font.bodySmall
              keyHandler: null
            }

            Text {
              anchors.fill: filter
              verticalAlignment: Text.AlignVCenter
              visible: filter.text === ""
              textFormat: Text.PlainText
              text: strings.t("search")
              color: root.bar.foreground
              opacity: 0.45
              font.family: root.bar.fontFamily
              font.pixelSize: Style.font.bodySmall
            }
          }

          ListView {
            id: profileView
            width: parent.width
            height: Math.min(contentHeight, Style.space(168))
            clip: true
            interactive: contentHeight > height
            boundsBehavior: Flickable.StopAtBounds
            model: Model.filterProfiles(root.profileList, filter.text)

            delegate: Button {
              required property var modelData
              width: ListView.view ? ListView.view.width : 0
              text: modelData.name
              leftAlign: true
              fontSize: Style.font.bodySmall
              foreground: root.bar.foreground
              fontFamily: root.bar.fontFamily
              active: modelData.active
              onClicked: root.useProfile(modelData.id)
            }
          }

          PlainText {
            width: parent.width
            visible: profileView.count === 0
            opacity: 0.6
            text: strings.t("noMatch")
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
