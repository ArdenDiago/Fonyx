// Home tab — ported from ui/index.html's #home section: L/R level meter,
// loudest-group spectrum, CPU/RAM/buffer/xruns tiles, groups list and
// master fader.
import QtQuick
import QtQuick.Layouts
import "../"
import "../DbMath.js" as DbMath

ColumnLayout {
    id: root
    required property var appWin
    required property var appTheme
    Layout.fillWidth: true
    spacing: 14

    property real peakL: 0
    property real peakR: 0
    property real cpu: 0
    property real ram: 0
    property int bufFrames: 0
    property int xruns: 0
    property var loudestBins: []

    function pollLive() {
        appWin.dcall("GetLevels", "", [], function(data) {
            const levels = data[0] || [];
            let pl = 0, pr = 0, loudestId = null, loudestPeak = -1;
            for (const row of levels) {
                const [id, l, r] = row;
                pl = Math.max(pl, l);
                pr = Math.max(pr, r);
                if (l + r > loudestPeak) { loudestPeak = l + r; loudestId = id; }
            }
            root.peakL = pl;
            root.peakR = pr;
            if (loudestId !== null) {
                appWin.dcall("GetSpectrum", "u", [loudestId], function(sd) {
                    root.loudestBins = sd[0] || [];
                });
            }
        });
        appWin.dcall("GetMetrics", "", [], function(data) {
            root.cpu = data[0]; root.ram = data[1]; root.bufFrames = data[2]; root.xruns = data[3];
        });
    }

    Timer { interval: 500; running: true; repeat: true; triggeredOnStart: true; onTriggered: root.pollLive() }

    RowLayout {
        Layout.fillWidth: true
        spacing: 14

        // ---- L/R level meter ----
        Rectangle {
            Layout.preferredWidth: 110
            Layout.preferredHeight: 300
            radius: 10
            color: root.appTheme.inset
            border.color: root.appTheme.border
            border.width: 1

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 12
                RowLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    spacing: 10
                    RowLayout {
                        Layout.fillHeight: true
                        Layout.alignment: Qt.AlignBottom
                        spacing: 10
                        Repeater {
                            model: [root.peakL, root.peakR]
                            delegate: Rectangle {
                                required property real modelData
                                Layout.preferredWidth: 20
                                Layout.preferredHeight: Math.max(1, DbMath.dbToFrac(DbMath.ampToDb(modelData)) * 276)
                                Layout.alignment: Qt.AlignBottom
                                radius: 3
                                gradient: Gradient {
                                    orientation: Gradient.Vertical
                                    GradientStop { position: 1.0; color: root.appTheme.success }
                                    GradientStop { position: 0.23; color: root.appTheme.success }
                                    GradientStop { position: 0.12; color: root.appTheme.warn }
                                    GradientStop { position: 0.0; color: root.appTheme.danger }
                                }
                            }
                        }
                    }
                    DbScale { Layout.fillHeight: true; dbMin: DbMath.METER_DB_MIN; dbMax: DbMath.METER_DB_MAX }
                }
                Text { text: "L / R"; color: root.appTheme.textDim; font.pixelSize: 10; font.family: "monospace" }
            }
        }

        // ---- spectrum ----
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 300
            radius: 10
            color: root.appTheme.inset
            border.color: root.appTheme.border
            border.width: 1
            SpectrumCanvas {
                anchors.fill: parent
                anchors.margins: 10
                series: [{ bins: root.loudestBins, color: root.appTheme.accent }]
            }
        }

        // ---- utility tiles + groups ----
        ColumnLayout {
            spacing: 10
            Layout.preferredWidth: 200
            Layout.maximumWidth: 200

            GridLayout {
                columns: 2
                columnSpacing: 8
                rowSpacing: 8
                Layout.fillWidth: true

                Repeater {
                    model: [
                        { label: "CPU %", value: root.cpu.toFixed(0) },
                        { label: "RAM %", value: root.ram.toFixed(0) },
                        { label: "Buffer", value: String(root.bufFrames) },
                        { label: "Xruns", value: String(root.xruns) },
                    ]
                    delegate: Rectangle {
                        required property var modelData
                        Layout.fillWidth: true
                        Layout.preferredHeight: 60
                        radius: 6
                        color: root.appTheme.inset
                        ColumnLayout {
                            anchors.centerIn: parent
                            spacing: 2
                            Text { text: parent.parent.modelData.value; color: root.appTheme.accent; font.pixelSize: 20; font.bold: true; font.family: "monospace"; Layout.alignment: Qt.AlignHCenter }
                            Text { text: parent.parent.modelData.label; color: root.appTheme.textDim; font.pixelSize: 9; Layout.alignment: Qt.AlignHCenter }
                        }
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                radius: 10
                color: root.appTheme.panel
                border.color: root.appTheme.border
                border.width: 1
                implicitHeight: groupsCol.implicitHeight + 24
                ColumnLayout {
                    id: groupsCol
                    anchors.fill: parent
                    anchors.margins: 12
                    spacing: 6
                    Text { text: "GROUPS"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
                    Repeater {
                        model: root.appWin.groups
                        delegate: Rectangle {
                            required property var modelData
                            Layout.fillWidth: true
                            Layout.preferredHeight: 28
                            radius: 6
                            color: root.appTheme.panel2
                            RowLayout {
                                anchors.fill: parent
                                anchors.margins: 8
                                Text { text: parent.parent.modelData[1]; color: root.appTheme.text; font.pixelSize: 11; Layout.fillWidth: true }
                                Text { text: parent.parent.modelData[2].toFixed(2); color: root.appTheme.text; font.pixelSize: 11; font.family: "monospace" }
                            }
                        }
                    }
                    Text {
                        visible: root.appWin.groups.length === 0
                        text: "No groups yet."
                        color: root.appTheme.textFaint
                        font.pixelSize: 11
                    }
                }
            }
        }

        // ---- master fader ----
        Rectangle {
            Layout.preferredWidth: 110
            Layout.fillHeight: true
            radius: 10
            color: root.appTheme.panel
            border.color: root.appTheme.accent
            border.width: 1
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 16
                RowLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    spacing: 6
                    Slider2 {
                        Layout.fillHeight: true
                        vertical: true
                        from: DbMath.METER_DB_MIN; to: DbMath.METER_DB_MAX
                        value: DbMath.gainToDb(root.appWin.masterGain)
                        trackColor: root.appTheme.inset
                        fillColor: root.appTheme.accent
                        handleColor: root.appTheme.text
                        onMoved: {
                            const gain = DbMath.dbToGain(value);
                            root.appWin.masterGain = gain;
                            root.appWin.dcall("SetMasterGain", "d", [gain.toFixed(3)]);
                        }
                    }
                    DbScale { Layout.fillHeight: true; dbMin: DbMath.METER_DB_MIN; dbMax: DbMath.METER_DB_MAX }
                }
                Text { text: "MASTER"; color: root.appTheme.accent; font.pixelSize: 11; font.bold: true; Layout.alignment: Qt.AlignHCenter }
                Text { text: DbMath.gainToDbText(root.appWin.masterGain); color: root.appTheme.text; font.pixelSize: 12; font.family: "monospace"; Layout.alignment: Qt.AlignHCenter }
            }
        }
    }
}
