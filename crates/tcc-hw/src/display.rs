// SPDX-License-Identifier: GPL-3.0-or-later
//! Contrats neutres pour le changement de mode (résolution et/ou taux) de
//! l'écran interne.
//!
//! Ce module ne dépend d'aucune session graphique : il définit les types
//! échangés entre `tcc-user` (agent de session, non privilégié) et les
//! backends X11/Wayland. Les implémentations réelles vivront dans `tcc-user`.
//!
//! Règles contractuelles :
//!
//! 1. Seuls les connecteurs internes sont ciblés : préfixes `eDP`, `LVDS`,
//!    `DSI` (insensible à la casse).
//! 2. La résolution et le taux sont optionnels : `None`/`0` conserve la valeur
//!    courante.
//! 3. Un mode déjà appliqué (résolution + taux, tolérance 0,5 Hz) ne provoque
//!    aucune écriture.
//! 4. Un écran externe n'est jamais modifié, même si l'appelant le nomme.
//! 5. Un mode introuvable produit une erreur typée, jamais un fallback
//!    silencieux vers une autre résolution ou un autre taux.

use crate::error::{HwError, Result};
use crate::sysfs::SysfsRoot;

/// Tolérance par défaut entre le taux demandé et le taux réel du mode.
pub const DEFAULT_REFRESH_TOLERANCE_HZ: f32 = 0.5;

/// Dimension maximale acceptée pour une résolution ciblée.
///
/// Alignée sur `tcc_core::MAX_RESOLUTION` sans créer de dépendance `tcc-hw →
/// tcc-core` : une valeur hors de cette borne est refusée avant tout accès au
/// backend.
pub const MAX_DISPLAY_DIMENSION: u32 = 16_384;

/// Préfixes de connecteurs internes.
const INTERNAL_CONNECTOR_PREFIXES: [&str; 3] = ["edp", "lvds", "dsi"];

/// Vrai si le nom de connecteur désigne un écran interne.
///
/// Exemples internes : `eDP-1`, `eDP-1-1`, `LVDS-1`, `DSI-1`.
/// Exemples externes : `HDMI-A-1`, `DP-1`, `DP-2`, `DVI-I-1`.
pub fn is_internal_connector(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    INTERNAL_CONNECTOR_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

/// Backend de session pressenti ou détecté.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayBackendKind {
    X11,
    Hyprland,
    Gnome,
    Kde,
    Sway,
    Wlroots,
    Unsupported,
}

impl DisplayBackendKind {
    /// Nom stable exposé à l'UI.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::X11 => "x11",
            Self::Hyprland => "hyprland",
            Self::Gnome => "gnome",
            Self::Kde => "kde",
            Self::Sway => "sway",
            Self::Wlroots => "wlroots",
            Self::Unsupported => "unsupported",
        }
    }

    /// Vrai si le backend peut appliquer un mode.
    pub const fn is_supported(self) -> bool {
        !matches!(self, Self::Unsupported)
    }
}

/// Mode d'affichage exposé par un backend.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: f32,
    /// Mode préféré annoncé par le compositeur/moniteur.
    pub preferred: bool,
    /// Mode actuellement actif sur cet output.
    pub current: bool,
}

impl DisplayMode {
    pub fn new(width: u32, height: u32, refresh_hz: f32) -> Self {
        Self {
            width,
            height,
            refresh_hz,
            preferred: false,
            current: false,
        }
    }
}

/// Sortie vidéo annoncée par le compositeur.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputInfo {
    /// Nom du connecteur (`eDP-1`, `HDMI-A-1`).
    pub name: String,
    /// Câble/panneau détecté.
    pub connected: bool,
    /// Sortie actuellement activée.
    pub enabled: bool,
    /// Renseigné par le backend ; `is_internal_connector` fait foi.
    pub internal: bool,
    pub current_mode: Option<DisplayMode>,
    pub modes: Vec<DisplayMode>,
}

