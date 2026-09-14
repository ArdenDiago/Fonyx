// Compressor input/output transfer-function graph — ported from
// ui/index.html's renderCompTransferGraph (same formula as dynamics.rs's
// gain computer).
import QtQuick

Canvas {
    id: root
    property real threshold: -18
    property real ratio: 2
    property real makeup: 0
    property bool enabled: true

    function compOutputDb(inputDb, thresholdDb, ratioVal) {
        if (inputDb > thresholdDb) {
            const over = inputDb - thresholdDb;
            return inputDb - over * (1 - 1 / Math.max(1, ratioVal));
        }
        return inputDb;
    }

    onPaint: {
        const ctx = getContext("2d");
        ctx.reset();
        const w = width, h = height, pad = 14, DB_MIN = -60, DB_MAX = 6;
        const toX = db => pad + ((db - DB_MIN) / (DB_MAX - DB_MIN)) * (w - 2 * pad);
        const toY = db => h - pad - ((db - DB_MIN) / (DB_MAX - DB_MIN)) * (h - 2 * pad);

        ctx.strokeStyle = "#2a2a33"; ctx.lineWidth = 1;
        ctx.beginPath(); ctx.moveTo(toX(DB_MIN), toY(0)); ctx.lineTo(toX(DB_MAX), toY(0)); ctx.stroke();
        ctx.beginPath(); ctx.moveTo(toX(0), toY(DB_MIN)); ctx.lineTo(toX(0), toY(DB_MAX)); ctx.stroke();

        ctx.strokeStyle = "#55555f"; ctx.setLineDash([3, 3]);
        ctx.beginPath(); ctx.moveTo(toX(DB_MIN), toY(DB_MIN)); ctx.lineTo(toX(DB_MAX), toY(DB_MAX)); ctx.stroke();
        ctx.setLineDash([]);

        ctx.strokeStyle = "#e0b04d"; ctx.globalAlpha = 0.7; ctx.setLineDash([2, 2]);
        ctx.beginPath(); ctx.moveTo(toX(root.threshold), toY(DB_MIN)); ctx.lineTo(toX(root.threshold), toY(DB_MAX)); ctx.stroke();
        ctx.setLineDash([]); ctx.globalAlpha = 1;

        const pts = [];
        for (let i = 0; i <= 40; i++) {
            const inputDb = DB_MIN + (i / 40) * (DB_MAX - DB_MIN);
            const outputDb = Math.min(DB_MAX, root.compOutputDb(inputDb, root.threshold, root.ratio) + root.makeup);
            pts.push([toX(inputDb), toY(outputDb)]);
        }
        ctx.beginPath();
        ctx.moveTo(pts[0][0], pts[0][1]);
        for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i][0], pts[i][1]);
        ctx.strokeStyle = root.enabled ? "#3ecbe0" : "#55555f";
        ctx.lineWidth = 2;
        ctx.setLineDash([]);
        ctx.stroke();

        ctx.fillStyle = "#8a8a96"; ctx.font = "8px monospace"; ctx.textAlign = "right"; ctx.textBaseline = "alphabetic";
        ctx.fillText(`${root.ratio.toFixed(1)}:1 @ ${root.threshold.toFixed(0)} dB`, w - 6, 11);
    }

    onThresholdChanged: requestPaint()
    onRatioChanged: requestPaint()
    onMakeupChanged: requestPaint()
    onEnabledChanged: requestPaint()
    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()
}
