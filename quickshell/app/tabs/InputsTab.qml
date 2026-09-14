// Inputs tab — ported from ui/index.html's #inputs section. A stream
// disappears from the Inputs list once routed into a real (non-Default)
// group, and shows up nested under that group instead, mirroring
// index.html's renderInputs exactly (Default's own streams stay listed
// here since Default isn't a deliberate assignment).
import QtQuick
import QtQuick.Layouts
import "../"

ColumnLayout {
    id: root
    required property var appWin
    required property var appTheme
    Layout.fillWidth: true
    spacing: 10

    // Client-side mute/solo bookkeeping — the daemon has no "is this
    // stream muted" query (ListStreams doesn't return it), so this app is
    // the only record of what SetStreamMuted calls it already made, same
    // limitation index.html's own mutedStreams/soloedStreams Sets have.
    property var mutedStreams: []
    property var soloedStreams: []

    function isStreamMuted(id) { return root.mutedStreams.indexOf(id) !== -1; }
    function isStreamSoloed(id) { return root.soloedStreams.indexOf(id) !== -1; }
    function isStreamEffectivelyMuted(id) {
        if (root.isStreamMuted(id)) return true;
        return root.soloedStreams.length > 0 && !root.isStreamSoloed(id);
    }
    function applyStreamMutes() {
        for (const s of root.appWin.streams) {
            root.appWin.dcall("SetStreamMuted", "ub", [s[0], root.isStreamEffectivelyMuted(s[0]) ? "true" : "false"]);
        }
    }
    function toggleStreamMute(id) {
        const idx = root.mutedStreams.indexOf(id);
        root.mutedStreams = idx === -1 ? root.mutedStreams.concat([id]) : root.mutedStreams.filter(x => x !== id);
        root.applyStreamMutes();
    }
    function toggleStreamSolo(id) {
        const idx = root.soloedStreams.indexOf(id);
        root.soloedStreams = idx === -1 ? root.soloedStreams.concat([id]) : root.soloedStreams.filter(x => x !== id);
        root.applyStreamMutes();
    }
    function clearAllStreamMute() { root.mutedStreams = []; root.applyStreamMutes(); }
    function clearAllStreamSolo() { root.soloedStreams = []; root.applyStreamMutes(); }

    // routedIds: every stream id already sitting in a *real* (non-Default)
    // group — those are the only ones actually excluded from the Inputs
    // list, same rule as index.html's own `routedIds`.
    function routedIds() {
        const s = new Set();
        for (const g of root.appWin.groups) if (!g[4]) for (const id of (g[3] || [])) s.add(id);
        return s;
    }
    function unroutedStreams() {
        const routed = root.routedIds();
        return root.appWin.streams.filter(s => !routed.has(s[0]));
    }
    function streamLabel(s) { return s[2] ? `${s[1]} — ${s[2]}` : s[1]; }
    function nameForStream(id) {
        const s = root.appWin.streams.find(x => x[0] === id);
        return s ? root.streamLabel(s) : `stream #${id}`;
    }
    function realGroups() { return root.appWin.groups.filter(g => !g[4]); }

    RowLayout {
        Layout.alignment: Qt.AlignRight
        spacing: 8
        Rectangle {
            implicitWidth: clearMuteText.implicitWidth + 20; implicitHeight: 26; radius: 6
            color: root.appTheme.panel2; border.color: root.appTheme.border; border.width: 1
            Text { id: clearMuteText; anchors.centerIn: parent; text: "Clear mute"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
            MouseArea { anchors.fill: parent; onClicked: root.clearAllStreamMute() }
        }
        Rectangle {
            implicitWidth: clearSoloText.implicitWidth + 20; implicitHeight: 26; radius: 6
            color: root.appTheme.panel2; border.color: root.appTheme.border; border.width: 1
            Text { id: clearSoloText; anchors.centerIn: parent; text: "Clear solo"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
            MouseArea { anchors.fill: parent; onClicked: root.clearAllStreamSolo() }
        }
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: 14

        // ---- Inputs card ----
        Rectangle {
            Layout.preferredWidth: 440
            radius: 10
            color: root.appTheme.panel
            border.color: root.appTheme.border
            border.width: 1
            implicitHeight: inputsCol.implicitHeight + 32

            ColumnLayout {
                id: inputsCol
                anchors.fill: parent
                anchors.margins: 16
                spacing: 6
                Text { text: "INPUTS"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }

                Repeater {
                    model: root.unroutedStreams()
                    delegate: Rectangle {
                        id: streamRow
                        required property var modelData
                        Layout.fillWidth: true
                        implicitHeight: 32
                        radius: 6
                        color: root.appTheme.panel2
                        RowLayout {
                            anchors.fill: parent
                            anchors.margins: 8
                            spacing: 8
                            Rectangle {
                                width: 8; height: 8; radius: 4
                                color: streamRow.modelData[3] ? root.appTheme.success : root.appTheme.textFaint
                            }
                            Text {
                                text: root.streamLabel(streamRow.modelData)
                                color: root.appTheme.text; font.pixelSize: 12; Layout.fillWidth: true; elide: Text.ElideRight
                            }
                            Rectangle {
                                width: 20; height: 20; radius: 5
                                color: root.isStreamMuted(streamRow.modelData[0]) ? root.appTheme.danger : root.appTheme.inset
                                border.color: root.appTheme.border; border.width: 1
                                Text { anchors.centerIn: parent; text: "M"; font.pixelSize: 9; font.bold: true; color: root.appTheme.text }
                                MouseArea { anchors.fill: parent; onClicked: root.toggleStreamMute(streamRow.modelData[0]) }
                            }
                            Rectangle {
                                width: 20; height: 20; radius: 5
                                color: root.isStreamSoloed(streamRow.modelData[0]) ? root.appTheme.warn : root.appTheme.inset
                                border.color: root.appTheme.border; border.width: 1
                                Text { anchors.centerIn: parent; text: "S"; font.pixelSize: 9; font.bold: true; color: root.appTheme.text }
                                MouseArea { anchors.fill: parent; onClicked: root.toggleStreamSolo(streamRow.modelData[0]) }
                            }
                            GroupCombo {
                                id: routeCombo
                                groups: root.realGroups()
                                Layout.preferredWidth: 120
                            }
                            Rectangle {
                                implicitWidth: 52; implicitHeight: 22; radius: 5
                                color: root.appTheme.accent
                                Text { anchors.centerIn: parent; text: "Route"; font.pixelSize: 10; font.bold: true; color: "#06272c" }
                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: {
                                        const gid = routeCombo.currentGroupId();
                                        if (gid === null) return;
                                        root.appWin.dcall("RouteStreamToGroup", "uu", [streamRow.modelData[0], gid], function() { root.appWin.refreshAll(); });
                                    }
                                }
                            }
                        }
                    }
                }
                Text {
                    visible: root.unroutedStreams().length === 0
                    text: "No app audio streams to assign right now."
                    color: root.appTheme.textFaint; font.pixelSize: 11; wrapMode: Text.WordWrap; Layout.fillWidth: true
                }
                Text {
                    text: "M mute · S solo (mutes every other stream until cleared)."
                    color: root.appTheme.textFaint; font.pixelSize: 11; wrapMode: Text.WordWrap; Layout.fillWidth: true
                }
            }
        }

        // ---- Groups card ----
        Rectangle {
            Layout.preferredWidth: 440
            radius: 10
            color: root.appTheme.panel
            border.color: root.appTheme.border
            border.width: 1
            implicitHeight: groupsCol.implicitHeight + 32

            ColumnLayout {
                id: groupsCol
                anchors.fill: parent
                anchors.margins: 16
                spacing: 6
                Text { text: "GROUPS"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }

                Repeater {
                    model: root.realGroups()
                    delegate: ColumnLayout {
                        id: groupBlock
                        required property var modelData
                        Layout.fillWidth: true
                        spacing: 2

                        Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: 30
                            radius: 6
                            color: root.appTheme.panel2
                            RowLayout {
                                anchors.fill: parent
                                anchors.margins: 8
                                Text { text: groupBlock.modelData[1]; color: root.appTheme.text; font.pixelSize: 12; Layout.fillWidth: true }
                                Rectangle {
                                    width: 20; height: 20; radius: 5; color: "transparent"
                                    Text { anchors.centerIn: parent; text: "✕"; color: root.appTheme.textDim; font.pixelSize: 12 }
                                    MouseArea {
                                        anchors.fill: parent
                                        onClicked: root.appWin.dcall("DeleteGroup", "u", [groupBlock.modelData[0]], function() { root.appWin.refreshAll(); })
                                    }
                                }
                            }
                        }
                        Repeater {
                            model: groupBlock.modelData[3] || []
                            delegate: Rectangle {
                                id: subRow
                                required property var modelData
                                Layout.fillWidth: true
                                Layout.leftMargin: 18
                                implicitHeight: 24
                                color: "transparent"
                                RowLayout {
                                    anchors.fill: parent
                                    spacing: 6
                                    Text {
                                        text: root.nameForStream(subRow.modelData)
                                        color: root.appTheme.textDim; font.pixelSize: 10; font.family: "monospace"; Layout.fillWidth: true; elide: Text.ElideRight
                                    }
                                    Rectangle {
                                        width: 18; height: 18; radius: 4
                                        color: root.isStreamMuted(subRow.modelData) ? root.appTheme.danger : root.appTheme.inset
                                        border.color: root.appTheme.border; border.width: 1
                                        Text { anchors.centerIn: parent; text: "M"; font.pixelSize: 8; font.bold: true; color: root.appTheme.text }
                                        MouseArea { anchors.fill: parent; onClicked: root.toggleStreamMute(subRow.modelData) }
                                    }
                                    Rectangle {
                                        width: 18; height: 18; radius: 4
                                        color: root.isStreamSoloed(subRow.modelData) ? root.appTheme.warn : root.appTheme.inset
                                        border.color: root.appTheme.border; border.width: 1
                                        Text { anchors.centerIn: parent; text: "S"; font.pixelSize: 8; font.bold: true; color: root.appTheme.text }
                                        MouseArea { anchors.fill: parent; onClicked: root.toggleStreamSolo(subRow.modelData) }
                                    }
                                    Rectangle {
                                        width: 18; height: 18; radius: 4; color: "transparent"
                                        Text { anchors.centerIn: parent; text: "✕"; color: root.appTheme.textDim; font.pixelSize: 11 }
                                        MouseArea {
                                            anchors.fill: parent
                                            onClicked: root.appWin.dcall("UnrouteStream", "u", [subRow.modelData], function() { root.appWin.refreshAll(); })
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Text {
                    visible: root.realGroups().length === 0
                    text: "No groups yet — add one below."
                    color: root.appTheme.textFaint; font.pixelSize: 11
                }

                RowLayout {
                    Layout.fillWidth: true
                    Layout.topMargin: 8
                    spacing: 6
                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: 30
                        radius: 5
                        color: root.appTheme.panel2
                        border.color: root.appTheme.border
                        border.width: 1
                        TextInput {
                            id: groupNameInput
                            anchors.fill: parent
                            anchors.margins: 8
                            color: root.appTheme.text
                            font.pixelSize: 12
                            verticalAlignment: TextInput.AlignVCenter
                            clip: true
                        }
                        Text {
                            anchors.left: parent.left; anchors.leftMargin: 8; anchors.verticalCenter: parent.verticalCenter
                            text: "Group name"; color: root.appTheme.textFaint; font.pixelSize: 12
                            visible: groupNameInput.text.length === 0
                        }
                    }
                    Rectangle {
                        implicitWidth: 54; implicitHeight: 30; radius: 5
                        color: root.appTheme.accent
                        Text { anchors.centerIn: parent; text: "+ Add"; font.pixelSize: 11; font.bold: true; color: "#06272c" }
                        MouseArea {
                            anchors.fill: parent
                            onClicked: {
                                const name = groupNameInput.text.trim();
                                if (!name) return;
                                root.appWin.dcall("CreateGroup", "s", [name], function() { root.appWin.refreshAll(); });
                                groupNameInput.text = "";
                            }
                        }
                    }
                }
            }
        }
    }
}
