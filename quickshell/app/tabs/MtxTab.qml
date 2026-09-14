// MTX Panel tab — ported from ui/index.html's #mtx section: interactive
// EQ graph (drag = freq/gain, wheel = Q), per-node knob readout,
// Compression/Gate sub-tabs with real transfer-curve graphs, and the
// plugin chain. Formulas and D-Bus argument shapes ported 1:1 — see
// EqGraph.qml/CompGraph.qml/GateDoor.qml for the ported math.
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls.Basic as Controls
import "../"
import "../DbMath.js" as DbMath

ColumnLayout {
    id: root
    required property var appWin
    required property var appTheme
    Layout.fillWidth: true
    spacing: 12

    property var bandState: ({})    // gid -> [{freq,gain,q,enabled}x6]
    property var filterState: ({})  // gid -> {hp:{freq,q,enabled}, lp:{freq,q,enabled}}
    property int selectedGroupId: -1
    property var selectedNode: ({ kind: "band", index: 0 })
    property var spectrumBins: []
    property string compGateSub: "comp"

    property real compThresh: -18
    property real compRatio: 2
    property real compAttack: 10
    property real compRelease: 100
    property real compMakeup: 0
    property bool compEn: false
    property real gateThresh: -50
    property real gateAttack: 2
    property real gateRelease: 150
    property real gateHold: 50
    property bool gateEn: false
    property real liveGr: 0
    property real liveGateGr: 0

    property var plugins: []           // [[index,pluginType,enabled,paramNames,paramValues], ...]
    property var availablePlugins: []  // [[typeId,displayName,paramNames], ...]

    readonly property var bandFreqs: [100, 300, 1000, 3000, 8000, 12000]
    readonly property var eqPresetNames: ["Flat", "Rock", "Pop", "Jazz", "Movie", "Bass Boost", "Vocal Boost"]
    readonly property var eqPresets: ({
        "Flat": [0, 0, 0, 0, 0, 0], "Rock": [4, -2, -1, 3, 4, 3], "Pop": [1, 0, 2, 3, 2, 1],
        "Jazz": [2, 1, 0, 1, 2, 1], "Movie": [3, 0, 1, 4, 2, 1],
        "Bass Boost": [6, 3, 0, 0, 0, 0], "Vocal Boost": [-2, 0, 3, 4, 1, -1],
    })

    function ensureBandState(gid) {
        if (!root.bandState[gid]) {
            const copy = Object.assign({}, root.bandState);
            copy[gid] = root.bandFreqs.map(f => ({ freq: f, gain: 0, q: 1, enabled: true }));
            root.bandState = copy;
        }
        if (!root.filterState[gid]) {
            const copy = Object.assign({}, root.filterState);
            copy[gid] = { hp: { freq: 80, q: 0.707, enabled: false }, lp: { freq: 18000, q: 0.707, enabled: false } };
            root.filterState = copy;
        }
    }
    function currentBands() { return root.bandState[root.selectedGroupId] || []; }
    function currentFilters() {
        return root.filterState[root.selectedGroupId] || { hp: { freq: 80, q: 0.707, enabled: false }, lp: { freq: 18000, q: 0.707, enabled: false } };
    }

    function updateBand(index, patch) {
        if (root.selectedGroupId < 0 || !root.bandState[root.selectedGroupId]) return;
        const arr = root.bandState[root.selectedGroupId].map((b, i) => i === index ? Object.assign({}, b, patch) : b);
        const copy = Object.assign({}, root.bandState);
        copy[root.selectedGroupId] = arr;
        root.bandState = copy;
        const b = arr[index];
        root.appWin.dcall("SetEqBand", "uudddb", [root.selectedGroupId, index, b.freq, b.gain, b.q, b.enabled ? "true" : "false"]);
    }
    function updateFilter(kind, patch) {
        if (root.selectedGroupId < 0 || !root.filterState[root.selectedGroupId]) return;
        const f = Object.assign({}, root.filterState[root.selectedGroupId][kind], patch);
        const kindCopy = Object.assign({}, root.filterState[root.selectedGroupId]);
        kindCopy[kind] = f;
        const copy = Object.assign({}, root.filterState);
        copy[root.selectedGroupId] = kindCopy;
        root.filterState = copy;
        root.appWin.dcall(kind === "hp" ? "SetHighpass" : "SetLowpass", "uddb", [root.selectedGroupId, f.freq, f.q, f.enabled ? "true" : "false"]);
    }

    function loadMtxGroup(gid) {
        root.selectedGroupId = gid;
        root.ensureBandState(gid);
        root.selectedNode = { kind: "band", index: 0 };
        root.appWin.dcall("GetGroupDetails", "u", [gid], function(data) {
            const bands = data[0], hp = data[1], lp = data[2], comp = data[3], gate = data[4];
            if (bands.length) {
                const copy = Object.assign({}, root.bandState);
                copy[gid] = bands.map(b => ({ freq: b[0], gain: b[1], q: b[2], enabled: b[3] }));
                root.bandState = copy;
            }
            const fcopy = Object.assign({}, root.filterState);
            fcopy[gid] = { hp: { freq: hp[0], q: hp[1], enabled: hp[2] }, lp: { freq: lp[0], q: lp[1], enabled: lp[2] } };
            root.filterState = fcopy;
            root.compThresh = comp[0]; root.compRatio = comp[1]; root.compAttack = comp[2];
            root.compRelease = comp[3]; root.compMakeup = comp[4]; root.compEn = comp[5];
            root.gateThresh = gate[0]; root.gateAttack = gate[1]; root.gateRelease = gate[2];
            root.gateHold = gate[3]; root.gateEn = gate[4];
        });
        root.refreshPlugins();
    }

    function applyComp() {
        if (root.selectedGroupId < 0) return;
        root.appWin.dcall("SetCompressor", "udddddb",
            [root.selectedGroupId, root.compThresh, root.compRatio, root.compAttack, root.compRelease, root.compMakeup, root.compEn ? "true" : "false"]);
    }
    function applyGate() {
        if (root.selectedGroupId < 0) return;
        root.appWin.dcall("SetGate", "uddddb",
            [root.selectedGroupId, root.gateThresh, root.gateAttack, root.gateRelease, root.gateHold, root.gateEn ? "true" : "false"]);
    }

    function refreshPlugins() {
        if (root.selectedGroupId < 0) { root.plugins = []; return; }
        if (!root.availablePlugins.length) {
            root.appWin.dcall("ListAvailablePlugins", "", [], function(data) { root.availablePlugins = data[0] || []; });
        }
        root.appWin.dcall("ListGroupPlugins", "u", [root.selectedGroupId], function(data) { root.plugins = data[0] || []; });
    }
    function pluginTypeName(typeId) {
        const p = root.availablePlugins.find(x => x[0] === typeId);
        return p ? p[1] : typeId;
    }

    function applyPreset(name) {
        const gains = root.eqPresets[name];
        if (!gains || root.selectedGroupId < 0) return;
        gains.forEach((g, i) => root.updateBand(i, { gain: g, enabled: true }));
    }

    function pollLive() {
        root.appWin.dcall("GetLevels", "", [], function(data) {
            for (const row of (data[0] || [])) {
                if (row[0] === root.selectedGroupId) { root.liveGr = row[3]; root.liveGateGr = row[4]; }
            }
        });
        if (root.selectedGroupId >= 0) {
            root.appWin.dcall("GetSpectrum", "u", [root.selectedGroupId], function(data) { root.spectrumBins = data[0] || []; });
        }
    }
    Timer { interval: 200; running: true; repeat: true; triggeredOnStart: true; onTriggered: root.pollLive() }

    Connections {
        target: root.appWin
        function onGroupsChanged() {
            for (const g of root.appWin.groups) root.ensureBandState(g[0]);
            if (root.selectedGroupId === -1 && root.appWin.groups.length) root.loadMtxGroup(root.appWin.groups[0][0]);
        }
    }
    Component.onCompleted: {
        for (const g of root.appWin.groups) root.ensureBandState(g[0]);
        if (root.appWin.groups.length) root.loadMtxGroup(root.appWin.groups[0][0]);
    }

    // ---- toolbar: group select, band swatches, preset ----
    RowLayout {
        Layout.fillWidth: true
        spacing: 12

        Controls.ComboBox {
            id: groupSelect
            model: root.appWin.groups.map(g => g[1])
            onActivated: (index) => { if (root.appWin.groups[index]) root.loadMtxGroup(root.appWin.groups[index][0]); }
        }

        RowLayout {
            spacing: 6
            Repeater {
                model: root.currentBands()
                delegate: Rectangle {
                    required property var modelData
                    required property int index
                    width: 18; height: 18; radius: 9
                    color: modelData.enabled ? root.appTheme.bandColors[index % 6] : "transparent"
                    border.color: root.appTheme.bandColors[index % 6]
                    border.width: root.selectedNode.kind === "band" && root.selectedNode.index === index ? 2.5 : 1.5
                    MouseArea { anchors.fill: parent; onClicked: root.selectedNode = { kind: "band", index: parent.index } }
                }
            }
            Repeater {
                model: ["hp", "lp"]
                delegate: Rectangle {
                    required property string modelData
                    readonly property var f: root.currentFilters()[modelData]
                    width: 16; height: 16; radius: 3
                    color: f.enabled ? "#dfe3e8" : "transparent"
                    border.color: "#dfe3e8"
                    border.width: root.selectedNode.kind === modelData ? 2.5 : 1.5
                    MouseArea { anchors.fill: parent; onClicked: root.selectedNode = { kind: parent.modelData } }
                }
            }
        }

        Item { Layout.fillWidth: true }

        Controls.ComboBox {
            id: presetSelect
            model: root.eqPresetNames
            displayText: "Preset…"
            onActivated: (index) => root.applyPreset(root.eqPresetNames[index])
        }
    }

    // ---- EQ graph ----
    Rectangle {
        Layout.fillWidth: true
        Layout.preferredHeight: 380
        radius: 10
        color: root.appTheme.inset
        border.color: root.appTheme.border
        border.width: 1
        EqGraph {
            anchors.fill: parent
            anchors.margins: 4
            bands: root.currentBands()
            hp: root.currentFilters().hp
            lp: root.currentFilters().lp
            spectrumBins: root.spectrumBins
            selectedNode: root.selectedNode
            bandColors: root.appTheme.bandColors
            onNodeSelected: (kind, index) => { root.selectedNode = kind === "band" ? { kind: kind, index: index } : { kind: kind }; }
            onBandDragged: (index, freq, gain, q) => root.updateBand(index, { freq: freq, gain: gain, q: q })
            onFilterDragged: (kind, freq, q) => root.updateFilter(kind, { freq: freq, q: q })
        }
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: 14

        // ---- selected node panel ----
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 130
            radius: 10
            color: root.appTheme.panel
            border.color: root.appTheme.border
            border.width: 1

            RowLayout {
                anchors.fill: parent
                anchors.margins: 16
                spacing: 20
                visible: root.selectedNode.kind === "band" && root.currentBands()[root.selectedNode.index] !== undefined

                Text {
                    text: "Band " + (root.selectedNode.index + 1)
                    color: root.appTheme.bandColors[root.selectedNode.index % 6]
                    font.bold: true; font.pixelSize: 12
                    Layout.preferredWidth: 70
                }
                ColumnLayout {
                    spacing: 2
                    Knob {
                        size: 46; from: 20; to: 20000; log: true
                        value: root.currentBands()[root.selectedNode.index] ? root.currentBands()[root.selectedNode.index].freq : 1000
                        displayText: value >= 1000 ? (value / 1000).toFixed(1) + "k" : String(Math.round(value))
                        onMoved: (v) => root.updateBand(root.selectedNode.index, { freq: Math.round(v) })
                    }
                    Text { text: "FREQ"; color: root.appTheme.textDim; font.pixelSize: 8; Layout.alignment: Qt.AlignHCenter }
                }
                ColumnLayout {
                    spacing: 2
                    Knob {
                        size: 46; from: -18; to: 18
                        value: root.currentBands()[root.selectedNode.index] ? root.currentBands()[root.selectedNode.index].gain : 0
                        displayText: (value >= 0 ? "+" : "") + value.toFixed(1) + " dB"
                        onMoved: (v) => root.updateBand(root.selectedNode.index, { gain: v })
                    }
                    Text { text: "GAIN"; color: root.appTheme.textDim; font.pixelSize: 8; Layout.alignment: Qt.AlignHCenter }
                }
                ColumnLayout {
                    spacing: 2
                    Knob {
                        size: 46; from: 0.1; to: 10; log: true
                        value: root.currentBands()[root.selectedNode.index] ? root.currentBands()[root.selectedNode.index].q : 1
                        displayText: value.toFixed(2)
                        onMoved: (v) => root.updateBand(root.selectedNode.index, { q: v })
                    }
                    Text { text: "Q"; color: root.appTheme.textDim; font.pixelSize: 8; Layout.alignment: Qt.AlignHCenter }
                }
                RowLayout {
                    spacing: 6
                    Rectangle {
                        width: 34; height: 18; radius: 9
                        color: (root.currentBands()[root.selectedNode.index] && root.currentBands()[root.selectedNode.index].enabled) ? root.appTheme.accentDim : root.appTheme.inset
                        border.color: root.appTheme.border; border.width: 1
                        Rectangle {
                            width: 12; height: 12; radius: 6
                            color: (root.currentBands()[root.selectedNode.index] && root.currentBands()[root.selectedNode.index].enabled) ? root.appTheme.accent : root.appTheme.textFaint
                            x: (root.currentBands()[root.selectedNode.index] && root.currentBands()[root.selectedNode.index].enabled) ? parent.width - width - 2 : 2
                            anchors.verticalCenter: parent.verticalCenter
                        }
                        MouseArea {
                            anchors.fill: parent
                            onClicked: {
                                const b = root.currentBands()[root.selectedNode.index];
                                if (b) root.updateBand(root.selectedNode.index, { enabled: !b.enabled });
                            }
                        }
                    }
                    Text { text: "Enabled"; color: root.appTheme.text; font.pixelSize: 11 }
                }
            }

            RowLayout {
                anchors.fill: parent
                anchors.margins: 16
                spacing: 20
                visible: root.selectedNode.kind === "hp" || root.selectedNode.kind === "lp"
                readonly property var f: root.currentFilters()[root.selectedNode.kind] || { freq: 80, q: 0.707, enabled: false }

                Text { text: root.selectedNode.kind.toUpperCase(); color: root.appTheme.text; font.bold: true; font.pixelSize: 12; Layout.preferredWidth: 40 }
                ColumnLayout {
                    spacing: 2
                    Knob {
                        size: 46; from: 20; to: 20000; log: true
                        value: root.currentFilters()[root.selectedNode.kind] ? root.currentFilters()[root.selectedNode.kind].freq : 80
                        displayText: value >= 1000 ? (value / 1000).toFixed(1) + "k" : String(Math.round(value))
                        onMoved: (v) => root.updateFilter(root.selectedNode.kind, { freq: Math.round(v) })
                    }
                    Text { text: "FREQ"; color: root.appTheme.textDim; font.pixelSize: 8; Layout.alignment: Qt.AlignHCenter }
                }
                ColumnLayout {
                    spacing: 2
                    Knob {
                        size: 46; from: 0.1; to: 10; log: true
                        value: root.currentFilters()[root.selectedNode.kind] ? root.currentFilters()[root.selectedNode.kind].q : 0.707
                        displayText: value.toFixed(2)
                        onMoved: (v) => root.updateFilter(root.selectedNode.kind, { q: v })
                    }
                    Text { text: "Q"; color: root.appTheme.textDim; font.pixelSize: 8; Layout.alignment: Qt.AlignHCenter }
                }
                RowLayout {
                    spacing: 6
                    Rectangle {
                        width: 34; height: 18; radius: 9
                        readonly property bool en: !!(root.currentFilters()[root.selectedNode.kind] || {}).enabled
                        color: en ? root.appTheme.accentDim : root.appTheme.inset
                        border.color: root.appTheme.border; border.width: 1
                        Rectangle { width: 12; height: 12; radius: 6; color: parent.en ? root.appTheme.accent : root.appTheme.textFaint; x: parent.en ? parent.width - width - 2 : 2; anchors.verticalCenter: parent.verticalCenter }
                        MouseArea {
                            anchors.fill: parent
                            onClicked: {
                                const f = root.currentFilters()[root.selectedNode.kind];
                                root.updateFilter(root.selectedNode.kind, { enabled: !f.enabled });
                            }
                        }
                    }
                    Text { text: "Enabled"; color: root.appTheme.text; font.pixelSize: 11 }
                }
            }
        }

        // ---- Compression / Gate ----
        Rectangle {
            Layout.preferredWidth: 260
            implicitHeight: compCol.implicitHeight + 32
            radius: 10
            color: root.appTheme.panel
            border.color: root.appTheme.border
            border.width: 1

            ColumnLayout {
                id: compCol
                anchors.fill: parent
                anchors.margins: 12
                spacing: 8

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 4
                    Rectangle {
                        Layout.fillWidth: true; implicitHeight: 26; radius: 5
                        color: root.compGateSub === "comp" ? root.appTheme.accent : root.appTheme.panel2
                        Text { anchors.centerIn: parent; text: "Compression"; font.pixelSize: 10; font.bold: true; color: root.compGateSub === "comp" ? "#06272c" : root.appTheme.textDim }
                        MouseArea { anchors.fill: parent; onClicked: root.compGateSub = "comp" }
                    }
                    Rectangle {
                        Layout.fillWidth: true; implicitHeight: 26; radius: 5
                        color: root.compGateSub === "gate" ? root.appTheme.accent : root.appTheme.panel2
                        Text { anchors.centerIn: parent; text: "Gate"; font.pixelSize: 10; font.bold: true; color: root.compGateSub === "gate" ? "#06272c" : root.appTheme.textDim }
                        MouseArea { anchors.fill: parent; onClicked: root.compGateSub = "gate" }
                    }
                }

                ColumnLayout {
                    visible: root.compGateSub === "comp"
                    Layout.fillWidth: true
                    spacing: 6
                    Rectangle {
                        Layout.fillWidth: true; Layout.preferredHeight: 110; radius: 8; color: root.appTheme.inset
                        CompGraph { anchors.fill: parent; threshold: root.compThresh; ratio: root.compRatio; makeup: root.compMakeup; enabled: root.compEn }
                        Text {
                            anchors.bottom: parent.bottom; anchors.bottomMargin: 6; anchors.horizontalCenter: parent.horizontalCenter
                            text: Math.min(0, root.liveGr).toFixed(1) + " dB GR"; color: root.appTheme.textDim; font.pixelSize: 10; font.family: "monospace"
                        }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        Text { text: "On"; color: root.appTheme.textDim; font.pixelSize: 11; Layout.fillWidth: true }
                        Rectangle {
                            width: 34; height: 18; radius: 9
                            color: root.compEn ? root.appTheme.accentDim : root.appTheme.inset
                            border.color: root.appTheme.border; border.width: 1
                            Rectangle { width: 12; height: 12; radius: 6; color: root.compEn ? root.appTheme.accent : root.appTheme.textFaint; x: root.compEn ? parent.width - width - 2 : 2; anchors.verticalCenter: parent.verticalCenter }
                            MouseArea { anchors.fill: parent; onClicked: { root.compEn = !root.compEn; root.applyComp(); } }
                        }
                    }
                    GridLayout {
                        columns: 3
                        columnSpacing: 6; rowSpacing: 6
                        Layout.fillWidth: true
                        ColumnLayout { spacing: 2; Knob { size: 40; from: -60; to: 0; value: root.compThresh; displayText: value.toFixed(0) + " dB"; onMoved: (v) => { root.compThresh = v; root.applyComp(); } } Text { text: "THRESHOLD"; color: root.appTheme.textDim; font.pixelSize: 7; Layout.alignment: Qt.AlignHCenter } }
                        ColumnLayout { spacing: 2; Knob { size: 40; from: 1; to: 20; log: true; value: root.compRatio; displayText: value.toFixed(1) + ":1"; onMoved: (v) => { root.compRatio = v; root.applyComp(); } } Text { text: "RATIO"; color: root.appTheme.textDim; font.pixelSize: 7; Layout.alignment: Qt.AlignHCenter } }
                        ColumnLayout { spacing: 2; Knob { size: 40; from: 0.1; to: 200; log: true; value: root.compAttack; displayText: value.toFixed(0) + "ms"; onMoved: (v) => { root.compAttack = v; root.applyComp(); } } Text { text: "ATTACK"; color: root.appTheme.textDim; font.pixelSize: 7; Layout.alignment: Qt.AlignHCenter } }
                        ColumnLayout { spacing: 2; Knob { size: 40; from: 1; to: 1000; log: true; value: root.compRelease; displayText: value.toFixed(0) + "ms"; onMoved: (v) => { root.compRelease = v; root.applyComp(); } } Text { text: "RELEASE"; color: root.appTheme.textDim; font.pixelSize: 7; Layout.alignment: Qt.AlignHCenter } }
                        ColumnLayout { spacing: 2; Knob { size: 40; from: 0; to: 24; value: root.compMakeup; displayText: (value >= 0 ? "+" : "") + value.toFixed(1) + " dB"; onMoved: (v) => { root.compMakeup = v; root.applyComp(); } } Text { text: "MAKEUP"; color: root.appTheme.textDim; font.pixelSize: 7; Layout.alignment: Qt.AlignHCenter } }
                    }
                }

                ColumnLayout {
                    visible: root.compGateSub === "gate"
                    Layout.fillWidth: true
                    spacing: 6
                    Rectangle {
                        Layout.fillWidth: true; Layout.preferredHeight: 110; radius: 8; color: root.appTheme.inset
                        GateDoor { anchors.fill: parent; liveGrDb: root.liveGateGr; enabled: root.gateEn }
                        Text {
                            anchors.bottom: parent.bottom; anchors.bottomMargin: 6; anchors.horizontalCenter: parent.horizontalCenter
                            text: Math.min(0, root.liveGateGr).toFixed(1) + " dB GR"; color: root.appTheme.textDim; font.pixelSize: 10; font.family: "monospace"
                        }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        Text { text: "On"; color: root.appTheme.textDim; font.pixelSize: 11; Layout.fillWidth: true }
                        Rectangle {
                            width: 34; height: 18; radius: 9
                            color: root.gateEn ? root.appTheme.accentDim : root.appTheme.inset
                            border.color: root.appTheme.border; border.width: 1
                            Rectangle { width: 12; height: 12; radius: 6; color: root.gateEn ? root.appTheme.accent : root.appTheme.textFaint; x: root.gateEn ? parent.width - width - 2 : 2; anchors.verticalCenter: parent.verticalCenter }
                            MouseArea { anchors.fill: parent; onClicked: { root.gateEn = !root.gateEn; root.applyGate(); } }
                        }
                    }
                    GridLayout {
                        columns: 3
                        columnSpacing: 6; rowSpacing: 6
                        Layout.fillWidth: true
                        ColumnLayout { spacing: 2; Knob { size: 40; from: -80; to: 0; value: root.gateThresh; displayText: value.toFixed(0) + " dB"; onMoved: (v) => { root.gateThresh = v; root.applyGate(); } } Text { text: "THRESHOLD"; color: root.appTheme.textDim; font.pixelSize: 7; Layout.alignment: Qt.AlignHCenter } }
                        ColumnLayout { spacing: 2; Knob { size: 40; from: 0.1; to: 100; log: true; value: root.gateAttack; displayText: value.toFixed(1) + "ms"; onMoved: (v) => { root.gateAttack = v; root.applyGate(); } } Text { text: "ATTACK"; color: root.appTheme.textDim; font.pixelSize: 7; Layout.alignment: Qt.AlignHCenter } }
                        ColumnLayout { spacing: 2; Knob { size: 40; from: 1; to: 1000; log: true; value: root.gateRelease; displayText: value.toFixed(0) + "ms"; onMoved: (v) => { root.gateRelease = v; root.applyGate(); } } Text { text: "RELEASE"; color: root.appTheme.textDim; font.pixelSize: 7; Layout.alignment: Qt.AlignHCenter } }
                        ColumnLayout { spacing: 2; Knob { size: 40; from: 0; to: 500; value: root.gateHold; displayText: value.toFixed(0) + "ms"; onMoved: (v) => { root.gateHold = v; root.applyGate(); } } Text { text: "HOLD"; color: root.appTheme.textDim; font.pixelSize: 7; Layout.alignment: Qt.AlignHCenter } }
                    }
                }
            }
        }
    }

    // ---- Plugins ----
    Rectangle {
        Layout.fillWidth: true
        radius: 10
        color: root.appTheme.panel
        border.color: root.appTheme.border
        border.width: 1
        implicitHeight: pluginCol.implicitHeight + 32
        ColumnLayout {
            id: pluginCol
            anchors.fill: parent
            anchors.margins: 16
            spacing: 6
            Text { text: "PLUGINS"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
            Repeater {
                model: root.plugins
                delegate: Rectangle {
                    id: pluginRow
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
                            width: 30; height: 16; radius: 8
                            color: pluginRow.modelData[2] ? root.appTheme.accentDim : root.appTheme.inset
                            border.color: pluginRow.modelData[2] ? root.appTheme.accent : root.appTheme.border; border.width: 1
                            Rectangle { width: 12; height: 12; radius: 6; color: pluginRow.modelData[2] ? root.appTheme.accent : root.appTheme.textFaint; x: pluginRow.modelData[2] ? parent.width - width - 2 : 2; anchors.verticalCenter: parent.verticalCenter }
                            MouseArea {
                                anchors.fill: parent
                                onClicked: root.appWin.dcall("SetGroupPluginBypass", "uub", [root.selectedGroupId, pluginRow.modelData[0], pluginRow.modelData[2] ? "true" : "false"], function() { root.refreshPlugins(); })
                            }
                        }
                        Text { text: root.pluginTypeName(pluginRow.modelData[1]); color: root.appTheme.text; font.pixelSize: 12; Layout.preferredWidth: 110; elide: Text.ElideRight }
                        Repeater {
                            model: pluginRow.modelData[3]
                            delegate: RowLayout {
                                required property string modelData
                                required property int index
                                spacing: 4
                                Text { text: parent.modelData; color: root.appTheme.textDim; font.pixelSize: 10 }
                                Rectangle {
                                    width: 56; height: 22; radius: 4; color: root.appTheme.inset; border.color: root.appTheme.border; border.width: 1
                                    TextInput {
                                        anchors.fill: parent; anchors.margins: 4
                                        text: pluginRow.modelData[4][parent.parent.index].toFixed(2)
                                        color: root.appTheme.text; font.pixelSize: 10; font.family: "monospace"
                                        horizontalAlignment: TextInput.AlignHCenter; verticalAlignment: TextInput.AlignVCenter
                                        onEditingFinished: root.appWin.dcall("SetGroupPluginParam", "uusd",
                                            [root.selectedGroupId, pluginRow.modelData[0], parent.parent.modelData, parseFloat(text) || 0],
                                            function() { root.refreshPlugins(); })
                                    }
                                }
                            }
                        }
                        Item { Layout.fillWidth: true }
                        Rectangle {
                            width: 20; height: 20; radius: 4; color: "transparent"; border.color: root.appTheme.border; border.width: 1
                            Text { anchors.centerIn: parent; text: "↑"; color: root.appTheme.textDim; font.pixelSize: 10 }
                            MouseArea {
                                anchors.fill: parent
                                enabled: pluginRow.modelData[0] > 0
                                onClicked: root.appWin.dcall("ReorderGroupPlugin", "uuu", [root.selectedGroupId, pluginRow.modelData[0], pluginRow.modelData[0] - 1], function() { root.refreshPlugins(); })
                            }
                        }
                        Rectangle {
                            width: 20; height: 20; radius: 4; color: "transparent"; border.color: root.appTheme.border; border.width: 1
                            Text { anchors.centerIn: parent; text: "↓"; color: root.appTheme.textDim; font.pixelSize: 10 }
                            MouseArea {
                                anchors.fill: parent
                                enabled: pluginRow.modelData[0] < root.plugins.length - 1
                                onClicked: root.appWin.dcall("ReorderGroupPlugin", "uuu", [root.selectedGroupId, pluginRow.modelData[0], pluginRow.modelData[0] + 1], function() { root.refreshPlugins(); })
                            }
                        }
                        Rectangle {
                            width: 20; height: 20; radius: 4; color: "transparent"
                            Text { anchors.centerIn: parent; text: "✕"; color: root.appTheme.textDim; font.pixelSize: 12 }
                            MouseArea {
                                anchors.fill: parent
                                onClicked: root.appWin.dcall("RemoveGroupPlugin", "uu", [root.selectedGroupId, pluginRow.modelData[0]], function() { root.refreshPlugins(); })
                            }
                        }
                    }
                }
            }
            Text { visible: root.plugins.length === 0; text: "No plugins in this group's chain."; color: root.appTheme.textFaint; font.pixelSize: 11 }
            RowLayout {
                Layout.topMargin: 6
                spacing: 6
                Controls.ComboBox {
                    id: addPluginType
                    model: root.availablePlugins.map(p => p[1])
                }
                Rectangle {
                    implicitWidth: 54; implicitHeight: 26; radius: 5; color: root.appTheme.accent
                    Text { anchors.centerIn: parent; text: "+ Add"; font.pixelSize: 11; font.bold: true; color: "#06272c" }
                    MouseArea {
                        anchors.fill: parent
                        onClicked: {
                            const p = root.availablePlugins[addPluginType.currentIndex];
                            if (!p || root.selectedGroupId < 0) return;
                            root.appWin.dcall("AddGroupPlugin", "us", [root.selectedGroupId, p[0]], function() { root.refreshPlugins(); });
                        }
                    }
                }
            }
        }
    }
}