impl OutputInfo {
    /// Construit une sortie en normalisant le drapeau `internal`.
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            internal: is_internal_connector(&name),
            name,
            connected: false,
            enabled: false,
            current_mode: None,
            modes: Vec::new(),
        }
    }

    /// Taux actif, si connu.
    pub fn current_refresh_hz(&self) -> Option<f32> {
        self.current_mode.as_ref().map(|mode| mode.refresh_hz)
    }
}

/// Écran interne validé.
#[derive(Debug, Clone, PartialEq)]
pub struct InternalOutput {
    info: OutputInfo,
}

impl InternalOutput {
    /// Valide qu'une sortie est bien interne. Retourne
    /// [`HwError::NotInternalOutput`] sinon.
    pub fn try_new(info: OutputInfo) -> Result<Self> {
        if !is_internal_connector(&info.name) {
            return Err(HwError::NotInternalOutput(info.name));
        }
        Ok(Self { info })
    }

    pub fn info(&self) -> &OutputInfo {
        &self.info
    }

    pub fn name(&self) -> &str {
        &self.info.name
    }

    pub fn current_refresh_hz(&self) -> Option<f32> {
        self.info.current_refresh_hz()
    }

    /// Modes partageant la résolution courante, triés par taux croissant.
    pub fn modes_at_current_resolution(&self) -> Vec<&DisplayMode> {
        let Some(current) = self.info.current_mode.as_ref() else {
            return Vec::new();
        };
        let mut modes: Vec<&DisplayMode> = self
            .info
            .modes
            .iter()
            .filter(|mode| mode.width == current.width && mode.height == current.height)
            .collect();
        modes.sort_by(|left, right| left.refresh_hz.total_cmp(&right.refresh_hz));
        modes
    }

    /// Cherche un mode à la résolution courante dont le taux est proche de
    /// `target_hz`.
    pub fn mode_for_rate(&self, target_hz: f32, tolerance_hz: f32) -> Option<&DisplayMode> {
        let current = self.info.current_mode.as_ref()?;
        self.info
            .modes
            .iter()
            .filter(|mode| mode.width == current.width && mode.height == current.height)
            .min_by(|left, right| {
                (left.refresh_hz - target_hz)
                    .abs()
                    .total_cmp(&(right.refresh_hz - target_hz).abs())
            })
            .filter(|mode| rates_match(mode.refresh_hz, target_hz, tolerance_hz))
    }

    /// Cherche un mode **exactement** à la résolution demandée et dont le taux
    /// est dans la tolérance.
    ///
    /// L'existence du mode dans `OutputInfo.modes` est garantie par
    /// construction. Aucun fallback vers une autre résolution ou un autre taux.
    pub fn mode_for_resolution_and_rate(
        &self,
        width: u32,
        height: u32,
        target_hz: f32,
        tolerance_hz: f32,
    ) -> Option<&DisplayMode> {
        self.info
            .modes
            .iter()
            .filter(|mode| mode.width == width && mode.height == height)
            .min_by(|left, right| {
                (left.refresh_hz - target_hz)
                    .abs()
                    .total_cmp(&(right.refresh_hz - target_hz).abs())
            })
            .filter(|mode| rates_match(mode.refresh_hz, target_hz, tolerance_hz))
    }
}

/// Deux taux sont équivalents dans la tolérance donnée.
pub fn rates_match(left: f32, right: f32, tolerance_hz: f32) -> bool {
    (left - right).abs() <= tolerance_hz.abs()
}

/// Vrai si le taux courant diffère déjà suffisamment de la cible.
pub fn needs_rate_change(current_hz: Option<f32>, target_hz: f32, tolerance_hz: f32) -> bool {
    match current_hz {
        Some(current) => !rates_match(current, target_hz, tolerance_hz),
        None => true,
    }
}

