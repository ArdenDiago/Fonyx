// Plain drag-handle slider, horizontal or vertical — ported from
// FonyxWidget.qml's inline Slider2 component (kept there unmodified;
// duplicated here as a standalone file so this app doesn't reach into the
// widget's internals). Avoids QtQuick.Controls' Slider, which pulls in a
// style module not always present in a bare Quickshell config.
import QtQuick

Item {
    id: root
    property real from: 0
    property real to: 1
    property real value: 0
    property color trackColor: "#444"
    property color fillColor: "#ddd"
    property color handleColor: "#ddd"
    property bool vertical: false
    signal moved()

    readonly property real frac: Math.max(0, Math.min(1, (root.value - root.from) / (root.to - root.from)))

    implicitWidth: vertical ? 22 : 120
    implicitHeight: vertical ? 120 : 16

    Rectangle {
        anchors.fill: parent
        anchors.topMargin: root.vertical ? 0 : 6
        anchors.bottomMargin: root.vertical ? 0 : 6
        anchors.leftMargin: root.vertical ? 6 : 0
        anchors.rightMargin: root.vertical ? 6 : 0
        radius: 3
        color: root.trackColor
    }
    Rectangle {
        anchors.left: root.vertical ? undefined : parent.left
        anchors.right: root.vertical ? undefined : undefined
        anchors.bottom: root.vertical ? parent.bottom : undefined
        anchors.top: root.vertical ? undefined : parent.top
        anchors.leftMargin: root.vertical ? 6 : 0
        anchors.topMargin: root.vertical ? 0 : 6
        anchors.bottomMargin: root.vertical ? 0 : 6
        width: root.vertical ? parent.width - 12 : root.frac * parent.width
        height: root.vertical ? root.frac * parent.height : parent.height - 12
        radius: 3
        color: root.fillColor
    }
    Rectangle {
        width: root.vertical ? 18 : 10
        height: root.vertical ? 10 : 16
        radius: 5
        color: root.handleColor
        x: root.vertical ? (parent.width - width) / 2 : Math.max(0, Math.min(parent.width - width, root.frac * (parent.width - width)))
        y: root.vertical ? Math.max(0, Math.min(parent.height - height, (1 - root.frac) * (parent.height - height))) : (parent.height - height) / 2
    }
    MouseArea {
        anchors.fill: parent
        preventStealing: true
        onPositionChanged: (mouse) => { if (pressed) update(mouse.x, mouse.y); }
        onPressed: (mouse) => update(mouse.x, mouse.y)
        function update(x, y) {
            const frac = root.vertical
                ? Math.max(0, Math.min(1, 1 - y / height))
                : Math.max(0, Math.min(1, x / width));
            root.value = root.from + frac * (root.to - root.from);
            root.moved();
        }
    }
}
