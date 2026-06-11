import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// One UML compartment node: header (name + stage), a parameter compartment, and
// a steps compartment. Each param/step value is double-click-to-edit; accepting
// an edit emits `edit(fieldHub, newValue, oldValue)` (the host applies it via
// the TGG backward-cascade edit and re-renders).
Rectangle {
    id: node

    property string hub: ""
    property var job: ({})
    property bool selected: false
    signal clicked()
    signal edit(string fieldHub, string newValue, string oldValue)

    radius: 9
    color: "#ffffff"
    border.color: selected ? "#3b82f6" : "#cdd2db"
    border.width: selected ? 2 : 1.25

    MouseArea {
        anchors.fill: parent
        z: -1
        onClicked: node.clicked()
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 0
        spacing: 0

        // Header
        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 12
            Layout.rightMargin: 12
            Layout.topMargin: 6
            Label {
                text: node.job.name || "(unnamed)"
                font.bold: true
                font.pixelSize: 13
                color: "#1b1f24"
                elide: Label.ElideRight
                Layout.fillWidth: true
            }
            Label {
                visible: !!node.job.stage
                text: node.job.stage || ""
                font.pixelSize: 11
                color: "#0e7490"
            }
        }

        // Parameter compartment
        Rectangle { visible: (node.job.params || []).length > 0; Layout.fillWidth: true; Layout.topMargin: 6; height: 1; color: "#e5e7eb" }
        Repeater {
            model: node.job.params || []
            delegate: EditableRow {
                Layout.fillWidth: true
                keyText: modelData.key + ": "
                valueText: modelData.value
                onAccepted: function(v) { node.edit(modelData.hub, v, modelData.value) }
            }
        }

        // Steps compartment
        Rectangle { visible: (node.job.steps || []).length > 0; Layout.fillWidth: true; Layout.topMargin: 6; height: 1; color: "#e5e7eb" }
        Repeater {
            model: node.job.steps || []
            delegate: EditableRow {
                Layout.fillWidth: true
                keyText: (index + 1) + " "
                valueText: modelData.value
                mono: true
                onAccepted: function(v) { node.edit(modelData.hub, v, modelData.value) }
            }
        }

        Item { Layout.fillHeight: true }
    }
}
