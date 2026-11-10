// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Définition de la ligne de commande (clap).

use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

/// Bus D-Bus à utiliser pour tccd (développement).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BusKind {
    /// Bus système (production).
    System,
    /// Bus de session (`tccd --dev-session`).
    Session,
}

/// Interrupteur on/off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Toggle {
    /// Activer.
    On,
    /// Désactiver.
    Off,
    /// Lire l'état.
    Status,
}

/// TuxedoRustCenter en ligne de commande.
#[derive(Debug, Parser)]
#[command(name = "tcc-cli", version, about, propagate_version = true)]
pub struct Cli {
    /// Sortie JSON compacte (une ligne), pour scripts et widgets.
    #[arg(long, short = 'j', global = true)]
    pub json: bool,
    /// Bus D-Bus de tccd (défaut : `TCC_DBUS_BUS`, sinon système).
    #[arg(long, global = true, value_enum)]
    pub bus: Option<BusKind>,
    /// Commande (défaut : `status`).
    #[command(subcommand)]
    pub command: Option<Cmd>,
}

/// Commandes.
#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// État courant : profil actif, PPD, alimentation.
    #[command(alias = "state")]
    Status,
    /// Le daemon répond-il ?
    Ping,
    /// Version du daemon.
    #[command(name = "daemon-version")]
    DaemonVersion,
    /// Collection complète de profils (JSON).
    Profiles,
    /// Ancien nom de `profile use`.
    #[command(name = "set-temp", hide = true)]
    SetTemp {
        /// Id ou nom.
        profile: String,
    },
    /// Ancien nom de `profile clear`.
    #[command(name = "clear-temp", hide = true)]
    ClearTemp,
    /// Lister, afficher, activer, créer, modifier, supprimer des profils.
    #[command(subcommand)]
    Profile(ProfileCmd),
    /// Afficher et modifier les réglages globaux.
    #[command(subcommand)]
    Settings(SettingsCmd),
    /// Ventilateurs.
    #[command(alias = "fan")]
    Fans,
    /// Snapshot matériel complet.
    #[command(alias = "hw")]
    Hardware,
    /// Minuterie d'extinction.
    #[command(alias = "shutdown-timer")]
    Timer,
    /// Collecte des capteurs (on|off|status).
    #[command(alias = "sensor")]
    Sensors {
        /// Action (défaut : status).
        #[arg(value_enum)]
        action: Option<Toggle>,
    },
    /// Résumé compact : profil, charge CPU, puissance CPU, batterie.
    Stats {
        /// Ne pas demander au daemon d'activer les capteurs.
        #[arg(long)]
        no_sensors: bool,
    },
    /// Charge de la batterie.
    #[command(subcommand)]
    Charging(ChargingCmd),
    /// Éclairage clavier : JSON des zones, `@fichier` ou `-`.
    Keyboard {
        /// `Vec<KeyboardState>` en JSON.
        states: String,
    },
    /// Lightbar : `off`, ou JSON / `@fichier` / `-`.
    Lightbar {
        /// État.
        #[arg(value_parser = parse_lightbar)]
        state: LightbarArg,
    },
    /// Verrou Fn.
    #[command(name = "fn-lock")]
    FnLock {
        /// on ou off.
        #[arg(value_enum)]
        state: Toggle,
    },
    /// Limites TDP (service admin matériel).
    #[command(subcommand)]
    Tdp(TdpCmd),
    /// Profil ODM (service admin matériel).
    Odm {
        /// Nom du profil ODM.
        name: String,
        /// Utiliser SetOdmProfileOverride (consentement requis).
        #[arg(long)]
        r#override: bool,
    },
    /// Offset cTGP NVIDIA en watts (service admin matériel).
    Ctgp {
        /// Offset en watts.
        #[arg(allow_hyphen_values = true)]
        offset: i32,
    },
    /// Écrans et modes (agent de session).
    Display,
    /// Écrire un contrôle UVC de la webcam (agent de session).
    Uvc {
        /// Chemin du périphérique (`/dev/videoN`).
        device: String,
        /// Clé du contrôle.
        key: String,
        /// Valeur.
        #[arg(allow_hyphen_values = true)]
        value: i32,
    },
    /// Service administratif : ping, version, contrat.
    Admin,
    /// Recharger la configuration (service admin).
    Reload,
    /// Générer l'autocomplétion du shell.
    Completions {
        /// bash, zsh, fish, elvish, powershell.
        shell: Shell,
    },
}

/// Argument de `lightbar`.
#[derive(Debug, Clone)]
pub enum LightbarArg {
    /// Éteindre (`null`).
    Off,
    /// JSON, `@fichier` ou `-`.
    Json(String),
}

fn parse_lightbar(raw: &str) -> Result<LightbarArg, String> {
    Ok(if raw.eq_ignore_ascii_case("off") {
        LightbarArg::Off
    } else {
        LightbarArg::Json(raw.to_owned())
    })
}

