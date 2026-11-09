// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter. Les valeurs numériques des
// presets sont reprises de DefaultProfiles.ts de TUXEDO Control Center
// (GPL-3.0-or-later) pour conserver le comportement de départ. Le code de
// construction est une réimplémentation originale.

//! Profils presets et profil personnalisé par défaut.
//!
//! Le daemon doit toujours pouvoir résoudre la matrice `(PPD, alimentation)`.
//! Ce module fournit les cinq profils de référence de TCC :
//!
//! | ID | Nom lisible |
//! |---|---|
//! | `__profile_max_energy_save__` | Max Energy Save |
//! | `__profile_silent__` | Quiet |
//! | `__office__` | Office |
//! | `__high_performance__` | High Performance |
//! | `__default_custom_profile__` | TUXEDO Defaults |
//!
//! Les identifiants sont **exactement** ceux de l'original, car ils sont
//! persistés dans `stateMap` et `stateMatrix`. Les noms affichés, eux, sont
//! lisibles : l'original utilisait l'identifiant comme nom, ce qui donne
//! `__profile_max_energy_save__` dans l'UI.
//!
//! Les valeurs génériques (luminosité, courbe de ventilateur, profil ODM,
//! TDP) proviennent de `DefaultProfiles.ts`. Elles ne prétendent pas couvrir
//! les quirks de chaque modèle TUXEDO : `tcc-hw` et `tcc-daemon` affinent
//! ensuite selon le DMI et les capacités détectées.
//!
//! # Fusion
//!
//! [`merge_with_default_profiles`] garantit que les cinq identifiants sont
//! présents sans jamais écraser un profil déjà chargé. Un profil utilisateur
//! qui porterait le même identifiant qu'un preset est conservé tel quel
//! (personnalisation sur place) ; les presets manquants sont ajoutés à la fin.
//! Les doublons d'identifiant déjà présents dans l'entrée sont éliminés en
//! gardant la première occurrence.
//!
//! ## Unicité des noms (BUG-QA-002)
//!
//! L'**identifiant** est la clé d'unicité fonctionnelle : il est persisté dans
//! `stateMap` et `stateMatrix`, et c'est lui que le moteur d'état résout.
//! [`crate::ProfileCollection::validate`] exige toutefois des noms uniques
//! (casse et espaces de bord ignorés), et la GUI rejette un payload invalide.
//!
//! Décision : la fusion conserve tous les profils et **renomme le profil
//! chargé** en conflit avec un suffixe explicite (`Office` → `Office (custom)`,
//! puis `Office (custom 2)`), sans jamais toucher à l'identifiant ni aux
//! presets, qui gardent leur nom canonique. Renommer plutôt que rejeter
//! préserve les références `stateMap`/`stateMatrix` et l'UX (aucun profil ne
//! disparaît) ; le daemon doit journaliser les notes de fusion pour expliquer
//! le changement. Voir `docs/rust/INTEGRATION.md` §3.

use std::collections::BTreeSet;

use crate::fan::FanCurve;
use crate::profile::{
    DEFAULT_CUSTOM_PROFILE_ID, DisplayProfile, HIGH_PERFORMANCE_PROFILE_ID,
    MAX_ENERGY_SAVE_PROFILE_ID, OFFICE_PROFILE_ID, OdmPowerLimits, OdmProfile, Profile,
    ProfileCollection, ProfileId, QUIET_PROFILE_ID,
};
use crate::validate::{MAX_PROFILE_NAME_LEN, normalized_name_key};

/// Identifiants des quatre presets fournis par TCC, dans l'ordre d'affichage.
pub const PRESET_PROFILE_IDS: [&str; 4] = [
    MAX_ENERGY_SAVE_PROFILE_ID,
    QUIET_PROFILE_ID,
    OFFICE_PROFILE_ID,
    HIGH_PERFORMANCE_PROFILE_ID,
];

/// Identifiants des quatre presets et du profil personnalisé par défaut.
pub const DEFAULT_PROFILE_IDS: [&str; 5] = [
    MAX_ENERGY_SAVE_PROFILE_ID,
    QUIET_PROFILE_ID,
    OFFICE_PROFILE_ID,
    HIGH_PERFORMANCE_PROFILE_ID,
    DEFAULT_CUSTOM_PROFILE_ID,
];

