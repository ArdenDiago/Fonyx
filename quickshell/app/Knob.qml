// Rotary knob: drag up/down to change value, 270-degree sweep — ported
// from ui/index.html's makeKnob() (same -135..+135deg range, same
// log/linear interpolation, same "120px of drag = full range" feel).
import QtQuick

Item {
    id: root
    property real from: 0
    property real to: 1
    property bool log: false
    property real value: 0
    property string displayText: value.toFixed(1)
    property int size: 40

    signal moved(real value)

    implicitWidth: size
    implicitHeight: size + 16

    function toFrac(v) {
        return root.log
            ? (Math.log(v) - Math.log(root.from)) / (Math.log(root.to) - Math.log(root.from))
            : (v - root.from) / (root.to - root.from);
    }
    function toVal(f) {
        f = Math.max(0, Math.min(1, f));
        return root.log
            ? Math.exp(Math.log(root.from) + f * (Math.log(root.to) - Math.log(root.from)))
            : root.from + f * (root.to - root.from);
    }

    Canvas {
        id: canvas
        width: root.size
        height: root.size
        anchors.horizontalCenter: parent.horizontalCenter
        onPaint: {
            const ctx = getContext("2d");
            ctx.reset();
            const cx = width / 2, cy = height / 2, r = width / 2 - 3;
            ctx.beginPath();
            ctx.arc(cx, cy, r, 0, 2 * Math.PI);
            ctx.fillStyle = "#202027";
            ctx.fill();
            ctx.strokeStyle = "#2c2c35";
            ctx.lineWidth = 1;
            ctx.stroke();

            const f = root.toFrac(root.value);
            const deg = -135 + f * 270;
            const rad = deg * Math.PI / 180;
            const L = r - 4;
            ctx.strokeStyle = "#3ecbe0";
            ctx.lineWidth = 3;
            ctx.lineCap = "round";
            ctx.beginPath();
            ctx.moveTo(cx, cy);
            ctx.lineTo(cx + L * Math.sin(rad), cy - L * Math.cos(rad));
            ctx.stroke();
        }
        Connections { target: root; function onValueChanged() { canvas.requestPaint(); } }
    }

    Text {
        anchors.top: canvas.bottom
        anchors.topMargin: 2
        anchors.horizontalCenter: parent.horizontalCenter
        text: root.displayText
        font.pixelSize: 10
        font.family: "monospace"
        color: "#e8e8ec"
    }

    MouseArea {
        anchors.fill: canvas
        preventStealing: true
        property real startY: 0
        property real startF: 0
        onPressed: (mouse) => { startY = mouse.y; startF = root.toFrac(root.value); }
        onPositionChanged: (mouse) => {
            if (!pressed) return;
            const f = startF + (startY - mouse.y) / 120;
            root.value = root.toVal(f);
            root.moved(root.value);
        }
    }
}
