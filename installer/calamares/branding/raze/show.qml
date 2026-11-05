import QtQuick 2.0;
import calamares.slideshow 1.0;

Presentation {
    id: presentation

    function nextSlide() { presentation.goToNextSlide(); }
    Timer { interval: 9000; running: presentation.activatedInCalamares; repeat: true; onTriggered: nextSlide() }

    function onActivate() { }
    function onLeave() { }

    Rectangle { anchors.fill: parent; color: "#0f0f10"; z: -1 }

    Slide {
        Image { id: logo; source: "logo-256.png"; width: 160; height: 160; anchors.horizontalCenter: parent.horizontalCenter; y: parent.height * 0.12 }
        Text {
            anchors.top: logo.bottom; anchors.topMargin: 24; anchors.horizontalCenter: parent.horizontalCenter
            width: parent.width * 0.8; horizontalAlignment: Text.AlignHCenter; wrapMode: Text.WordWrap
            color: "#e4e4e7"; font.pixelSize: 20
            text: "<b>Raze Linux</b><br/>Rust-Powered. Performance-Driven. Minimal by Design."
        }
    }
    Slide {
        Text {
            anchors.centerIn: parent; width: parent.width * 0.8; wrapMode: Text.WordWrap; color: "#e4e4e7"; font.pixelSize: 17
            text: "<b>Declarative and reproducible.</b><br/><br/>Your whole system is described in /etc/nixos. Every change creates a new generation, and you can roll back from the boot menu or with <tt>rix rollback</tt>."
        }
    }
    Slide {
        Text {
            anchors.centerIn: parent; width: parent.width * 0.8; wrapMode: Text.WordWrap; color: "#e4e4e7"; font.pixelSize: 17
            text: "<b>Rix package manager.</b><br/><br/><tt>rix search</tt>, <tt>rix install</tt>, <tt>rix remove</tt> and <tt>rix generations</tt> work on the full nixpkgs collection, with binary caches and atomic upgrades."
        }
    }
    Slide {
        Text {
            anchors.centerIn: parent; width: parent.width * 0.8; wrapMode: Text.WordWrap; color: "#e4e4e7"; font.pixelSize: 17
            text: "<b>Keyboard first.</b><br/><br/>Super+Enter opens a terminal, Super+D the launcher, Super+Escape the power menu, Super+Shift+/ shows every shortcut."
        }
    }
}
