// Quickshell quick-access widget (PROJECT_SPEC.md Section 4, 7.4;
// build-order stage 8). Thin client only: no DSP or routing logic here,
// same rule as the Tauri UI's backend.
//
// Deliberately NOT full parity with the Tauri UI's tabs (Scenes, Rules,
// Recording, Doctor, Settings, EQ/compressor/gate editing stay desktop-app
// only) — this is a bar/quick-access popup, not a second full frontend.
// Scope is: per-group faders, mute, solo, output routing, and master gain —
// the things worth a glance-and-tweak from a dropdown.
//
// Quickshell has no generic "call any D-Bus method" QML type (verified
// against quickshell.org/docs — only protocol-specific wrappers exist:
// Quickshell.DBusMenu, Quickshell.Services.{Mpris,UPower,SystemTray,...}).
// For a custom interface like org.mtxpanel.Daemon, the documented,
// actually-shipped mechanism is Quickshell.Io's Process type shelling out
// to `busctl`, reading its `--json=short` output — the same pattern the
// project's own README uses to demonstrate the daemon (`busctl --user
// call ...`). This replaces the earlier stub, which deliberately left the
// call unwired rather than guess at a QML D-Bus API that doesn't exist.

import QtQuick
import QtQuick.Layouts
import Quickshell.Io

