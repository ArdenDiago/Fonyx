// Faders tab — ported from ui/index.html's #faders section: per-group
// vertical fader with link/mute/solo, plus the master fader. Mute/solo/
// link logic ported from FonyxWidget.qml's own onFaderDragged/
// toggleMuteGroup/toggleSoloGroup (same client-side-overlay-on-real-gain
// trick as index.html's faderValues/soloedGroups).
import QtQuick
import QtQuick.Layouts
import "../"
import "../DbMath.js" as DbMath

ColumnLayout {
    id: root
    required property var appWin
    required property var appTheme
    Layout.fillWidth: true
    spacing: 10

    property var faderValues: ({})
    property var mutedGroups: []
    property var soloedGroups: []
    property var linkedGroups: []

    function isMuted(id) { return root.mutedGroups.indexOf(id) !== -1; }
    function isSoloedOn(id) { return root.soloedGroups.indexOf(id) !== -1; }
    function isSoloedOut(id) { return root.soloedGroups.length > 0 && !root.isSoloedOn(id); }
    function isEffectivelyMuted(id) { return root.isMuted(id) || root.isSoloedOut(id); }
    function isLinked(id) { return root.linkedGroups.indexOf(id) !== -1; }

    function seedFaderValue(id, gain) {
        if (root.faderValues[id] === undefined) {
            const copy = Object.assign({}, root.faderValues);
            copy[id] = gain;
            root.faderValues = copy;
        }
    }
    function applyGroupGain(id) {
        const intended = root.faderValues[id] !== undefined ? root.faderValues[id] : 0;
        root.appWin.dcall("SetGain", "ud", [id, (root.isEffectivelyMuted(id) ? 0 : intended).toFixed(4)]);
    }

    function onFaderDragged(id, db) {
        const newGain = DbMath.dbToGain(db);
        const prevGain = root.faderValues[id] !== undefined ? root.faderValues[id] : newGain;
        const copy = Object.assign({}, root.faderValues);
        copy[id] = newGain;
        const dragged = [id];
        if (root.isLinked(id)) {
            const delta = newGain - prevGain;
            const maxGain = DbMath.dbToGain(DbMath.METER_DB_MAX);
            for (const otherId of root.linkedGroups) {
                if (otherId === id) continue;
                const otherPrev = copy[otherId] !== undefined ? copy[otherId] : 0;
                copy[otherId] = Math.max(0, Math.min(maxGain, otherPrev + delta));
                dragged.push(otherId);
            }
        }
        root.faderValues = copy;
        for (const gid of dragged) root.applyGroupGain(gid);
    }

    function toggleLink(id) {
        const idx = root.linkedGroups.indexOf(id);
        root.linkedGroups = idx === -1 ? root.linkedGroups.concat([id]) : root.linkedGroups.filter(x => x !== id);
    }
    function toggleMute(id) {
        const idx = root.mutedGroups.indexOf(id);
        root.mutedGroups = idx === -1 ? root.mutedGroups.concat([id]) : root.mutedGroups.filter(x => x !== id);
        root.applyGroupGain(id);
    }
    function toggleSolo(id) {
        const idx = root.soloedGroups.indexOf(id);
        root.soloedGroups = idx === -1 ? root.soloedGroups.concat([id]) : root.soloedGroups.filter(x => x !== id);
        for (const g of root.appWin.groups) root.applyGroupGain(g[0]);
    }
    function clearAllSolo() {
        root.soloedGroups = [];
        for (const g of root.appWin.groups) root.applyGroupGain(g[0]);
    }

    Connections {
        target: root.appWin
        function onGroupsChanged() { for (const g of root.appWin.groups) root.seedFaderValue(g[0], g[2]); }
    }
    Component.onCompleted: { for (const g of root.appWin.groups) root.seedFaderValue(g[0], g[2]); }

    RowLayout {
        Layout.alignment: Qt.AlignRight
        Rectangle {
            implicitWidth: clearSoloTxt.implicitWidth + 20; implicitHeight: 26; radius: 6
            color: root.appTheme.panel2; border.color: root.appTheme.border; border.width: 1
            Text { id: clearSoloTxt; anchors.centerIn: parent; text: "Clear solo"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
            MouseArea { anchors.fill: parent; onClicked: root.clearAllSolo() }
        }
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: 14

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 320
            radius: 10
            color: root.appTheme.panel
            border.color: root.appTheme.border
            border.width: 1

            RowLayout {
                anchors.fill: parent
                anchors.margins: 20
                spacing: 16

                Repeater {
                    model: root.appWin.groups
                    delegate: Rectangle {
                        id: col
                        required property var modelData
                        readonly property int gid: modelData[0]
                        readonly property real gain: root.faderValues[gid] !== undefined ? root.faderValues[gid] : modelData[2]
                        Layout.preferredWidth: 96
                        Layout.fillHeight: true
                        radius: 8
                        color: root.appTheme.panel2

                        ColumnLayout {
                            anchors.fill: parent
                            anchors.margins: 12
                            spacing: 8

                            RowLayout {
                                Layout.alignment: Qt.AlignHCenter
                                spacing: 4
                                Rectangle {
                                    width: 22; height: 22; radius: 5
                                    color: root.isLinked(col.gid) ? root.appTheme.accentDim : root.appTheme.inset
                                    border.color: root.isLinked(col.gid) ? root.appTheme.accent : root.appTheme.border; border.width: 1
                                    Text { anchors.centerIn: parent; text: "L"; font.pixelSize: 9; font.bold: true; color: root.isLinked(col.gid) ? root.appTheme.accent : root.appTheme.textFaint }
                                    MouseArea { anchors.fill: parent; onClicked: root.toggleLink(col.gid) }
                                }
                                Rectangle {
                                    width: 22; height: 22; radius: 5
                                    color: root.isMuted(col.gid) ? root.appTheme.danger : root.appTheme.success
                                    border.color: root.appTheme.border; border.width: 1
                                    Text { anchors.centerIn: parent; text: "M"; font.pixelSize: 9; font.bold: true; color: root.isMuted(col.gid) ? "#2c0906" : "#06270f" }
                                    MouseArea { anchors.fill: parent; onClicked: root.toggleMute(col.gid) }
                                }
                                Rectangle {
                                    width: 22; height: 22; radius: 5
                                    color: root.isSoloedOn(col.gid) ? root.appTheme.warn : root.appTheme.inset
                                    border.color: root.appTheme.border; border.width: 1
                                    Text { anchors.centerIn: parent; text: "S"; font.pixelSize: 9; font.bold: true; color: root.isSoloedOn(col.gid) ? "#2c1f06" : root.appTheme.textFaint }
                                    MouseArea { anchors.fill: parent; onClicked: root.toggleSolo(col.gid) }
                                }
                            }

                            Text {
                                text: col.modelData[1]
                                color: root.appTheme.text; font.pixelSize: 11; font.bold: true
                                Layout.alignment: Qt.AlignHCenter
                                elide: Text.ElideRight
                                Layout.maximumWidth: 80
                            }

                            RowLayout {
                                Layout.fillHeight: true
                                Layout.alignment: Qt.AlignHCenter
                                spacing: 4
                                opacity: root.isSoloedOut(col.gid) ? 0.45 : 1.0

                                Slider2 {
                                    Layout.fillHeight: true
                                    vertical: true
                                    from: DbMath.METER_DB_MIN; to: DbMath.METER_DB_MAX
                                    value: DbMath.gainToDb(col.gain)
                                    trackColor: root.appTheme.inset
                                    fillColor: root.appTheme.accent
                                    handleColor: root.appTheme.text
                                    onMoved: root.onFaderDragged(col.gid, value)
                                }
                                DbScale { Layout.fillHeight: true; dbMin: DbMath.METER_DB_MIN; dbMax: DbMath.METER_DB_MAX }
                            }

                            Text {
                                text: DbMath.gainToDbText(col.gain)
                                color: root.appTheme.text; font.pixelSize: 12; font.bold: true; font.family: "monospace"
                                Layout.alignment: Qt.AlignHCenter
                            }
                        }
                    }
                }

                Text {
                    visible: root.appWin.groups.length === 0
                    text: "No groups yet."
                    color: root.appTheme.textFaint
                }
            }
        }

        Rectangle {
            Layout.preferredWidth: 110
            Layout.fillHeight: true
            radius: 10
            color: root.appTheme.panel
            border.color: root.appTheme.accent
            border.width: 1
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 16
                RowLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    spacing: 6
                    Slider2 {
                        Layout.fillHeight: true
                        vertical: true
                        from: DbMath.METER_DB_MIN; to: DbMath.METER_DB_MAX
                        value: DbMath.gainToDb(root.appWin.masterGain)
                        trackColor: root.appTheme.inset
                        fillColor: root.appTheme.accent
                        handleColor: root.appTheme.text
                        onMoved: {
                            const gain = DbMath.dbToGain(value);
                            root.appWin.masterGain = gain;
                            root.appWin.dcall("SetMasterGain", "d", [gain.toFixed(3)]);
                        }
                    }
                    DbScale { Layout.fillHeight: true; dbMin: DbMath.METER_DB_MIN; dbMax: DbMath.METER_DB_MAX }
                }
                Text { text: "MASTER"; color: root.appTheme.accent; font.pixelSize: 11; font.bold: true; Layout.alignment: Qt.AlignHCenter }
                Text { text: DbMath.gainToDbText(root.appWin.masterGain); color: root.appTheme.text; font.pixelSize: 12; font.family: "monospace"; Layout.alignment: Qt.AlignHCenter }
            }
        }
    }

    Text {
        text: "L link (delta together) · M mute · S solo (mutes every other group until cleared)."
        color: root.appTheme.textFaint; font.pixelSize: 11
    }
}
