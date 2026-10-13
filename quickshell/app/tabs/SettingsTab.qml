// Settings tab — ported from ui/index.html's #settings section. Autostart
// is a Tauri-side command (settings.rs, systemctl --user), not a D-Bus
// method on org.fonyx.Daemon — not reachable from this app via busctl.
// Known gap: this toggle is disabled here with an explanatory note rather
// than silently doing nothing.
import QtQuick
import QtQuick.Layouts

ColumnLayout {
    id: root
    required property var appWin
    required property var appTheme
    Layout.fillWidth: true
    spacing: 10

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
            RowLayout {
                Layout.fillWidth: true
                Text { text: "Start daemon at login"; color: root.appTheme.text; font.pixelSize: 12; Layout.fillWidth: true }
                Rectangle {
                    width: 34; height: 18; radius: 9
                    color: root.appTheme.inset
                    border.color: root.appTheme.border; border.width: 1
                    opacity: 0.5
                    Rectangle { width: 12; height: 12; radius: 6; color: root.appTheme.textFaint; x: 2; anchors.verticalCenter: parent.verticalCenter }
                }
            }
            Text {
                text: "Autostart is controlled by the Tauri UI (systemctl --user), not exposed on the daemon's D-Bus API — toggle it from the desktop app. Rest of Settings (theme, device selection, buffer size) are still placeholders there too."
                color: root.appTheme.textFaint; font.pixelSize: 11; wrapMode: Text.WordWrap; Layout.fillWidth: true
            }
        }
    }
}
