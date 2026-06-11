import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import Pipewright

// "Human" runbook tab — renders the IR as a readable description of
// what the pipeline does and how to run it by hand. The backend
// (pipeline_render_html) returns {overview, toc, jobs, skipped};
// the TOC on the left jumps the rendered view on the right.
Item {
    id: root

    property string sourceText: ""
    property string sourceKind: "gitlab"
    property string sourcePath: ""
    property string selectedJob: ""
    property string locale: "en"          // runbook prose language

    // {title, summary, sectionIndex} — sectionIndex points into sectionsModel.
    ListModel { id: tocModel }
    // {heading, html} — section 0 is the overview.
    ListModel { id: sectionsModel }

    property string errorText: ""
    property var pendingExport: null   // {content, ext} awaiting a save path

    // Export the runbook as md / html / doc(rtf): get the content from the
    // backend, then prompt for a save path and write it.
    function exportAs(format) {
        if (!sourceText) return
        try {
            const r = JSON.parse(BridgeApi.exportRunbook(sourceText, sourceKind, format, root.locale))
            if (r.error) { root.errorText = r.error; return }
            root.pendingExport = r
            saveDialog.currentFile = "file:runbook." + r.ext
            saveDialog.open()
        } catch (e) { root.errorText = "" + e }
    }

    FileDialog {
        id: saveDialog
        fileMode: FileDialog.SaveFile
        title: qsTr("Export runbook")
        onAccepted: {
            if (!root.pendingExport) return
            const path = selectedFile.toString().replace(/^file:\/\//, "")
            const res = BridgeApi.writeFile(path, root.pendingExport.content)
            if (!res.ok) root.errorText = res.error || qsTr("write failed")
            root.pendingExport = null
        }
    }

    function refresh() {
        tocModel.clear()
        sectionsModel.clear()
        errorText = ""
        if (!sourceText) return
        const raw = BridgeApi.renderHuman(sourceText, sourceKind, "push", "main", sourcePath, root.locale)
        try {
            const rb = JSON.parse(raw)
            if (rb.error) {
                errorText = rb.error
                return
            }
            sectionsModel.append({ heading: qsTr("Overview"), html: rb.overview || "" })
            const jobs = rb.jobs || []
            for (let i = 0; i < jobs.length; ++i) {
                sectionsModel.append({ heading: jobs[i].title, html: jobs[i].html })
            }
            if (rb.skipped && rb.skipped.length > 0) {
                sectionsModel.append({ heading: qsTr("Skipped jobs"), html: rb.skipped })
            }
            const toc = rb.toc || []
            for (let i = 0; i < toc.length; ++i) {
                // section 0 is the overview, so job i lands at i + 1.
                tocModel.append({
                    title: toc[i].title,
                    summary: toc[i].summary || "",
                    sectionIndex: i + 1
                })
            }
        } catch (e) {
            errorText = qsTr("Could not render runbook: %1").arg(e)
        }
    }

    function jumpToSection(index) {
        contentList.positionViewAtIndex(index, ListView.Beginning)
    }

    // When a job is focused elsewhere, scroll the runbook to it.
    onSelectedJobChanged: {
        if (!selectedJob) return
        for (let i = 0; i < tocModel.count; ++i) {
            if (tocModel.get(i).title === selectedJob) {
                jumpToSection(tocModel.get(i).sectionIndex)
                break
            }
        }
    }

    onSourceTextChanged: refresh()
    onSourceKindChanged: refresh()
    onLocaleChanged: refresh()

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // error banner
        Rectangle {
            Layout.fillWidth: true
            visible: root.errorText.length > 0
            color: "#fdecea"
            height: visible ? errLabel.implicitHeight + 12 : 0
            Label {
                id: errLabel
                anchors.fill: parent
                anchors.margins: 6
                text: root.errorText
                color: "#c62828"
                wrapMode: Label.WordWrap
            }
        }

        SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            orientation: Qt.Horizontal

            // ----- left: table of contents -----
            Rectangle {
                SplitView.preferredWidth: 240
                SplitView.minimumWidth: 160
                color: "#fafafa"
                ColumnLayout {
                    anchors.fill: parent
                    spacing: 0
                    Label {
                        text: qsTr("Contents")
                        font.bold: true
                        padding: 8
                    }
                    ListView {
                        id: tocList
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        model: tocModel
                        delegate: ItemDelegate {
                            width: tocList.width
                            onClicked: root.jumpToSection(sectionIndex)
                            contentItem: ColumnLayout {
                                spacing: 1
                                Label {
                                    text: title
                                    font.pixelSize: 12
                                    elide: Label.ElideRight
                                    Layout.fillWidth: true
                                }
                                Label {
                                    text: summary
                                    font.pixelSize: 10
                                    color: "#888"
                                }
                            }
                        }
                    }
                }
            }

            // ----- right: rendered runbook -----
            Rectangle {
                SplitView.fillWidth: true
                color: "#ffffff"
                ColumnLayout {
                    anchors.fill: parent
                    spacing: 0
                    RowLayout {
                        Layout.fillWidth: true
                        Label {
                            text: qsTr("Runbook — what this pipeline does")
                            font.bold: true
                            padding: 8
                            Layout.fillWidth: true
                        }
                        Label { text: qsTr("Export:"); color: "#6b7280"; font.pixelSize: 11 }
                        Button { text: qsTr("Markdown"); enabled: !!root.sourceText; onClicked: root.exportAs("md") }
                        Button { text: qsTr("HTML"); enabled: !!root.sourceText; onClicked: root.exportAs("html") }
                        Button { text: qsTr("Word"); enabled: !!root.sourceText; onClicked: root.exportAs("doc") }
                        ToolSeparator {}
                        Button {
                            text: qsTr("Refresh")
                            onClicked: root.refresh()
                        }
                    }
                    ListView {
                        id: contentList
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        model: sectionsModel
                        spacing: 8
                        delegate: Item {
                            width: contentList.width
                            implicitHeight: sectionCol.implicitHeight + 16
                            ColumnLayout {
                                id: sectionCol
                                x: 12
                                width: parent.width - 24
                                spacing: 4
                                Label {
                                    text: heading
                                    font.bold: true
                                    font.pixelSize: 16
                                    color: "#111827"
                                    Layout.fillWidth: true
                                    wrapMode: Label.WordWrap
                                }
                                Text {
                                    text: html
                                    textFormat: Text.RichText
                                    wrapMode: Text.WordWrap
                                    Layout.fillWidth: true
                                    font.pixelSize: 13
                                }
                            }
                        }
                        // empty-state hint
                        Label {
                            anchors.centerIn: parent
                            visible: sectionsModel.count === 0 && root.errorText.length === 0
                            text: qsTr("Open a pipeline to see its runbook.")
                            color: "#999"
                        }
                    }
                }
            }
        }
    }
}
