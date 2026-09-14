// Gate visualization — a literal door that closes in proportion to live
// gate gain-reduction, ported from ui/index.html's renderGateDoor.
import QtQuick

Canvas {
    id: root
    property real liveGrDb: 0
    property bool enabled: true

    onPaint: {
        const ctx = getContext("2d");
        ctx.reset();
        const w = width, h = height;
        const closedFrac = Math.max(0, Math.min(1, -root.liveGrDb / 24));
        const openTop = h * (20 / 110), openBottom = h - h * (12 / 110), openLeft = w * (10 / 200), openRight = w - w * (10 / 200);
        const doorW = ((openRight - openLeft) / 2) * closedFrac;

        ctx.globalAlpha = 0.3;
        ctx.fillStyle = root.enabled ? "#1c5c66" : "#2a2a33";
        ctx.fillRect(openLeft, openTop, openRight - openLeft, openBottom - openTop);
        ctx.globalAlpha = 1;

        ctx.globalAlpha = 0.9;
        ctx.fillStyle = "#e0b04d";
        ctx.fillRect(openLeft, openTop, doorW, openBottom - openTop);
        ctx.fillRect(openRight - doorW, openTop, doorW, openBottom - openTop);
        ctx.globalAlpha = 1;

        ctx.strokeStyle = "#2c2c35"; ctx.lineWidth = 2;
        ctx.strokeRect(openLeft, openTop, openRight - openLeft, openBottom - openTop);

        ctx.fillStyle = "#8a8a96"; ctx.font = "bold 9px monospace"; ctx.textAlign = "center"; ctx.textBaseline = "alphabetic";
        const state = !root.enabled ? "BYPASSED" : closedFrac > 0.05 ? "CLOSING" : "OPEN";
        ctx.fillText(state, w / 2, 13);
    }

    onLiveGrDbChanged: requestPaint()
    onEnabledChanged: requestPaint()
    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()
}