Item {
    id: root
    implicitWidth: 300
    implicitHeight: content.anchors.topMargin + content.implicitHeight + content.anchors.bottomMargin

    // Same dark-card palette as the Tauri UI's --panel/--border/--accent/
    // --text-*/--warn variables (ui/index.html) so the two surfaces read as
    // one app rather than a styled window next to a bare control list.
    readonly property color colBg: "#1b1b21"
    readonly property color colBorder: "#2c2c35"
    readonly property color colInset: "#0c0c10"
    readonly property color colText: "#e8e8ec"
    readonly property color colTextDim: "#8a8a96"
    readonly property color colTextFaint: "#55555f"
    readonly property color colAccent: "#3ecbe0"
    readonly property color colWarn: "#e0b04d"

    property var groups: [] // [[id, name, gain, routedStreams, protected], ...] from ListGroups
    property var outputs: [] // [[id, name, description, kind, available, isDefault, sampleRate, channels], ...] from ListOutputs
    property var routesByGroup: ({}) // groupId -> [[outputId, gain, enabled, mode, syncMode, delayMs, syncIncluded], ...]
    property real masterGain: 1.0
    property int pollMs: 3000

    // Mute/solo are client-side overlays on top of the server's real gain,
    // same trick the Tauri UI uses (ui/index.html's faderValues/soloedGroups)
    // — the server only ever stores one gain number per group, so "muted"
    // and "the fader you'd go back to" have to live somewhere else, or
    // un-muting would have nothing to restore to but a hardcoded 1.0.
    property var faderValues: ({}) // groupId -> intended linear gain, seeded once, never overwritten by polling
    property var mutedGroups: []
    property var soloedGroups: []
    // Same "delta together" link as the Tauri UI's toggleLink/setGain
    // (ui/index.html) — linked faders move by the same *gain delta* when
    // one is dragged, not to the same absolute value, so two groups at
    // different levels stay offset instead of snapping to match.
    property var linkedGroups: []

    signal pollTick()

    // Same -40..+12dB scale and dB-linear fader mapping as the Tauri UI's
    // own METER_DB_MIN/MAX (ui/index.html) — a 0..2 linear gain slider
    // buries most of the audible range in the bottom ~10% of the track,
    // the same bug that file's own comment documents fixing there.
    readonly property real dbMin: -40
    readonly property real dbMax: 12
    function gainToDb(gain) { return gain > 1e-4 ? 20 * Math.log(gain) / Math.LN10 : root.dbMin; }
    function dbToGain(db) { return db <= root.dbMin ? 0 : Math.pow(10, db / 20); }
    function dbText(gain) {
        if (gain <= 1e-4) return "-∞ dB";
        const db = root.gainToDb(gain);
        return (db >= 0 ? "+" : "") + db.toFixed(1) + " dB";
    }

    function busctlCall(method, signature, args) {
        return ["busctl", "--user", "--json=short", "call",
            "org.mtxpanel.Daemon", "/org/mtxpanel/Daemon", "org.mtxpanel.Daemon",
            method].concat(signature ? [signature].concat(args) : []);
    }

    function seedFaderValue(groupId, gain) {
        if (root.faderValues[groupId] === undefined) {
            var copy = Object.assign({}, root.faderValues);
            copy[groupId] = gain;
            root.faderValues = copy;
        }
    }

    Process {
        id: listGroupsProc
        command: root.busctlCall("ListGroups", "", [])
        stdout: StdioCollector {
            onStreamFinished: {
                try {
                    const reply = JSON.parse(text);
                    // busctl --json=short wraps a(usd...) as {"type":"...","data":[[[id,name,gain,...],...]]}
                    const list = reply.data[0] || [];
                    for (const g of list) root.seedFaderValue(g[0], g[2]);
                    root.groups = list;
                } catch (e) {
                    console.warn("MtxPanelWidget: failed to parse ListGroups reply:", e, text);
                }
            }
        }
    }

    Process {
        id: listOutputsProc
        command: root.busctlCall("ListOutputs", "", [])
        stdout: StdioCollector {
            onStreamFinished: {
                try {
                    root.outputs = JSON.parse(text).data[0] || [];
                } catch (e) {
                    console.warn("MtxPanelWidget: failed to parse ListOutputs reply:", e, text);
                }
            }
        }
    }

    Process {
        id: getMasterGainProc
        command: root.busctlCall("GetMasterGain", "", [])
        stdout: StdioCollector {
            onStreamFinished: {
                try {
                    root.masterGain = JSON.parse(text).data[0];
                } catch (e) {
                    console.warn("MtxPanelWidget: failed to parse GetMasterGain reply:", e, text);
                }
            }
        }
    }

    Timer {
        interval: root.pollMs
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            listGroupsProc.running = true;
            listOutputsProc.running = true;
            getMasterGainProc.running = true;
            root.pollTick();
        }
    }

    // One ListGroupOutputRoutes call per group — there's no bulk "routes for
    // every group" D-Bus call, but a handful of busctl shells every 3s is
    // cheap next to the DSP thread this is only ever reading state from.
    // Non-visual: lives directly under root like the Process/Timer above,
    // not inside `content`, so it never affects the visible layout.
    Repeater {
        model: root.groups
        delegate: Item {
            id: routeFetcher
            required property var modelData
            Process {
                id: routeProc
                command: root.busctlCall("ListGroupOutputRoutes", "u", [String(routeFetcher.modelData[0])])
                stdout: StdioCollector {
                    onStreamFinished: {
                        try {
                            const reply = JSON.parse(text);
                            var copy = Object.assign({}, root.routesByGroup);
                            copy[routeFetcher.modelData[0]] = reply.data[0] || [];
                            root.routesByGroup = copy;
                        } catch (e) {
                            console.warn("MtxPanelWidget: failed to parse ListGroupOutputRoutes reply:", e, text);
                        }
                    }
                }
            }
            Connections { target: root; function onPollTick() { routeProc.running = true; } }
            Component.onCompleted: routeProc.running = true
        }
    }

    function isMuted(groupId) { return root.mutedGroups.indexOf(groupId) !== -1; }
    function isSoloedOn(groupId) { return root.soloedGroups.indexOf(groupId) !== -1; }
    function isSoloedOut(groupId) { return root.soloedGroups.length > 0 && !root.isSoloedOn(groupId); }
    function isEffectivelyMuted(groupId) { return root.isMuted(groupId) || root.isSoloedOut(groupId); }

    function setGain(groupId, value) {
        setGainProc.command = root.busctlCall("SetGain", "ud", [String(groupId), value.toFixed(3)]);
        setGainProc.running = true;
    }
    Process { id: setGainProc }

    // Sends the *actually applied* gain (0 while muted/soloed-out, the
    // user's real intended value otherwise) without touching that intended
    // value — the split this whole mute/solo scheme exists for.
    function applyGroupGain(groupId) {
        const intended = root.faderValues[groupId] !== undefined ? root.faderValues[groupId] : 0;
        root.setGain(groupId, root.isEffectivelyMuted(groupId) ? 0 : intended);
    }

    function isLinked(groupId) { return root.linkedGroups.indexOf(groupId) !== -1; }

    function toggleLinkGroup(groupId) {
        const idx = root.linkedGroups.indexOf(groupId);
        root.linkedGroups = idx === -1 ? root.linkedGroups.concat([groupId]) : root.linkedGroups.filter(g => g !== groupId);
    }

    function onFaderDragged(groupId, db) {
        const newGain = root.dbToGain(db);
        const prevGain = root.faderValues[groupId] !== undefined ? root.faderValues[groupId] : newGain;
        var copy = Object.assign({}, root.faderValues);
        copy[groupId] = newGain;

        const dragged = [groupId];
        if (root.isLinked(groupId)) {
            const delta = newGain - prevGain;
            const maxGain = root.dbToGain(root.dbMax);
            for (const otherId of root.linkedGroups) {
                if (otherId === groupId) continue;
                const otherPrev = copy[otherId] !== undefined ? copy[otherId] : 0;
                copy[otherId] = Math.max(0, Math.min(maxGain, otherPrev + delta));
                dragged.push(otherId);
            }
        }

        root.faderValues = copy;
        for (const id of dragged) root.applyGroupGain(id);
    }

    function toggleMuteGroup(groupId) {
        const idx = root.mutedGroups.indexOf(groupId);
        root.mutedGroups = idx === -1 ? root.mutedGroups.concat([groupId]) : root.mutedGroups.filter(g => g !== groupId);
        root.applyGroupGain(groupId);
    }

    // Solo changes every group's effective mute state at once (everyone
    // else goes quiet), not just the one clicked — same "Clear solo"-style
    // semantics as the Tauri UI's toggleSolo (ui/index.html).
    function toggleSoloGroup(groupId) {
        const idx = root.soloedGroups.indexOf(groupId);
        root.soloedGroups = idx === -1 ? root.soloedGroups.concat([groupId]) : root.soloedGroups.filter(g => g !== groupId);
        for (const g of root.groups) root.applyGroupGain(g[0]);
    }

    function isRouted(groupId, outputId) {
        const routes = root.routesByGroup[groupId] || [];
        for (const r of routes) if (r[0] === outputId && r[2]) return true;
        return false;
    }

    function toggleOutputRoute(groupId, outputId, currentlyRouted) {
        outputRouteProc.command = currentlyRouted
            ? root.busctlCall("RemoveGroupOutputRoute", "us", [String(groupId), outputId])
            : root.busctlCall("SetGroupOutputRoute", "usdb", [String(groupId), outputId, "1.000", "true"]);
        outputRouteProc.running = true;
    }
    Process { id: outputRouteProc }

    function setMasterGainValue(db) {
        root.masterGain = root.dbToGain(db);
        setMasterGainProc.command = root.busctlCall("SetMasterGain", "d", [root.masterGain.toFixed(3)]);
        setMasterGainProc.running = true;
    }
    Process { id: setMasterGainProc }

    // "Fieldset" framing, like a terminal TUI's `┌─ Label ─┐` box (or a
    // network-manager quick-panel section): the title sits directly on the
    // border line rather than above a separate divider, so the border reads
    // as one labelled frame instead of a plain box.
    Rectangle {
        id: frame
        anchors.fill: parent
        anchors.topMargin: titleLabel.height / 2
        radius: 6
        color: root.colBg
        border.color: root.colAccent
        border.width: 1
    }

    Rectangle {
        id: titleBg
        x: 8
        y: 0
        width: titleLabel.implicitWidth + 8
        height: titleLabel.implicitHeight
        color: root.colBg
    }

    Text {
        id: titleLabel
        x: titleBg.x + 4
        y: 0
        text: "MTX Panel"
        font.bold: true
        font.pixelSize: 12
        color: root.colAccent
    }

    ColumnLayout {
        id: content
        anchors.fill: parent
        anchors.topMargin: titleLabel.height + 10
        anchors.leftMargin: 12
        anchors.rightMargin: 12
        anchors.bottomMargin: 12
        spacing: 10

        RowLayout {
            Layout.fillWidth: true
            Text { text: "Master"; color: root.colText; font.bold: true; font.pixelSize: 11; Layout.preferredWidth: 76 }
            Slider2 {
                Layout.fillWidth: true
                trackColor: root.colInset
                fillColor: root.colAccent
                handleColor: root.colText
                from: root.dbMin; to: root.dbMax
                value: root.gainToDb(root.masterGain)
                onMoved: root.setMasterGainValue(value)
            }
            Text {
                text: root.dbText(root.masterGain)
                font.pixelSize: 10
                color: root.colTextDim
                Layout.preferredWidth: 52
                horizontalAlignment: Text.AlignRight
            }
        }

        Rectangle { Layout.fillWidth: true; height: 1; color: root.colBorder }

        Repeater {
            model: root.groups
            delegate: ColumnLayout {
                id: groupBlock
                required property var modelData
                Layout.fillWidth: true
                spacing: 4
                // Dimmed (not silenced-looking) while some *other* group is
                // soloed — distinct from the mute dot, which is this group's
                // own explicit choice.
                opacity: root.isSoloedOut(groupBlock.modelData[0]) ? 0.45 : 1.0

                RowLayout {
                    Layout.fillWidth: true
                    Text {
                        text: groupBlock.modelData[1]
                        color: root.colText
                        font.pixelSize: 11
                        Layout.preferredWidth: 76
                        elide: Text.ElideRight
                    }
                    Slider2 {
                        Layout.fillWidth: true
                        trackColor: root.colInset
                        fillColor: root.colAccent
                        handleColor: root.colText
                        from: root.dbMin; to: root.dbMax
                        value: root.gainToDb(root.faderValues[groupBlock.modelData[0]] !== undefined
                            ? root.faderValues[groupBlock.modelData[0]] : groupBlock.modelData[2])
                        onMoved: root.onFaderDragged(groupBlock.modelData[0], value)
                    }
                    Text {
                        text: root.dbText(groupBlock.modelData[2])
                        font.pixelSize: 10
                        color: root.colTextDim
                        Layout.preferredWidth: 52
                        horizontalAlignment: Text.AlignRight
                    }
                    // Plain colored shapes instead of emoji glyphs (🔗/🔇/🔊)
                    // — this machine (and potentially any bare Quickshell/
                    // Noctalia install) has no emoji font, which renders
                    // those as a missing-glyph box. Vector shapes have no
                    // font dependency; link gets a square instead of a
                    // circle so it doesn't read as a third mute/solo dot.
                    Rectangle {
                        width: 10; height: 10; radius: 2
                        color: root.isLinked(groupBlock.modelData[0]) ? root.colAccent : root.colTextFaint
                        MouseArea {
                            anchors.fill: parent; anchors.margins: -4
                            onClicked: root.toggleLinkGroup(groupBlock.modelData[0])
                        }
                    }
                    Rectangle {
                        width: 10; height: 10; radius: 5
                        color: root.isMuted(groupBlock.modelData[0]) ? root.colTextFaint : root.colAccent
                        MouseArea {
                            anchors.fill: parent; anchors.margins: -4
                            onClicked: root.toggleMuteGroup(groupBlock.modelData[0])
                        }
                    }
                    Rectangle {
                        width: 10; height: 10; radius: 5
                        color: root.isSoloedOn(groupBlock.modelData[0]) ? root.colWarn : root.colTextFaint
                        MouseArea {
                            anchors.fill: parent; anchors.margins: -4
                            onClicked: root.toggleSoloGroup(groupBlock.modelData[0])
                        }
                    }
                }

                Flow {
                    Layout.fillWidth: true
                    spacing: 4
                    Repeater {
                        model: root.outputs.filter(o => o[4]) // available only
                        delegate: Rectangle {
                            id: chip
                            required property var modelData
                            readonly property string outId: modelData[0]
                            readonly property string label: modelData[2] || modelData[1]
                            // groupBlock.modelData, not the bare `modelData` —
                            // this delegate's own `modelData` (the output
                            // tuple) shadows the outer per-group one.
                            readonly property bool routed: root.isRouted(groupBlock.modelData[0], outId)
                            radius: 8
                            color: routed ? root.colAccent : root.colInset
                            border.color: root.colBorder
                            border.width: routed ? 0 : 1
                            width: chipText.implicitWidth + 12
                            height: chipText.implicitHeight + 6
                            Text {
                                id: chipText
                                anchors.centerIn: parent
                                text: chip.label.length > 14 ? chip.label.substring(0, 13) + "…" : chip.label
                                font.pixelSize: 9
                                color: chip.routed ? root.colBg : root.colTextDim
                            }
                            MouseArea {
                                anchors.fill: parent
                                onClicked: root.toggleOutputRoute(groupBlock.modelData[0], chip.outId, chip.routed)
                            }
                        }
                    }
                }
            }
        }

        Text {
            visible: root.groups.length === 0
            text: "No groups (is mtxpanel-daemon running?)"
            font.pixelSize: 10
            color: root.colTextFaint
        }
    }

    // QtQuick.Controls' Slider pulls in a style module that isn't always
    // present in a bare Quickshell config; a plain drag-handle avoids that
    // dependency entirely — one Rectangle, no extra import.
    component Slider2: Item {
        id: sliderRoot
        property real from: 0
        property real to: 1
        property real value: 0
        property color trackColor: "#444"
        property color fillColor: "#ddd"
        property color handleColor: "#ddd"
        signal moved()
        height: 16

        readonly property real frac: Math.max(0, Math.min(1, (sliderRoot.value - sliderRoot.from) / (sliderRoot.to - sliderRoot.from)))

        Rectangle { anchors.fill: parent; anchors.topMargin: 6; anchors.bottomMargin: 6; radius: 3; color: sliderRoot.trackColor }
        Rectangle {
            anchors.top: parent.top; anchors.bottom: parent.bottom
            anchors.topMargin: 6; anchors.bottomMargin: 6
            width: sliderRoot.frac * parent.width
            radius: 3
            color: sliderRoot.fillColor
        }
        Rectangle {
            width: 10; height: 16; radius: 5; color: sliderRoot.handleColor
            x: Math.max(0, Math.min(parent.width - width, sliderRoot.frac * (parent.width - width)))
        }
        MouseArea {
            anchors.fill: parent
            onPositionChanged: (mouse) => { if (pressed) update(mouse.x); }
            onPressed: (mouse) => update(mouse.x)
            function update(x) {
                const frac = Math.max(0, Math.min(1, x / width));
                sliderRoot.value = sliderRoot.from + frac * (sliderRoot.to - sliderRoot.from);
                sliderRoot.moved();
            }
        }
    }
}
