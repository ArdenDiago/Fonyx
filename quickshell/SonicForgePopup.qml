// Standalone fader popup: SonicForgeWidget in its own window, for a
// compositor keybind (toggle with `pkill -f SonicForgePopup.qml || quickshell -p .../SonicForgePopup.qml`).
import QtQuick
import Quickshell

ShellRoot {
    FloatingWindow {
        title: "SonicForge Faders"
        implicitWidth: widget.implicitWidth
        implicitHeight: widget.implicitHeight
        color: "transparent"

        SonicForgeWidget { id: widget; anchors.fill: parent }
    }
}
