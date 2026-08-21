// Quickshell quick-access widget (PROJECT_SPEC.md Section 4, 7.4;
// build-order stage 8 — build this once the D-Bus interface is stable).
//
// Thin client only: no DSP or routing logic here, ever. Every action below
// is a straight call into org.mtxpanel.Daemon on the session bus
// (Section 8). Left as a shell because Quickshell's D-Bus QML API
// (Quickshell.DBusMenu / Quickshell.Io, exact import depends on the
// Quickshell version in the user's shell config) should be checked against
// current docs before wiring the real calls in.

import QtQuick

Item {
    id: root
    width: 240
    height: 320

    // TODO(stage 8): bind to org.mtxpanel.Daemon over the session bus and
    // list groups, mute/fader-move each one, and let the user switch which
    // group an app stream feeds — mirroring the Faders tab (Section 7.4).

    Column {
        anchors.fill: parent
        spacing: 4

        Text { text: "MTX Panel" }
        Text { text: "(not yet wired to org.mtxpanel.Daemon)" }
    }
}
