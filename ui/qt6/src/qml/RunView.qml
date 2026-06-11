import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Pipewright

// Local Docker execution of the loaded pipeline via the `pipewright` CLI:
// jobs in dependency order, each in its image, stdout/stderr streamed live.
// (A "replay a server-side pipeline" mode existed in the pre-rebuild CLI and
// was removed with it — the rebuilt `pipewright` binary has no such command;
// re-adding it is a backend feature first, see docs/gaps-backlog.md.)
Item {
    id: root

    property string sourceText: ""
    property string sourceKind: "gitlab"
    property string sourcePath: ""
    property string cliPath: ""
    property string dockerHost: ""            // optional DOCKER_HOST override (Settings)
    property string jobFilter: ""
    property string triggerEvent: "push"
    property string ref: "main"
    property string selectedJob: ""           // optional highlight from the job list

    // capability cache
    property var caps: null                   // {overall, summary, features:[…]}
    property string overallLevel: "Possible"
    property var jobNames: []                  // job names for the filter dropdown (from inspect)
    // Whether the current platform runs locally at all (argo/tekton/… don't).
    property bool runnable: true
    property string notRunnableReason: ""

    // Resolution order: explicit Settings value, then a best-effort
    // discovery (PATH, Cargo target/, ~/.cargo/bin, Homebrew). The
    // discovery hint is the opened pipeline file, which usually lives
    // in the same repo the CLI was built in. Empty string means
    // "no CLI was found anywhere" — caller surfaces a friendly error.
    function defaultCliPath() {
        if (cliPath.length > 0) return cliPath
        return BridgeApi.discoverCliPath(sourcePath)
    }

    function refreshCaps() {
        caps = null
        overallLevel = "Possible"
        jobNames = []
        // Runnability of the platform itself (independent of the pipeline).
        try {
            const rs = JSON.parse(BridgeApi.runSupport(sourceKind))
            runnable = !!rs.runnable
            notRunnableReason = rs.reason || ""
        } catch (e) { runnable = true; notRunnableReason = "" }
        if (!sourceText) return
        // Portability profile (for the "Overall" hint).
        try {
            const obj = JSON.parse(BridgeApi.capabilities(sourceText, sourceKind, sourcePath))
            if (!obj.error) { caps = obj; overallLevel = obj.overall || "Possible" }
        } catch (e) { /* ignore */ }
        // Job names for the filter — the IR is the source of truth.
        try {
            const insp = JSON.parse(BridgeApi.inspect(sourceText, sourceKind, "push", "main", sourcePath))
            const names = []
            for (const j of ((insp.pipeline && insp.pipeline.jobs) || [])) names.push(j.name)
            jobNames = names
        } catch (e) { /* ignore */ }
    }

    function blockingReason() {
        // The only hard block is a platform whose pipelines don't run locally
        // (argo/tekton/dagger/jenkins/earthly/aws_codepipeline). Per-job
        // failures are reported live by the runner.
        if (!runnable) return notRunnableReason
        return ""
    }

    // Programmatic entry point used by the job-list "Run locally?"
    // dialog. Respects the same blocking-reason guards as the manual
    // Run button.
    function triggerRun() {
        refreshCaps()
        if (blockingReason().length > 0) {
            outputArea.append("[blocked] " + blockingReason())
            return
        }
        if (sourcePath.length === 0) {
            outputArea.append("[blocked] no pipeline file path set")
            return
        }
        const exe = defaultCliPath()
        if (exe.length === 0) {
            outputArea.append("[blocked] couldn't find the `pipewright` binary — "
                + "set its full path in Edit → Settings → CLI path.")
            return
        }
        runner.start(exe, buildArgs(), runEnv())
    }

    // Child-process env overrides: the configured Docker host (Settings →
    // Local runner) reaches the runner as DOCKER_HOST. Empty = inherit.
    function runEnv() {
        return dockerHost.length > 0 ? { "DOCKER_HOST": dockerHost } : {}
    }

    // Mount mode for the local run: "ro" (read-only, default/safe), "rw"
    // (read-write in place — the pipeline can modify the real directory),
    // or "rw-copy" (read-write on a throwaway copy).
    property string mountMode: "ro"

    function buildArgs() {
        // `pipeline run` takes the YAML file as a positional arg. The
        // platform is passed explicitly so a manual override in the UI's
        // platform picker carries through (the CLI would otherwise
        // re-detect from the file and could disagree).
        const args = ["run", "-p", sourceKind, "--trigger", triggerEvent, "--ref", ref]
        if (jobFilter.length > 0) {
            args.push("--job"); args.push(jobFilter)
        }
        if (mountMode === "rw") args.push("--rw")
        else if (mountMode === "rw-copy") args.push("--rw-copy")
        args.push(sourcePath)
        return args
    }

    // Run, but if the user picked read-write-in-place (the one that can modify
    // their real directory), confirm first — a foreign pipeline is untrusted code.
    function startRun() {
        if (mountMode === "rw") {
            confirmRwDialog.open()
        } else {
            runner.start(root.defaultCliPath(), root.buildArgs(), root.runEnv())
        }
    }

    Dialog {
        id: confirmRwDialog
        title: qsTr("Run with write access?")
        modal: true
        anchors.centerIn: Overlay.overlay
        standardButtons: Dialog.Ok | Dialog.Cancel
        Label {
            width: 360
            wrapMode: Label.WordWrap
            text: qsTr("The pipeline's commands will run with write access to the "
                + "real source directory and can modify or delete files there. "
                + "Only continue for a pipeline you trust. To run a writable build "
                + "safely, cancel and choose “read-write (copy)” instead.")
        }
        onAccepted: runner.start(root.defaultCliPath(), root.buildArgs(), root.runEnv())
    }

    onSourceTextChanged: refreshCaps()
    onSourceKindChanged: refreshCaps()

    ProcessRunner {
        id: runner
        onStdoutLine: outputArea.append(line)
        onStderrLine: outputArea.append("[stderr] " + line)
        onStarted: outputArea.append("--- started: "
            + root.defaultCliPath() + " " + root.buildArgs().join(" ") + " ---")
        onFinished: outputArea.append("--- exit " + exitCode + " ---")
        onErrorOccurred: outputArea.append("[error] " + message)
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 8
        spacing: 8

        Frame {
            Layout.fillWidth: true
            ColumnLayout {
                anchors.fill: parent
                spacing: 6

                RowLayout {
                    Label { text: qsTr("CLI binary:") }
                    TextField {
                        text: root.cliPath
                        placeholderText: qsTr("pipewright (on PATH, or full path — set persistently in Edit → Settings)")
                        Layout.fillWidth: true
                        onEditingFinished: root.cliPath = text
                    }
                }

                RowLayout {
                    Label { text: qsTr("Pipeline file:") }
                    TextField {
                        text: root.sourcePath
                        placeholderText: qsTr("Open via File → Open")
                        Layout.fillWidth: true
                        onEditingFinished: root.sourcePath = text
                    }
                }
                RowLayout {
                    Label { text: qsTr("Job filter:") }
                    ComboBox {
                        id: jobCombo
                        editable: true
                        Layout.preferredWidth: 220
                        model: [""].concat(root.jobNames)
                        onEditTextChanged: root.jobFilter = editText
                        onActivated: root.jobFilter = currentText
                    }
                    Label {
                        text: root.jobFilter ? qsTr("(only this job)") : qsTr("(all jobs)")
                        color: "#666"
                        font.bold: true
                    }
                    Label { text: qsTr("Event:") }
                    ComboBox {
                        model: ["push", "tag", "merge_request", "schedule", "manual", "external"]
                        Layout.preferredWidth: 140
                        onActivated: root.triggerEvent = model[currentIndex]
                    }
                    Label { text: qsTr("Ref:") }
                    TextField {
                        text: root.ref
                        Layout.preferredWidth: 120
                        onEditingFinished: root.ref = text
                    }
                }

                RowLayout {
                    Label { text: qsTr("Source access:") }
                    ComboBox {
                        id: mountCombo
                        Layout.preferredWidth: 280
                        textRole: "label"
                        model: [
                            { label: qsTr("read-only (safe — default)"), mode: "ro" },
                            { label: qsTr("read-write (copy — safe build)"), mode: "rw-copy" },
                            { label: qsTr("read-write (in place — can modify files)"), mode: "rw" },
                        ]
                        onActivated: root.mountMode = model[currentIndex].mode
                    }
                    Label {
                        text: root.mountMode === "rw"
                              ? qsTr("⚠ commands can modify your files")
                              : (root.mountMode === "rw-copy"
                                 ? qsTr("writes go to a throwaway copy")
                                 : qsTr("nothing can be modified"))
                        color: root.mountMode === "rw" ? "#c62828" : "#666"
                        font.bold: root.mountMode === "rw"
                    }
                }

                // ---- not-runnable banner (translate-only platform) -----
                // Bound to the `runnable` property (not the blockingReason()
                // function) so it updates reactively when the platform changes.
                Rectangle {
                    visible: !root.runnable
                    Layout.fillWidth: true
                    color: "#fff3e0"
                    radius: 3
                    height: blockingLabel.implicitHeight + 12
                    Label {
                        id: blockingLabel
                        anchors.fill: parent
                        anchors.margins: 6
                        text: qsTr("✋ %1 pipelines can't run locally — %2 Use Inspect / Migrate instead.")
                            .arg(root.sourceKind).arg(root.notRunnableReason)
                        color: "#92400e"
                        wrapMode: Label.WordWrap
                    }
                }

                RowLayout {
                    Button {
                        text: runner.running ? qsTr("Running…") : qsTr("Run")
                        enabled: !runner.running
                                 && root.runnable
                                 && root.sourcePath.length > 0
                        onClicked: root.startRun()
                    }
                    Button {
                        text: qsTr("Stop")
                        enabled: runner.running
                        onClicked: runner.stop()
                    }
                    Button {
                        text: qsTr("Clear output")
                        onClicked: outputArea.text = ""
                    }
                    Item { Layout.fillWidth: true }
                    Label {
                        text: qsTr("Portability: %1").arg(root.overallLevel)
                        color: {
                            switch (root.overallLevel) {
                                case "Possible":            return "#2e7d32"
                                case "PossibleWithCaveats": return "#ed6c02"
                                case "NotPossible":         return "#c62828"
                                default:                    return "#666"
                            }
                        }
                        font.bold: true
                    }
                }
            }
        }

        ScrollView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            TextArea {
                id: outputArea
                readOnly: true
                font.family: "Menlo"
                font.pixelSize: 11
                wrapMode: TextEdit.NoWrap
                placeholderText: qsTr("Stdout / stderr will stream here.")
            }
        }
    }
}
