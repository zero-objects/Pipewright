import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Pipewright

// Capability profile of the loaded pipeline, derived from the Hub-IR: which
// feature constructs (services, caching, matrix, secrets, conditions, …) it
// uses and how many of each. `overall` is a source-side portability hint
// ("Possible" = only universal constructs; "PossibleWithCaveats" = uses
// features that aren't universal across platforms). A target-specific verdict
// lives in the Migrate tab.
Item {
    id: root

    property string sourceText: ""
    property string sourceKind: "gitlab"
    property string sourcePath: ""
    property string selectedJob: ""

    property string overall: "—"
    property string summary: ""
    ListModel { id: featuresModel }

    function refresh() {
        featuresModel.clear()
        overall = "—"; summary = ""
        if (!sourceText) return
        try {
            const obj = JSON.parse(BridgeApi.capabilities(sourceText, sourceKind, sourcePath))
            if (obj.error) { overall = "(error)"; summary = obj.error; return }
            overall = obj.overall || "—"
            summary = obj.summary || ""
            for (const f of (obj.features || [])) featuresModel.append({ feat: f })
        } catch (e) {
            overall = "(parse error)"; summary = "" + e
        }
    }

    onSourceTextChanged: refresh()
    onSourceKindChanged: refresh()

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 8
        spacing: 8

        RowLayout {
            Layout.fillWidth: true
            Label { text: qsTr("Portability:"); font.bold: true }
            Label {
                text: root.overall
                color: switch (root.overall) {
                    case "Possible":            "#2e7d32"; break
                    case "PossibleWithCaveats": "#ed6c02"; break
                    case "NotPossible":         "#c62828"; break
                    default:                    "#666"
                }
                font.bold: true
            }
            Item { Layout.fillWidth: true }
            Button { text: qsTr("Re-analyze"); onClicked: root.refresh() }
        }

        Label {
            text: root.summary
            color: "#555"
            visible: root.summary.length > 0
            wrapMode: Label.WordWrap
            Layout.fillWidth: true
        }

        Label {
            text: qsTr("Capabilities in use")
            font.bold: true
            visible: featuresModel.count > 0
        }

        ListView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: featuresModel
            spacing: 2
            delegate: Rectangle {
                width: ListView.view.width
                height: frow.implicitHeight + 10
                color: index % 2 === 0 ? "#fafafa" : "#ffffff"
                RowLayout {
                    id: frow
                    anchors.fill: parent
                    anchors.margins: 5
                    spacing: 8
                    Label {
                        text: feat.label
                        font.pixelSize: 13
                        Layout.fillWidth: true
                    }
                    Label {
                        text: qsTr("×%1").arg(feat.count)
                        color: "#6b7280"
                        font.pixelSize: 11
                    }
                    Rectangle {
                        radius: 3
                        color: feat.universal ? "#dbeafe" : "#fef3c7"
                        implicitWidth: tag.implicitWidth + 10
                        implicitHeight: tag.implicitHeight + 4
                        Label {
                            id: tag
                            anchors.centerIn: parent
                            text: feat.universal ? qsTr("universal") : qsTr("caveat")
                            font.pixelSize: 9
                            color: feat.universal ? "#1e40af" : "#92400e"
                        }
                    }
                }
            }

            Label {
                anchors.centerIn: parent
                visible: featuresModel.count === 0 && root.overall === "—"
                text: qsTr("Open a pipeline to profile its capabilities.")
                color: "#999"
            }
        }
    }
}
