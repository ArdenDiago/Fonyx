// Standalone fader popup: FonyxWidget in its own window, for a
// compositor keybind (toggle with `pkill -f FonyxPopup.qml || quickshell -p .../FonyxPopup.qml`).
import QtQuick
import Quickshell

ShellRoot {
    FloatingWindow {
        title: "Fonyx Faders"
        implicitWidth: widget.implicitWidth
        implicitHeight: widget.implicitHeight
        color: "transparent"

        FonyxWidget { id: widget; anchors.fill: parent }
    }
}
