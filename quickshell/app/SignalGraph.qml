// Doctor's signal-flow graph — ported from ui/index.html's refreshGraph():
// same 3-column layered layout (apps left, groups middle, outputs right),
// same node/edge data from the daemon's GetGraph.
import QtQuick

Canvas {
    id: root
    property var nodes: [] // [{id,label,kind,state}, ...]
    property var edges: [] // [{from,to}, ...]

    function layout() {
        const columns = { app: 0.12, group: 0.5, output: 0.88 };
        const byKind = { app: [], group: [], output: [] };
        for (const n of root.nodes) if (byKind[n.kind]) byKind[n.kind].push(n);
        const pos = {};
        for (const kind of Object.keys(byKind)) {
            const items = byKind[kind];
            items.forEach((n, i) => {
                pos[n.id] = { x: columns[kind] * width, y: (height / (items.length + 1)) * (i + 1) };
            });
        }
        return pos;
    }

    onPaint: {
        const ctx = getContext("2d");
        ctx.reset();
        const pos = root.layout();
        ctx.strokeStyle = "#2c2c35";
        ctx.lineWidth = 1.5;
        for (const e of root.edges) {
            const a = pos[e.from], b = pos[e.to];
            if (!a || !b) continue;
            ctx.beginPath();
            ctx.moveTo(a.x + 60, a.y);
            ctx.lineTo(b.x - 60, b.y);
            ctx.stroke();
        }
        for (const n of root.nodes) {
            const p = pos[n.id];
            if (!p) continue;
            const color = (n.state === "unavailable" || n.state === "idle") ? "#8a8a96" : "#3ecbe0";
            ctx.fillStyle = "#202027";
            ctx.strokeStyle = color;
            ctx.lineWidth = 1.5;
            const x = p.x - 55, y = p.y - 16, w = 110, h = 32, r = 8;
            ctx.beginPath();
            ctx.moveTo(x + r, y);
            ctx.arcTo(x + w, y, x + w, y + h, r);
            ctx.arcTo(x + w, y + h, x, y + h, r);
            ctx.arcTo(x, y + h, x, y, r);
            ctx.arcTo(x, y, x + w, y, r);
            ctx.closePath();
            ctx.fill();
            ctx.stroke();
            ctx.fillStyle = "#e8e8ec";
            ctx.font = "11px sans-serif";
            ctx.textAlign = "center";
            ctx.textBaseline = "middle";
            const label = n.label.length > 16 ? n.label.slice(0, 15) + "…" : n.label;
            ctx.fillText(label, p.x, p.y);
        }
    }

    Connections { target: root; function onNodesChanged() { root.requestPaint(); } function onEdgesChanged() { root.requestPaint(); } }
    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()
}
