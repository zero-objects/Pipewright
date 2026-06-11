import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import QtCore

Dialog {
    id: dialog
    title: qsTr("Settings")
    modal: true
    standardButtons: Dialog.Ok | Dialog.Cancel
    width: 540

    FileDialog {
        id: recipesConfigDialog
        title: qsTr("Choose recipe-sources config")
        nameFilters: [qsTr("YAML files (*.yml *.yaml)"), qsTr("All files (*)")]
        onAccepted: recipesConfigField.text = selectedFile.toString().replace(/^file:\/\//, "")
    }

    // Aliases let Main.qml bind RunView/MigrationView/etc. directly to
    // persisted values without each tab re-instantiating its own
    // Settings { ... } block (which would double-write and race).
    property alias cliPath:        settings.cliPath
    property alias cacheDir:       settings.cacheDir
    property alias dockerHost:     settings.dockerHost
    property alias recipesConfig:  settings.recipesConfig

    // Persisted across sessions via QSettings. On macOS the values
    // live in ~/Library/Preferences/org.zero-principle.pipewright.plist; on
    // Linux $XDG_CONFIG_HOME/ZeroObjects/Pipewright.conf.
    Settings {
        id: settings
        category: "general"
        property string cliPath: ""
        property string cacheDir: ""
        property string dockerHost: ""
        property string recipesConfig: ""
    }

    onAccepted: {
        settings.cliPath = cliField.text
        settings.cacheDir = cacheField.text
        settings.dockerHost = dockerField.text
        settings.recipesConfig = recipesConfigField.text
    }

    onOpened: {
        cliField.text             = settings.cliPath
        cacheField.text           = settings.cacheDir
        dockerField.text          = settings.dockerHost
        recipesConfigField.text   = settings.recipesConfig
    }

    contentItem: ColumnLayout {
        spacing: 10

        GroupBox {
            title: qsTr("CLI")
            Layout.fillWidth: true
            ColumnLayout {
                anchors.fill: parent
                RowLayout {
                    Label { text: qsTr("pipeline-cli path:"); Layout.preferredWidth: 160 }
                    TextField {
                        id: cliField
                        placeholderText: qsTr("(use PATH)")
                        Layout.fillWidth: true
                    }
                }
                RowLayout {
                    Label { text: qsTr("Cache dir:"); Layout.preferredWidth: 160 }
                    TextField {
                        id: cacheField
                        placeholderText: qsTr("(use system default)")
                        Layout.fillWidth: true
                    }
                }
            }
        }

        GroupBox {
            title: qsTr("Recipes")
            Layout.fillWidth: true
            ColumnLayout {
                anchors.fill: parent
                RowLayout {
                    Label { text: qsTr("Sources config:"); Layout.preferredWidth: 160 }
                    TextField {
                        id: recipesConfigField
                        placeholderText: qsTr("(standard library only)")
                        Layout.fillWidth: true
                    }
                    Button {
                        text: qsTr("Browse…")
                        onClicked: recipesConfigDialog.open()
                    }
                }
                Label {
                    text: qsTr("A YAML file listing extra recipe sources (local dirs or git repos). Git sources are cloned into the cache dir above. See recipes-sources.example.yml.")
                    wrapMode: Label.WordWrap
                    Layout.fillWidth: true
                    color: "#666"
                    font.pixelSize: 10
                }
            }
        }

        GroupBox {
            title: qsTr("Local runner")
            Layout.fillWidth: true
            ColumnLayout {
                anchors.fill: parent
                RowLayout {
                    Label { text: qsTr("Docker host:"); Layout.preferredWidth: 160 }
                    TextField {
                        id: dockerField
                        placeholderText: qsTr("unix:///var/run/docker.sock")
                        Layout.fillWidth: true
                    }
                }
                Label {
                    text: qsTr("Passed to the local runner as DOCKER_HOST. Leave empty to use the default socket (Docker Desktop's user socket is found automatically).")
                    wrapMode: Label.WordWrap
                    Layout.fillWidth: true
                    color: "#666"
                    font.pixelSize: 10
                }
            }
        }
    }
}
