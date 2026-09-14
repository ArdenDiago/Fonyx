// Interactive EQ graph — ported from ui/index.html's #eq-graph: RBJ biquad
// magnitude response (mirrors daemon/src/dsp/biquad.rs exactly), draggable
// band/HP/LP nodes (drag = freq/gain, wheel = Q), spectrum overlay.
import QtQuick

Item {
    id: root
    property var bands: []          // [{freq,gain,q,enabled}, ...]
    property var hp: ({ freq: 80, q: 0.707, enabled: false })
    property var lp: ({ freq: 18000, q: 0.707, enabled: false })
    property var spectrumBins: []
    property var selectedNode: ({ kind: "band", index: 0 })
    property var bandColors: ["#3ecbe0", "#e0b04d", "#e058b0", "#7ee858", "#a06be8", "#e8584f"]

    signal nodeSelected(string kind, int index)
    signal bandDragged(int index, real freq, real gain, real q)
    signal filterDragged(string kind, real freq, real q)

    readonly property int padL: 40
    readonly property int padR: 14
    readonly property int padT: 10
    readonly property int padB: 22
    readonly property real freqMin: 20
    readonly property real freqMax: 20000
    readonly property real dbRange: 18

    function plotW() { return width - padL - padR; }
    function plotH() { return height - padT - padB; }
    function freqToX(f) { return padL + (Math.log(f) - Math.log(freqMin)) / (Math.log(freqMax) - Math.log(freqMin)) * plotW(); }
    function xToFreq(x) { return Math.exp(Math.log(freqMin) + Math.max(0, Math.min(1, (x - padL) / plotW())) * (Math.log(freqMax) - Math.log(freqMin))); }
    function dbToY(db) { return padT + plotH() / 2 - Math.max(-dbRange, Math.min(dbRange, db)) / dbRange * (plotH() / 2); }
    function yToDb(y) { return -((y - (padT + plotH() / 2)) / (plotH() / 2)) * dbRange; }

    function biquadDbAt(freqHz, c) {
        const w = 2 * Math.PI * freqHz / 48000;
        const cosw = Math.cos(w), cos2w = Math.cos(2 * w), sinw = Math.sin(w), sin2w = Math.sin(2 * w);
        const reNum = c.b0 + c.b1 * cosw + c.b2 * cos2w;
        const imNum = -c.b1 * sinw - c.b2 * sin2w;
        const reDen = 1 + c.a1 * cosw + c.a2 * cos2w;
        const imDen = -c.a1 * sinw - c.a2 * sin2w;
        return 20 * Math.log10(Math.hypot(reNum, imNum) / Math.max(Math.hypot(reDen, imDen), 1e-9));
    }
    function peakingCoeffs(freq, gainDb, q) {
        const a = Math.pow(10, gainDb / 40);
        const w0 = 2 * Math.PI * freq / 48000;
        const alpha = Math.sin(w0) / (2 * q);
        const cosw0 = Math.cos(w0);
        const a0 = 1 + alpha / a;
        return { b0: (1 + alpha * a) / a0, b1: (-2 * cosw0) / a0, b2: (1 - alpha * a) / a0, a1: (-2 * cosw0) / a0, a2: (1 - alpha / a) / a0 };
    }
    function highpassCoeffs(freq, q) {
        const w0 = 2 * Math.PI * freq / 48000, alpha = Math.sin(w0) / (2 * q), cosw0 = Math.cos(w0), a0 = 1 + alpha;
        return { b0: ((1 + cosw0) / 2) / a0, b1: (-(1 + cosw0)) / a0, b2: ((1 + cosw0) / 2) / a0, a1: (-2 * cosw0) / a0, a2: (1 - alpha) / a0 };
    }
    function lowpassCoeffs(freq, q) {
        const w0 = 2 * Math.PI * freq / 48000, alpha = Math.sin(w0) / (2 * q), cosw0 = Math.cos(w0), a0 = 1 + alpha;
        return { b0: ((1 - cosw0) / 2) / a0, b1: (1 - cosw0) / a0, b2: ((1 - cosw0) / 2) / a0, a1: (-2 * cosw0) / a0, a2: (1 - alpha) / a0 };
    }
    function fmtFreq(f) { return f >= 1000 ? (f / 1000).toFixed(f >= 10000 ? 0 : 1) + "k" : String(Math.round(f)); }

    function nodeAt(mx, my) {
        for (const kind of ["hp", "lp"]) {
            const f = root[kind];
            const x = root.freqToX(f.freq), y = root.dbToY(0);
            if (Math.abs(mx - x) < 10 && Math.abs(my - y) < 10) return { kind: kind, index: -1 };
        }
        for (let i = root.bands.length - 1; i >= 0; i--) {
            const b = root.bands[i];
            const x = root.freqToX(b.freq), y = root.dbToY(b.gain);
            const dx = mx - x, dy = my - y;
            if (dx * dx + dy * dy < 121) return { kind: "band", index: i };
        }
        return null;
    }

    property var dragTarget: null

    Canvas {
        id: canvas
        anchors.fill: parent
        onPaint: {
            const ctx = getContext("2d");
            ctx.reset();
            const FREQ_TICKS = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000];
            const DB_TICKS = [-18, -12, -6, 0, 6, 12, 18];
            ctx.lineWidth = 1;
            for (const f of FREQ_TICKS) {
                const x = root.freqToX(f);
                ctx.strokeStyle = "#2a2a33";
                ctx.beginPath(); ctx.moveTo(x, root.padT); ctx.lineTo(x, root.height - root.padB); ctx.stroke();
                ctx.fillStyle = "#55555f"; ctx.font = "9px monospace"; ctx.textAlign = "center"; ctx.textBaseline = "alphabetic";
                ctx.fillText(root.fmtFreq(f), x, root.height - 8);
            }
            for (const db of DB_TICKS) {
                const y = root.dbToY(db);
                ctx.strokeStyle = db === 0 ? "#40404c" : "#2a2a33";
                ctx.beginPath(); ctx.moveTo(root.padL, y); ctx.lineTo(root.width - root.padR, y); ctx.stroke();
                ctx.fillStyle = "#55555f"; ctx.textAlign = "left"; ctx.textBaseline = "alphabetic";
                ctx.fillText((db > 0 ? "+" : "") + db, 4, y + 3);
            }

            if (root.spectrumBins && root.spectrumBins.length) {
                const baseline = root.dbToY(0), bottom = root.height - root.padB;
                const n = root.spectrumBins.length;
                const colW = root.plotW() / n;
                const logLo = Math.log(40), logHi = Math.log(18000);
                ctx.fillStyle = "rgba(138,138,150,0.18)";
                for (let i = 0; i < n; i++) {
                    const frac = Math.max(0, Math.min(1, (root.spectrumBins[i] + 60) / 60));
                    const lo = Math.exp(logLo + (i / n) * (logHi - logLo));
                    const hi = Math.exp(logLo + ((i + 1) / n) * (logHi - logLo));
                    const x = root.freqToX(Math.sqrt(lo * hi)) - colW / 2;
                    const barH = frac * (bottom - baseline);
                    ctx.fillRect(x, bottom - barH, colW * 0.85, barH);
                }
            }

            const pts = [];
            for (let i = 0; i <= 140; i++) {
                const freq = root.freqMin * Math.pow(root.freqMax / root.freqMin, i / 140);
                let db = 0;
                for (const b of root.bands) if (b.enabled) db += root.biquadDbAt(freq, root.peakingCoeffs(b.freq, b.gain, b.q));
                if (root.hp.enabled) db += root.biquadDbAt(freq, root.highpassCoeffs(root.hp.freq, root.hp.q));
                if (root.lp.enabled) db += root.biquadDbAt(freq, root.lowpassCoeffs(root.lp.freq, root.lp.q));
                pts.push([root.freqToX(freq), root.dbToY(db)]);
            }
            ctx.beginPath();
            ctx.moveTo(pts[0][0], pts[0][1]);
            for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i][0], pts[i][1]);
            ctx.strokeStyle = "#3ecbe0"; ctx.lineWidth = 2; ctx.stroke();
            ctx.lineTo(root.width - root.padR, root.dbToY(0));
            ctx.lineTo(root.padL, root.dbToY(0));
            ctx.closePath();
            ctx.fillStyle = "rgba(62,203,224,0.08)";
            ctx.fill();

            root.bands.forEach((b, i) => {
                const x = root.freqToX(b.freq), y = root.dbToY(b.gain);
                const selected = root.selectedNode.kind === "band" && root.selectedNode.index === i;
                ctx.beginPath();
                ctx.arc(x, y, b.enabled ? (selected ? 9 : 7) : 5, 0, 2 * Math.PI);
                ctx.fillStyle = b.enabled ? root.bandColors[i] : "transparent";
                ctx.globalAlpha = b.enabled ? 1 : 0.55;
                ctx.fill();
                ctx.strokeStyle = root.bandColors[i];
                ctx.lineWidth = selected ? 2.5 : 1.5;
                ctx.stroke();
                ctx.globalAlpha = 1;
                if (b.enabled) {
                    ctx.fillStyle = "#0c0c10"; ctx.font = "bold 8px monospace"; ctx.textAlign = "center"; ctx.textBaseline = "middle";
                    ctx.fillText(String(i + 1), x, y + 0.5);
                }
            });

            for (const kind of ["hp", "lp"]) {
                const f = root[kind];
                const x = root.freqToX(f.freq), y = root.dbToY(0);
                const selected = root.selectedNode.kind === kind;
                ctx.beginPath();
                ctx.rect(x - 6, y - 6, 12, 12);
                ctx.fillStyle = f.enabled ? "#dfe3e8" : "transparent";
                ctx.globalAlpha = f.enabled ? 1 : 0.55;
                ctx.fill();
                ctx.strokeStyle = "#dfe3e8";
                ctx.lineWidth = selected ? 2.5 : 1.5;
                ctx.stroke();
                ctx.globalAlpha = 1;
                if (f.enabled) {
                    ctx.fillStyle = "#0c0c10"; ctx.font = "bold 8px monospace"; ctx.textAlign = "center"; ctx.textBaseline = "middle";
                    ctx.fillText(kind.toUpperCase(), x, y + 0.5);
                }
            }
        }
    }

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton
        preventStealing: true
        onPressed: (mouse) => {
            const node = root.nodeAt(mouse.x, mouse.y);
            if (!node) { mouse.accepted = false; return; }
            root.dragTarget = node;
            root.nodeSelected(node.kind, node.index);
        }
        onPositionChanged: (mouse) => {
            if (!root.dragTarget) return;
            const freq = Math.round(root.xToFreq(mouse.x));
            if (root.dragTarget.kind === "band") {
                const gain = Math.max(-18, Math.min(18, root.yToDb(mouse.y)));
                root.bandDragged(root.dragTarget.index, freq, gain, root.bands[root.dragTarget.index].q);
            } else {
                root.filterDragged(root.dragTarget.kind, freq, root[root.dragTarget.kind].q);
            }
        }
        onReleased: root.dragTarget = null

        onWheel: (wheel) => {
            const node = root.nodeAt(wheel.x, wheel.y);
            if (!node) return;
            const factor = Math.pow(1.08, wheel.angleDelta.y / 120);
            if (node.kind === "band") {
                const b = root.bands[node.index];
                root.bandDragged(node.index, b.freq, b.gain, Math.max(0.1, Math.min(10, b.q * factor)));
            } else {
                const f = root[node.kind];
                root.filterDragged(node.kind, f.freq, Math.max(0.1, Math.min(10, f.q * factor)));
            }
        }
    }

    Connections {
        target: root
        function onBandsChanged() { canvas.requestPaint(); }
        function onHpChanged() { canvas.requestPaint(); }
        function onLpChanged() { canvas.requestPaint(); }
        function onSpectrumBinsChanged() { canvas.requestPaint(); }
        function onSelectedNodeChanged() { canvas.requestPaint(); }
    }
    onWidthChanged: canvas.requestPaint()
    onHeightChanged: canvas.requestPaint()
}
