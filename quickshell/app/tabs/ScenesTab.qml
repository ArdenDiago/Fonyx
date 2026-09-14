// Scenes tab — ported from ui/index.html's #scenes section
// (NewImprovenments/01-audio-scenes.md). Duplicate uses an inline name
// field instead of a native prompt() dialog (QML has none) — a small,
// disclosed UI difference from index.html, not a missing feature.
import QtQuick
import QtQuick.Layouts

ColumnLayout {
    id: root
    required property var appWin
    required property var appTheme
    Layout.fillWidth: true
    spacing: 10

    property var scenes: [] // [[id, name, desc], ...]

    function refresh() {
        root.appWin.dcall("ListScenes", "", [], function(data) { root.scenes = data[0] || []; });
    }
    Component.onCompleted: root.refresh()

    RowLayout {
        Layout.alignment: Qt.AlignRight
        spacing: 8
        Rectangle {
            implicitWidth: 180; implicitHeight: 28; radius: 5
            color: root.appTheme.panel2; border.color: root.appTheme.border; border.width: 1
            TextInput {
                id: newSceneName
                anchors.fill: parent; anchors.margins: 6
                color: root.appTheme.text; font.pixelSize: 12; verticalAlignment: TextInput.AlignVCenter; clip: true
            }
            Text {
                anchors.left: parent.left; anchors.leftMargin: 6; anchors.verticalCenter: parent.verticalCenter
                text: "Scene name"; color: root.appTheme.textFaint; font.pixelSize: 12
                visible: newSceneName.text.length === 0
            }
        }
        Rectangle {
            implicitWidth: captureTxt.implicitWidth + 20; implicitHeight: 28; radius: 6
            color: root.appTheme.panel2; border.color: root.appTheme.border; border.width: 1
            Text { id: captureTxt; anchors.centerIn: parent; text: "Capture current as new scene"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
            MouseArea {
                anchors.fill: parent
                onClicked: {
                    const name = newSceneName.text.trim();
                    if (!name) return;
                    root.appWin.dcall("CaptureCurrentAsScene", "s", [name], function() { root.refresh(); });
                    newSceneName.text = "";
                }
            }
        }
    }

    Flow {
        Layout.fillWidth: true
        spacing: 14

        Repeater {
            model: root.scenes
            delegate: Rectangle {
                id: card
                required property var modelData
                property bool duplicating: false
                width: 260
                implicitHeight: col.implicitHeight + 32
                radius: 10
                color: root.appTheme.panel
                border.color: root.appTheme.border
                border.width: 1

                ColumnLayout {
                    id: col
                    anchors.fill: parent
                    anchors.margins: 16
                    spacing: 6
                    Text { text: card.modelData[1]; color: root.appTheme.text; font.pixelSize: 13; font.bold: true }
                    Text {
                        visible: card.modelData[2] && card.modelData[2].length > 0
                        text: card.modelData[2]; color: root.appTheme.textFaint; font.pixelSize: 11; wrapMode: Text.WordWrap; Layout.fillWidth: true
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 6
                        Rectangle {
                            implicitWidth: actTxt.implicitWidth + 16; implicitHeight: 24; radius: 5
                            color: root.appTheme.accent
                            Text { id: actTxt; anchors.centerIn: parent; text: "▶ Activate"; font.pixelSize: 10; font.bold: true; color: "#06272c" }
                            MouseArea {
                                anchors.fill: parent
                                onClicked: root.appWin.dcall("ActivateScene", "u", [card.modelData[0]], function() { root.appWin.refreshAll(); })
                            }
                        }
                        Rectangle {
                            implicitWidth: updTxt.implicitWidth + 16; implicitHeight: 24; radius: 5
                            color: root.appTheme.panel2; border.color: root.appTheme.border; border.width: 1
                            Text { id: updTxt; anchors.centerIn: parent; text: "↻ Update"; font.pixelSize: 10; color: root.appTheme.textDim }
                            MouseArea {
                                anchors.fill: parent
                                onClicked: root.appWin.dcall("UpdateScene", "u", [card.modelData[0]], function() { root.refresh(); })
                            }
                        }
                        Rectangle {
                            implicitWidth: 24; implicitHeight: 24; radius: 5
                            color: root.appTheme.panel2; border.color: root.appTheme.border; border.width: 1
                            Text { anchors.centerIn: parent; text: "⎘"; font.pixelSize: 11; color: root.appTheme.textDim }
                            MouseArea { anchors.fill: parent; onClicked: card.duplicating = !card.duplicating }
                        }
                        Rectangle {
                            implicitWidth: 24; implicitHeight: 24; radius: 5; color: "transparent"
                            Text { anchors.centerIn: parent; text: "✕"; color: root.appTheme.textDim; font.pixelSize: 12 }
                            MouseArea {
                                anchors.fill: parent
                                onClicked: root.appWin.dcall("DeleteScene", "u", [card.modelData[0]], function() { root.refresh(); })
                            }
                        }
                    }
                    RowLayout {
                        visible: card.duplicating
                        Layout.fillWidth: true
                        Rectangle {
                            Layout.fillWidth: true; implicitHeight: 24; radius: 4
                            color: root.appTheme.inset; border.color: root.appTheme.border; border.width: 1
                            TextInput {
                                id: dupName
                                anchors.fill: parent; anchors.margins: 4
                                color: root.appTheme.text; font.pixelSize: 11; verticalAlignment: TextInput.AlignVCenter; clip: true
                            }
                        }
                        Rectangle {
                            implicitWidth: 40; implicitHeight: 24; radius: 4
                            color: root.appTheme.accent
                            Text { anchors.centerIn: parent; text: "OK"; font.pixelSize: 10; font.bold: true; color: "#06272c" }
                            MouseArea {
                                anchors.fill: parent
                                onClicked: {
                                    const n = dupName.text.trim();
                                    if (!n) return;
                                    root.appWin.dcall("DuplicateScene", "us", [card.modelData[0], n], function() { root.refresh(); });
                                    card.duplicating = false;
                                    dupName.text = "";
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Text {
        visible: root.scenes.length === 0
        text: "No scenes yet — set groups/routing up the way you want, then capture it above."
        color: root.appTheme.textFaint; font.pixelSize: 11
    }
    Text {
        text: "A scene snapshots every group's gain/EQ/HPF/LPF/compressor/gate/output-routes plus which app is routed where."
        color: root.appTheme.textFaint; font.pixelSize: 11; wrapMode: Text.WordWrap; Layout.fillWidth: true
    }
}