/// Connecteur interne lu directement depuis DRM.
///
/// Le daemon ne connaît ni les modes ni le backend de session : ces
/// informations restent fournies par l'agent `tccd-user`. Cette structure ne
/// décrit que la présence du connecteur dans `/sys/class/drm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrmConnector {
    /// Nom du connecteur sans préfixe carte (`eDP-1`, `LVDS-1`, `DSI-1`).
    pub name: String,
    /// Câble/panneau détecté (`status == connected`).
    pub connected: bool,
}

/// Lit le premier connecteur interne DRM.
///
/// Parcourt `/sys/class/drm/card*-*` et retient le premier connecteur interne
/// **connecté** ; à défaut, le premier connecteur interne trouvé
/// (`connected: false`). Retourne `Ok(None)` si `/sys/class/drm` est absent ou
/// ne contient aucun connecteur interne. Aucune erreur n'est produite pour un
/// matériel absent.
///
/// # Errors
///
/// Erreur d'E/S réelle lors de la lecture des entrées ou des statuts.
pub fn read_internal_connector(root: &SysfsRoot) -> Result<Option<DrmConnector>> {
    let mut fallback: Option<DrmConnector> = None;
    for entry in root.list_dir("class/drm")? {
        let Some(connector) = connector_from_drm_entry(&entry) else {
            continue;
        };
        if !is_internal_connector(connector) {
            continue;
        }
        let status = root.read_optional_string(format!("class/drm/{entry}/status"))?;
        let connected = status
            .as_deref()
            .is_some_and(|value| value.trim() == "connected");
        let candidate = DrmConnector {
            name: connector.to_owned(),
            connected,
        };
        if connected {
            return Ok(Some(candidate));
        }
        if fallback.is_none() {
            fallback = Some(candidate);
        }
    }
    Ok(fallback)
}

/// Extrait `eDP-1` depuis `card0-eDP-1`.
fn connector_from_drm_entry(entry: &str) -> Option<&str> {
    let (card, connector) = entry.split_once('-')?;
    if !card.starts_with("card") || connector.is_empty() {
        return None;
    }
    Some(connector)
}

/// Sélectionne l'écran interne parmi une liste d'outputs.
///
/// Priorité au premier connecteur interne connecté. Un panneau interne
/// déconnecté n'est pas retourné : appliquer un mode dessus n'aurait pas de
/// sens et pourrait échouer au réveil.
pub fn select_internal_output(outputs: &[OutputInfo]) -> Option<InternalOutput> {
    outputs
        .iter()
        .filter(|output| output.connected && is_internal_connector(&output.name))
        .find_map(|output| InternalOutput::try_new(output.clone()).ok())
}

/// Résultat d'une demande d'application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Le backend a appliqué le mode.
    Applied,
    /// Le taux courant correspondait déjà à la cible.
    AlreadyApplied,
    /// Le backend ne sait pas appliquer ce changement.
    Unsupported,
}

/// Backend d'affichage, implémenté par `tcc-user`.
///
/// Le backend tourne dans la session utilisateur ; il ne doit jamais écrire
/// `/etc` ni ouvrir de périphérique privilégié.
pub trait DisplayModeBackend: Send {
    /// Nom stable pour les logs et l'état exposé à l'UI.
    fn backend_name(&self) -> &'static str;

    /// Type de backend.
    fn kind(&self) -> DisplayBackendKind;

    /// Liste les sorties et leurs modes.
    fn list_outputs(&mut self) -> Result<Vec<OutputInfo>>;

    /// Applique un mode à un écran interne.
    fn apply_refresh_rate(
        &mut self,
        output: &InternalOutput,
        mode: &DisplayMode,
    ) -> Result<ApplyOutcome>;

    /// Vrai si le backend est utilisable dans la session courante.
    fn is_available(&self) -> bool {
        true
    }
}

