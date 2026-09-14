// Doctor tab — ported from ui/index.html's #doctor section: 7 health
// checks + the signal-flow graph (NewImprovenments/08 + 06).
import QtQuick
import QtQuick.Layouts
import "../"

ColumnLayout {
    id: root
    required property var appWin
    required property var appTheme
    Layout.fillWidth: true
    spacing: 14

    property string score: ""
    property var checks: [] // [[name,severity,message], ...]
    property var graphNodes: []
    property var graphEdges: []

    function applyHealth(score, checks) {
        root.score = score;
        root.checks = checks;
    }
    function refresh() {
        root.appWin.dcall("RunHealthChecks", "", [], function(data) { root.applyHealth(data[0], data[1] || []); });
        root.appWin.dcall("GetGraph", "", [], function(data) {
            root.graphNodes = (data[0] || []).map(n => ({ id: n[0], label: n[1], kind: n[2], state: n[3] }));
            root.graphEdges = (data[1] || []).map(e => ({ from: e[0], to: e[1] }));
        });
    }
    Component.onCompleted: root.refresh()

    Rectangle {
        Layout.fillWidth: true
        radius: 10
        color: root.appTheme.panel
        border.color: root.appTheme.border
        border.width: 1
        implicitHeight: healthCol.implicitHeight + 32
        ColumnLayout {
            id: healthCol
            anchors.fill: parent
            anchors.margins: 16
            spacing: 8
            RowLayout {
                Layout.fillWidth: true
                Text { text: "HEALTH"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
                Text {
                    text: root.score.toUpperCase()
                    color: root.score === "ok" ? root.appTheme.textDim : root.appTheme.danger
                    font.pixelSize: 11
                    Layout.alignment: Qt.AlignRight
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignRight
                }
            }
            Rectangle {
                implicitWidth: rechkTxt.implicitWidth + 20; implicitHeight: 26; radius: 6
                color: root.appTheme.panel2; border.color: root.appTheme.border; border.width: 1
                Text { id: rechkTxt; anchors.centerIn: parent; text: "↻ Re-check"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
                MouseArea { anchors.fill: parent; onClicked: root.refresh() }
            }
            Repeater {
                model: root.checks
                delegate: Rectangle {
                    id: checkRow
                    required property var modelData
                    Layout.fillWidth: true
                    implicitHeight: 30
                    radius: 6
                    color: root.appTheme.panel2
                    RowLayout {
                        anchors.fill: parent
                        anchors.margins: 8
                        spacing: 8
                        Rectangle {
                            width: 8; height: 8; radius: 4
                            color: checkRow.modelData[1] === "ok" ? root.appTheme.success
                                : checkRow.modelData[1] === "error" ? root.appTheme.danger
                                : checkRow.modelData[1] === "warning" ? root.appTheme.warn
                                : root.appTheme.textFaint
                        }
                        Text { text: checkRow.modelData[0]; color: root.appTheme.text; font.pixelSize: 12; Layout.preferredWidth: 140; elide: Text.ElideRight }
                        Text {
                            text: checkRow.modelData[2]; color: root.appTheme.textFaint; font.pixelSize: 11
                            Layout.fillWidth: true; wrapMode: Text.WordWrap
                        }
                    }
                }
            }
        }
    }

    Rectangle {
        Layout.fillWidth: true
        Layout.preferredHeight: 420
        radius: 10
        color: root.appTheme.panel
        border.color: root.appTheme.border
        border.width: 1
        ColumnLayout {
            anchors.fill: parent
            anchors.margins: 16
            spacing: 8
            Text { text: "SIGNAL FLOW"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
            Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                radius: 8
                color: root.appTheme.inset
                SignalGraph {
                    anchors.fill: parent
                    nodes: root.graphNodes
                    edges: root.graphEdges
                }
            }
        }
    }
}