/// Spécification interne commune aux cinq profils.
struct PresetSpec {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    brightness: u8,
    use_brightness: bool,
    refresh_rate: Option<f32>,
    resolution: Option<(u32, u32)>,
    fan_profile: &'static str,
    odm_profile: Option<&'static str>,
    tdp_values: &'static [f64],
}

fn from_spec(spec: PresetSpec) -> Profile {
    let mut profile = Profile::new(spec.id, spec.name);
    profile.description = spec.description.to_owned();
    let (x_resolution, y_resolution) = match spec.resolution {
        Some((width, height)) => (Some(width), Some(height)),
        None => (None, None),
    };
    profile.display = DisplayProfile {
        brightness: spec.brightness,
        use_brightness: spec.use_brightness,
        refresh_rate: spec.refresh_rate,
        use_refresh_rate: false,
        x_resolution,
        y_resolution,
        use_resolution: false,
    };
    profile.fan.fan_profile = spec.fan_profile.to_owned();
    profile.fan.custom_fan_curve = FanCurve::default_custom();
    profile.odm_profile = OdmProfile {
        name: spec.odm_profile.map(str::to_owned),
    };
    profile.odm_power_limits = OdmPowerLimits {
        tdp_values: spec.tdp_values.to_vec(),
    };
    profile
}

/// Profil preset « Max Energy Save ».
///
/// Luminosité 40 %, ventilateur `Silent`, ODM `power_save`, TDP 5/10/15 W.
/// `refreshRate` vaut 60 et la résolution 1920×1080 comme dans l'original,
/// mais `useRefRate`/`useResolution` restent faux : ces valeurs ne sont pas
/// appliquées.
#[must_use]
pub fn max_energy_save_profile() -> Profile {
    from_spec(PresetSpec {
        id: MAX_ENERGY_SAVE_PROFILE_ID,
        name: "Max Energy Save",
        description: "",
        brightness: 40,
        use_brightness: true,
        refresh_rate: Some(60.0),
        resolution: Some((1920, 1080)),
        fan_profile: "Silent",
        odm_profile: Some("power_save"),
        tdp_values: &[5.0, 10.0, 15.0],
    })
}

/// Profil preset « Quiet ».
///
/// Luminosité 50 %, ventilateur `Silent`, ODM `power_save`, TDP 10/15/25 W.
#[must_use]
pub fn quiet_profile() -> Profile {
    from_spec(PresetSpec {
        id: QUIET_PROFILE_ID,
        name: "Quiet",
        description: "",
        brightness: 50,
        use_brightness: true,
        refresh_rate: None,
        resolution: None,
        fan_profile: "Silent",
        odm_profile: Some("power_save"),
        tdp_values: &[10.0, 15.0, 25.0],
    })
}

/// Profil preset « Office ».
///
/// Luminosité 60 %, ventilateur `Quiet`, ODM `enthusiast`, TDP 25/35/35 W.
#[must_use]
pub fn office_profile() -> Profile {
    from_spec(PresetSpec {
        id: OFFICE_PROFILE_ID,
        name: "Office",
        description: "",
        brightness: 60,
        use_brightness: true,
        refresh_rate: None,
        resolution: None,
        fan_profile: "Quiet",
        odm_profile: Some("enthusiast"),
        tdp_values: &[25.0, 35.0, 35.0],
    })
}

/// Profil preset « High Performance ».
///
/// Luminosité 60 %, ventilateur `Balanced`, ODM `overboost`,
/// TDP 60/60/70 W.
#[must_use]
pub fn high_performance_profile() -> Profile {
    from_spec(PresetSpec {
        id: HIGH_PERFORMANCE_PROFILE_ID,
        name: "High Performance",
        description: "",
        brightness: 60,
        use_brightness: true,
        refresh_rate: None,
        resolution: None,
        fan_profile: "Balanced",
        odm_profile: Some("overboost"),
        tdp_values: &[60.0, 60.0, 70.0],
    })
}