/// Planification d'un changement de mode interne.
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayModePlan {
    /// Le mode courant correspond déjà à la résolution et au taux demandés.
    AlreadyApplied {
        output: InternalOutput,
        mode: DisplayMode,
    },
    /// Un mode interne existant doit être appliqué.
    Apply {
        output: InternalOutput,
        mode: DisplayMode,
    },
}

/// Normalise une résolution optionnelle.
///
/// `(None, None)` et `(Some(0), Some(0))` signifient « conserver la résolution
/// courante ». Une résolution partielle ou hors bornes est refusée.
fn normalize_resolution(width: Option<u32>, height: Option<u32>) -> Result<Option<(u32, u32)>> {
    match (width, height) {
        (None, None) | (Some(0), Some(0)) => Ok(None),
        (Some(width), Some(height))
            if width > 0
                && height > 0
                && width <= MAX_DISPLAY_DIMENSION
                && height <= MAX_DISPLAY_DIMENSION =>
        {
            Ok(Some((width, height)))
        }
        (width, height) => Err(HwError::InvalidValue {
            context: "résolution interne",
            value: format!("{width:?}x{height:?}"),
        }),
    }
}

fn validate_target_rate(refresh_hz: f32) -> Result<()> {
    if !refresh_hz.is_finite() || refresh_hz <= 0.0 {
        return Err(HwError::InvalidValue {
            context: "taux de rafraîchissement",
            value: refresh_hz.to_string(),
        });
    }
    Ok(())
}

/// Sélectionne le mode interne à appliquer **sans écrire**.
///
/// - `width`/`height` `None` ou `0` : conserve la résolution courante.
/// - `refresh_hz` `None` : conserve le taux courant.
/// - La résolution et le taux sont cherchés ensemble ; une combinaison absente
///   produit [`HwError::DisplayModeNotFound`], jamais un fallback arbitraire.
/// - Seul l'écran interne connecté est considéré (`eDP`/`LVDS`/`DSI`).
///
/// # Errors
///
/// - [`HwError::InvalidValue`] pour une résolution partielle/hors bornes ou un
///   taux invalide ;
/// - [`HwError::NoDisplayBackend`] si aucun écran interne connecté ;
/// - [`HwError::DisplayModeNotFound`] si la combinaison demandée n'existe pas.
pub fn plan_internal_display_mode(
    outputs: &[OutputInfo],
    width: Option<u32>,
    height: Option<u32>,
    refresh_hz: Option<f32>,
    tolerance_hz: f32,
) -> Result<DisplayModePlan> {
    let target_resolution = normalize_resolution(width, height)?;
    if let Some(rate) = refresh_hz {
        validate_target_rate(rate)?;
    }

    let Some(internal) = select_internal_output(outputs) else {
        return Err(HwError::NoDisplayBackend);
    };
    let Some(current) = internal.info().current_mode.clone() else {
        return Err(HwError::DisplayModeNotFound(format!(
            "mode courant indisponible sur {}",
            internal.name()
        )));
    };
    let (target_width, target_height) =
        target_resolution.unwrap_or((current.width, current.height));
    let target_hz = refresh_hz.unwrap_or(current.refresh_hz);

    if current.width == target_width
        && current.height == target_height
        && rates_match(current.refresh_hz, target_hz, tolerance_hz)
    {
        return Ok(DisplayModePlan::AlreadyApplied {
            output: internal,
            mode: current,
        });
    }

    let Some(mode) = internal
        .mode_for_resolution_and_rate(target_width, target_height, target_hz, tolerance_hz)
        .cloned()
    else {
        return Err(HwError::DisplayModeNotFound(format!(
            "{target_width}x{target_height} @ {target_hz} Hz sur {}",
            internal.name()
        )));
    };
    Ok(DisplayModePlan::Apply {
        output: internal,
        mode,
    })
}

