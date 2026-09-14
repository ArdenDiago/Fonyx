// Recording tab — ported from ui/index.html's #recording section
// (NewImprovenments/09-per-app-recording.md). WAV only, same as there.
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls.Basic as Controls
import "../"

ColumnLayout {
    id: root
    required property var appWin
    required property var appTheme
    Layout.fillWidth: true
    spacing: 14

    property var recordings: [] // [[id,groupId,path,elapsedSecs,framesWritten], ...]

    function refresh() {
        root.appWin.dcall("ListRecordings", "", [], function(data) { root.recordings = data[0] || []; });
    }
    Component.onCompleted: root.refresh()

    Rectangle {
        Layout.fillWidth: true
        radius: 10
        color: root.appTheme.panel
        border.color: root.appTheme.border
        border.width: 1
        implicitHeight: recCol.implicitHeight + 32
        ColumnLayout {
            id: recCol
            anchors.fill: parent
            anchors.margins: 16
            spacing: 8
            Text { text: "RECORD A GROUP"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
            Flow {
                Layout.fillWidth: true
                spacing: 6
                GroupCombo { id: recGroup; groups: root.appWin.groups }
                Controls.ComboBox {
                    id: recFormat
                    model: ["WAV (32-bit float)", "WAV (16-bit PCM)"]
                    property var codes: ["wav32", "wav16"]
                }
                Rectangle {
                    width: 280; height: 28; radius: 5; color: root.appTheme.panel2; border.color: root.appTheme.border; border.width: 1
                    TextInput { id: recPath; anchors.fill: parent; anchors.margins: 6; color: root.appTheme.text; font.pixelSize: 11; verticalAlignment: TextInput.AlignVCenter; clip: true }
                    Text {
                        anchors.left: parent.left; anchors.leftMargin: 6; anchors.verticalCenter: parent.verticalCenter
                        text: "/home/you/Music/recording.wav"; color: root.appTheme.textFaint; font.pixelSize: 11
                        visible: recPath.text.length === 0
                    }
                }
                Rectangle {
                    implicitWidth: 70; height: 28; radius: 5; color: root.appTheme.accent
                    Text { anchors.centerIn: parent; text: "● Record"; font.pixelSize: 11; font.bold: true; color: "#06272c" }
                    MouseArea {
                        anchors.fill: parent
                        onClicked: {
                            const gid = recGroup.currentGroupId();
                            const path = recPath.text.trim();
                            if (gid === null || !path) return;
                            root.appWin.dcall("StartRecording", "uss", [gid, path, recFormat.codes[recFormat.currentIndex]], function() { root.refresh(); });
                        }
                    }
                }
            }
            Text {
                text: "Recording taps the group's signal after its full DSP chain (EQ/compressor/gate/plugins) without touching its normal playback route. FLAC/Opus aren't implemented — WAV only."
                color: root.appTheme.textFaint; font.pixelSize: 11; wrapMode: Text.WordWrap; Layout.fillWidth: true
            }
        }
    }

    Rectangle {
        Layout.fillWidth: true
        radius: 10
        color: root.appTheme.panel
        border.color: root.appTheme.border
        border.width: 1
        implicitHeight: activeCol.implicitHeight + 32
        ColumnLayout {
            id: activeCol
            anchors.fill: parent
            anchors.margins: 16
            spacing: 6
            Text { text: "ACTIVE RECORDINGS"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
            Repeater {
                model: root.recordings
                delegate: Rectangle {
                    id: recRow
                    required property var modelData
                    Layout.fillWidth: true
                    implicitHeight: 28
                    radius: 6
                    color: root.appTheme.panel2
                    RowLayout {
                        anchors.fill: parent
                        anchors.margins: 8
                        spacing: 8
                        Rectangle { width: 8; height: 8; radius: 4; color: root.appTheme.success }
                        Text {
                            text: recRow.modelData[2].split("/").pop()
                            color: root.appTheme.text; font.pixelSize: 12; Layout.fillWidth: true; elide: Text.ElideRight
                        }
                        Text {
                            text: Math.floor(recRow.modelData[3] / 60) + ":" + String(Math.floor(recRow.modelData[3] % 60)).padStart(2, "0") + " · " + recRow.modelData[4] + " frames"
                            color: root.appTheme.textDim; font.pixelSize: 11; font.family: "monospace"
                        }
                        Rectangle {
                            implicitWidth: 50; height: 22; radius: 5; color: "transparent"; border.color: root.appTheme.border; border.width: 1
                            Text { anchors.centerIn: parent; text: "■ Stop"; font.pixelSize: 10; color: root.appTheme.danger }
                            MouseArea {
                                anchors.fill: parent
                                onClicked: root.appWin.dcall("StopRecording", "u", [recRow.modelData[0]], function() { root.refresh(); })
                            }
                        }
                    }
                }
            }
            Text { visible: root.recordings.length === 0; text: "Nothing recording right now."; color: root.appTheme.textFaint; font.pixelSize: 11 }
        }
    }
}
