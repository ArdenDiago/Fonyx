// Settings tab — ported from ui/index.html's #settings section: daemon and
// Quickshell autostart, and the preferred output device. Autostart isn't a
// daemon method; like the desktop app (ui/src-tauri/src/settings.rs) this
// runs `systemctl --user enable|disable --quiet <unit>` itself.
import QtQuick
import QtQuick.Layouts
import Quickshell.Io
import "../"

ColumnLayout {
    id: root
    required property var appWin
    required property var appTheme
    Layout.fillWidth: true
    spacing: 10

    property string preferredOutput: ""

    Component.onCompleted: root.appWin.dcall("GetPreferredDefaultOutput", "", [], function(data) { root.preferredOutput = data[0]; })

    component UnitSwitch: RowLayout {
        id: sw
        required property string label
        required property string unit
        property bool on: false
        Layout.fillWidth: true
        function refresh() { check.running = true; }
        Component.onCompleted: sw.refresh()
        Process {
            id: check
            command: ["systemctl", "--user", "is-enabled", "--quiet", sw.unit]
            onExited: (code) => { sw.on = code === 0; }
        }
        Process {
            id: change
            command: ["systemctl", "--user", sw.on ? "disable" : "enable", "--quiet", sw.unit]
            onExited: sw.refresh()
        }
        Text { text: sw.label; color: root.appTheme.text; font.pixelSize: 12; Layout.fillWidth: true }
        Rectangle {
            width: 34; height: 18; radius: 9
            color: sw.on ? root.appTheme.accentDim : root.appTheme.inset
            border.color: sw.on ? root.appTheme.accent : root.appTheme.border; border.width: 1
            Rectangle { width: 12; height: 12; radius: 6; color: sw.on ? root.appTheme.accent : root.appTheme.textFaint; x: sw.on ? parent.width - width - 2 : 2; anchors.verticalCenter: parent.verticalCenter }
            MouseArea { anchors.fill: parent; onClicked: change.running = true }
        }
    }

    Rectangle {
        Layout.preferredWidth: 440
        radius: 10
        color: root.appTheme.panel
        border.color: root.appTheme.border
        border.width: 1
        implicitHeight: col.implicitHeight + 32
        ColumnLayout {
            id: col
            anchors.fill: parent
            anchors.margins: 16
            spacing: 10
            Text { text: "SETTINGS"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
            UnitSwitch { label: "Start daemon at login"; unit: "fonyx-daemon.service" }
            UnitSwitch { label: "Start this Quickshell app at login"; unit: "fonyx-quickshell.service" }
            Text {
                text: "A unit must be installed first (packaging/install-user.sh)."
                color: root.appTheme.textFaint; font.pixelSize: 11; wrapMode: Text.WordWrap; Layout.fillWidth: true
            }

            RowLayout {
                Layout.fillWidth: true
                Text { text: "Preferred output device"; color: root.appTheme.text; font.pixelSize: 12; Layout.fillWidth: true }
                OutputCombo {
                    id: preferredCombo
                    Layout.preferredWidth: 200
                    // "Auto" first, then every output.
                    outputs: [["", "Auto"]].concat(root.appWin.outputs)
                    currentIndex: Math.max(0, outputs.findIndex(o => o[0] === root.preferredOutput))
                    onActivated: (index) => {
                        root.preferredOutput = outputs[index][0];
                        root.appWin.dcall("SetPreferredDefaultOutput", "s", [root.preferredOutput]);
                    }
                }
            }
            Text {
                text: "Only affects a group with no explicit output routing of its own (Outputs tab)."
                color: root.appTheme.textFaint; font.pixelSize: 11; wrapMode: Text.WordWrap; Layout.fillWidth: true
            }
        }
    }
}
