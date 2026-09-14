// Vertical dB tick ruler beside a fader/meter — ported from ui/index.html's
// .db-scale/dbScaleHtml() (same METER_TICKS_DB, same fractional placement).
import QtQuick

Item {
    id: root
    property real dbMin: -40
    property real dbMax: 12
    readonly property var ticks: [12, 6, 0, -10, -20, -30, -40]
    implicitWidth: 26

    function frac(db) { return Math.max(0, Math.min(1, (db - root.dbMin) / (root.dbMax - root.dbMin))); }

    Repeater {
        model: root.ticks
        delegate: Row {
            required property real modelData
            spacing: 3
            y: root.height * (1 - root.frac(modelData)) - height / 2
            Rectangle {
                width: 5; height: 1
                color: modelData === 0 ? "#3ecbe0" : "#40404c"
                anchors.verticalCenter: parent.verticalCenter
            }
            Text {
                text: (modelData > 0 ? "+" : "") + modelData
                font.pixelSize: 8
                font.family: "monospace"
                color: modelData === 0 ? "#8a8a96" : "#55555f"
            }
        }
    }
}