/// Applique résolution et/ou taux à l'écran interne uniquement.
///
/// `width`/`height` à `None` (ou `0`) conservent la résolution courante ;
/// `refresh_hz` à `None` conserve le taux courant. Une seule opération est
/// demandée au backend, avec un [`DisplayMode`] complet et vérifié.
///
/// # Errors
///
/// Voir [`plan_internal_display_mode`].
pub fn apply_internal_display_mode<B: DisplayModeBackend + ?Sized>(
    backend: &mut B,
    width: Option<u32>,
    height: Option<u32>,
    refresh_hz: Option<f32>,
    tolerance_hz: f32,
) -> Result<ApplyOutcome> {
    // Valider avant tout accès backend (une entrée invalide ne doit pas
    // déclencher d'énumération de modes).
    normalize_resolution(width, height)?;
    if let Some(rate) = refresh_hz {
        validate_target_rate(rate)?;
    }
    if !backend.is_available() {
        return Ok(ApplyOutcome::Unsupported);
    }
    let outputs = backend.list_outputs()?;
    match plan_internal_display_mode(&outputs, width, height, refresh_hz, tolerance_hz)? {
        DisplayModePlan::AlreadyApplied { .. } => Ok(ApplyOutcome::AlreadyApplied),
        DisplayModePlan::Apply { output, mode } => backend.apply_refresh_rate(&output, &mode),
    }
}

