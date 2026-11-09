// SPDX-License-Identifier: GPL-3.0-or-later
//
// Build script cxx-qt : lie Qt 6, enregistre le module QML « Tcc » et génère
// les ponts Rust/QObject. Les fichiers QML restent chargés depuis le système
// de fichiers pour permettre l'itération sans recompilation.

fn main() {
    cxx_qt_build::CxxQtBuilder::new_qml_module(cxx_qt_build::QmlModule::new("Tcc"))
        .qt_module("Core")
        .qt_module("Gui")
        .qt_module("Quick")
        .qt_module("QuickControls2")
        // Qt Labs Platform (tray/menus natifs) exige Qt Widgets/QApplication.
        .qt_module("Widgets")
        .cpp_file("src/qapplication_shim.cpp")
        .file("src/daemon_client.rs")
        .build();

    // Certains éditeurs de liens (binutils récents, Fedora) appliquent
    // `-Wl,--as-needed` avant les archives générées par cxx-qt : `-lQt6Widgets`
    // est alors éliminé et `QApplication` reste non résolu. Déclarer la
    // bibliothèque ici la place en fin de ligne de commande, après les objets.
    println!("cargo:rustc-link-lib=dylib=Qt6Widgets");
}
