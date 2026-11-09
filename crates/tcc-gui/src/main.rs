// SPDX-License-Identifier: GPL-3.0-or-later
//
// Point d'entrée Qt 6 natif de TuxedoRustCenter.

// cxx-qt génère de la glue C++/Rust qui contient de l'`unsafe` ; il est
// confiné au module `daemon_client`. Le code écrit à la main reste sans unsafe.
#![deny(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
//
// Le shell QML est chargé par QQmlApplicationEngine. Aucune logique métier
// n'est exécutée ici.
//
// # Overrides de développement (D7 / R-14)
//
// `TCC_QML_DIR`, `TCC_DBUS_SERVICE` et `TCC_DBUS_BUS` ne sont lus que si la
// feature Cargo `dev-overrides` est active. Sans elle, le binaire utilise
// `/usr/share/tuxedo-rust-center/qml` et `com.tuxedocomputers.tccd` sur le bus
// système : une variable d'environnement ne peut pas détourner un binaire
// release vers un QML ou un service arbitraire.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use cxx_qt_lib::{QQmlApplicationEngine, QQuickStyle, QString, QUrl};

#[allow(unsafe_code, clippy::unwrap_used, clippy::expect_used)]
mod daemon_client;
mod display_view;

mod autostart;
mod fan_view;
mod hardware_view;
mod power_blocker;
mod profile_edit;
mod profile_transfer;
mod settings_edit;

const APP_NAME: &str = "tuxedo-rust-center";
const APP_DISPLAY_NAME: &str = "TUXEDO Control Center";
const ORG_NAME: &str = "TuxedoRustCenter";
const ORG_DOMAIN: &str = "tuxedorustcenter.local";
/// Répertoire QML installé (FHS).
const INSTALLED_QML_DIR: &str = "/usr/share/tuxedo-rust-center/qml";

/// QApplication minimale via une glue C++ (Qt Labs Platform / tray natif).
#[allow(unsafe_code)]
mod qapp {
    use std::ffi::{CString, c_void};

    unsafe extern "C" {
        fn tcc_qapplication_new() -> *mut c_void;
        fn tcc_qapplication_exec(app: *mut c_void) -> i32;
        fn tcc_qapplication_set_identity(
            app: *mut c_void,
            name: *const std::ffi::c_char,
            display: *const std::ffi::c_char,
            version: *const std::ffi::c_char,
            organization: *const std::ffi::c_char,
            domain: *const std::ffi::c_char,
        );
        fn tcc_qapplication_delete(app: *mut c_void);
    }

    /// QApplication possédée par le thread principal.
    pub struct Application(*mut c_void);

    impl Application {
        /// Crée l'application (arguments factices ; les options sont déjà
        /// analysées en Rust).
        pub fn new() -> Self {
            // `QApplication` termine le processus si aucun backend graphique
            // n'est disponible ; aucun pointeur nul n'est attendu ici.
            Self(unsafe { tcc_qapplication_new() })
        }

        /// Renseigne l'identité utilisée par QSettings et les dialogues.
        pub fn set_identity(
            &self,
            name: &str,
            display: &str,
            version: &str,
            organization: &str,
            domain: &str,
        ) {
            let name = CString::new(name).unwrap_or_default();
            let display = CString::new(display).unwrap_or_default();
            let version = CString::new(version).unwrap_or_default();
            let organization = CString::new(organization).unwrap_or_default();
            let domain = CString::new(domain).unwrap_or_default();
            unsafe {
                tcc_qapplication_set_identity(
                    self.0,
                    name.as_ptr(),
                    display.as_ptr(),
                    version.as_ptr(),
                    organization.as_ptr(),
                    domain.as_ptr(),
                );
            }
        }

        /// Boucle d'événements Qt.
        pub fn exec(&self) -> i32 {
            unsafe { tcc_qapplication_exec(self.0) }
        }
    }

    impl Drop for Application {
        fn drop(&mut self) {
            unsafe { tcc_qapplication_delete(self.0) };
        }
    }
}

/// Mode de démarrage demandé en ligne de commande.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchMode {
    /// Fenêtre visible.
    Window,
    /// Icône tray si disponible, sinon fenêtre visible (repli QML).
    Tray,
}

