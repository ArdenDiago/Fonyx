// Full-parity Quickshell rebuild of ui/index.html's Tauri UI — all 9 tabs
// (Home/Inputs/MTX Panel/Faders/Outputs/Scenes/Rules/Recording/Doctor),
// same dark theme, same daemon D-Bus API, driven the same busctl-shellout
// way as SonicForgeWidget.qml (which stays untouched — it's a separate
// bar-popup, this is the full desktop-replacement surface).
//
// Architecture mirrors index.html's own: one shared poll loop
// (refreshAll-equivalent) feeds `groups`/`streams`/`outputs` down to every
// tab as plain arrays (busctl's own a(...) tuple shape, same field order
// the daemon's D-Bus signatures define — no client-side remapping to
// objects, same "positional tuple" convention index.html's own invoke()
// results use).
import QtQuick
import QtQuick.Layouts
import Quickshell
import "./app"
import "./app/tabs"

ShellRoot {
    id: shellRoot

    FloatingWindow {
        id: win
        visible: true
        implicitWidth: 1180
        implicitHeight: 760
        title: "SonicForge"

        // ---- shared state, refreshed on a 5s cycle like index.html's
        // refreshAll(); levels/metrics polled by HomeTab/MtxTab themselves
        // on a faster cycle like its pollLive() (200ms there — 500ms here,
        // busctl's own process-spawn overhead is much higher than an
        // in-process D-Bus call, so a tighter interval would just pile up
        // overlapping calls). ----
        property var groups: []      // [[id,name,gain,routedStreamIds,protected], ...]
        property var streams: []     // [[id,name,service,running], ...]
        property var outputs: []     // [[id,name,description,kind,available,isDefault,sampleRate,channels], ...]
        property real masterGain: 0
        property string connState: "" // "" | "reconnecting" | "degraded"
        property string connText: "Connecting…"
        property bool daemonWasDown: true
        property int tabIndex: 0

        function setConn(state, text) { win.connState = state; win.connText = text; }

        // Generic fire-and-forget / result D-Bus call, used by every tab.
        property Component dbusCallComp: Component { DbusCall {} }
        function dcall(method, signature, args, onResult, onFail) {
            const obj = win.dbusCallComp.createObject(win, {
                method: method, signature: signature || "", args: args || [],
                onResult: onResult || null,
                onFail: onFail || function(e) {
                    win.daemonWasDown = true;
                    win.setConn("reconnecting", "Reconnecting…");
                },
            });
            obj.fire();
        }

        function onCallOk() {
            if (win.daemonWasDown) {
                win.daemonWasDown = false;
                win.setConn("", "Connected");
            }
        }

        function refreshAll() {
            win.dcall("ListGroups", "", [], function(data) { win.groups = data[0] || []; win.onCallOk(); });
            win.dcall("ListStreams", "", [], function(data) { win.streams = data[0] || []; });
            win.dcall("ListOutputs", "", [], function(data) { win.outputs = data[0] || []; });
            win.dcall("GetMasterGain", "", [], function(data) { win.masterGain = data[0]; });
            win.pollHealth();
            if (rulesTabLoader.item) rulesTabLoader.item.refresh();
            if (scenesTabLoader.item) scenesTabLoader.item.refresh();
            if (recordingTabLoader.item) recordingTabLoader.item.refresh();
            if (outputsTabLoader.item) outputsTabLoader.item.refresh();
        }

        function pollHealth() {
            win.dcall("RunHealthChecks", "", [], function(data) {
                win.onCallOk();
                const score = data[0];
                if (!win.daemonWasDown) win.setConn(score === "ok" ? "" : "degraded", score === "ok" ? "Connected" : "Backend degraded");
                if (doctorTabLoader.item) doctorTabLoader.item.applyHealth(score, data[1] || []);
            });
        }

        Timer { interval: 5000; running: true; repeat: true; triggeredOnStart: true; onTriggered: win.refreshAll() }

        Theme { id: theme }

        Rectangle {
            anchors.fill: parent
            color: theme.void_

            ColumnLayout {
                anchors.fill: parent
                spacing: 0

                RowLayout {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 44
                    Layout.leftMargin: 12
                    Layout.rightMargin: 12
                    spacing: 2

                    Repeater {
                        model: ["Home", "Inputs", "MTX Panel", "Faders", "Outputs", "Scenes", "Rules", "Recording", "Doctor", "Settings"]
                        delegate: Rectangle {
                            required property string modelData
                            required property int index
                            color: "transparent"
                            Layout.preferredWidth: label.implicitWidth + 24
                            Layout.preferredHeight: 44
                            Rectangle {
                                anchors.bottom: parent.bottom
                                width: parent.width
                                height: 2
                                color: win.tabIndex === index ? theme.accent : "transparent"
                            }
                            Text {
                                id: label
                                anchors.centerIn: parent
                                text: parent.modelData
                                font.pixelSize: 12
                                font.bold: true
                                color: win.tabIndex === index ? theme.accent : theme.textDim
                            }
                            MouseArea { anchors.fill: parent; onClicked: win.tabIndex = index }
                        }
                    }

                    Item { Layout.fillWidth: true }

                    Rectangle {
                        radius: 999
                        color: theme.panel2
                        border.color: win.connState === "reconnecting" ? theme.danger : win.connState === "degraded" ? theme.warn : theme.border
                        border.width: 1
                        Layout.preferredWidth: connRow.implicitWidth + 20
                        Layout.preferredHeight: 22
                        RowLayout {
                            id: connRow
                            anchors.centerIn: parent
                            spacing: 6
                            Rectangle {
                                width: 7; height: 7; radius: 4
                                color: win.connState === "reconnecting" ? theme.danger : win.connState === "degraded" ? theme.warn : theme.success
                            }
                            Text {
                                text: win.connText
                                font.pixelSize: 11
                                font.bold: true
                                color: win.connState === "reconnecting" ? theme.danger : win.connState === "degraded" ? theme.warn : theme.textDim
                            }
                        }
                    }
                }

                Rectangle { Layout.fillWidth: true; height: 1; color: theme.border }

                Flickable {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    contentWidth: width
                    contentHeight: pageStack.implicitHeight
                    clip: true

                    ColumnLayout {
                        id: pageStack
                        width: parent.width

                        Loader {
                            id: homeTabLoader
                            Layout.fillWidth: true
                            Layout.margins: 16
                            active: true
                            visible: win.tabIndex === 0
                            sourceComponent: HomeTab { appWin: win; appTheme: theme }
                        }
                        Loader {
                            id: inputsTabLoader
                            Layout.fillWidth: true
                            Layout.margins: 16
                            active: true
                            visible: win.tabIndex === 1
                            sourceComponent: InputsTab { appWin: win; appTheme: theme }
                        }
                        Loader {
                            id: mtxTabLoader
                            Layout.fillWidth: true
                            Layout.margins: 16
                            active: true
                            visible: win.tabIndex === 2
                            sourceComponent: MtxTab { appWin: win; appTheme: theme }
                        }
                        Loader {
                            id: fadersTabLoader
                            Layout.fillWidth: true
                            Layout.margins: 16
                            active: true
                            visible: win.tabIndex === 3
                            sourceComponent: FadersTab { appWin: win; appTheme: theme }
                        }
                        Loader {
                            id: outputsTabLoader
                            Layout.fillWidth: true
                            Layout.margins: 16
                            active: true
                            visible: win.tabIndex === 4
                            sourceComponent: OutputsTab { appWin: win; appTheme: theme }
                        }
                        Loader {
                            id: scenesTabLoader
                            Layout.fillWidth: true
                            Layout.margins: 16
                            active: true
                            visible: win.tabIndex === 5
                            sourceComponent: ScenesTab { appWin: win; appTheme: theme }
                        }
                        Loader {
                            id: rulesTabLoader
                            Layout.fillWidth: true
                            Layout.margins: 16
                            active: true
                            visible: win.tabIndex === 6
                            sourceComponent: RulesTab { appWin: win; appTheme: theme }
                        }
                        Loader {
                            id: recordingTabLoader
                            Layout.fillWidth: true
                            Layout.margins: 16
                            active: true
                            visible: win.tabIndex === 7
                            sourceComponent: RecordingTab { appWin: win; appTheme: theme }
                        }
                        Loader {
                            id: doctorTabLoader
                            Layout.fillWidth: true
                            Layout.margins: 16
                            active: true
                            visible: win.tabIndex === 8
                            sourceComponent: DoctorTab { appWin: win; appTheme: theme }
                            onLoaded: { item.refresh(); }
                        }
                        Loader {
                            id: settingsTabLoader
                            Layout.fillWidth: true
                            Layout.margins: 16
                            active: true
                            visible: win.tabIndex === 9
                            sourceComponent: SettingsTab { appWin: win; appTheme: theme }
                        }
                    }
                }
            }
        }

        Component.onCompleted: win.refreshAll()
    }
}
