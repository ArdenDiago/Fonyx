// Smooth filled spectrum curve(s) or a jittered dot-cloud variant, ported
// from ui/index.html's renderSpectrumWave/renderDotSpectrum (quadratic
// through-midpoints smoothing, same dBFS-ish [-60,0] -> 0..1 mapping).
import QtQuick

Canvas {
    id: root
    property var series: [] // [{bins:[...], color:"#.."}, ...]
    property bool dotMode: false

    function dbToFrac01(db) { return Math.max(0, Math.min(1, (db + 60) / 60)); }

    onPaint: {
        const ctx = getContext("2d");
        ctx.reset();
        if (root.dotMode) {
            const bins = (root.series[0] && root.series[0].bins) || [];
            if (!bins.length) return;
            const DOTS = 4;
            for (let i = 0; i < bins.length; i++) {
                const cx = (i / (bins.length - 1)) * width;
                const cy = height - dbToFrac01(bins[i]) * height;
                for (let d = 0; d < DOTS; d++) {
                    ctx.beginPath();
                    ctx.fillStyle = "#3ecbe0";
                    ctx.globalAlpha = 0.2 + Math.random() * 0.6;
                    ctx.arc(cx + (Math.random() - 0.5) * (width / bins.length) * 0.8,
                            cy + (Math.random() - 0.5) * 16, 1.6, 0, 2 * Math.PI);
                    ctx.fill();
                }
            }
            ctx.globalAlpha = 1;
            return;
        }
        for (const s of root.series) {
            const bins = s.bins;
            if (!bins || !bins.length) continue;
            const pts = bins.map((db, i) => [ (i / (bins.length - 1)) * width, height - dbToFrac01(db) * height ]);
            ctx.beginPath();
            ctx.moveTo(pts[0][0], pts[0][1]);
            if (pts.length < 3) {
                for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i][0], pts[i][1]);
            } else {
                ctx.lineTo((pts[0][0] + pts[1][0]) / 2, (pts[0][1] + pts[1][1]) / 2);
                for (let i = 1; i < pts.length - 1; i++) {
                    const [x0, y0] = pts[i], [x1, y1] = pts[i + 1];
                    ctx.quadraticCurveTo(x0, y0, (x0 + x1) / 2, (y0 + y1) / 2);
                }
                ctx.lineTo(pts[pts.length - 1][0], pts[pts.length - 1][1]);
            }
            ctx.strokeStyle = s.color;
            ctx.lineWidth = 2;
            ctx.stroke();
            ctx.lineTo(width, height);
            ctx.lineTo(0, height);
            ctx.closePath();
            ctx.globalAlpha = 0.12;
            ctx.fillStyle = s.color;
            ctx.fill();
            ctx.globalAlpha = 1;
        }
    }

    Connections { target: root; function onSeriesChanged() { root.requestPaint(); } }
}
