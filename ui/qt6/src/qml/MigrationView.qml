import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import Pipewright

Item {
    id: root

    property string sourceText: ""
    property string sourceKind: "gitlab"
    property string sourcePath: ""
    property string target: "github"      // any engine platform key

    property string targetYaml: ""
    property var frictions: []             // [{ severity, message, where }]

    function refresh() {
        targetYaml = ""
        frictions = []
        if (!sourceText) return
        const raw = BridgeApi.migrate(sourceText, sourceKind, target, sourcePath)
        try {
            const obj = JSON.parse(raw)
            if (obj.error) {
                targetYaml = "# error: " + obj.error
                return
            }
            targetYaml = obj.yaml || ""
            frictions = (obj.report && obj.report.items) || []
        } catch (e) {
            targetYaml = "# parse error: " + e
        }
    }

    onSourceTextChanged: refresh()
    onSourceKindChanged: refresh()
    onTargetChanged: refresh()

    FileDialog {
        id: saveDialog
        fileMode: FileDialog.SaveFile
        title: qsTr("Save migrated pipeline")
        // Suggest the target platform's conventional file name (engine-sourced).
        currentFile: "file:///" + BridgeApi.defaultFileName(root.target)
        onAccepted: {
            const xhr = new XMLHttpRequest()
            xhr.open("PUT", selectedFile)
            xhr.setRequestHeader("Content-Type", "text/plain")
            xhr.send(targetYaml)
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 8
        spacing: 8

        // toolbar
        RowLayout {
            Layout.fillWidth: true
            spacing: 8
            Label { text: qsTr("Migrate to:"); font.bold: true }
            ComboBox {
                model: BridgeApi.platforms()
                currentIndex: model.indexOf(root.target)
                Layout.preferredWidth: 140
                onActivated: root.target = model[currentIndex]
            }
            Item { Layout.fillWidth: true }
            Label {
                text: {
                    const n = frictions.length
                    if (n === 0) return qsTr("No frictions")
                    return qsTr("%1 friction(s)").arg(n)
                }
                color: frictions.length > 0 ? "#ed6c02" : "#2e7d32"
            }
            Button { text: qsTr("Re-migrate"); onClicked: root.refresh() }
            Button {
                text: qsTr("Save target as…")
                enabled: targetYaml.length > 0
                onClicked: saveDialog.open()
            }
        }

        // three panes: source | target | friction. Each pane gets an
        // explicit preferred width so the SplitView's auto-distribution
        // doesn't collapse the source pane to zero (which caused the
        // header labels to overlap on first paint).
        SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            orientation: Qt.Horizontal

            Frame {
                SplitView.preferredWidth: 420
                SplitView.minimumWidth: 200
                ColumnLayout {
                    anchors.fill: parent
                    Label { text: qsTr("Source (%1)").arg(root.sourceKind); font.bold: true }
                    ScrollView {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        TextArea {
                            id: sourcePane
                            text: root.sourceText
                            readOnly: true
                            font.family: "Menlo"
                            font.pixelSize: 11
                            wrapMode: TextEdit.NoWrap
                            Component.onCompleted: BridgeApi.installHighlighter(textDocument, root.sourceKind)
                            Connections {
                                target: root
                                function onSourceKindChanged() {
                                    BridgeApi.installHighlighter(sourcePane.textDocument, root.sourceKind)
                                }
                            }
                        }
                    }
                }
            }

            Frame {
                SplitView.fillWidth: true
                SplitView.minimumWidth: 240
                ColumnLayout {
                    anchors.fill: parent
                    Label { text: qsTr("Target (%1)").arg(root.target); font.bold: true }
                    ScrollView {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        TextArea {
                            id: targetPane
                            text: root.targetYaml
                            readOnly: true
                            font.family: "Menlo"
                            font.pixelSize: 11
                            wrapMode: TextEdit.NoWrap
                            Component.onCompleted: BridgeApi.installHighlighter(textDocument, root.target)
                            Connections {
                                target: root
                                function onTargetChanged() {
                                    BridgeApi.installHighlighter(targetPane.textDocument, root.target)
                                }
                            }
                        }
                    }
                }
            }

            Frame {
                SplitView.preferredWidth: 320
                SplitView.minimumWidth: 200
                ColumnLayout {
                    anchors.fill: parent
                    Label { text: qsTr("Friction report"); font.bold: true }
                    ListView {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        model: frictions
                        spacing: 4
                        delegate: Rectangle {
                            width: ListView.view.width
                            height: layout.implicitHeight + 12
                            color: {
                                switch (modelData.severity) {
                                    case "info":         return "#e3f2fd"
                                    case "approximated": return "#fff3e0"
                                    case "manual":       return "#ffebee"
                                    default:             return "#f5f5f5"
                                }
                            }
                            radius: 3

                            RowLayout {
                                id: layout
                                anchors.fill: parent
                                anchors.margins: 6
                                spacing: 6
                                Label {
                                    // pipeline-migrate serialises Severity in
                                    // lowercase (serde rename_all = "lowercase");
                                    // match against that, not the Rust enum
                                    // variant case.
                                    text: {
                                        switch (modelData.severity) {
                                            case "info":         return "ℹ"
                                            case "approximated": return "≈"
                                            case "manual":       return "✋"
                                            default:             return "?"
                                        }
                                    }
                                    font.pixelSize: 16
                                    Layout.alignment: Qt.AlignTop
                                }
                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 2
                                    Label {
                                        // Show both severity and the feature
                                        // tag (e.g. "approximated · rules:if")
                                        // so the user knows what category of
                                        // construct flagged.
                                        text: modelData.feature
                                            ? modelData.severity + " · " + modelData.feature
                                            : modelData.severity
                                        font.bold: true
                                        font.pixelSize: 10
                                        color: "#444"
                                    }
                                    Label {
                                        // Hub-IR field is `note`, not `message`
                                        // — that's why the body was blank.
                                        text: modelData.note || ""
                                        wrapMode: Label.WordWrap
                                        Layout.fillWidth: true
                                        font.pixelSize: 11
                                    }
                                    Label {
                                        visible: !!modelData.job
                                        text: modelData.job
                                            ? qsTr("job: %1").arg(modelData.job)
                                            : ""
                                        font.pixelSize: 9
                                        color: "#666"
                                        font.family: "Menlo"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
