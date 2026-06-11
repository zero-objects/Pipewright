import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Pipewright

// A QML-native interactive pipeline editor: stage swimlanes, UML compartment
// nodes (header + params + steps), dependency edges, selection, and in-place
// field editing wired to the TGG backward-cascade edit (BridgeApi.editField).
// Geometry comes from BridgeApi.renderSvgWithLayout's `layout` descriptor;
// per-node content from BridgeApi.inspect; the two are joined by hub GhostId.
Item {
    id: root

    property string sourceText: ""
    property string sourceKind: "gitlab"
    property string sourcePath: ""
    property string selectedJob: ""
    property string selectedHub: ""    // hub GhostId of the selected node
    property string recipesConfig: ""  // recipe sources config (from Settings)
    property string cacheDir: ""
    // Emitted when an edit produced new source (Main adopts it as sourceText).
    signal editApplied(string newYaml)

    // Apply a structural op result ({ok,yaml}) and clear the selection.
    function applyStructural(raw) {
        try {
            const r = JSON.parse(raw)
            if (r.ok && r.yaml) { root.selectedHub = ""; root.selectedJob = ""; root.editApplied(r.yaml) }
        } catch (e) { /* error surfaced elsewhere */ }
    }
    function duplicateSelected() {
        if (selectedHub) applyStructural(BridgeApi.duplicate(sourceText, sourceKind, selectedHub))
    }
    function deleteSelected() {
        if (selectedHub) applyStructural(BridgeApi.remove(sourceText, sourceKind, selectedHub))
    }

    property var layout: null            // {width,height,jobs,lanes,edges}
    property var contentByHub: ({})      // hub -> {name,stage,params,steps,condition}
    property var geomByHub: ({})         // hub -> {x,y,w,h}

    function refresh() {
        if (!sourceText) { layout = null; return }
        try {
            const lay = JSON.parse(BridgeApi.renderSvgWithLayout(
                sourceText, sourceKind, "push", "main", false, false, sourcePath))
            layout = lay.error ? null : (lay.layout || null)
        } catch (e) { layout = null }

        const content = {}
        const geom = {}
        try {
            const insp = JSON.parse(BridgeApi.inspect(
                sourceText, sourceKind, "push", "main", sourcePath))
            for (const j of ((insp.pipeline && insp.pipeline.jobs) || [])) content[j.hub] = j
        } catch (e) { /* leave content empty */ }
        if (root.layout) for (const b of (root.layout.jobs || [])) geom[b.hub] = b
        contentByHub = content
        geomByHub = geom
        edgeCanvas.requestPaint()
    }

    function applyEdit(hub, newValue, oldValue) {
        if (!hub || newValue === oldValue) return
        try {
            const r = JSON.parse(BridgeApi.editField(sourceText, sourceKind, hub, newValue))
            if (r.ok && r.yaml) root.editApplied(r.yaml)
        } catch (e) { /* error surfaced elsewhere */ }
    }

    // Open the recipe picker (also reachable via the "+ Recipe…" button).
    function openRecipePicker() { recipePicker.openPicker() }

    // Apply recipe `id` to the current pipeline (merge its jobs in) and adopt
    // the re-emitted source.
    function applyRecipe(id) {
        try {
            const r = JSON.parse(BridgeApi.applyRecipe(sourceText, sourceKind, id, recipesConfig, cacheDir))
            if (r.ok && r.yaml) { recipePicker.close(); root.editApplied(r.yaml) }
            else if (r.error) recipePicker.errorText = r.error
        } catch (e) { recipePicker.errorText = "" + e }
    }

    onSourceTextChanged: refresh()
    onSourceKindChanged: refresh()
    Component.onCompleted: refresh()

    Flickable {
        id: flick
        anchors.fill: parent
        contentWidth: Math.max(root.layout ? root.layout.width : width, width)
        contentHeight: Math.max(root.layout ? root.layout.height : height, height)
        clip: true
        ScrollBar.vertical: ScrollBar {}
        ScrollBar.horizontal: ScrollBar {}

        // Empty / no-render hint.
        Label {
            visible: !root.layout || (root.layout.jobs || []).length === 0
            anchors.centerIn: parent
            text: qsTr("Open or paste a pipeline to render its graph.")
            color: "#888"
        }

        // Stage swimlane bands + headers.
        Repeater {
            model: root.layout ? (root.layout.lanes || []) : []
            delegate: Rectangle {
                x: modelData.x
                y: 0
                width: modelData.width
                height: flick.contentHeight
                color: index % 2 === 0 ? "#f3f5f9" : "#eef1f6"
                Label {
                    anchors.horizontalCenter: parent.horizontalCenter
                    y: 4
                    text: modelData.stage
                    font.bold: true
                    font.pixelSize: 12
                    color: "#374151"
                }
            }
        }

        // Dependency edges (drawn under the nodes).
        Canvas {
            id: edgeCanvas
            anchors.fill: parent
            onPaint: {
                const ctx = getContext("2d")
                ctx.reset()
                ctx.strokeStyle = "#9aa3af"
                ctx.fillStyle = "#9aa3af"
                ctx.lineWidth = 1.5
                const edges = (root.layout && root.layout.edges) || []
                for (const e of edges) {
                    const a = root.geomByHub[e.from]
                    const b = root.geomByHub[e.to]
                    if (!a || !b) continue
                    const x1 = a.x + a.w, y1 = a.y + a.h / 2
                    const x2 = b.x, y2 = b.y + b.h / 2
                    const cx = x1 + (x2 - x1) / 2
                    ctx.beginPath()
                    ctx.moveTo(x1, y1)
                    ctx.bezierCurveTo(cx, y1, cx, y2, x2, y2)
                    ctx.stroke()
                    // arrowhead
                    ctx.beginPath()
                    ctx.moveTo(x2, y2)
                    ctx.lineTo(x2 - 7, y2 - 4)
                    ctx.lineTo(x2 - 7, y2 + 4)
                    ctx.closePath()
                    ctx.fill()
                }
            }
        }

        // Job nodes.
        Repeater {
            id: nodeRepeater
            model: root.layout ? (root.layout.jobs || []) : []
            delegate: JobNode {
                x: modelData.x
                y: modelData.y
                width: modelData.w
                height: modelData.h
                hub: modelData.hub
                job: root.contentByHub[modelData.hub] || ({})
                selected: root.selectedHub === modelData.hub
                onClicked: { root.selectedJob = (job.name || ""); root.selectedHub = modelData.hub }
                onEdit: function(fieldHub, newValue, oldValue) { root.applyEdit(fieldHub, newValue, oldValue) }
            }
        }
    }

    // Always-visible action: apply a recipe to the whole pipeline.
    Rectangle {
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.margins: 10
        z: 10
        radius: 8
        color: "#ffffff"
        border.color: "#cdd2db"
        implicitWidth: addRecipeBtn.implicitWidth + 16
        implicitHeight: addRecipeBtn.implicitHeight + 12
        Button {
            id: addRecipeBtn
            anchors.centerIn: parent
            text: qsTr("+ Recipe…")
            onClicked: root.openRecipePicker()
        }
    }

    // Recipe picker: searchable registry list; clicking a recipe applies it.
    Dialog {
        id: recipePicker
        property string errorText: ""
        title: qsTr("Apply a recipe")
        modal: true
        width: 460
        height: 480
        anchors.centerIn: Overlay.overlay
        standardButtons: Dialog.Close

        ListModel { id: pickerModel }

        function openPicker() {
            errorText = ""
            pickerSearch.text = ""
            reloadPicker("")
            open()
        }
        function reloadPicker(query) {
            pickerModel.clear()
            try {
                const obj = JSON.parse(BridgeApi.listRecipes(query, "name", root.recipesConfig, root.cacheDir))
                if (obj.error) { errorText = obj.error; return }
                for (const r of (obj.recipes || [])) pickerModel.append({ rec: r })
            } catch (e) { errorText = "" + e }
        }

        contentItem: ColumnLayout {
            spacing: 6
            TextField {
                id: pickerSearch
                Layout.fillWidth: true
                placeholderText: qsTr("Search recipes…")
                onTextChanged: recipePicker.reloadPicker(text)
            }
            Label {
                visible: recipePicker.errorText.length > 0
                text: recipePicker.errorText
                color: "#b91c1c"
                font.pixelSize: 10
                wrapMode: Label.WordWrap
                Layout.fillWidth: true
            }
            ListView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: pickerModel
                delegate: ItemDelegate {
                    width: ListView.view.width
                    onClicked: root.applyRecipe(rec.id)
                    contentItem: ColumnLayout {
                        spacing: 2
                        Label { text: rec.id + "  (" + rec.source + ")"; font.bold: true }
                        Label { text: rec.description; font.pixelSize: 10; color: "#666"; elide: Label.ElideRight; Layout.fillWidth: true }
                        // Declared ports — what the recipe consumes / produces,
                        // so applying it is port-aware (full visual wiring is a
                        // follow-up; source pipelines carry no port metadata to
                        // auto-match against).
                        Label {
                            visible: (rec.inputs && rec.inputs.length > 0) || (rec.outputs && rec.outputs.length > 0)
                            text: "in: " + (rec.inputs || []).map(function(p){ return p.name }).join(", ")
                                + "   out: " + (rec.outputs || []).map(function(p){ return p.name }).join(", ")
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

    // Floating action toolbar — appears when a node is selected.
    Rectangle {
        visible: root.selectedHub.length > 0
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.margins: 10
        z: 10
        radius: 8
        color: "#ffffff"
        border.color: "#cdd2db"
        implicitWidth: actions.implicitWidth + 16
        implicitHeight: actions.implicitHeight + 12
        RowLayout {
            id: actions
            anchors.centerIn: parent
            spacing: 6
            Button { text: qsTr("Duplicate"); onClicked: root.duplicateSelected() }
            Button { text: qsTr("Delete"); onClicked: root.deleteSelected() }
        }
    }
}