/// Profil personnalisé par défaut « TUXEDO Defaults ».
///
/// Luminosité 100 % non appliquée, ventilateur `Balanced`, aucun profil ODM,
/// aucune valeur TDP. C'est le profil de repli du moteur d'état.
#[must_use]
pub fn default_custom_profile() -> Profile {
    let mut profile = from_spec(PresetSpec {
        id: DEFAULT_CUSTOM_PROFILE_ID,
        name: "TUXEDO Defaults",
        description: "Edit profile to change behaviour",
        brightness: 100,
        use_brightness: false,
        refresh_rate: None,
        resolution: None,
        fan_profile: "Balanced",
        odm_profile: None,
        tdp_values: &[],
    });
    profile.fan.custom_fan_curve = FanCurve::default_custom();
    profile
}

/// Collection des cinq profils de référence.
///
/// L'ordre est celui de l'UI : les quatre presets, puis le profil
/// personnalisé par défaut.
#[must_use]
pub fn default_profiles() -> ProfileCollection {
    ProfileCollection::new(vec![
        max_energy_save_profile(),
        quiet_profile(),
        office_profile(),
        high_performance_profile(),
        default_custom_profile(),
    ])
}

/// Suffixe du premier renommage d'un profil chargé en conflit de nom.
const RENAMED_SUFFIX: &str = " (custom)";

/// Fusionne des profils chargés avec les cinq profils de référence.
///
/// - les profils chargés sont conservés dans leur ordre, y compris un profil
///   personnalisé qui réutilise l'identifiant d'un preset ;
/// - les presets et le profil par défaut absents sont ajoutés à la fin ;
/// - les doublons d'identifiant de l'entrée sont éliminés : la première
///   occurrence gagne ;
/// - les collisions de **nom** sont résolues en renommant le profil chargé
///   (`Office` → `Office (custom)`), jamais le preset ; l'identifiant reste
///   inchangé. La collection retournée a donc des noms uniques au sens de
///   [`crate::ProfileCollection::validate`], tant que les profils d'entrée
///   étaient eux-mêmes valides.
///
/// Le renommage est déterministe et idempotent : refusionner le résultat ne
/// change plus les noms.
///
/// La fonction ne valide pas les autres champs : `tcc-daemon` doit appeler
/// [`ProfileCollection::validate`] après la fusion, puis
/// [`crate::validate::validate_settings_references`].
#[must_use]
pub fn merge_with_default_profiles(loaded: ProfileCollection) -> ProfileCollection {
    let presets = default_profiles();

    // Identifiants déjà fournis par l'entrée : les presets correspondants ne
    // seront pas ajoutés, leur nom n'est donc pas réservé.
    let loaded_ids: BTreeSet<&ProfileId> = loaded.ids().collect();

    // Noms des presets qui vont réellement être ajoutés. Ce sont les seuls
    // noms canoniques à protéger : un preset personnalisé sur place garde son
    // nom et n'entre pas en collision avec lui-même.
    let reserved_names: BTreeSet<String> = presets
        .iter()
        .filter(|preset| !loaded_ids.contains(&preset.id))
        .map(|preset| normalized_name_key(&preset.name))
        .collect();

    let mut merged = ProfileCollection::default();
    let mut taken_names: BTreeSet<String> = BTreeSet::new();

    for mut profile in loaded {
        // Un identifiant déjà fusionné gagne : la première occurrence est
        // conservée (contrat historique de la fonction).
        if merged.contains(&profile.id) {
            continue;
        }

        let key = normalized_name_key(&profile.name);
        // Un nom vide est invalide et sera signalé par la validation ; le
        // renommer masquerait l'erreur au lieu de la remonter.
        if !key.is_empty() && (reserved_names.contains(&key) || taken_names.contains(&key)) {
            profile.name = free_profile_name(&profile.name, &reserved_names, &taken_names);
        }
        if !key.is_empty() {
            taken_names.insert(normalized_name_key(&profile.name));
        }
        merged.upsert(profile);
    }

    for preset in presets {
        if !merged.contains(&preset.id) {
            taken_names.insert(normalized_name_key(&preset.name));
            merged.upsert(preset);
        }
    }

    merged
}

