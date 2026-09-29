// Standalone fader popup: MtxPanelWidget in its own window, for a
// compositor keybind (toggle with `pkill -f MtxPanelPopup.qml || quickshell -p .../MtxPanelPopup.qml`).
import QtQuick
import Quickshell

ShellRoot {
    FloatingWindow {
        title: "MTX Panel Faders"
        implicitWidth: widget.implicitWidth
        implicitHeight: widget.implicitHeight
        color: "transparent"

        MtxPanelWidget { id: widget; anchors.fill: parent }
    }
}