/// Sous-commandes de `profile`.
#[derive(Debug, Subcommand)]
pub enum ProfileCmd {
    /// Collection complète en JSON (identique à `profiles`).
    #[command(hide = true)]
    Dump,
    /// Lister les profils (`*` = actif).
    #[command(alias = "ls")]
    List,
    /// Afficher un profil (id ou nom).
    Show {
        /// Id ou nom.
        profile: String,
    },
    /// Profil actif résolu.
    Active,
    /// Activer un profil temporairement (jusqu'au prochain changement d'état).
    #[command(alias = "set-temp")]
    Use {
        /// Id ou nom.
        profile: String,
    },
    /// Supprimer l'override temporaire.
    #[command(alias = "clear-temp")]
    Clear,
    /// Créer un profil par défaut.
    Create {
        /// Nom.
        name: String,
    },
    /// Copier un profil (preset inclus).
    Copy {
        /// Profil source (id ou nom).
        from: String,
        /// Nom de la copie.
        name: String,
    },
    /// Supprimer un profil personnalisé.
    #[command(alias = "rm")]
    Delete {
        /// Id ou nom.
        profile: String,
    },
    /// Renommer un profil personnalisé.
    Rename {
        /// Id ou nom.
        profile: String,
        /// Nouveau nom.
        name: String,
    },
    /// Lire un champ : `display.brightness`, `cpu.governor`...
    Get {
        /// Id ou nom.
        profile: String,
        /// Chemin pointé.
        key: String,
    },
    /// Modifier un champ : `profile set Jeu display.brightness 80`.
    Set {
        /// Id ou nom.
        profile: String,
        /// Chemin pointé (camelCase, comme le JSON).
        key: String,
        /// Valeur JSON (`80`, `true`, `null`) ou texte.
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    /// Exporter un profil en JSON (stdout si pas de fichier).
    Export {
        /// Id ou nom.
        profile: String,
        /// Fichier de sortie.
        file: Option<String>,
    },
    /// Importer un profil JSON (fichier ou `-`) ; un nouvel id est attribué.
    Import {
        /// Fichier JSON.
        file: String,
    },
}

/// Sous-commandes de `settings`.
#[derive(Debug, Subcommand)]
pub enum SettingsCmd {
    /// Tous les réglages (JSON).
    Show,
    /// Lire un réglage : `fahrenheit`, `stateMap.power_ac`...
    Get {
        /// Chemin pointé.
        key: String,
    },
    /// Modifier un réglage : `settings set fahrenheit true`.
    Set {
        /// Chemin pointé.
        key: String,
        /// Valeur JSON ou texte.
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
}

/// Sous-commandes de `charging`.
#[derive(Debug, Subcommand)]
pub enum ChargingCmd {
    /// État de charge (domaine `charging` du snapshot).
    Show,
    /// Profil de charge.
    Profile {
        /// Nom.
        name: String,
    },
    /// Priorité de charge.
    Priority {
        /// Nom.
        name: String,
    },
    /// Seuil de début de charge (0-100).
    Start {
        /// Pourcentage.
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },
    /// Seuil de fin de charge (0-100).
    End {
        /// Pourcentage.
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },
    /// Type de charge (Standard, Fast, Custom...).
    Type {
        /// Nom.
        name: String,
    },
}

/// Sous-commandes de `tdp`.
#[derive(Debug, Subcommand)]
pub enum TdpCmd {
    /// Valeurs TDP courantes.
    Get,
    /// Définir 1 à 3 limites en watts.
    Set {
        /// Watts (PL1 [PL2 [PL4]]).
        #[arg(required = true, num_args = 1..=3)]
        watts: Vec<i32>,
    },
    /// Revenir aux maxima firmware.
    Reset,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn definition_is_consistent() {
        Cli::command().debug_assert();
    }

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("tcc-cli").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn default_is_status() {
        assert!(parse(&[]).command.is_none());
    }

    #[test]
    fn legacy_commands_still_parse() {
        assert!(matches!(
            parse(&["set-temp", "x"]).command,
            Some(Cmd::SetTemp { .. })
        ));
        assert!(matches!(
            parse(&["clear-temp"]).command,
            Some(Cmd::ClearTemp)
        ));
        assert!(matches!(parse(&["fan"]).command, Some(Cmd::Fans)));
        assert!(matches!(parse(&["hw"]).command, Some(Cmd::Hardware)));
        assert!(matches!(
            parse(&["sensors", "on"]).command,
            Some(Cmd::Sensors {
                action: Some(Toggle::On)
            })
        ));
    }

    #[test]
    fn profile_use_aliases() {
        assert!(matches!(
            parse(&["profile", "use", "x"]).command,
            Some(Cmd::Profile(ProfileCmd::Use { .. }))
        ));
        assert!(matches!(
            parse(&["profile", "set-temp", "x"]).command,
            Some(Cmd::Profile(ProfileCmd::Use { .. }))
        ));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(Cli::try_parse_from(["tcc-cli", "charging", "end", "101"]).is_err());
        assert!(Cli::try_parse_from(["tcc-cli", "tdp", "set"]).is_err());
        assert!(Cli::try_parse_from(["tcc-cli", "tdp", "set", "1", "2", "3", "4"]).is_err());
        assert!(Cli::try_parse_from(["tcc-cli", "nope"]).is_err());
    }

    #[test]
    fn negative_values_are_accepted() {
        assert!(Cli::try_parse_from(["tcc-cli", "ctgp", "-5"]).is_ok());
        assert!(
            Cli::try_parse_from([
                "tcc-cli",
                "profile",
                "set",
                "a",
                "display.refreshRate",
                "-1"
            ])
            .is_ok()
        );
    }
}