/// Orchestration commune aux backends : conserve la résolution courante et ne
/// change que le taux.
///
/// Cette fonction est conservée pour les appelants historiques ; elle délègue à
/// [`apply_internal_display_mode`] avec une résolution `None`.
///
/// # Errors
///
/// Voir [`plan_internal_display_mode`].
pub fn apply_internal_refresh_rate<B: DisplayModeBackend + ?Sized>(
    backend: &mut B,
    target_hz: f32,
    tolerance_hz: f32,
) -> Result<ApplyOutcome> {
    apply_internal_display_mode(backend, None, None, Some(target_hz), tolerance_hz)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(name: &str, connected: bool, current: Option<f32>, rates: &[f32]) -> OutputInfo {
        let mut output = OutputInfo::new(name);
        output.connected = connected;
        output.enabled = current.is_some();
        let width = 2560;
        let height = 1440;
        output.modes = rates
            .iter()
            .map(|rate| DisplayMode {
                width,
                height,
                refresh_hz: *rate,
                preferred: *rate == rates[0],
                current: current
                    .map(|value| (value - *rate).abs() < f32::EPSILON)
                    .unwrap_or(false),
            })
            .collect();
        output.current_mode = current.map(|rate| DisplayMode {
            width,
            height,
            refresh_hz: rate,
            preferred: false,
            current: true,
        });
        output
    }

    #[test]
    fn connector_detection() {
        assert!(is_internal_connector("eDP-1"));
        assert!(is_internal_connector("eDP-1-1"));
        assert!(is_internal_connector("LVDS-1"));
        assert!(is_internal_connector("DSI-1"));
        assert!(is_internal_connector("edp-2"));
        assert!(!is_internal_connector("HDMI-A-1"));
        assert!(!is_internal_connector("DP-1"));
        assert!(!is_internal_connector("DVI-I-1"));
    }

    #[test]
    fn selection_ignores_external_outputs() {
        let outputs = vec![
            output("HDMI-A-1", true, Some(60.0), &[60.0, 120.0]),
            output("eDP-1", true, Some(60.0), &[60.0, 165.0]),
        ];
        assert_eq!(select_internal_output(&outputs).unwrap().name(), "eDP-1");
    }

    #[test]
    fn selection_none_when_internal_disconnected() {
        let outputs = vec![
            output("eDP-1", false, None, &[]),
            output("DP-1", true, Some(60.0), &[60.0]),
        ];
        assert!(select_internal_output(&outputs).is_none());
    }

    #[test]
    fn internal_output_rejects_external_name() {
        let err = InternalOutput::try_new(output("DP-1", true, Some(60.0), &[60.0])).unwrap_err();
        assert!(matches!(err, HwError::NotInternalOutput(_)));
    }

    #[test]
    fn mode_matching_uses_tolerance() {
        let mut output = output(
            "eDP-1",
            true,
            Some(60.0),
            &[48.0, 59.94, 60.0, 120.0, 165.0],
        );
        output.current_mode = Some(DisplayMode::new(2560, 1440, 59.94));
        let internal = InternalOutput::try_new(output).unwrap();
        assert_eq!(internal.mode_for_rate(60.0, 0.5).unwrap().refresh_hz, 60.0);
        // 59.94 est à 0,06 Hz de 60 : considéré identique par `needs_rate_change`.
        assert!(!needs_rate_change(Some(59.94), 60.0, 0.5));
        // Une cible absente ne doit pas provoquer de changement de résolution.
        assert!(internal.mode_for_rate(75.0, 0.5).is_none());
    }

    #[test]
    fn modes_at_current_resolution_excludes_other_resolutions() {
        let mut output = output("eDP-1", true, Some(60.0), &[60.0, 165.0]);
        output.modes.push(DisplayMode::new(1920, 1080, 60.0));
        let internal = InternalOutput::try_new(output).unwrap();
        let rates: Vec<f32> = internal
            .modes_at_current_resolution()
            .iter()
            .map(|mode| mode.refresh_hz)
            .collect();
        assert_eq!(rates, vec![60.0, 165.0]);
    }

    struct FakeBackend {
        outputs: Vec<OutputInfo>,
        calls: usize,
        last: Option<(String, DisplayMode)>,
    }

    impl FakeBackend {
        fn new(outputs: Vec<OutputInfo>) -> Self {
            Self {
                outputs,
                calls: 0,
                last: None,
            }
        }
    }

    impl DisplayModeBackend for FakeBackend {
        fn backend_name(&self) -> &'static str {
            "fake"
        }

        fn kind(&self) -> DisplayBackendKind {
            DisplayBackendKind::Unsupported
        }

        fn list_outputs(&mut self) -> Result<Vec<OutputInfo>> {
            Ok(self.outputs.clone())
        }

        fn apply_refresh_rate(
            &mut self,
            output: &InternalOutput,
            mode: &DisplayMode,
        ) -> Result<ApplyOutcome> {
            self.calls += 1;
            self.last = Some((output.name().to_owned(), mode.clone()));
            Ok(ApplyOutcome::Applied)
        }
    }

    #[test]
    fn orchestration_applies_only_when_needed() {
        let mut backend = FakeBackend::new(vec![output("eDP-1", true, Some(60.0), &[60.0, 165.0])]);
        assert_eq!(
            apply_internal_refresh_rate(&mut backend, 60.2, 0.5).unwrap(),
            ApplyOutcome::AlreadyApplied
        );
        assert_eq!(backend.calls, 0);
        assert_eq!(
            apply_internal_refresh_rate(&mut backend, 165.0, 0.5).unwrap(),
            ApplyOutcome::Applied
        );
        assert_eq!(backend.calls, 1);
    }

    #[test]
    fn orchestration_never_touches_external_only() {
        let mut backend = FakeBackend::new(vec![output("DP-1", true, Some(60.0), &[60.0, 165.0])]);
        assert!(matches!(
            apply_internal_refresh_rate(&mut backend, 165.0, 0.5),
            Err(HwError::NoDisplayBackend)
        ));
        assert_eq!(backend.calls, 0);
    }

    #[test]
    fn orchestration_reports_missing_mode() {
        let mut backend = FakeBackend::new(vec![output("eDP-1", true, Some(60.0), &[60.0])]);
        assert!(matches!(
            apply_internal_refresh_rate(&mut backend, 165.0, 0.5),
            Err(HwError::DisplayModeNotFound(_))
        ));
        assert_eq!(backend.calls, 0);
    }

    #[test]
    fn invalid_target_rate_is_rejected() {
        let mut backend = FakeBackend::new(vec![]);
        assert!(apply_internal_refresh_rate(&mut backend, 0.0, 0.5).is_err());
        assert!(apply_internal_refresh_rate(&mut backend, f32::NAN, 0.5).is_err());
    }

    fn output_with_modes(
        name: &str,
        current: Option<DisplayMode>,
        modes: &[(u32, u32, f32)],
    ) -> OutputInfo {
        let mut output = OutputInfo::new(name);
        output.connected = true;
        output.enabled = current.is_some();
        output.modes = modes
            .iter()
            .map(|(width, height, refresh_hz)| DisplayMode {
                width: *width,
                height: *height,
                refresh_hz: *refresh_hz,
                preferred: false,
                current: current.as_ref().is_some_and(|mode| {
                    mode.width == *width
                        && mode.height == *height
                        && (mode.refresh_hz - *refresh_hz).abs() < f32::EPSILON
                }),
            })
            .collect();
        output.current_mode = current;
        output
    }

    #[test]
    fn resolution_only_keeps_current_rate() {
        let mut backend = FakeBackend::new(vec![output_with_modes(
            "eDP-1",
            Some(DisplayMode::new(2560, 1440, 60.0)),
            &[(2560, 1440, 60.0), (1920, 1080, 60.0)],
        )]);
        assert_eq!(
            apply_internal_display_mode(&mut backend, Some(1920), Some(1080), None, 0.5)
                .expect("résolution seule"),
            ApplyOutcome::Applied
        );
        let (name, mode) = backend.last.as_ref().expect("mode appliqué");
        assert_eq!(name, "eDP-1");
        assert_eq!((mode.width, mode.height), (1920, 1080));
        assert_eq!(mode.refresh_hz, 60.0);
    }

    #[test]
    fn resolution_and_rate_are_applied_together() {
        let mut backend = FakeBackend::new(vec![output_with_modes(
            "eDP-1",
            Some(DisplayMode::new(2560, 1440, 60.0)),
            &[(2560, 1440, 60.0), (1920, 1080, 60.0), (1920, 1080, 165.0)],
        )]);
        assert_eq!(
            apply_internal_display_mode(&mut backend, Some(1920), Some(1080), Some(165.0), 0.5)
                .expect("résolution + taux"),
            ApplyOutcome::Applied
        );
        let (_, mode) = backend.last.as_ref().expect("mode appliqué");
        assert_eq!((mode.width, mode.height), (1920, 1080));
        assert_eq!(mode.refresh_hz, 165.0);
    }

    #[test]
    fn missing_rate_at_resolution_is_an_error_without_write() {
        let mut backend = FakeBackend::new(vec![output_with_modes(
            "eDP-1",
            Some(DisplayMode::new(2560, 1440, 60.0)),
            &[(2560, 1440, 60.0), (1920, 1080, 60.0)],
        )]);
        assert!(matches!(
            apply_internal_display_mode(&mut backend, Some(1920), Some(1080), Some(165.0), 0.5),
            Err(HwError::DisplayModeNotFound(_))
        ));
        assert_eq!(backend.calls, 0, "aucune écriture sans mode exact");
    }

    #[test]
    fn resolution_search_never_uses_external_output_modes() {
        let mut backend = FakeBackend::new(vec![
            output_with_modes(
                "DP-1",
                Some(DisplayMode::new(1920, 1080, 60.0)),
                &[(1920, 1080, 165.0)],
            ),
            output_with_modes(
                "eDP-1",
                Some(DisplayMode::new(2560, 1440, 60.0)),
                &[(2560, 1440, 60.0), (1920, 1080, 60.0)],
            ),
        ]);
        assert!(matches!(
            apply_internal_display_mode(&mut backend, Some(1920), Some(1080), Some(165.0), 0.5),
            Err(HwError::DisplayModeNotFound(_))
        ));
        assert_eq!(backend.calls, 0);
    }

    #[test]
    fn identical_resolution_and_rate_is_idempotent() {
        let mut backend = FakeBackend::new(vec![output_with_modes(
            "eDP-1",
            Some(DisplayMode::new(1920, 1080, 60.0)),
            &[(1920, 1080, 60.0), (1920, 1080, 165.0)],
        )]);
        assert_eq!(
            apply_internal_display_mode(&mut backend, Some(1920), Some(1080), Some(60.2), 0.5)
                .expect("idempotent"),
            ApplyOutcome::AlreadyApplied
        );
        assert_eq!(backend.calls, 0);
    }

    #[test]
    fn zero_resolution_keeps_current_resolution() {
        let mut backend = FakeBackend::new(vec![output_with_modes(
            "eDP-1",
            Some(DisplayMode::new(2560, 1440, 60.0)),
            &[(2560, 1440, 60.0), (2560, 1440, 165.0)],
        )]);
        assert_eq!(
            apply_internal_display_mode(&mut backend, Some(0), Some(0), Some(165.0), 0.5)
                .expect("0 = résolution courante"),
            ApplyOutcome::Applied
        );
        let (_, mode) = backend.last.as_ref().expect("mode appliqué");
        assert_eq!((mode.width, mode.height), (2560, 1440));
        assert_eq!(mode.refresh_hz, 165.0);
    }

    #[test]
    fn partial_resolution_is_rejected() {
        let mut backend = FakeBackend::new(vec![output_with_modes(
            "eDP-1",
            Some(DisplayMode::new(2560, 1440, 60.0)),
            &[(2560, 1440, 60.0)],
        )]);
        assert!(matches!(
            apply_internal_display_mode(&mut backend, Some(1920), None, None, 0.5),
            Err(HwError::InvalidValue { .. })
        ));
        assert_eq!(backend.calls, 0);
    }

    #[test]
    fn missing_current_mode_is_reported() {
        let mut backend = FakeBackend::new(vec![output_with_modes(
            "eDP-1",
            None,
            &[(2560, 1440, 60.0)],
        )]);
        assert!(matches!(
            apply_internal_display_mode(&mut backend, Some(1920), Some(1080), None, 0.5),
            Err(HwError::DisplayModeNotFound(_))
        ));
        assert_eq!(backend.calls, 0);
    }

    fn write_drm(root: &std::path::Path, entry: &str, status: &str) {
        let path = root.join("class/drm").join(entry);
        std::fs::create_dir_all(&path).expect("drm dir");
        std::fs::write(path.join("status"), format!("{status}\n")).expect("status");
    }

    #[test]
    fn reads_connected_internal_connector_from_drm() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_drm(dir.path(), "card0-DP-1", "connected");
        write_drm(dir.path(), "card0-eDP-1", "connected");
        let connector = read_internal_connector(&SysfsRoot::new(dir.path()))
            .expect("lecture")
            .expect("connecteur");
        assert_eq!(connector.name, "eDP-1");
        assert!(connector.connected);
    }

    #[test]
    fn disconnected_internal_connector_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_drm(dir.path(), "card1-LVDS-1", "disconnected");
        let connector = read_internal_connector(&SysfsRoot::new(dir.path()))
            .expect("lecture")
            .expect("connecteur");
        assert_eq!(connector.name, "LVDS-1");
        assert!(!connector.connected);
    }

    #[test]
    fn external_or_absent_drm_yields_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_drm(dir.path(), "card0-HDMI-A-1", "connected");
        assert!(
            read_internal_connector(&SysfsRoot::new(dir.path()))
                .expect("lecture")
                .is_none()
        );

        let empty = tempfile::tempdir().expect("tempdir");
        assert!(
            read_internal_connector(&SysfsRoot::new(empty.path()))
                .expect("lecture")
                .is_none()
        );
    }
}
