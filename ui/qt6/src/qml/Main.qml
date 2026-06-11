import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import Pipewright

ApplicationWindow {
    id: root
    width: 1280
    height: 800
    visible: true
    title: (root.dirty ? "• " : "")
        + (root.sourcePath.length > 0 ? root.sourcePath : qsTr("Pipewright"))
        + (root.sourcePath.length > 0 ? qsTr(" — Pipewright") : "")

    property string sourceText: ""
    property string sourceKind: "gitlab"
    property string sourcePath: ""
    // Unsaved-changes flag: set whenever an edit (Source tab, DAG field edit,
    // recipe apply) mutates sourceText after a load; cleared on save/open.
    property bool dirty: false
    // All engine-supported platforms (single source of truth — from the FFI,
    // never a hard-coded subset). Drives every platform picker.
    readonly property var platformList: BridgeApi.platforms()
    // App-wide content+chrome language (en/de). Threaded to every view's prose
    // locale; the UI chrome follows via the Qt translator (see langCombo).
    property string appLocale: "en"

    // Headless smoke self-check (PIPEWRIGHT_SMOKE=1, set as a context property
    // by main.cpp). Exercises the FFI bridge + a render path on the real UI and
    // exits with 0 (pass) / 1 (fail) — an end-to-end smoke test runnable
    // offscreen in CI. No-op in normal runs.
    Timer {
        interval: 1200
        running: (typeof smokeMode !== "undefined") && smokeMode
        repeat: false
        onTriggered: {
            var ok = false
            try {
                if (typeof smokeFile !== "undefined" && smokeFile && smokeFile.length > 0) {
                    root.loadFile("file://" + smokeFile)
                }
                var src = root.sourceText.length > 0 ? root.sourceText : "build:\n  script:\n    - echo hi\n"
                var kind = root.sourceText.length > 0 ? root.sourceKind : "gitlab"
                var svg = BridgeApi.renderSvg(src, kind, "push", "main", false, false, "")
                var caps = JSON.parse(BridgeApi.capabilities(src, kind, ""))
                ok = root.platformList.length >= 17 && svg.indexOf("<svg") >= 0 && !!caps.overall
                console.log("SMOKE: platforms=" + root.platformList.length
                    + " svg=" + (svg.indexOf("<svg") >= 0)
                    + " caps=" + (caps.overall || "none") + " => " + (ok ? "PASS" : "FAIL"))
            } catch (e) {
                console.log("SMOKE: exception " + e + " => FAIL")
            }
            Qt.exit(ok ? 0 : 1)
        }
    }
    // Screenshot mode: load the fixture and show the DAG tab, then leave the
    // window up for a screen capture. No exit. (PIPEWRIGHT_SHOT=1)
    Timer {
        interval: 1400
        running: (typeof shotMode !== "undefined") && shotMode
        repeat: false
        onTriggered: {
            if (typeof smokeFile !== "undefined" && smokeFile && smokeFile.length > 0) {
                root.loadFile("file://" + smokeFile)
            }
            rightTabs.currentIndex = 1   // DAG tab
        }
    }
    property string statusText: qsTr("No pipeline loaded.")
    property string selectedJob: ""             // empty = nothing focused

    // Window-wide drop target — drag a file from Finder anywhere on
    // top of Pipewright and we open it. Works around the macOS
    // native panel hiding dotfiles (Finder respects the global
    // ⇧⌘. setting; once that's flipped, you can grab .gitlab-ci.yml
    // and drop it here).
    DropArea {
        anchors.fill: parent
        onDropped: (drop) => {
            if (drop.hasUrls && drop.urls.length > 0) {
                root.loadFile(drop.urls[0])
                drop.accept()
            }
        }
    }

    // ----- menu --------------------------------------------------------
    menuBar: MenuBar {
        Menu {
            title: qsTr("&File")
            Action {
                text: qsTr("&Open…")
                shortcut: StandardKey.Open
                onTriggered: fileDialog.open()
            }
            MenuSeparator {}
            Action {
                text: qsTr("&Save")
                shortcut: StandardKey.Save
                // Save back to the loaded file; if the pipeline was pasted (no
                // path), fall through to Save As.
                enabled: root.dirty || root.sourcePath.length > 0
                onTriggered: root.sourcePath.length > 0 ? root.saveToPath(root.sourcePath) : saveAsDialog.open()
            }
            Action {
                text: qsTr("Save &As…")
                shortcut: StandardKey.SaveAs
                enabled: root.sourceText.length > 0
                onTriggered: saveAsDialog.open()
            }
            MenuSeparator {}
            Action { text: qsTr("&Quit"); shortcut: StandardKey.Quit; onTriggered: Qt.quit() }
        }
        Menu {
            title: qsTr("&Edit")
            Action {
                text: qsTr("&Settings…")
                shortcut: StandardKey.Preferences
                onTriggered: settingsDialog.open()
            }
        }
        Menu {
            title: qsTr("&Help")
            Action {
                text: qsTr("&About FFI")
                onTriggered: aboutPopup.open()
            }
        }
    }

    SettingsDialog { id: settingsDialog }

    // Save As — suggests the platform's conventional file name when the
    // pipeline was pasted (no path yet).
    FileDialog {
        id: saveAsDialog
        fileMode: FileDialog.SaveFile
        title: qsTr("Save pipeline as…")
        currentFile: root.sourcePath.length > 0
            ? "file://" + root.sourcePath
            : "file:///" + BridgeApi.defaultFileName(root.sourceKind)
        onAccepted: root.saveToPath(selectedFile.toString().replace(/^file:\/\//, ""))
    }

    Dialog {
        id: runConfirm
        property string jobName: ""
        title: qsTr("Run locally?")
        modal: true
        standardButtons: Dialog.Cancel
        anchors.centerIn: parent
        contentItem: ColumnLayout {
            spacing: 8
            Label {
                text: qsTr("Run job '%1' on this machine using the local Docker runner?")
                    .arg(runConfirm.jobName)
                wrapMode: Label.WordWrap
                Layout.preferredWidth: 360
            }
            Label {
                text: qsTr("The Run tab will open and stream stdout/stderr live.")
                color: "#666"
                font.pixelSize: 11
                wrapMode: Label.WordWrap
                Layout.preferredWidth: 360
            }
        }
        footer: DialogButtonBox {
            Button {
                text: qsTr("Run")
                DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
                onClicked: {
                    runConfirm.accept()
                    rightTabs.currentIndex = 5     // Run tab
                    runView.jobFilter = runConfirm.jobName
                    runView.triggerRun()
                }
            }
            Button {
                text: qsTr("Cancel")
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
                onClicked: runConfirm.reject()
            }
        }
    }

    FileDialog {
        id: fileDialog
        // macOS hides dotfiles by default in the native panel; the
        // hint in the title is the cheapest discoverable cure for
        // users who don't know about ⇧⌘. The Jenkinsfile filter
        // entry isn't a glob (the file has no extension) — it lives
        // in its own line so users can see at a glance what counts
        // as a pipeline.
        title: qsTr("Open pipeline definition  (⇧⌘. shows hidden files like .gitlab-ci.yml)")
        nameFilters: [
            qsTr("All files (*)"),
            qsTr("GitLab CI (*.gitlab-ci.yml *.yml *.yaml)"),
            qsTr("GitHub Actions (*.yml *.yaml)"),
            qsTr("Jenkinsfile (Jenkinsfile *.Jenkinsfile)")
        ]
        onAccepted: root.loadFile(selectedFile)
    }

    Popup {
        id: aboutPopup
        anchors.centerIn: parent
        modal: true
        padding: 20
        contentItem: ColumnLayout {
            spacing: 6
            Label { text: qsTr("Pipewright — Qt6 frontend"); font.bold: true; font.pixelSize: 14 }
            Label { text: qsTr("Backend: %1").arg(BridgeApi.version()) }
        }
    }

    // ----- model + actions --------------------------------------------
    ListModel { id: jobsModel }

    function loadFile(fileUrl) {
        const path = fileUrl.toString().replace(/^file:\/\//, "")
        sourcePath = path
        const result = BridgeApi.readFile(path)
        if (!result.ok) {
            statusText = qsTr("Read error: %1").arg(result.error)
            return
        }
        sourceText = result.content
        dirty = false                 // freshly loaded = clean
        detectAndInspect()
    }

    // Write the current (possibly edited) source back to `path` and adopt it as
    // the loaded file. Clears the dirty flag on success.
    function saveToPath(path) {
        const res = BridgeApi.writeFile(path, sourceText)
        if (!res.ok) {
            statusText = qsTr("Save error: %1").arg(res.error || qsTr("write failed"))
            return
        }
        sourcePath = path
        dirty = false
        statusText = qsTr("Saved %1").arg(path)
    }

    function detectAndInspect() {
        // Prefer the file name (.gitlab-ci.yml, Jenkinsfile, …) over content —
        // the loaded path is the strongest signal. Pasted source has no path,
        // so this falls back to content detection.
        const detectRaw = BridgeApi.detectWithPath(sourcePath, sourceText)
        try {
            const det = JSON.parse(detectRaw)
            if (det.kind && det.kind !== "unknown") {
                sourceKind = det.kind
                platformCombo.currentIndex = root.platformList.indexOf(det.kind)
            }
        } catch (e) {
            // ignore — leave manual override
        }
        inspect()
    }

    // Selecting a job in the list syncs `selectedJob` (so other tabs
    // can light up) and jumps the Source view to the job's
    // definition. Byte offset comes from the Hub-IR provenance
    // range, which is byte-accurate because the CST kept token
    // spans. Run-on-click is intentionally NOT triggered here —
    // a double-click opens the guarded "Run locally?" prompt.
    function focusJob(jobName, byteStart) {
        selectedJob = jobName
        rightTabs.currentIndex = 0           // Source tab
        sourceArea.cursorPosition = byteStart
        sourceArea.forceActiveFocus()
    }

    function promptRunJob(jobName) {
        runConfirm.jobName = jobName
        runConfirm.open()
    }

    function inspect() {
        jobsModel.clear()
        const raw = BridgeApi.inspect(sourceText, sourceKind, "push", "main", sourcePath)
        try {
            const obj = JSON.parse(raw)
            if (obj.error) {
                statusText = qsTr("Error: %1").arg(obj.error)
                return
            }
            const jobs = (obj.pipeline && obj.pipeline.jobs) || []
            for (let i = 0; i < jobs.length; ++i) {
                const j = jobs[i]
                jobsModel.append({
                    name: j.name || "(unnamed)",
                    stage: j.stage || "",
                    stepCount: (j.steps || []).length,
                    depends: (j.needs || []).join(", "),
                    byteStart: j.byte_start || 0
                })
            }
            statusText = qsTr("Loaded %1 — %2 jobs (%3)")
                .arg(sourcePath || qsTr("<inline>"))
                .arg(jobs.length)
                .arg(sourceKind)
        } catch (e) {
            statusText = qsTr("Parse error: %1").arg(e)
        }
    }

    // ----- layout -----------------------------------------------------
    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // top bar — file / platform / status
        Rectangle {
            Layout.fillWidth: true
            height: 44
            color: "#f3f3f3"
            RowLayout {
                anchors.fill: parent
                anchors.margins: 6
                spacing: 10
                Button {
                    text: qsTr("Open…")
                    onClicked: fileDialog.open()
                }
                // Path field bypasses the native macOS panel entirely —
                // tab-complete in the user's terminal, paste here,
                // Enter. The two-line UX problem (native dialog hides
                // dotfiles, no API to override) goes away for anyone
                // who already has the path in their clipboard.
                TextField {
                    placeholderText: qsTr("…or paste full path  (~/proj/.gitlab-ci.yml)")
                    Layout.preferredWidth: 320
                    onAccepted: {
                        const trimmed = text.trim()
                        if (trimmed.length > 0) {
                            root.loadFile("file://" + trimmed)
                            text = ""
                        }
                    }
                }
                Label { text: qsTr("Platform:") }
                ComboBox {
                    id: platformCombo
                    model: root.platformList
                    currentIndex: root.platformList.indexOf("gitlab")
                    onActivated: {
                        sourceKind = model[currentIndex]
                        if (sourceText.length > 0) inspect()
                    }
                }
                Label {
                    Layout.fillWidth: true
                    text: statusText
                    elide: Label.ElideMiddle
                    color: "#444"
                }
                // App-wide language: drives the prose locale across every view
                // (Runbook, Recipes, generated descriptions) AND the UI chrome
                // via the installed Qt translator (BridgeApi.setLanguage).
                Label { text: qsTr("Language:") }
                ComboBox {
                    id: langCombo
                    model: ["en", "de"]
                    currentIndex: model.indexOf(root.appLocale)
                    Layout.preferredWidth: 70
                    onActivated: {
                        root.appLocale = model[currentIndex]
                        BridgeApi.setLanguage(root.appLocale)
                    }
                }
            }
        }

        SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            orientation: Qt.Horizontal

            // left — jobs list
            Rectangle {
                SplitView.preferredWidth: 360
                SplitView.minimumWidth: 200
                color: "#fafafa"
                ColumnLayout {
                    anchors.fill: parent
                    spacing: 0
                    Label {
                        text: qsTr("Jobs (%1)").arg(jobsModel.count)
                        font.bold: true
                        padding: 8
                    }
                    ListView {
                        id: jobsList
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        model: jobsModel
                        delegate: ItemDelegate {
                            id: jobItem
                            width: jobsList.width
                            highlighted: root.selectedJob === name
                            onClicked: root.focusJob(name, byteStart)
                            onDoubleClicked: root.promptRunJob(name)
                            ToolTip.text: qsTr("Click: jump to definition · Double-click: run locally")
                            ToolTip.visible: hovered
                            ToolTip.delay: 600
                            contentItem: ColumnLayout {
                                spacing: 2
                                Label { text: name; font.bold: true }
                                Label {
                                    text: (stage.length > 0 ? stage + " · " : "") + qsTr("%1 step(s)").arg(stepCount)
                                    font.pixelSize: 10
                                    color: "#666"
                                }
                                Label {
                                    visible: depends.length > 0
                                    text: qsTr("after: %1").arg(depends)
                                    font.pixelSize: 10
                                    color: "#888"
                                }
                            }
                        }
                    }
                }
            }

            // right — tabs: Source | DAG | Capabilities
            Rectangle {
                SplitView.fillWidth: true
                color: "#ffffff"
                ColumnLayout {
                    anchors.fill: parent
                    spacing: 0

                    TabBar {
                        id: rightTabs
                        Layout.fillWidth: true
                        TabButton { text: qsTr("Source") }
                        TabButton { text: qsTr("DAG") }
                        TabButton { text: qsTr("Capabilities") }
                        TabButton { text: qsTr("Migrate") }
                        TabButton { text: qsTr("Recipes") }
                        TabButton { text: qsTr("Run") }
                        TabButton { text: qsTr("Runbook") }
                    }

                    StackLayout {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        currentIndex: rightTabs.currentIndex

                        // ----- Source tab
                        ColumnLayout {
                            spacing: 0
                            Label {
                                text: sourcePath || qsTr("Pasted source")
                                font.bold: true
                                padding: 8
                                elide: Label.ElideLeft
                                Layout.fillWidth: true
                            }
                            ScrollView {
                                Layout.fillWidth: true
                                Layout.fillHeight: true
                                TextArea {
                                    id: sourceArea
                                    text: sourceText
                                    placeholderText: qsTr("build:\n  script:\n    - cargo build\n")
                                    wrapMode: TextEdit.NoWrap
                                    font.family: "Menlo"
                                    font.pixelSize: 12
                                    Component.onCompleted: BridgeApi.installHighlighter(textDocument, sourceKind)
                                    Connections {
                                        target: root
                                        function onSourceKindChanged() {
                                            BridgeApi.installHighlighter(sourceArea.textDocument, root.sourceKind)
                                        }
                                    }
                                    // Debounce re-parsing: a single keystroke must
                                    // NOT run the whole forward cascade (seconds on a
                                    // large pipeline → a frozen window per character).
                                    // Mark dirty + remember the text immediately, but
                                    // re-inspect only after a typing pause.
                                    property string pendingText: ""
                                    Timer {
                                        id: reparseTimer
                                        interval: 450
                                        onTriggered: {
                                            sourceText = sourceArea.pendingText
                                            detectAndInspect()
                                        }
                                    }
                                    onTextChanged: {
                                        // `text !== sourceText` is true only for a
                                        // USER edit; a programmatic load binds
                                        // text = sourceText, so it won't fire.
                                        if (text !== sourceText) {
                                            pendingText = text
                                            root.dirty = true
                                            reparseTimer.restart()
                                        }
                                    }
                                }
                            }
                        }

                        // ----- DAG tab
                        GraphEditor {
                            sourceText: root.sourceText
                            sourceKind: root.sourceKind
                            sourcePath: root.sourcePath
                            selectedJob: root.selectedJob
                            recipesConfig: settingsDialog.recipesConfig
                            cacheDir: settingsDialog.cacheDir
                            // An in-place field edit returns new source — adopt
                            // it as the shared source so the editor + every view
                            // re-render from the edited pipeline.
                            onEditApplied: function(newYaml) {
                                root.sourceText = newYaml
                                root.dirty = true
                                if (root.sourceText.length > 0) root.inspect()
                            }
                        }

                        // ----- Capabilities tab
                        CapabilitiesView {
                            sourceText: root.sourceText
                            sourceKind: root.sourceKind
                            sourcePath: root.sourcePath
                            selectedJob: root.selectedJob
                        }

                        // ----- Migrate tab
                        MigrationView {
                            sourceText: root.sourceText
                            sourceKind: root.sourceKind
                            sourcePath: root.sourcePath
                        }

                        // ----- Recipes tab
                        RecipesView {
                            locale: root.appLocale
                            sourceText: root.sourceText
                            sourceKind: root.sourceKind
                            sourcePath: root.sourcePath
                            recipesConfig: settingsDialog.recipesConfig
                            cacheDir: settingsDialog.cacheDir
                            // Applying a recipe to the current pipeline returns
                            // new source — adopt it as the shared source so the
                            // editor + every view re-render from it.
                            onApplyToPipeline: function(newYaml) {
                                root.sourceText = newYaml
                                root.dirty = true
                                if (root.sourceText.length > 0) root.inspect()
                            }
                        }

                        // ----- Run tab
                        RunView {
                            id: runView
                            sourcePath: root.sourcePath
                            sourceText: root.sourceText
                            sourceKind: root.sourceKind
                            selectedJob: root.selectedJob
                            cliPath: settingsDialog.cliPath
                            dockerHost: settingsDialog.dockerHost
                        }

                        // ----- Runbook tab (human-readable view)
                        HumanView {
                            locale: root.appLocale
                            sourceText: root.sourceText
                            sourceKind: root.sourceKind
                            sourcePath: root.sourcePath
                            selectedJob: root.selectedJob
                        }
                    }
                }
            }
        }
    }
}
