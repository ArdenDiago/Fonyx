// Quickshell quick-access widget (PROJECT_SPEC.md Section 4, 7.4;
// build-order stage 8). Thin client only: no DSP or routing logic here,
// same rule as the Tauri UI's backend.
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
    width: 220
    height: 260

    property var groups: [] // [[id, name, gain], ...] from ListGroups
    property int pollMs: 3000

    function busctlCall(method, signature, args) {
        return ["busctl", "--user", "--json=short", "call",
            "org.mtxpanel.Daemon", "/org/mtxpanel/Daemon", "org.mtxpanel.Daemon",
            method].concat(signature ? [signature].concat(args) : []);
    }

    Process {
        id: listGroupsProc
        command: busctlCall("ListGroups", "", [])
        stdout: StdioCollector {
            onStreamFinished: {
                try {
                    const reply = JSON.parse(text);
                    // busctl --json=short wraps a(usd) as {"type":"a(usd)","data":[[[id,name,gain],...]]}
                    root.groups = reply.data[0] || [];
                } catch (e) {
                    console.warn("MtxPanelWidget: failed to parse ListGroups reply:", e, text);
                }
            }
        }
    }

    Timer {
        interval: root.pollMs
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: listGroupsProc.running = true
    }

    function setGain(groupId, value) {
        setGainProc.command = busctlCall("SetGain", "ud", [String(groupId), value.toFixed(3)]);
        setGainProc.running = true;
    }
    Process { id: setGainProc }

    function toggleMute(groupId, currentGain) {
        // No separate mute flag in the D-Bus interface (Section 8) — same
        // "0 gain vs. last non-zero gain" convention as a DAW mute button,
        // kept client-side same as the Faders tab has no server-side mute.
        setGain(groupId, currentGain > 0 ? 0 : 1);
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 8
        spacing: 4

        Text { text: "MTX Panel"; font.bold: true }

        Repeater {
            model: root.groups
            delegate: RowLayout {
                Layout.fillWidth: true
                required property var modelData
                Text { text: modelData[1]; Layout.preferredWidth: 80; elide: Text.ElideRight }
                Slider2 {
                    Layout.fillWidth: true
                    from: 0; to: 2; value: modelData[2]
                    onMoved: root.setGain(modelData[0], value)
                }
                Text {
                    text: modelData[2] <= 0 ? "🔇" : "🔊"
                    MouseArea { anchors.fill: parent; onClicked: root.toggleMute(modelData[0], modelData[2]) }
                }
            }
        }

        Text {
            visible: root.groups.length === 0
            text: "No groups (is mtxpanel-daemon running?)"
            font.pixelSize: 10
            opacity: 0.7
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
        signal moved()
        height: 16

        Rectangle { anchors.fill: parent; anchors.topMargin: 6; anchors.bottomMargin: 6; radius: 3; color: "#444" }
        Rectangle {
            width: 10; height: 16; radius: 5; color: "#ddd"
            x: Math.max(0, Math.min(parent.width - width, (sliderRoot.value - sliderRoot.from) / (sliderRoot.to - sliderRoot.from) * (parent.width - width)))
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