/// Analyse les arguments (sans le nom du programme).
///
/// Seul `--tray` est accepté ; tout autre argument est une erreur explicite
/// (pas d'analyse permissive d'options inconnues).
fn parse_launch_mode<I>(arguments: I) -> Result<LaunchMode, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut mode = LaunchMode::Window;
    for argument in arguments {
        match argument.to_str() {
            Some("--tray") => mode = LaunchMode::Tray,
            Some(other) => return Err(format!("argument inconnu : {other}")),
            None => return Err("argument non UTF-8".to_owned()),
        }
    }
    Ok(mode)
}

fn main() {
    let launch_mode = match parse_launch_mode(std::env::args_os().skip(1)) {
        Ok(mode) => mode,
        Err(error) => {
            eprintln!("tcc-gui: {error}");
            eprintln!("usage: tuxedo-rust-center [--tray]");
            std::process::exit(2);
        }
    };
    let start_in_tray = launch_mode == LaunchMode::Tray;

    let app = qapp::Application::new();
    app.set_identity(
        APP_NAME,
        APP_DISPLAY_NAME,
        env!("CARGO_PKG_VERSION"),
        ORG_NAME,
        ORG_DOMAIN,
    );

    // Style de base : tous les contrôles visibles sont redéfinis dans
    // qml/components. Doit être configuré avant le chargement du QML.
    QQuickStyle::set_style(&QString::from("Basic"));

    let qml = qml_directory();
    let main_qml = qml.join("Main.qml");
    if !main_qml.is_file() {
        eprintln!(
            "tcc-gui: Main.qml introuvable dans {} (paquet incomplet ?)",
            qml.display()
        );
        std::process::exit(1);
    }

    let mut engine = QQmlApplicationEngine::new();
    // Propriété initiale du root QML : la fenêtre démarre masquée si `--tray`
    // est demandé ; le QML la rend visible si le tray n'est pas disponible.
    let mut initial = cxx_qt_lib::QMap::<cxx_qt_lib::QMapPair_QString_QVariant>::default();
    initial.insert(
        QString::from("startInTray"),
        cxx_qt_lib::QVariant::from(&start_in_tray),
    );
    engine.pin_mut().set_initial_properties(&initial);
    let url = QUrl::from_local_file(&QString::from(main_qml.to_string_lossy().as_ref()));
    engine.pin_mut().load(&url);

    let code = app.exec();
    drop(app);
    std::process::exit(code);
}

/// Répertoire QML.
///
/// Sans `dev-overrides` : uniquement le chemin installé. En build de
/// développement (`debug_assertions`), l'arbre source sert de repli pour
/// `cargo run` — chemin fixé à la compilation, jamais contrôlé par
/// l'environnement.
fn qml_directory() -> PathBuf {
    #[cfg(feature = "dev-overrides")]
    if let Some(path) = std::env::var_os("TCC_QML_DIR") {
        return PathBuf::from(path);
    }

    let installed = Path::new(INSTALLED_QML_DIR);
    if installed.is_dir() {
        return installed.to_path_buf();
    }

    #[cfg(debug_assertions)]
    {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("qml")
    }

    #[cfg(not(debug_assertions))]
    {
        installed.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::{LaunchMode, parse_launch_mode};
    use std::ffi::OsString;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn launch_mode_defaults_to_window() {
        assert_eq!(
            parse_launch_mode(args(&[])).expect("défaut"),
            LaunchMode::Window
        );
    }

    #[test]
    fn tray_flag_is_parsed() {
        assert_eq!(
            parse_launch_mode(args(&["--tray"])).expect("tray"),
            LaunchMode::Tray
        );
        // Doublon accepté (idempotent).
        assert_eq!(
            parse_launch_mode(args(&["--tray", "--tray"])).expect("tray doublon"),
            LaunchMode::Tray
        );
    }

    #[test]
    fn unknown_arguments_are_rejected() {
        for values in [&["--hidden"][..], &["--tray", "extra"][..], &["-h"][..]] {
            assert!(
                parse_launch_mode(args(values)).is_err(),
                "arguments: {values:?}"
            );
        }
    }
}
