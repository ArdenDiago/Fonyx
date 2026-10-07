// Shared color palette, ported 1:1 from ui/index.html's :root CSS custom
// properties, so this app and the Tauri UI read as one product.
import QtQuick

QtObject {
    readonly property color void_: "#131317"
    readonly property color panel: "#1b1b21"
    readonly property color panel2: "#202027"
    readonly property color inset: "#0c0c10"
    readonly property color border: "#2c2c35"
    readonly property color text: "#e8e8ec"
    readonly property color textDim: "#8a8a96"
    readonly property color textFaint: "#55555f"
    readonly property color accent: "#3ecbe0"
    readonly property color accentDim: "#1c5c66"
    readonly property color danger: "#e8564f"
    readonly property color success: "#4fd67a"
    readonly property color warn: "#e0b04d"
    readonly property var bandColors: ["#3ecbe0", "#e0b04d", "#e058b0", "#7ee858", "#a06be8", "#e8584f"]
}
