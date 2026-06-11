import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// One double-click-to-edit line inside a JobNode compartment: a muted key label
// and a value that swaps to a TextField on double-click. Accepting emits
// `accepted(newValue)`.
RowLayout {
    id: row

    property string keyText: ""
    property string valueText: ""
    property bool mono: false
    property bool editing: false
    signal accepted(string newValue)

    Layout.leftMargin: 12
    Layout.rightMargin: 12
    spacing: 6

    Label {
        text: row.keyText
        color: "#6b7280"
        font.pixelSize: 11
        Layout.alignment: Qt.AlignTop
    }

    Label {
        visible: !row.editing
        text: row.valueText
        color: "#1b1f24"
        font.pixelSize: 11
        font.family: row.mono ? "Menlo" : Qt.application.font.family
        elide: Label.ElideRight
        Layout.fillWidth: true
        MouseArea {
            anchors.fill: parent
            onDoubleClicked: row.editing = true
        }
    }

    TextField {
        visible: row.editing
        text: row.valueText
        font.pixelSize: 11
        font.family: row.mono ? "Menlo" : Qt.application.font.family
        Layout.fillWidth: true
        onVisibleChanged: if (visible) { forceActiveFocus(); selectAll() }
        onEditingFinished: {
            row.editing = false
            if (text !== row.valueText) row.accepted(text)
        }
        Keys.onEscapePressed: { text = row.valueText; row.editing = false }
    }
}
