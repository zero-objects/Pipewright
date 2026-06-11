import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import Pipewright

// Registry-backed recipe browser. Lists the embedded standard library plus any
// user sources (local dirs / git repos) declared in the configured sources
// file, with search and sort. Shows each recipe's curated description/doc and a
// generated, localized *structural* description (via the prose doc mechanism).
// Recipes can be composed into a standalone pipeline (fold-apply, namespaced so
// they never collide) or applied to the currently-open pipeline.
Item {
    id: root

    // Current pipeline (bound from Main) — the "apply to current" target.
    property string sourceText: ""
    property string sourceKind: "gitlab"
    property string sourcePath: ""
    // Recipe source configuration (from Settings).
    property string recipesConfig: ""
    property string cacheDir: ""

    property string query: ""
    property string sortKey: "name"
    property string target: "gitlab"
    property string locale: "en"          // language of the generated description
    property var selected: null           // the focused recipe object
    property string generatedDoc: ""      // prose structural description (markdown)
    property string composedYaml: ""
    property string warningsText: ""

    // Emitted when "Apply to current pipeline" produced new source.
    signal applyToPipeline(string newYaml)

    ListModel { id: available }           // browse results
    ListModel { id: composition }         // ordered ids for the standalone compose

    Component.onCompleted: reload()
    onRecipesConfigChanged: reload()

    function reload() {
        available.clear()
        warningsText = ""
        const raw = BridgeApi.listRecipes(query, sortKey, recipesConfig, cacheDir)
        try {
            const obj = JSON.parse(raw)
            if (obj.error) { warningsText = obj.error; return }
            for (const r of (obj.recipes || [])) available.append({ rec: r })
            if (obj.warnings && obj.warnings.length > 0)
                warningsText = obj.warnings.join("  •  ")
        } catch (e) {
            warningsText = qsTr("could not load recipes: %1").arg(e)
        }
    }

    function selectRecipe(rec) {
        selected = rec
        generatedDoc = qsTr("_generating…_")
        const raw = BridgeApi.describeRecipe(rec.id, locale, recipesConfig, cacheDir)
        try {
            const obj = JSON.parse(raw)
            generatedDoc = obj.error ? (qsTr("(no generated description: %1)").arg(obj.error)) : (obj.markdown || "")
        } catch (e) {
            generatedDoc = qsTr("(no generated description)")
        }
    }

    function addToComposition(id) {
        for (let i = 0; i < composition.count; ++i)
            if (composition.get(i).id === id) return
        composition.append({ id: id })
        composePreview()
    }
    function moveUp(idx)   { if (idx > 0) { composition.move(idx, idx - 1, 1); composePreview() } }
    function moveDown(idx) { if (idx < composition.count - 1) { composition.move(idx, idx + 1, 1); composePreview() } }
    function removeAt(idx) { composition.remove(idx); composePreview() }

    // Compose the selected recipes into a standalone pipeline by folding the
    // single-recipe apply over an empty start — each apply merges the recipe's
    // namespaced jobs, so order is preserved and ids never collide.
    function composePreview() {
        if (composition.count === 0) { composedYaml = ""; return }
        let yaml = ""
        for (let i = 0; i < composition.count; ++i) {
            const id = composition.get(i).id
            const raw = BridgeApi.applyRecipe(yaml, target, id, recipesConfig, cacheDir)
            try {
                const obj = JSON.parse(raw)
                if (obj.error) { composedYaml = "# error: " + obj.error; return }
                yaml = obj.yaml || yaml
            } catch (e) { composedYaml = "# error: " + e; return }
        }
        composedYaml = yaml
    }
    onTargetChanged: composePreview()
    // App-wide language changed → re-render the focused recipe's generated
    // description in the new locale.
    onLocaleChanged: if (selected) selectRecipe(selected)

    // Apply the focused recipe to the currently-open pipeline and hand the new
    // source up to Main (which re-renders every view).
    function applyToCurrent() {
        if (!selected) return
        const raw = BridgeApi.applyRecipe(sourceText, sourceKind, selected.id, recipesConfig, cacheDir)
        try {
            const obj = JSON.parse(raw)
            if (obj.error) { warningsText = obj.error; return }
            if (obj.yaml) root.applyToPipeline(obj.yaml)
        } catch (e) { warningsText = qsTr("apply failed: %1").arg(e) }
    }

    FileDialog {
        id: saveDialog
        fileMode: FileDialog.SaveFile
        title: qsTr("Save composed pipeline")
        // Suggest the target platform's conventional file name (engine-sourced).
        currentFile: "file:///" + BridgeApi.defaultFileName(root.target)
        onAccepted: BridgeApi.writeFile(selectedFile.toString().replace(/^file:\/\//, ""), composedYaml)
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 8
        spacing: 8

        // ----- toolbar: search / sort / language / target ----------------
        RowLayout {
            Layout.fillWidth: true
            spacing: 8
            TextField {
                id: searchField
                placeholderText: qsTr("Search recipes (id, description, tags)…")
                Layout.preferredWidth: 260
                onTextChanged: { root.query = text; root.reload() }
            }
            Label { text: qsTr("Sort:") }
            ComboBox {
                model: [qsTr("Name"), qsTr("Tag"), qsTr("Source")]
                Layout.preferredWidth: 110
                onActivated: { root.sortKey = ["name", "tag", "source"][currentIndex]; root.reload() }
            }
            Button {
                text: qsTr("↻ Reload")
                // Re-runs source discovery: git sources are re-cloned / pulled
                // into the cache, so this picks up upstream recipe changes.
                ToolTip.text: qsTr("Re-pull configured git sources and refresh the list")
                ToolTip.visible: hovered
                onClicked: root.reload()
            }
            Item { Layout.fillWidth: true }
            Label { text: qsTr("Target:") }
            ComboBox {
                model: BridgeApi.platforms()
                currentIndex: model.indexOf(root.target)
                Layout.preferredWidth: 120
                onActivated: root.target = model[currentIndex]
            }
        }

        Label {
            visible: warningsText.length > 0
            text: warningsText
            color: "#b45309"
            font.pixelSize: 10
            wrapMode: Label.WordWrap
            Layout.fillWidth: true
        }

        SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            orientation: Qt.Horizontal

            // ----- registry list -----------------------------------------
            Frame {
                SplitView.preferredWidth: 260
                ColumnLayout {
                    anchors.fill: parent
                    Label { text: qsTr("Recipes (%1)").arg(available.count); font.bold: true }
                    ListView {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        model: available
                        delegate: ItemDelegate {
                            width: ListView.view.width
                            highlighted: root.selected && root.selected.id === rec.id
                            onClicked: root.selectRecipe(rec)
                            onDoubleClicked: root.addToComposition(rec.id)
                            contentItem: ColumnLayout {
                                spacing: 2
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label { text: rec.id; font.bold: true; Layout.fillWidth: true; elide: Label.ElideRight }
                                    Rectangle {
                                        radius: 3
                                        color: rec.source === "standard" ? "#dbeafe" : "#dcfce7"
                                        implicitWidth: srcLabel.implicitWidth + 8
                                        implicitHeight: srcLabel.implicitHeight + 4
                                        Label { id: srcLabel; anchors.centerIn: parent; text: rec.source; font.pixelSize: 9; color: "#374151" }
                                    }
                                }
                                Label {
                                    text: rec.description
                                    font.pixelSize: 10
                                    color: "#666"
                                    elide: Label.ElideRight
                                    Layout.fillWidth: true
                                }
                                Label {
                                    visible: rec.tags && rec.tags.length > 0
                                    text: (rec.tags || []).join(" · ")
                                    font.pixelSize: 9
                                    color: "#9aa3af"
                                    elide: Label.ElideRight
                                    Layout.fillWidth: true
                                }
                            }
                        }
                    }
                }
            }

            // ----- detail -------------------------------------------------
            Frame {
                SplitView.fillWidth: true
                ColumnLayout {
                    anchors.fill: parent
                    spacing: 6

                    Label {
                        visible: !root.selected
                        text: qsTr("Select a recipe to see its description.\nDouble-click a recipe to add it to the composition.")
                        color: "#888"
                        Layout.fillWidth: true
                    }

                    RowLayout {
                        visible: root.selected
                        Layout.fillWidth: true
                        Label {
                            text: root.selected ? (root.selected.id + (root.selected.version ? " " + root.selected.version : "")) : ""
                            font.bold: true
                            font.pixelSize: 15
                            Layout.fillWidth: true
                        }
                        Button { text: qsTr("Add to composition"); onClicked: root.addToComposition(root.selected.id) }
                        Button {
                            text: qsTr("Apply to current pipeline")
                            enabled: root.sourceText.length > 0
                            onClicked: root.applyToCurrent()
                        }
                    }

                    ScrollView {
                        visible: root.selected
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        ColumnLayout {
                            width: parent.parent.width
                            spacing: 8

                            Label {
                                visible: root.selected && root.selected.description.length > 0
                                text: root.selected ? root.selected.description : ""
                                wrapMode: Label.WordWrap
                                Layout.fillWidth: true
                            }
                            Label {
                                visible: root.selected && root.selected.tags && root.selected.tags.length > 0
                                text: qsTr("Tags: %1").arg(root.selected ? (root.selected.tags || []).join(", ") : "")
                                font.pixelSize: 11
                                color: "#666"
                                Layout.fillWidth: true
                            }
                            Label {
                                visible: root.selected && root.selected.requirements && root.selected.requirements.length > 0
                                text: qsTr("Requires: %1").arg(root.selected ? (root.selected.requirements || []).join(", ") : "")
                                font.pixelSize: 11
                                color: "#666"
                                Layout.fillWidth: true
                            }
                            Label {
                                visible: root.selected && root.selected.jobs && root.selected.jobs.length > 0
                                text: qsTr("Jobs: %1").arg(root.selected ? (root.selected.jobs || []).join(", ") : "")
                                font.pixelSize: 11
                                color: "#666"
                                wrapMode: Label.WordWrap
                                Layout.fillWidth: true
                            }

                            // curated long-form documentation (markdown)
                            Label {
                                visible: root.selected && root.selected.doc.length > 0
                                text: qsTr("Documentation")
                                font.bold: true
                            }
                            Text {
                                visible: root.selected && root.selected.doc.length > 0
                                text: root.selected ? root.selected.doc : ""
                                textFormat: Text.MarkdownText
                                wrapMode: Text.WordWrap
                                Layout.fillWidth: true
                            }

                            // generated, localized structural description
                            Label { text: qsTr("Generated description (%1)").arg(root.locale); font.bold: true }
                            Text {
                                text: root.generatedDoc
                                textFormat: Text.MarkdownText
                                wrapMode: Text.WordWrap
                                color: "#374151"
                                Layout.fillWidth: true
                            }
                        }
                    }
                }
            }

            // ----- composition + preview ----------------------------------
            Frame {
                SplitView.preferredWidth: 300
                ColumnLayout {
                    anchors.fill: parent
                    Label { text: qsTr("Composition (%1)").arg(composition.count); font.bold: true }
                    ListView {
                        Layout.fillWidth: true
                        Layout.preferredHeight: 120
                        clip: true
                        model: composition
                        delegate: Rectangle {
                            width: ListView.view.width
                            height: crow.implicitHeight + 8
                            color: index % 2 === 0 ? "#fafafa" : "#ffffff"
                            RowLayout {
                                id: crow
                                anchors.fill: parent
                                anchors.margins: 4
                                spacing: 4
                                Label { text: (index + 1) + ". " + id; Layout.fillWidth: true; elide: Label.ElideMiddle }
                                ToolButton { text: "▲"; onClicked: root.moveUp(index) }
                                ToolButton { text: "▼"; onClicked: root.moveDown(index) }
                                ToolButton { text: "✕"; onClicked: root.removeAt(index) }
                            }
                        }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        Label { text: qsTr("Composed (%1)").arg(target); font.bold: true; Layout.fillWidth: true }
                        Button { text: qsTr("Save…"); enabled: composedYaml.length > 0; onClicked: saveDialog.open() }
                    }
                    ScrollView {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        TextArea {
                            text: root.composedYaml
                            readOnly: true
                            font.family: "Menlo"
                            font.pixelSize: 11
                            wrapMode: TextEdit.NoWrap
                            placeholderText: qsTr("Double-click recipes to compose a standalone pipeline.")
                        }
                    }
                }
            }
        }
    }
}
