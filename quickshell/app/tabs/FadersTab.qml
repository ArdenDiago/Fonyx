// Faders tab — ported from ui/index.html's #faders section: per-group
// vertical fader with link/mute/solo, pan and mono/stereo, fader groups,
// plus the master fader. Mute, solo, links and fader groups are the
// daemon's (ListGroupMixState, ListFaderGroups), shared with the desktop
// app and the CLI; the daemon works out what each group is heard at, so
// a fader only ever sends its own value.
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

    function isMuted(id) { return root.appWin.mixOf(id)[1]; }
    function isSoloedOn(id) { return root.appWin.mixOf(id)[2]; }
    function isLinked(id) { return root.appWin.mixOf(id)[3]; }
    function anySolo() { return root.appWin.mixState.some(m => m[2]); }
    function isSoloedOut(id) { return root.anySolo() && !root.isSoloedOn(id); }
    function fgroupsOf(id) { return root.appWin.faderGroups.filter(f => f[4].indexOf(id) !== -1); }
    // What the group is heard at, for the display only: own fader × its
    // fader groups, capped at the fader's top (the daemon's heard_gain).
    function heardGain(id) {
        const own = root.faderValues[id] !== undefined ? root.faderValues[id] : 1;
        const f = root.fgroupsOf(id).reduce((acc, fg) => acc * DbMath.dbToGain(fg[2]), 1);
        return Math.min(DbMath.dbToGain(DbMath.METER_DB_MAX), own * f);
    }

    function seedFaderValue(id, gain) {
        if (root.faderValues[id] === undefined) {
            const copy = Object.assign({}, root.faderValues);
            copy[id] = gain;
            root.faderValues = copy;
        }
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
            for (const m of root.appWin.mixState) {
                if (!m[3] || m[0] === id) continue;
                const otherPrev = copy[m[0]] !== undefined ? copy[m[0]] : 0;
                copy[m[0]] = Math.max(0, Math.min(maxGain, otherPrev + delta));
                dragged.push(m[0]);
            }
        }
        root.faderValues = copy;
        for (const gid of dragged) root.appWin.dcall("SetGain", "ud", [gid, root.faderValues[gid].toFixed(4)]);
    }

    function clearAllSolo() {
        for (const m of root.appWin.mixState) if (m[2]) root.appWin.setMix(m[0], 2, false);
    }

    function setPan(id, pan, mono) {
        root.appWin.pans = root.appWin.pans.filter(p => p[0] !== id).concat([[id, pan, mono]]);
        root.appWin.dcall("SetPan", "udb", [id, pan.toFixed(3), mono ? "true" : "false"]);
    }

    // Fader groups: SaveFaderGroup replaces one whole (id = u32::MAX makes one).
    readonly property string newId: "4294967295"
    function saveFaderGroup(fg) {
        root.appWin.dcall("SaveFaderGroup", "usdbau",
            [fg[0], fg[1], fg[2].toFixed(2), fg[3] ? "true" : "false", fg[4].length].concat(fg[4]),
            function() { root.appWin.refreshMix(); });
    }
    function withFaderGroup(fgId, change) {
        const fg = root.appWin.faderGroups.find(f => f[0] === fgId);
        if (!fg) return;
        const copy = [fg[0], fg[1], fg[2], fg[3], fg[4].slice()];
        change(copy);
        root.appWin.faderGroups = root.appWin.faderGroups.map(f => f[0] === fgId ? copy : f);
        root.saveFaderGroup(copy);
    }
    function toggleMember(fgId, groupId) {
        root.withFaderGroup(fgId, fg => {
            const i = fg[4].indexOf(groupId);
            if (i === -1) fg[4].push(groupId); else fg[4].splice(i, 1);
        });
    }
    function faderGroupTop(fg) {
        // Stop where the loudest member reaches +12 dB, so members stay in ratio.
        let peak = -Infinity;
        for (const m of fg[4]) {
            const own = root.faderValues[m] !== undefined ? root.faderValues[m] : 1;
            if (own <= 1e-4) continue;
            const others = root.fgroupsOf(m).filter(o => o[0] !== fg[0]).reduce((s, o) => s + o[2], 0);
            peak = Math.max(peak, DbMath.gainToDb(own) + others);
        }
        return isFinite(peak) ? Math.max(DbMath.METER_DB_MIN + 1, DbMath.METER_DB_MAX - peak) : DbMath.METER_DB_MAX;
    }
    function nonCueGroups() {
        // Cue has no fader: it always plays at full level.
        return root.appWin.groups.filter(g => g[0] !== root.appWin.cueGroupId);
    }

    Connections {
        target: root.appWin
        function onGroupsChanged() { for (const g of root.appWin.groups) root.seedFaderValue(g[0], g[2]); }
    }
    Component.onCompleted: { for (const g of root.appWin.groups) root.seedFaderValue(g[0], g[2]); }

    component SmallButton: Rectangle {
        id: btn
        property string label: ""
        property bool on: false
        property color onColor: root.appTheme.accentDim
        property color onText: root.appTheme.accent
        signal clicked()
        width: Math.max(22, btnText.implicitWidth + 10); height: 22; radius: 5
        color: btn.on ? btn.onColor : root.appTheme.inset
        border.color: root.appTheme.border; border.width: 1
        Text { id: btnText; anchors.centerIn: parent; text: btn.label; font.pixelSize: 9; font.bold: true; color: btn.on ? btn.onText : root.appTheme.textFaint }
        MouseArea { anchors.fill: parent; onClicked: btn.clicked() }
    }

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
            Layout.preferredHeight: 380
            radius: 10
            color: root.appTheme.panel
            border.color: root.appTheme.border
            border.width: 1

            RowLayout {
                anchors.fill: parent
                anchors.margins: 20
                spacing: 16

                Repeater {
                    model: root.nonCueGroups()
                    delegate: Rectangle {
                        id: col
                        required property var modelData
                        readonly property int gid: modelData[0]
                        readonly property var pan: root.appWin.panOf(gid)
                        Layout.preferredWidth: 96
                        Layout.fillHeight: true
                        radius: 8
                        color: root.appTheme.panel2
                        border.color: root.fgroupsOf(gid).length ? root.appTheme.accent : "transparent"
                        border.width: 1

                        ColumnLayout {
                            anchors.fill: parent
                            anchors.margins: 12
                            spacing: 6

                            RowLayout {
                                Layout.alignment: Qt.AlignHCenter
                                spacing: 4
                                SmallButton { label: "L"; on: root.isLinked(col.gid); onClicked: root.appWin.setMix(col.gid, 3, !on) }
                                SmallButton { label: "M"; on: root.isMuted(col.gid); onColor: root.appTheme.danger; onText: "#2c0906"; onClicked: root.appWin.setMix(col.gid, 1, !on) }
                                SmallButton { label: "S"; on: root.isSoloedOn(col.gid); onColor: root.appTheme.warn; onText: "#2c1f06"; onClicked: root.appWin.setMix(col.gid, 2, !on) }
                            }

                            Text {
                                text: col.modelData[1]
                                color: root.appTheme.text; font.pixelSize: 11; font.bold: true
                                Layout.alignment: Qt.AlignHCenter
                                elide: Text.ElideRight
                                Layout.maximumWidth: 80
                            }

                            RowLayout {
                                Layout.alignment: Qt.AlignHCenter
                                spacing: 2
                                SmallButton { label: "Mono"; on: col.pan[2]; onClicked: root.setPan(col.gid, col.pan[1], true) }
                                SmallButton { label: "Stereo"; on: !col.pan[2]; onClicked: root.setPan(col.gid, col.pan[1], false) }
                            }
                            Knob {
                                Layout.alignment: Qt.AlignHCenter
                                size: 30
                                from: -1; to: 1
                                value: col.pan[1]
                                displayText: Math.abs(value) < 0.04 ? "C" : (value < 0 ? "L" : "R") + Math.round(Math.abs(value) * 100)
                                // Snaps to centre near the middle, so "C" is easy to hit.
                                onMoved: (v) => root.setPan(col.gid, Math.abs(v) < 0.04 ? 0 : v, col.pan[2])
                            }

                            RowLayout {
                                Layout.fillHeight: true
                                Layout.alignment: Qt.AlignHCenter
                                spacing: 4
                                opacity: root.isSoloedOut(col.gid) || root.isMuted(col.gid) ? 0.45 : 1.0

                                Slider2 {
                                    Layout.fillHeight: true
                                    vertical: true
                                    from: DbMath.METER_DB_MIN; to: DbMath.METER_DB_MAX
                                    value: DbMath.gainToDb(root.heardGain(col.gid))
                                    trackColor: root.appTheme.inset
                                    fillColor: root.appTheme.accent
                                    handleColor: root.appTheme.text
                                    onMoved: {
                                        // The slider shows what's heard; take the fader groups back out.
                                        const f = root.fgroupsOf(col.gid).reduce((acc, fg) => acc * DbMath.dbToGain(fg[2]), 1) || 1;
                                        root.onFaderDragged(col.gid, DbMath.gainToDb(DbMath.dbToGain(value) / f));
                                    }
                                }
                                DbScale { Layout.fillHeight: true; dbMin: DbMath.METER_DB_MIN; dbMax: DbMath.METER_DB_MAX }
                            }

                            Text {
                                text: DbMath.gainToDbText(root.heardGain(col.gid))
                                color: root.appTheme.text; font.pixelSize: 12; font.bold: true; font.family: "monospace"
                                Layout.alignment: Qt.AlignHCenter
                            }
                        }
                    }
                }

                // Fader groups: one level on top of their members' faders.
                Repeater {
                    model: root.appWin.faderGroups
                    delegate: Rectangle {
                        id: fcol
                        required property var modelData
                        readonly property int fid: modelData[0]
                        Layout.preferredWidth: 96
                        Layout.fillHeight: true
                        radius: 8
                        color: root.appTheme.panel2
                        border.color: root.appTheme.accent; border.width: 1
                        ColumnLayout {
                            anchors.fill: parent
                            anchors.margins: 12
                            spacing: 6
                            RowLayout {
                                Layout.alignment: Qt.AlignHCenter
                                spacing: 4
                                SmallButton { label: "M"; on: fcol.modelData[3]; onColor: root.appTheme.danger; onText: "#2c0906"; onClicked: root.withFaderGroup(fcol.fid, fg => fg[3] = !fg[3]) }
                                SmallButton { label: "✕"; onClicked: root.appWin.dcall("DeleteFaderGroup", "u", [fcol.fid], function() { root.appWin.refreshMix(); }) }
                            }
                            Text { text: fcol.modelData[1]; color: root.appTheme.accent; font.pixelSize: 11; font.bold: true; Layout.alignment: Qt.AlignHCenter; elide: Text.ElideRight; Layout.maximumWidth: 80 }
                            RowLayout {
                                Layout.fillHeight: true
                                Layout.alignment: Qt.AlignHCenter
                                spacing: 4
                                Slider2 {
                                    Layout.fillHeight: true
                                    vertical: true
                                    from: DbMath.METER_DB_MIN; to: root.faderGroupTop(fcol.modelData)
                                    value: fcol.modelData[2]
                                    trackColor: root.appTheme.inset
                                    fillColor: root.appTheme.accent
                                    handleColor: root.appTheme.text
                                    onMoved: { const v = value; root.withFaderGroup(fcol.fid, fg => fg[2] = v); }
                                }
                            }
                            Text { text: DbMath.gainToDbText(DbMath.dbToGain(fcol.modelData[2])); color: root.appTheme.text; font.pixelSize: 12; font.family: "monospace"; Layout.alignment: Qt.AlignHCenter }
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

    // ---- Fader groups: make one, pick its members ----
    Rectangle {
        Layout.fillWidth: true
        radius: 10
        color: root.appTheme.panel
        border.color: root.appTheme.border
        border.width: 1
        implicitHeight: fgCol.implicitHeight + 28
        ColumnLayout {
            id: fgCol
            anchors.fill: parent
            anchors.margins: 14
            spacing: 6
            Text { text: "FADER GROUPS"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
            Repeater {
                model: root.appWin.faderGroups
                delegate: RowLayout {
                    id: fgRow
                    required property var modelData
                    spacing: 6
                    Text { text: fgRow.modelData[1]; color: root.appTheme.accent; font.pixelSize: 12; font.bold: true; Layout.preferredWidth: 110; elide: Text.ElideRight }
                    Repeater {
                        model: root.nonCueGroups()
                        delegate: SmallButton {
                            required property var modelData
                            label: modelData[1]
                            on: fgRow.modelData[4].indexOf(modelData[0]) !== -1
                            onClicked: root.toggleMember(fgRow.modelData[0], modelData[0])
                        }
                    }
                }
            }
            RowLayout {
                spacing: 6
                Rectangle {
                    Layout.preferredWidth: 180; implicitHeight: 26; radius: 6
                    color: root.appTheme.inset; border.color: root.appTheme.border; border.width: 1
                    TextInput {
                        id: fgName
                        anchors.fill: parent; anchors.margins: 6
                        color: root.appTheme.text; font.pixelSize: 12
                        verticalAlignment: TextInput.AlignVCenter
                        clip: true
                        onAccepted: addFg.create()
                    }
                    Text {
                        anchors.left: parent.left; anchors.leftMargin: 6; anchors.verticalCenter: parent.verticalCenter
                        text: "Fader group name"; color: root.appTheme.textFaint; font.pixelSize: 12
                        visible: fgName.text.length === 0
                    }
                }
                SmallButton {
                    id: addFg
                    label: "+ Fader group"
                    function create() {
                        const name = fgName.text.trim();
                        if (!name) return;
                        root.appWin.dcall("SaveFaderGroup", "usdbau", [root.newId, name, "0", "false", 0], function() { root.appWin.refreshMix(); });
                        fgName.text = "";
                    }
                    onClicked: create()
                }
            }
        }
    }

    Text {
        text: "L link (delta together) · M mute · S solo (mutes every other group until cleared) · knob pans. Shared with the desktop app."
        color: root.appTheme.textFaint; font.pixelSize: 11
    }
}
