// Generic one-off D-Bus call: shells out to `busctl --json=short`, same
// mechanism as SonicForgeWidget.qml's busctlCall (Quickshell has no generic
// "call any D-Bus method" QML type — verified against quickshell.org/docs).
// Used via Component.createObject + fire() for the ~50 distinct methods
// this full app needs, instead of one hand-named Process per method.
import QtQuick
import Quickshell.Io

Process {
    id: root
    property string method: ""
    property string signature: ""
    property var args: []
    // function(data) — `data` is busctl --json=short's top-level "data"
    // array (one entry per D-Bus out-argument), or null on parse failure.
    property var onResult: null
    property var onFail: null

    command: ["busctl", "--user", "--json=short", "call",
        "org.sonicforge.Daemon", "/org/sonicforge/Daemon", "org.sonicforge.Daemon", root.method]
        .concat(root.signature ? [root.signature].concat(root.args.map(a => String(a))) : [])

    stdout: StdioCollector {
        onStreamFinished: {
            const text = this.text;
            try {
                const reply = text.trim() ? JSON.parse(text) : { data: [] };
                if (root.onResult) root.onResult(reply.data);
            } catch (e) {
                if (root.onFail) root.onFail(e);
                else console.warn("DbusCall failed:", root.method, e, text);
            }
            root.destroy();
        }
    }

    function fire() { root.running = true; }
}
