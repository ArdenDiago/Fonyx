// Outputs tab — ported from ui/index.html's #outputs section: one card
// per PipeWire sink, with per-group route gain + sync (off/automatic/
// manual delay compensation). Kind icons skipped (plain text instead) —
// this machine has no emoji font (same finding MtxPanelWidget.qml's own
// comment already made; the Tauri screenshot shows the same missing-glyph
// box for these).
import QtQuick
import QtQuick.Layouts
import "../"
import "../DbMath.js" as DbMath

ColumnLayout {
    id: root
    required property var appWin
    required property var appTheme
    Layout.fillWidth: true
    spacing: 10

    property var routesByGroup: ({}) // groupId -> [[outputId,gain,enabled,mode,syncMode,manualDelayMs,syncIncluded], ...]
    property real maxCompensationMs: 150
    property bool maxCompLoaded: false

    function refresh() {
        for (const g of root.appWin.groups) {
            const gid = g[0];
            root.appWin.dcall("ListGroupOutputRoutes", "u", [gid], function(data) {
                const copy = Object.assign({}, root.routesByGroup);
                copy[gid] = data[0] || [];
                root.routesByGroup = copy;
            });
        }
        if (!root.maxCompLoaded) {
            root.maxCompLoaded = true;
            root.appWin.dcall("GetMaxCompensationMs", "", [], function(data) { root.maxCompensationMs = data[0]; });
        }
    }
    Component.onCompleted: root.refresh()

    function routeFor(outputId, groupId) {
        const routes = root.routesByGroup[groupId] || [];
        return routes.find(r => r[0] === outputId) || null;
    }
    function groupsRoutedTo(outputId) {
        return root.appWin.groups.filter(g => root.routeFor(outputId, g[0]) !== null);
    }
    function groupsNotRoutedTo(outputId) {
        return root.appWin.groups.filter(g => root.routeFor(outputId, g[0]) === null);
    }

    RowLayout {
        Layout.alignment: Qt.AlignRight
        spacing: 6
        Text { text: "Maximum compensation"; color: root.appTheme.textDim; font.pixelSize: 12 }
        Rectangle {
            implicitWidth: 60; implicitHeight: 24; radius: 4
            color: root.appTheme.panel2; border.color: root.appTheme.border; border.width: 1
            TextInput {
                anchors.fill: parent
                anchors.margins: 4
                text: root.maxCompensationMs.toFixed(0)
                color: root.appTheme.text
                font.pixelSize: 12
                horizontalAlignment: TextInput.AlignHCenter
                verticalAlignment: TextInput.AlignVCenter
                validator: IntValidator { bottom: 0; top: 500 }
                onEditingFinished: {
                    root.maxCompensationMs = parseFloat(text) || 0;
                    root.appWin.dcall("SetMaxCompensationMs", "d", [root.maxCompensationMs.toFixed(1)]);
                }
            }
        }
        Text { text: "ms"; color: root.appTheme.textDim; font.pixelSize: 12 }
    }

    Flow {
        Layout.fillWidth: true
        spacing: 14

        Repeater {
            model: root.appWin.outputs
            delegate: Rectangle {
                id: card
                required property var modelData
                readonly property string outId: modelData[0]
                width: 300
                implicitHeight: cardCol.implicitHeight + 32
                radius: 10
                color: root.appTheme.panel
                border.color: root.appTheme.border
                border.width: 1

                ColumnLayout {
                    id: cardCol
                    anchors.fill: parent
                    anchors.margins: 16
                    spacing: 6

                    RowLayout {
                        Layout.fillWidth: true
                        Text { text: card.modelData[1]; color: root.appTheme.text; font.pixelSize: 13; font.bold: true; Layout.fillWidth: true; elide: Text.ElideRight }
                        Text {
                            text: card.modelData[4] ? (card.modelData[5] ? "Connected · Default" : "Connected") : "Unavailable"
                            color: card.modelData[4] ? root.appTheme.textDim : root.appTheme.danger
                            font.pixelSize: 11
                        }
                    }
                    Text {
                        text: card.modelData[2] + (card.modelData[4] ? ` · ${card.modelData[6]}Hz · ${card.modelData[7]}ch` : "")
                        color: root.appTheme.textFaint; font.pixelSize: 11; wrapMode: Text.WordWrap; Layout.fillWidth: true
                    }

                    Repeater {
                        model: root.groupsRoutedTo(card.outId)
                        delegate: ColumnLayout {
                            id: routeRow
                            required property var modelData
                            readonly property var route: root.routeFor(card.outId, modelData[0])
                            Layout.fillWidth: true
                            Layout.topMargin: 8
                            spacing: 4

                            Rectangle { Layout.fillWidth: true; height: 1; color: root.appTheme.border }

                            RowLayout {
                                Layout.fillWidth: true
                                Text { text: routeRow.modelData[1]; color: root.appTheme.text; font.pixelSize: 12; Layout.fillWidth: true }
                                Rectangle {
                                    width: 30; height: 16; radius: 8
                                    color: routeRow.route[2] ? root.appTheme.accentDim : root.appTheme.inset
                                    border.color: routeRow.route[2] ? root.appTheme.accent : root.appTheme.border; border.width: 1
                                    Rectangle {
                                        width: 12; height: 12; radius: 6
                                        color: routeRow.route[2] ? root.appTheme.accent : root.appTheme.textFaint
                                        x: routeRow.route[2] ? parent.width - width - 2 : 2
                                        anchors.verticalCenter: parent.verticalCenter
                                    }
                                    MouseArea {
                                        anchors.fill: parent
                                        onClicked: root.appWin.dcall("SetGroupOutputRoute", "usdb",
                                            [routeRow.modelData[0], card.outId, routeRow.route[1].toFixed(3), routeRow.route[2] ? "false" : "true"],
                                            function() { root.refresh(); })
                                    }
                                }
                                Rectangle {
                                    width: 18; height: 18; radius: 4; color: "transparent"
                                    Text { anchors.centerIn: parent; text: "✕"; color: root.appTheme.textDim; font.pixelSize: 11 }
                                    MouseArea {
                                        anchors.fill: parent
                                        onClicked: root.appWin.dcall("RemoveGroupOutputRoute", "us", [routeRow.modelData[0], card.outId], function() { root.refresh(); })
                                    }
                                }
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: 6
                                Slider2 {
                                    Layout.fillWidth: true
                                    from: DbMath.METER_DB_MIN; to: DbMath.METER_DB_MAX
                                    value: DbMath.gainToDb(routeRow.route[1])
                                    trackColor: root.appTheme.inset
                                    fillColor: root.appTheme.accent
                                    handleColor: root.appTheme.text
                                    onMoved: {
                                        const gain = DbMath.dbToGain(value);
                                        root.appWin.dcall("SetGroupOutputRoute", "usdb", [routeRow.modelData[0], card.outId, gain.toFixed(3), "true"]);
                                    }
                                }
                                Text {
                                    text: DbMath.gainToDbText(routeRow.route[1])
                                    color: root.appTheme.textDim; font.pixelSize: 11; font.family: "monospace"
                                    Layout.preferredWidth: 60; horizontalAlignment: Text.AlignRight
                                }
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: 6
                                Text { text: "Sync"; color: root.appTheme.textDim; font.pixelSize: 11 }
                                Repeater {
                                    model: ["off", "automatic", "manual"]
                                    delegate: Rectangle {
                                        required property string modelData
                                        implicitWidth: syncLbl.implicitWidth + 12; implicitHeight: 20; radius: 4
                                        color: routeRow.route[4] === modelData ? root.appTheme.accent : root.appTheme.inset
                                        border.color: root.appTheme.border; border.width: 1
                                        Text { id: syncLbl; anchors.centerIn: parent; text: parent.modelData; font.pixelSize: 9; color: routeRow.route[4] === parent.modelData ? "#06272c" : root.appTheme.textDim }
                                        MouseArea {
                                            anchors.fill: parent
                                            onClicked: root.appWin.dcall("SetOutputSynchronization", "ussd",
                                                [routeRow.modelData[0], card.outId, parent.modelData, routeRow.route[5]],
                                                function() { root.refresh(); })
                                        }
                                    }
                                }
                                Text {
                                    visible: routeRow.route[4] === "manual"
                                    text: routeRow.route[5].toFixed(0) + " ms"
                                    color: root.appTheme.textDim; font.pixelSize: 11; font.family: "monospace"
                                }
                            }
                        }
                    }

                    ColumnLayout {
                        visible: root.groupsNotRoutedTo(card.outId).length > 0
                        Layout.fillWidth: true
                        Layout.topMargin: 8
                        spacing: 4
                        Rectangle { Layout.fillWidth: true; height: 1; color: root.appTheme.border }
                        RowLayout {
                            Layout.fillWidth: true
                            GroupCombo {
                                id: addCombo
                                groups: root.groupsNotRoutedTo(card.outId)
                                Layout.fillWidth: true
                            }
                            Rectangle {
                                implicitWidth: 60; implicitHeight: 24; radius: 5
                                color: root.appTheme.panel2; border.color: root.appTheme.border; border.width: 1
                                Text { anchors.centerIn: parent; text: "+ Route"; font.pixelSize: 10; color: root.appTheme.textDim }
                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: {
                                        const gid = addCombo.currentGroupId();
                                        if (gid === null) return;
                                        root.appWin.dcall("SetGroupOutputRoute", "usdb", [gid, card.outId, "1.000", "true"], function() { root.refresh(); });
                                    }
                                }
                            }
                        }
                    }
                    Text {
                        visible: root.groupsRoutedTo(card.outId).length === 0
                        text: "No groups explicitly routed here yet."
                        color: root.appTheme.textFaint; font.pixelSize: 11
                    }
                }
            }
        }

        Text {
            visible: root.appWin.outputs.length === 0
            text: "No PipeWire outputs discovered yet."
            color: root.appTheme.textFaint
        }
    }

    Text {
        text: "Compensation only ever adds delay to a faster output so it arrives closer to the slowest one in the group — it never speeds a slow device up."
        color: root.appTheme.textFaint; font.pixelSize: 11; wrapMode: Text.WordWrap; Layout.fillWidth: true
    }
}
