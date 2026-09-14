// Rules tab — ported from ui/index.html's #rules section: auto-route
// rules, device-connected rules, the rule list, and intelligent ducking.
import QtQuick
import QtQuick.Layouts
import "../"

ColumnLayout {
    id: root
    required property var appWin
    required property var appTheme
    Layout.fillWidth: true
    spacing: 14

    property var rules: []       // [[id,name,enabled,priority,cooldownMs,trigger,actions], ...]
    property var duckingRules: [] // [[id,enabled,trigger,targets,reductionDb,attackMs,holdMs,releaseMs,thresholdDb], ...]

    function refresh() {
        root.appWin.dcall("ListRules", "", [], function(data) { root.rules = data[0] || []; });
        root.appWin.dcall("ListDuckingRules", "", [], function(data) { root.duckingRules = data[0] || []; });
    }
    Component.onCompleted: root.refresh()

    function groupName(id) {
        const g = root.appWin.groups.find(x => x[0] === id);
        return g ? g[1] : String(id);
    }

    RowLayout {
        Layout.fillWidth: true
        spacing: 14

        // ---- Auto-route on app start ----
        Rectangle {
            Layout.fillWidth: true
            radius: 10
            color: root.appTheme.panel
            border.color: root.appTheme.border
            border.width: 1
            implicitHeight: arCol.implicitHeight + 32
            ColumnLayout {
                id: arCol
                anchors.fill: parent
                anchors.margins: 16
                spacing: 8
                Text { text: "AUTO-ROUTE ON APP START"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
                Flow {
                    Layout.fillWidth: true
                    spacing: 6
                    Rectangle {
                        width: 110; height: 28; radius: 5; color: root.appTheme.panel2; border.color: root.appTheme.border; border.width: 1
                        TextInput { id: arName; anchors.fill: parent; anchors.margins: 6; color: root.appTheme.text; font.pixelSize: 11; verticalAlignment: TextInput.AlignVCenter; clip: true }
                        Text { anchors.left: parent.left; anchors.leftMargin: 6; anchors.verticalCenter: parent.verticalCenter; text: "Rule name"; color: root.appTheme.textFaint; font.pixelSize: 11; visible: arName.text.length === 0 }
                    }
                    Rectangle {
                        width: 160; height: 28; radius: 5; color: root.appTheme.panel2; border.color: root.appTheme.border; border.width: 1
                        TextInput { id: arAppId; anchors.fill: parent; anchors.margins: 6; color: root.appTheme.text; font.pixelSize: 11; verticalAlignment: TextInput.AlignVCenter; clip: true }
                        Text { anchors.left: parent.left; anchors.leftMargin: 6; anchors.verticalCenter: parent.verticalCenter; text: "App id (blank = any)"; color: root.appTheme.textFaint; font.pixelSize: 11; visible: arAppId.text.length === 0 }
                    }
                    GroupCombo { id: arGroup; groups: root.appWin.groups }
                    Rectangle {
                        implicitWidth: 54; height: 28; radius: 5; color: root.appTheme.accent
                        Text { anchors.centerIn: parent; text: "+ Add"; font.pixelSize: 11; font.bold: true; color: "#06272c" }
                        MouseArea {
                            anchors.fill: parent
                            onClicked: {
                                const gid = arGroup.currentGroupId();
                                if (gid === null) return;
                                const name = arName.text.trim() || "Auto-route";
                                root.appWin.dcall("CreateAutoRouteRule", "sssu", [name, "route_app_to_group", arAppId.text.trim(), gid], function() { root.refresh(); });
                                arName.text = ""; arAppId.text = "";
                            }
                        }
                    }
                }
                Text {
                    text: "App id must match exactly what the daemon reports (e.g. appid:org.mozilla.firefox) — check Doctor's Signal Flow or daemon logs."
                    color: root.appTheme.textFaint; font.pixelSize: 11; wrapMode: Text.WordWrap; Layout.fillWidth: true
                }
            }
        }

        // ---- Route when a device connects ----
        Rectangle {
            Layout.fillWidth: true
            radius: 10
            color: root.appTheme.panel
            border.color: root.appTheme.border
            border.width: 1
            implicitHeight: dcCol.implicitHeight + 32
            ColumnLayout {
                id: dcCol
                anchors.fill: parent
                anchors.margins: 16
                spacing: 8
                Text { text: "ROUTE WHEN A DEVICE CONNECTS"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
                Flow {
                    Layout.fillWidth: true
                    spacing: 6
                    Rectangle {
                        width: 110; height: 28; radius: 5; color: root.appTheme.panel2; border.color: root.appTheme.border; border.width: 1
                        TextInput { id: dcName; anchors.fill: parent; anchors.margins: 6; color: root.appTheme.text; font.pixelSize: 11; verticalAlignment: TextInput.AlignVCenter; clip: true }
                        Text { anchors.left: parent.left; anchors.leftMargin: 6; anchors.verticalCenter: parent.verticalCenter; text: "Rule name"; color: root.appTheme.textFaint; font.pixelSize: 11; visible: dcName.text.length === 0 }
                    }
                    OutputCombo { id: dcOutput; outputs: root.appWin.outputs }
                    GroupCombo { id: dcGroup; groups: root.appWin.groups }
                    Rectangle {
                        implicitWidth: 54; height: 28; radius: 5; color: root.appTheme.accent
                        Text { anchors.centerIn: parent; text: "+ Add"; font.pixelSize: 11; font.bold: true; color: "#06272c" }
                        MouseArea {
                            anchors.fill: parent
                            onClicked: {
                                const oid = dcOutput.currentOutputId();
                                const gid = dcGroup.currentGroupId();
                                if (oid === null || gid === null) return;
                                const name = dcName.text.trim() || "Device connected";
                                root.appWin.dcall("CreateDeviceConnectedRule", "ssud", [name, oid, gid, "1.000"], function() { root.refresh(); });
                                dcName.text = "";
                            }
                        }
                    }
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
        implicitHeight: rulesCol.implicitHeight + 32
        ColumnLayout {
            id: rulesCol
            anchors.fill: parent
            anchors.margins: 16
            spacing: 6
            Text { text: "RULES"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
            Repeater {
                model: root.rules
                delegate: Rectangle {
                    id: ruleRow
                    required property var modelData
                    Layout.fillWidth: true
                    implicitHeight: 28
                    radius: 6
                    color: root.appTheme.panel2
                    RowLayout {
                        anchors.fill: parent
                        anchors.margins: 8
                        spacing: 8
                        Rectangle {
                            width: 30; height: 16; radius: 8
                            color: ruleRow.modelData[2] ? root.appTheme.accentDim : root.appTheme.inset
                            border.color: ruleRow.modelData[2] ? root.appTheme.accent : root.appTheme.border; border.width: 1
                            Rectangle {
                                width: 12; height: 12; radius: 6
                                color: ruleRow.modelData[2] ? root.appTheme.accent : root.appTheme.textFaint
                                x: ruleRow.modelData[2] ? parent.width - width - 2 : 2
                                anchors.verticalCenter: parent.verticalCenter
                            }
                            MouseArea {
                                anchors.fill: parent
                                onClicked: root.appWin.dcall("SetRuleEnabled", "ub", [ruleRow.modelData[0], ruleRow.modelData[2] ? "false" : "true"], function() { root.refresh(); })
                            }
                        }
                        Text { text: ruleRow.modelData[1]; color: root.appTheme.text; font.pixelSize: 12; Layout.preferredWidth: 100; elide: Text.ElideRight }
                        Text {
                            text: ruleRow.modelData[5] + " → " + ruleRow.modelData[6]
                            color: root.appTheme.textDim; font.pixelSize: 11; font.family: "monospace"; Layout.fillWidth: true; elide: Text.ElideRight
                        }
                        Rectangle {
                            width: 20; height: 20; radius: 5; color: "transparent"
                            Text { anchors.centerIn: parent; text: "✕"; color: root.appTheme.textDim; font.pixelSize: 12 }
                            MouseArea { anchors.fill: parent; onClicked: root.appWin.dcall("DeleteRule", "u", [ruleRow.modelData[0]], function() { root.refresh(); }) }
                        }
                    }
                }
            }
            Text { visible: root.rules.length === 0; text: "No rules yet."; color: root.appTheme.textFaint; font.pixelSize: 11 }
        }
    }

    // ---- Ducking ----
    Rectangle {
        Layout.fillWidth: true
        radius: 10
        color: root.appTheme.panel
        border.color: root.appTheme.border
        border.width: 1
        implicitHeight: duckCol.implicitHeight + 32
        ColumnLayout {
            id: duckCol
            anchors.fill: parent
            anchors.margins: 16
            spacing: 6
            Text { text: "DUCKING"; color: root.appTheme.textDim; font.pixelSize: 11; font.bold: true }
            RowLayout {
                spacing: 6
                GroupCombo { id: duckTrigger; groups: root.appWin.groups }
                GroupCombo { id: duckTarget; groups: root.appWin.groups }
                Rectangle {
                    width: 56; height: 26; radius: 4; color: root.appTheme.inset; border.color: root.appTheme.border; border.width: 1
                    TextInput {
                        id: duckReduction
                        anchors.fill: parent; anchors.margins: 4
                        text: "12"; color: root.appTheme.text; font.pixelSize: 11
                        horizontalAlignment: TextInput.AlignHCenter; verticalAlignment: TextInput.AlignVCenter
                        validator: IntValidator { bottom: 0; top: 60 }
                    }
                }
                Text { text: "dB"; color: root.appTheme.textDim; font.pixelSize: 11 }
                Rectangle {
                    implicitWidth: 54; height: 26; radius: 5; color: root.appTheme.accent
                    Text { anchors.centerIn: parent; text: "+ Add"; font.pixelSize: 11; font.bold: true; color: "#06272c" }
                    MouseArea {
                        anchors.fill: parent
                        onClicked: {
                            const trig = duckTrigger.currentGroupId();
                            const targ = duckTarget.currentGroupId();
                            if (trig === null || targ === null) return;
                            const reduction = parseFloat(duckReduction.text) || 12;
                            root.appWin.dcall("CreateDuckingRule", "uauddddd",
                                [trig, [targ], reduction.toFixed(1), "50.0", "200.0", "500.0", "-40.0"],
                                function() { root.refresh(); });
                        }
                    }
                }
            }
            Repeater {
                model: root.duckingRules
                delegate: Rectangle {
                    id: duckRow
                    required property var modelData
                    Layout.fillWidth: true
                    implicitHeight: 28
                    radius: 6
                    color: root.appTheme.panel2
                    RowLayout {
                        anchors.fill: parent
                        anchors.margins: 8
                        spacing: 8
                        Rectangle {
                            width: 30; height: 16; radius: 8
                            color: duckRow.modelData[1] ? root.appTheme.accentDim : root.appTheme.inset
                            border.color: duckRow.modelData[1] ? root.appTheme.accent : root.appTheme.border; border.width: 1
                            Rectangle {
                                width: 12; height: 12; radius: 6
                                color: duckRow.modelData[1] ? root.appTheme.accent : root.appTheme.textFaint
                                x: duckRow.modelData[1] ? parent.width - width - 2 : 2
                                anchors.verticalCenter: parent.verticalCenter
                            }
                            MouseArea {
                                anchors.fill: parent
                                onClicked: root.appWin.dcall("SetDuckingRuleEnabled", "ub", [duckRow.modelData[0], duckRow.modelData[1] ? "false" : "true"], function() { root.refresh(); })
                            }
                        }
                        Text {
                            text: root.groupName(duckRow.modelData[2]) + " ducks " + duckRow.modelData[3].map(t => root.groupName(t)).join(", ")
                            color: root.appTheme.text; font.pixelSize: 12; Layout.fillWidth: true; elide: Text.ElideRight
                        }
                        Rectangle {
                            width: 20; height: 20; radius: 5; color: "transparent"
                            Text { anchors.centerIn: parent; text: "✕"; color: root.appTheme.textDim; font.pixelSize: 12 }
                            MouseArea { anchors.fill: parent; onClicked: root.appWin.dcall("DeleteDuckingRule", "u", [duckRow.modelData[0]], function() { root.refresh(); }) }
                        }
                    }
                }
            }
            Text { visible: root.duckingRules.length === 0; text: "No ducking rules yet."; color: root.appTheme.textFaint; font.pixelSize: 11 }
            Text {
                text: "When the trigger group's level rises above -40dB, every target group ducks by the configured amount, then releases smoothly."
                color: root.appTheme.textFaint; font.pixelSize: 11; wrapMode: Text.WordWrap; Layout.fillWidth: true
            }
        }
    }
}
