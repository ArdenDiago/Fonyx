// Group picker dropdown, used everywhere index.html has a <select> of
// groups (Inputs' route picker, Outputs' add-route picker, Rules/Ducking/
// Recording group selects). QtQuick.Controls.Basic — the style-agnostic
// pure-QML control set, always present regardless of the system's
// configured Qt Quick Controls style (unlike plain QtQuick.Controls,
// which Slider2's own comment already found unreliable in a bare
// Quickshell config) — so this app gets a real dropdown instead of a
// custom-built popup for every one of these ~6 use sites.
// Known simplification: default Controls.Basic chrome, not a pixel match
// of index.html's dark-styled <select>.
import QtQuick.Controls.Basic as Controls

Controls.ComboBox {
    id: root
    property var groups: [] // [[id, name, ...], ...]
    model: root.groups.map(g => g[1])

    function currentGroupId() {
        return root.currentIndex >= 0 && root.groups[root.currentIndex] ? root.groups[root.currentIndex][0] : null;
    }
}
