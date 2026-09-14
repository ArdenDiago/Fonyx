// Output picker dropdown — same pattern as GroupCombo.qml but for
// win.outputs tuples (string ids, not numeric group ids).
import QtQuick.Controls.Basic as Controls

Controls.ComboBox {
    id: root
    property var outputs: [] // [[id, name, description, kind, available, isDefault, sampleRate, channels], ...]
    model: root.outputs.map(o => o[1])

    function currentOutputId() {
        return root.currentIndex >= 0 && root.outputs[root.currentIndex] ? root.outputs[root.currentIndex][0] : null;
    }
}