/// Construit un nom libre en ajoutant ` (custom)`, puis ` (custom 2)`, etc.
///
/// Le nom est tronqué si nécessaire pour rester sous
/// [`MAX_PROFILE_NAME_LEN`] ; le suffixe n'est jamais coupé. Le résultat est
/// garanti absent de `reserved` et de `taken`.
fn free_profile_name(base: &str, reserved: &BTreeSet<String>, taken: &BTreeSet<String>) -> String {
    let mut index = 0_u32;
    loop {
        let suffix = if index == 0 {
            RENAMED_SUFFIX.to_owned()
        } else {
            format!(" (custom {index})")
        };
        let candidate = with_suffix(base, &suffix);
        let key = normalized_name_key(&candidate);
        if !reserved.contains(&key) && !taken.contains(&key) {
            return candidate;
        }
        index += 1;
    }
}

/// Concatène `base` et `suffix` en restant sous [`MAX_PROFILE_NAME_LEN`].
///
/// La troncature se fait sur des caractères Unicode entiers et supprime les
/// espaces de fin de la base pour éviter `Office  (custom)`.
fn with_suffix(base: &str, suffix: &str) -> String {
    let max_base = MAX_PROFILE_NAME_LEN.saturating_sub(suffix.chars().count());
    let mut truncated: String = base.chars().take(max_base).collect();
    truncated.truncate(truncated.trim_end().len());
    truncated.push_str(suffix);
    truncated
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        DEFAULT_PROFILE_IDS, PRESET_PROFILE_IDS, default_custom_profile, default_profiles,
        high_performance_profile, max_energy_save_profile, merge_with_default_profiles,
        office_profile, quiet_profile,
    };
    use crate::profile::{
        DEFAULT_CUSTOM_PROFILE_ID, HIGH_PERFORMANCE_PROFILE_ID, MAX_ENERGY_SAVE_PROFILE_ID,
        OFFICE_PROFILE_ID, Profile, ProfileCollection, ProfileId, QUIET_PROFILE_ID,
    };
    use crate::state::{
        LegacyStateMap, PowerSource, PpdProfile, ResolutionSource, StateKey, StateMatrix,
        StateResolver,
    };
    use crate::validate::{MAX_PROFILE_NAME_LEN, validate_settings_references};

    #[test]
    fn default_collection_contains_the_five_ids() {
        let profiles = default_profiles();
        assert_eq!(profiles.len(), 5);
        let ids: BTreeSet<&str> = profiles.ids().map(ProfileId::as_str).collect();
        for id in DEFAULT_PROFILE_IDS {
            assert!(ids.contains(id), "profil manquant: {id}");
        }
        assert_eq!(ids.len(), DEFAULT_PROFILE_IDS.len());
    }

    #[test]
    fn preset_ids_are_exactly_the_original_ones() {
        assert_eq!(
            PRESET_PROFILE_IDS,
            [
                "__profile_max_energy_save__",
                "__profile_silent__",
                "__office__",
                "__high_performance__"
            ]
        );
        assert_eq!(DEFAULT_CUSTOM_PROFILE_ID, "__default_custom_profile__");
    }

    #[test]
    fn preset_names_are_readable() {
        let profiles = default_profiles();
        let names: Vec<&str> = profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "Max Energy Save",
                "Quiet",
                "Office",
                "High Performance",
                "TUXEDO Defaults"
            ]
        );
    }

    #[test]
    fn default_collection_is_valid() {
        default_profiles().validate().unwrap();
    }

    #[test]
    fn generic_values_match_the_original_defaults() {
        let max = max_energy_save_profile();
        assert_eq!(max.id.as_str(), MAX_ENERGY_SAVE_PROFILE_ID);
        assert_eq!(max.display.brightness, 40);
        assert!(max.display.use_brightness);
        assert_eq!(max.display.refresh_rate, Some(60.0));
        assert!(!max.display.use_refresh_rate);
        assert_eq!(max.display.x_resolution, Some(1920));
        assert_eq!(max.display.y_resolution, Some(1080));
        assert_eq!(max.fan.fan_profile, "Silent");
        assert_eq!(max.odm_profile.name.as_deref(), Some("power_save"));
        assert_eq!(max.odm_power_limits.tdp_values, vec![5.0, 10.0, 15.0]);

        let quiet = quiet_profile();
        assert_eq!(quiet.id.as_str(), QUIET_PROFILE_ID);
        assert_eq!(quiet.display.brightness, 50);
        assert_eq!(quiet.fan.fan_profile, "Silent");
        assert_eq!(quiet.odm_power_limits.tdp_values, vec![10.0, 15.0, 25.0]);

        let office = office_profile();
        assert_eq!(office.id.as_str(), OFFICE_PROFILE_ID);
        assert_eq!(office.display.brightness, 60);
        assert_eq!(office.fan.fan_profile, "Quiet");
        assert_eq!(office.odm_profile.name.as_deref(), Some("enthusiast"));
        assert_eq!(office.odm_power_limits.tdp_values, vec![25.0, 35.0, 35.0]);

        let high = high_performance_profile();
        assert_eq!(high.id.as_str(), HIGH_PERFORMANCE_PROFILE_ID);
        assert_eq!(high.display.brightness, 60);
        assert_eq!(high.fan.fan_profile, "Balanced");
        assert_eq!(high.odm_profile.name.as_deref(), Some("overboost"));
        assert_eq!(high.odm_power_limits.tdp_values, vec![60.0, 60.0, 70.0]);

        let custom = default_custom_profile();
        assert_eq!(custom.id.as_str(), DEFAULT_CUSTOM_PROFILE_ID);
        assert_eq!(custom.name, "TUXEDO Defaults");
        assert_eq!(custom.display.brightness, 100);
        assert!(!custom.display.use_brightness);
        assert_eq!(custom.fan.fan_profile, "Balanced");
        assert!(custom.odm_profile.name.is_none());
        assert!(custom.odm_power_limits.tdp_values.is_empty());
        assert!(!custom.fan.custom_fan_curve.table_cpu.is_empty());
    }

    #[test]
    fn suggested_matrix_resolves_against_default_profiles() {
        let profiles = default_profiles();
        let mut resolver = StateResolver::new(
            StateMatrix::suggested_default(),
            LegacyStateMap::default(),
            ProfileId::new(DEFAULT_CUSTOM_PROFILE_ID),
        );
        resolver.set_known_profiles(Some(profiles.id_set()));

        let cases = [
            (
                PpdProfile::PowerSaver,
                PowerSource::Ac,
                MAX_ENERGY_SAVE_PROFILE_ID,
            ),
            (
                PpdProfile::PowerSaver,
                PowerSource::Battery,
                MAX_ENERGY_SAVE_PROFILE_ID,
            ),
            (PpdProfile::Balanced, PowerSource::Ac, OFFICE_PROFILE_ID),
            (PpdProfile::Balanced, PowerSource::Battery, QUIET_PROFILE_ID),
            (
                PpdProfile::Performance,
                PowerSource::Ac,
                HIGH_PERFORMANCE_PROFILE_ID,
            ),
            (
                PpdProfile::Performance,
                PowerSource::Battery,
                OFFICE_PROFILE_ID,
            ),
        ];

        for (ppd, power, expected) in cases {
            let resolved = resolver.resolve_for_key(&StateKey::new(Some(ppd), power));
            assert_eq!(
                resolved.source,
                ResolutionSource::StateMatrix,
                "fallback inattendu pour {power}"
            );
            assert_eq!(resolved.profile_id.as_str(), expected);
            assert!(profiles.contains(&resolved.profile_id));
        }
    }

    #[test]
    fn suggested_matrix_and_state_map_references_are_valid() {
        let profiles = default_profiles();
        let mut settings = crate::settings::Settings::default();
        settings.with_suggested_matrix();
        validate_settings_references(&settings, &profiles).unwrap();
    }

    #[test]
    fn merge_keeps_custom_profile_with_preset_id() {
        let customized_office = Profile::new(OFFICE_PROFILE_ID, "Mon bureau");
        let loaded =
            ProfileCollection::new(vec![customized_office, Profile::new("mine", "Mon profil")]);

        let merged = merge_with_default_profiles(loaded);
        assert_eq!(merged.len(), 6);
        assert_eq!(
            merged.get_by_id(OFFICE_PROFILE_ID).unwrap().name,
            "Mon bureau"
        );
        for id in DEFAULT_PROFILE_IDS {
            assert!(merged.get_by_id(id).is_some(), "preset manquant: {id}");
        }
    }

    #[test]
    fn merge_does_not_duplicate_ids() {
        let merged = merge_with_default_profiles(default_profiles());
        assert_eq!(merged.len(), 5);
        assert_eq!(merged, default_profiles());
    }

    #[test]
    fn merge_deduplicates_loaded_profiles() {
        let loaded = ProfileCollection::new(vec![
            Profile::new("dup", "Premier"),
            Profile::new("dup", "Second"),
        ]);
        let merged = merge_with_default_profiles(loaded);
        let ids: BTreeSet<&str> = merged.ids().map(ProfileId::as_str).collect();
        assert_eq!(ids.len(), merged.len());
        assert_eq!(merged.len(), 6);
        assert_eq!(merged.get_by_id("dup").unwrap().name, "Premier");
    }

    #[test]
    fn merge_renames_loaded_profile_on_preset_name_collision() {
        // BUG-QA-002 : « Office » chargé + preset `__office__`. L'identifiant
        // reste la clé ; le profil utilisateur est renommé, pas le preset.
        let loaded = ProfileCollection::new(vec![Profile::new("custom-office", "Office")]);
        let merged = merge_with_default_profiles(loaded);

        merged.validate().unwrap();
        assert_eq!(
            merged.get_by_id("custom-office").unwrap().name,
            "Office (custom)"
        );
        assert_eq!(merged.get_by_id(OFFICE_PROFILE_ID).unwrap().name, "Office");
    }

    #[test]
    fn merge_renames_loaded_profiles_sharing_a_name() {
        let loaded = ProfileCollection::new(vec![
            Profile::new("bureau-a", "Bureau"),
            Profile::new("bureau-b", "BUREAU"),
        ]);
        let merged = merge_with_default_profiles(loaded);

        merged.validate().unwrap();
        assert_eq!(merged.get_by_id("bureau-a").unwrap().name, "Bureau");
        assert_eq!(
            merged.get_by_id("bureau-b").unwrap().name,
            "BUREAU (custom)"
        );
    }

    #[test]
    fn merge_renames_preset_customized_to_another_preset_name() {
        // Un preset personnalisé sur place dont le nom usurpe un preset qui
        // va être ajouté est renommé lui aussi.
        let loaded = ProfileCollection::new(vec![Profile::new(OFFICE_PROFILE_ID, "Quiet")]);
        let merged = merge_with_default_profiles(loaded);

        merged.validate().unwrap();
        assert_eq!(
            merged.get_by_id(OFFICE_PROFILE_ID).unwrap().name,
            "Quiet (custom)"
        );
        assert_eq!(merged.get_by_id(QUIET_PROFILE_ID).unwrap().name, "Quiet");
    }

    #[test]
    fn merge_truncates_renamed_names_without_exceeding_the_limit() {
        let base = "A".repeat(MAX_PROFILE_NAME_LEN);
        let loaded = ProfileCollection::new(vec![
            Profile::new("long-a", base.clone()),
            Profile::new("long-b", base),
        ]);
        let merged = merge_with_default_profiles(loaded);

        merged.validate().unwrap();
        let renamed = merged.get_by_id("long-b").unwrap();
        assert!(renamed.name.chars().count() <= MAX_PROFILE_NAME_LEN);
        assert!(renamed.name.ends_with(" (custom)"));
    }

    #[test]
    fn merge_is_idempotent() {
        let once = merge_with_default_profiles(ProfileCollection::default());
        let twice = merge_with_default_profiles(once.clone());
        assert_eq!(once, twice);
    }

    #[test]
    fn factory_profiles_do_not_share_mutable_state() {
        let mut first = default_profiles();
        first
            .get_mut(&ProfileId::new(OFFICE_PROFILE_ID))
            .unwrap()
            .name = "Modifié".to_owned();
        let second = default_profiles();
        assert_eq!(second.get_by_id(OFFICE_PROFILE_ID).unwrap().name, "Office");
    }
}
