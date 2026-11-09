// SPDX-License-Identifier: GPL-3.0-or-later
//! Accès à `/dev/tuxedo_io` (module noyau `tuxedo_io`, interfaces Clevo et
//! Uniwill).
//!
//! Les numéros `ioctl` sont recopiés de
//! `tuxedo-control-center-master/src/native-lib/tuxedo_io_lib/tuxedo_io_ioctl.h`
//! (GPL-3.0-or-later, TUXEDO Computers GmbH) et réencodés ici avec les mêmes
//! macro-noyau `_IO`/`_IOR`/`_IOW`. Point important : le header original passe
//! des **types pointeur** à `_IOR`/`_IOW` (`int32_t*`, `char*`), donc la taille
//! encodée est `sizeof(pointeur)` (8 sur x86_64), pas `sizeof(int32_t)`.
//!
//! Toute la couche `unsafe` est concentrée dans [`real::RealTransport`] :
//! ouverture contrôlée, un `ioctl` par opération, validation préalable des
//! index et des plages. Aucun `unwrap` n'est utilisé sur les chemins de
//! production.
//!
//! Le module noyau n'est pas modifié par ce crate. Les appelants doivent
//! appeler [`TuxedoIo::set_fans_auto`] avant de rendre la main (voir
//! `shutdown`) pour éviter un ventilateur bloqué en manuel après un arrêt
//! brutal.

use std::io;

use crate::error::{HwError, Result};

/// Chemin du périphérique noyau.
pub const DEVICE_PATH: &str = "/dev/tuxedo_io";
/// API minimale attendue du module (`MOD_API_MIN_VERSION` de l'original).
pub const MOD_API_MIN_VERSION: &str = "0.2.6";
/// Nombre maximal de TDP configurables exposés par Uniwill.
pub const MAX_TDP: usize = 3;
/// Nombre maximal de ventilateurs Clevo.
pub const MAX_CLEVO_FANS: usize = 3;
/// Nombre maximal de ventilateurs Uniwill.
pub const MAX_UNIWILL_FANS: usize = 2;

// ---------------------------------------------------------------------------
// Encodage ioctl Linux
// ---------------------------------------------------------------------------

const IOC_NRBITS: u32 = 8;
const IOC_TYPEBITS: u32 = 8;
const IOC_SIZEBITS: u32 = 14;
const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;
const IOC_NONE: u32 = 0;
const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;

/// Taille encodée : le header C utilise `int32_t*` et `char*`.
const PTR_SIZE: usize = std::mem::size_of::<*const u8>();

const fn ioc(dir: u32, kind: u8, nr: u8, size: usize) -> u64 {
    ((dir as u64) << IOC_DIRSHIFT)
        | ((size as u64) << IOC_SIZESHIFT)
        | ((kind as u64) << IOC_TYPESHIFT)
        | ((nr as u64) << IOC_NRSHIFT)
}

const fn ior(kind: u8, nr: u8) -> u64 {
    ioc(IOC_READ, kind, nr, PTR_SIZE)
}

const fn iow(kind: u8, nr: u8) -> u64 {
    ioc(IOC_WRITE, kind, nr, PTR_SIZE)
}

const fn io_none(kind: u8, nr: u8) -> u64 {
    ioc(IOC_NONE, kind, nr, 0)
}

/// `IOCTL_MAGIC` du header.
pub const IOCTL_MAGIC: u8 = 0xEC;
const MAGIC_READ_CL: u8 = IOCTL_MAGIC + 1;
const MAGIC_WRITE_CL: u8 = IOCTL_MAGIC + 2;
const MAGIC_READ_UW: u8 = IOCTL_MAGIC + 3;
const MAGIC_WRITE_UW: u8 = IOCTL_MAGIC + 4;

// Général.
/// `R_MOD_VERSION` — version texte du module (pointeur `char*`).
pub const R_MOD_VERSION: u64 = ior(IOCTL_MAGIC, 0x00);
/// `R_HWCHECK_CL` — 1 si l'interface Clevo est présente.
pub const R_HWCHECK_CL: u64 = ior(IOCTL_MAGIC, 0x05);
/// `R_HWCHECK_UW` — 1 si l'interface Uniwill est présente.
pub const R_HWCHECK_UW: u64 = ior(IOCTL_MAGIC, 0x06);

// Clevo — lecture.
pub const R_CL_HW_IF_STR: u64 = ior(MAGIC_READ_CL, 0x00);
pub const R_CL_FANINFO1: u64 = ior(MAGIC_READ_CL, 0x10);
pub const R_CL_FANINFO2: u64 = ior(MAGIC_READ_CL, 0x11);
pub const R_CL_FANINFO3: u64 = ior(MAGIC_READ_CL, 0x12);
pub const R_CL_WEBCAM_SW: u64 = ior(MAGIC_READ_CL, 0x13);
pub const R_CL_FLIGHTMODE_SW: u64 = ior(MAGIC_READ_CL, 0x14);
pub const R_CL_TOUCHPAD_SW: u64 = ior(MAGIC_READ_CL, 0x15);

// Clevo — écriture.
pub const W_CL_FANSPEED: u64 = iow(MAGIC_WRITE_CL, 0x10);
pub const W_CL_FANAUTO: u64 = iow(MAGIC_WRITE_CL, 0x11);
pub const W_CL_WEBCAM_SW: u64 = iow(MAGIC_WRITE_CL, 0x12);
pub const W_CL_FLIGHTMODE_SW: u64 = iow(MAGIC_WRITE_CL, 0x13);
pub const W_CL_TOUCHPAD_SW: u64 = iow(MAGIC_WRITE_CL, 0x14);
pub const W_CL_PERF_PROFILE: u64 = iow(MAGIC_WRITE_CL, 0x15);

// Uniwill — lecture.
pub const R_UW_HW_IF_STR: u64 = ior(MAGIC_READ_UW, 0x00);
pub const R_UW_MODEL_ID: u64 = ior(MAGIC_READ_UW, 0x01);
pub const R_UW_FANSPEED: u64 = ior(MAGIC_READ_UW, 0x10);
pub const R_UW_FANSPEED2: u64 = ior(MAGIC_READ_UW, 0x11);
pub const R_UW_FAN_TEMP: u64 = ior(MAGIC_READ_UW, 0x12);
pub const R_UW_FAN_TEMP2: u64 = ior(MAGIC_READ_UW, 0x13);
pub const R_UW_MODE: u64 = ior(MAGIC_READ_UW, 0x14);
pub const R_UW_MODE_ENABLE: u64 = ior(MAGIC_READ_UW, 0x15);
pub const R_UW_FANS_OFF_AVAILABLE: u64 = ior(MAGIC_READ_UW, 0x16);
pub const R_UW_FANS_MIN_SPEED: u64 = ior(MAGIC_READ_UW, 0x17);
pub const R_UW_TDP0: u64 = ior(MAGIC_READ_UW, 0x18);
pub const R_UW_TDP1: u64 = ior(MAGIC_READ_UW, 0x19);
pub const R_UW_TDP2: u64 = ior(MAGIC_READ_UW, 0x1A);
pub const R_UW_TDP0_MIN: u64 = ior(MAGIC_READ_UW, 0x1B);
pub const R_UW_TDP1_MIN: u64 = ior(MAGIC_READ_UW, 0x1C);
pub const R_UW_TDP2_MIN: u64 = ior(MAGIC_READ_UW, 0x1D);
pub const R_UW_TDP0_MAX: u64 = ior(MAGIC_READ_UW, 0x1E);
pub const R_UW_TDP1_MAX: u64 = ior(MAGIC_READ_UW, 0x1F);
pub const R_UW_TDP2_MAX: u64 = ior(MAGIC_READ_UW, 0x20);
pub const R_UW_PROFS_AVAILABLE: u64 = ior(MAGIC_READ_UW, 0x21);

// Uniwill — écriture.
pub const W_UW_FANSPEED: u64 = iow(MAGIC_WRITE_UW, 0x10);
pub const W_UW_FANSPEED2: u64 = iow(MAGIC_WRITE_UW, 0x11);
pub const W_UW_MODE: u64 = iow(MAGIC_WRITE_UW, 0x12);
pub const W_UW_MODE_ENABLE: u64 = iow(MAGIC_WRITE_UW, 0x13);
/// `W_UW_FANAUTO` est un `_IO` sans argument.
pub const W_UW_FANAUTO: u64 = io_none(MAGIC_WRITE_UW, 0x14);
pub const W_UW_TDP0: u64 = iow(MAGIC_WRITE_UW, 0x15);
pub const W_UW_TDP1: u64 = iow(MAGIC_WRITE_UW, 0x16);
pub const W_UW_TDP2: u64 = iow(MAGIC_WRITE_UW, 0x17);
pub const W_UW_PERF_PROF: u64 = iow(MAGIC_WRITE_UW, 0x18);

/// Requêtes `R_CL_FANINFO*` indexées par ventilateur.
pub const CL_FANINFO: [u64; MAX_CLEVO_FANS] = [R_CL_FANINFO1, R_CL_FANINFO2, R_CL_FANINFO3];
/// Requêtes `R_UW_FANSPEED*`.
pub const UW_FANSPEED: [u64; MAX_UNIWILL_FANS] = [R_UW_FANSPEED, R_UW_FANSPEED2];
/// Requêtes `R_UW_FAN_TEMP*`.
pub const UW_FAN_TEMP: [u64; MAX_UNIWILL_FANS] = [R_UW_FAN_TEMP, R_UW_FAN_TEMP2];
/// Requêtes `W_UW_FANSPEED*`.
pub const UW_FANSPEED_WRITE: [u64; MAX_UNIWILL_FANS] = [W_UW_FANSPEED, W_UW_FANSPEED2];
/// Requêtes `R_UW_TDP*`.
pub const UW_TDP_READ: [u64; MAX_TDP] = [R_UW_TDP0, R_UW_TDP1, R_UW_TDP2];
/// Requêtes `R_UW_TDP*_MIN`.
pub const UW_TDP_MIN: [u64; MAX_TDP] = [R_UW_TDP0_MIN, R_UW_TDP1_MIN, R_UW_TDP2_MIN];
/// Requêtes `R_UW_TDP*_MAX`.
pub const UW_TDP_MAX: [u64; MAX_TDP] = [R_UW_TDP0_MAX, R_UW_TDP1_MAX, R_UW_TDP2_MAX];
/// Requêtes `W_UW_TDP*`.
pub const UW_TDP_WRITE: [u64; MAX_TDP] = [W_UW_TDP0, W_UW_TDP1, W_UW_TDP2];

const CLEVO_MAX_FAN_RAW: u32 = 0xFF;
const UNIWILL_MAX_FAN_RAW: u32 = 0xC8;

/// Profils ODM Clevo (`clevoPerformanceProfilesToArgument`).
const CLEVO_ODM_PROFILES: [(&str, i32); 4] = [
    ("quiet", 0x00),
    ("power_saving", 0x01),
    ("performance", 0x02),
    ("entertainment", 0x03),
];

/// Profils ODM Uniwill (`uniwillPerformanceProfilesToArgument`).
const UNIWILL_ODM_PROFILES: [(&str, i32); 3] = [
    ("power_save", 0x01),
    ("enthusiast", 0x02),
    ("overboost", 0x03),
];

// ---------------------------------------------------------------------------
// Transport bas niveau
// ---------------------------------------------------------------------------

/// Transport brut vers `/dev/tuxedo_io`.
///
/// Les implémentations réelles et factices sont interchangeables pour les
/// tests. Les méthodes retournent `io::Result` : la couche haute ajoute le nom
/// de la requête et le contexte.
pub trait TuxedoIoTransport: Send + Sync {
    fn ioctl_read_i32(&self, request: u64) -> io::Result<i32>;
    fn ioctl_write_i32(&self, request: u64, value: i32) -> io::Result<()>;
    fn ioctl_write_none(&self, request: u64) -> io::Result<()>;
    fn ioctl_read_string(&self, request: u64) -> io::Result<String>;
}

/// Implémentation réelle, seul endroit du crate avec `unsafe`.
#[allow(unsafe_code)]
pub mod real {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::fd::{AsRawFd, RawFd};
    use std::os::unix::fs::OpenOptionsExt;

    use super::{DEVICE_PATH, TuxedoIoTransport};
    use crate::error::{HwError, Result};

    /// Taille du tampon de version. Le module original utilise 20 octets ; on
    /// garde une marge défensive sans jamais dépasser ce tampon.
    const VERSION_BUFFER_LEN: usize = 64;

    /// Fichier `/dev/tuxedo_io` ouvert en lecture/écriture.
    #[derive(Debug)]
    pub struct RealTransport {
        file: File,
    }

    impl RealTransport {
        /// Ouvre `/dev/tuxedo_io`.
        ///
        /// `O_NOFOLLOW` évite qu'un lien symbolique local détourne l'ouverture
        /// privilégiée. `O_CLOEXEC` évite la fuite du descripteur vers un
        /// processus enfant.
        pub fn open() -> Result<Self> {
            let mut options = OpenOptions::new();
            options
                .read(true)
                .write(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            let file = options
                .open(DEVICE_PATH)
                .map_err(|source| HwError::Unavailable {
                    device: DEVICE_PATH,
                    source,
                })?;
            Ok(Self { file })
        }

        fn fd(&self) -> RawFd {
            self.file.as_raw_fd()
        }
    }

    impl TuxedoIoTransport for RealTransport {
        fn ioctl_read_i32(&self, request: u64) -> io::Result<i32> {
            let mut value: i32 = 0;
            // SAFETY: `request` provient exclusivement des constantes de ce
            // module. `value` est un `i32` vivant sur la pile pour toute la
            // durée de l'appel ; le pilote noyau écrit au plus
            // `sizeof(int32_t)` octets à cette adresse (contrat du header
            // `tuxedo_io_ioctl.h`, argument `int32_t*`).
            let result =
                unsafe { libc::ioctl(self.fd(), request as libc::c_ulong, &mut value as *mut i32) };
            if result < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(value)
            }
        }

        fn ioctl_write_i32(&self, request: u64, value: i32) -> io::Result<()> {
            // SAFETY: même contrat que ci-dessus ; la valeur est copiée par le
            // noyau pendant l'appel, le pointeur reste valide.
            let result =
                unsafe { libc::ioctl(self.fd(), request as libc::c_ulong, &value as *const i32) };
            if result < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }

        fn ioctl_write_none(&self, request: u64) -> io::Result<()> {
            // SAFETY: requête `_IO` sans argument (`W_UW_FANAUTO`).
            let result = unsafe { libc::ioctl(self.fd(), request as libc::c_ulong) };
            if result < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }

        fn ioctl_read_string(&self, request: u64) -> io::Result<String> {
            let mut buffer = [0u8; VERSION_BUFFER_LEN];
            // SAFETY: le noyau écrit une chaîne bornée par le contrat du
            // pilote (20 octets pour la version). Le tampon fait 64 octets ;
            // `from_bytes_until_nul` ne lit jamais au-delà du NUL ou du
            // tampon. Aucun pointeur ne survit à l'appel.
            let result =
                unsafe { libc::ioctl(self.fd(), request as libc::c_ulong, buffer.as_mut_ptr()) };
            if result < 0 {
                return Err(io::Error::last_os_error());
            }
            let end = buffer
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(buffer.len());
            Ok(String::from_utf8_lossy(&buffer[..end]).trim().to_owned())
        }
    }
}

/// Backend factice pour les tests et la CI sans laptop TUXEDO.
///
/// Il reproduit la sémantique des deux interfaces (encodage Clevo sur 32 bits,
/// plages Uniwill, TDP) sans ouvrir de périphérique.
pub mod fake {
    use std::io;
    use std::sync::{Arc, Mutex};

    use super::{
        CL_FANINFO, MAX_CLEVO_FANS, MAX_TDP, MAX_UNIWILL_FANS, R_CL_FANINFO1, R_CL_FANINFO2,
        R_CL_FANINFO3, R_CL_HW_IF_STR, R_CL_WEBCAM_SW, R_HWCHECK_CL, R_HWCHECK_UW, R_MOD_VERSION,
        R_UW_FAN_TEMP, R_UW_FAN_TEMP2, R_UW_FANS_MIN_SPEED, R_UW_FANS_OFF_AVAILABLE, R_UW_FANSPEED,
        R_UW_FANSPEED2, R_UW_HW_IF_STR, R_UW_MODE, R_UW_MODE_ENABLE, R_UW_MODEL_ID,
        R_UW_PROFS_AVAILABLE, TuxedoIoTransport, UW_FAN_TEMP, UW_FANSPEED, UW_FANSPEED_WRITE,
        UW_TDP_MAX, UW_TDP_MIN, UW_TDP_READ, UW_TDP_WRITE, W_CL_FANAUTO, W_CL_FANSPEED,
        W_CL_PERF_PROFILE, W_CL_WEBCAM_SW, W_UW_FANAUTO, W_UW_FANSPEED, W_UW_FANSPEED2, W_UW_MODE,
        W_UW_MODE_ENABLE, W_UW_PERF_PROF,
    };

    #[derive(Debug)]
    struct State {
        clevo: bool,
        uniwill: bool,
        version: String,
        model_id: i32,
        clevo_fans: [u8; MAX_CLEVO_FANS],
        clevo_temps: [i32; MAX_CLEVO_FANS],
        clevo_webcam: bool,
        clevo_profile: i32,
        uw_fans: [u8; MAX_UNIWILL_FANS],
        uw_temps: [i32; MAX_UNIWILL_FANS],
        uw_mode_enable: bool,
        uw_profiles: i32,
        uw_profile: i32,
        uw_tdp: [i32; MAX_TDP],
        uw_tdp_min: [i32; MAX_TDP],
        uw_tdp_max: [i32; MAX_TDP],
    }

    impl State {
        fn new(clevo: bool, uniwill: bool) -> Self {
            Self {
                clevo,
                uniwill,
                version: "0.2.6".to_owned(),
                model_id: 0,
                clevo_fans: [0; MAX_CLEVO_FANS],
                clevo_temps: [45, 50, 55],
                clevo_webcam: true,
                clevo_profile: 0x02,
                uw_fans: [0; MAX_UNIWILL_FANS],
                uw_temps: [45, 55],
                uw_mode_enable: false,
                uw_profiles: 3,
                uw_profile: 0x03,
                uw_tdp: [35, 45, 60],
                uw_tdp_min: [10, 10, 20],
                uw_tdp_max: [80, 100, 120],
            }
        }
    }

    /// Transport factice.
    #[derive(Debug, Clone)]
    pub struct FakeTransport {
        state: Arc<Mutex<State>>,
    }

    impl Default for FakeTransport {
        fn default() -> Self {
            Self::clevo()
        }
    }

    impl FakeTransport {
        /// Machine Clevo : 3 ventilateurs, pas de TDP, webcam pilotable.
        pub fn clevo() -> Self {
            Self {
                state: Arc::new(Mutex::new(State::new(true, false))),
            }
        }

        /// Machine Uniwill : 2 ventilateurs, 3 TDP, pas de webcam.
        pub fn uniwill() -> Self {
            Self {
                state: Arc::new(Mutex::new(State::new(false, true))),
            }
        }

        /// Aucune interface reconnue.
        pub fn unsupported() -> Self {
            Self {
                state: Arc::new(Mutex::new(State::new(false, false))),
            }
        }

        /// Force la version du module (tests de compatibilité).
        pub fn with_version(self, version: &str) -> Self {
            if let Ok(mut state) = self.state.lock() {
                state.version = version.to_owned();
            }
            self
        }

        /// Force le nombre de profils ODM Uniwill exposés.
        pub fn with_profiles(self, count: i32) -> Self {
            if let Ok(mut state) = self.state.lock() {
                state.uw_profiles = count.max(0);
            }
            self
        }

        /// Force les limites TDP Uniwill.
        pub fn with_tdp_limits(self, min: [i32; MAX_TDP], max: [i32; MAX_TDP]) -> Self {
            if let Ok(mut state) = self.state.lock() {
                state.uw_tdp_min = min;
                state.uw_tdp_max = max;
            }
            self
        }

        /// Valeur brute du profil ODM courant (test).
        pub fn odm_profile_value(&self) -> i32 {
            self.lock()
                .map(|state| state.uw_profile)
                .unwrap_or_default()
        }

        /// Force la valeur brute du profil ODM courant (test).
        pub fn set_odm_profile_value(&self, value: i32) {
            if let Ok(mut state) = self.state.lock() {
                state.uw_profile = value;
            }
        }

        /// Valeur TDP courante simulée (test).
        pub fn tdp_current(&self, index: usize) -> Option<i32> {
            self.lock()
                .ok()
                .and_then(|state| state.uw_tdp.get(index).copied())
        }

        /// Force la température d'un ventilateur Clevo.
        pub fn set_clevo_temperature(&self, fan: usize, celsius: i32) {
            if let Ok(mut state) = self.state.lock() {
                if let Some(slot) = state.clevo_temps.get_mut(fan) {
                    *slot = celsius;
                }
            }
        }

        fn lock(&self) -> io::Result<std::sync::MutexGuard<'_, State>> {
            self.state
                .lock()
                .map_err(|_| io::Error::other("état factice empoisonné"))
        }
    }

    fn invalid() -> io::Error {
        io::Error::from_raw_os_error(libc::EINVAL)
    }

    fn tdp_index(request: u64, table: &[u64; MAX_TDP]) -> Option<usize> {
        table.iter().position(|value| *value == request)
    }

    impl TuxedoIoTransport for FakeTransport {
        fn ioctl_read_i32(&self, request: u64) -> io::Result<i32> {
            let state = self.lock()?;
            match request {
                R_HWCHECK_CL => Ok(i32::from(state.clevo)),
                R_HWCHECK_UW => Ok(i32::from(state.uniwill)),
                R_UW_MODEL_ID => Ok(state.model_id),
                R_CL_FANINFO1 | R_CL_FANINFO2 | R_CL_FANINFO3 => {
                    let index = CL_FANINFO
                        .iter()
                        .position(|value| *value == request)
                        .ok_or_else(invalid)?;
                    let speed = i32::from(state.clevo_fans[index]);
                    let temp = state.clevo_temps[index] & 0xFF;
                    // Bits 0-7 : vitesse brute, bits 8-15 : temp1,
                    // bits 16-23 : temp2 (l'original utilise temp2).
                    Ok(speed | (temp << 8) | (temp << 16))
                }
                R_CL_WEBCAM_SW => Ok(i32::from(state.clevo_webcam)),
                R_UW_FANSPEED | R_UW_FANSPEED2 => {
                    let index = UW_FANSPEED
                        .iter()
                        .position(|value| *value == request)
                        .ok_or_else(invalid)?;
                    Ok(i32::from(state.uw_fans[index]))
                }
                R_UW_FAN_TEMP | R_UW_FAN_TEMP2 => {
                    let index = UW_FAN_TEMP
                        .iter()
                        .position(|value| *value == request)
                        .ok_or_else(invalid)?;
                    Ok(state.uw_temps[index])
                }
                R_UW_MODE_ENABLE => Ok(i32::from(state.uw_mode_enable)),
                R_UW_MODE => Ok(0),
                R_UW_FANS_OFF_AVAILABLE => Ok(1),
                R_UW_FANS_MIN_SPEED => Ok(20),
                R_UW_PROFS_AVAILABLE => Ok(state.uw_profiles),
                _ => {
                    if let Some(index) = tdp_index(request, &UW_TDP_READ) {
                        return Ok(state.uw_tdp[index]);
                    }
                    if let Some(index) = tdp_index(request, &UW_TDP_MIN) {
                        return Ok(state.uw_tdp_min[index]);
                    }
                    if let Some(index) = tdp_index(request, &UW_TDP_MAX) {
                        return Ok(state.uw_tdp_max[index]);
                    }
                    Err(invalid())
                }
            }
        }

        fn ioctl_write_i32(&self, request: u64, value: i32) -> io::Result<()> {
            let mut state = self.lock()?;
            match request {
                W_CL_FANSPEED => {
                    state.clevo_fans = [
                        (value & 0xFF) as u8,
                        ((value >> 8) & 0xFF) as u8,
                        ((value >> 16) & 0xFF) as u8,
                    ];
                    Ok(())
                }
                W_CL_FANAUTO => {
                    state.clevo_fans = [0; MAX_CLEVO_FANS];
                    Ok(())
                }
                W_CL_WEBCAM_SW => {
                    state.clevo_webcam = value != 0;
                    Ok(())
                }
                W_CL_PERF_PROFILE => {
                    state.clevo_profile = value;
                    Ok(())
                }
                W_UW_FANSPEED | W_UW_FANSPEED2 => {
                    let index = UW_FANSPEED_WRITE
                        .iter()
                        .position(|candidate| *candidate == request)
                        .ok_or_else(invalid)?;
                    state.uw_fans[index] = value.clamp(0, 255) as u8;
                    Ok(())
                }
                W_UW_MODE_ENABLE => {
                    state.uw_mode_enable = value != 0;
                    Ok(())
                }
                W_UW_MODE => Ok(()),
                W_UW_PERF_PROF => {
                    state.uw_profile = value;
                    Ok(())
                }
                _ => {
                    if let Some(index) = tdp_index(request, &UW_TDP_WRITE) {
                        state.uw_tdp[index] = value;
                        return Ok(());
                    }
                    Err(invalid())
                }
            }
        }

        fn ioctl_write_none(&self, request: u64) -> io::Result<()> {
            let mut state = self.lock()?;
            if request == W_UW_FANAUTO {
                state.uw_fans = [0; MAX_UNIWILL_FANS];
                Ok(())
            } else {
                Err(invalid())
            }
        }

        fn ioctl_read_string(&self, request: u64) -> io::Result<String> {
            let state = self.lock()?;
            match request {
                R_MOD_VERSION => Ok(state.version.clone()),
                R_CL_HW_IF_STR => Ok("clevo".to_owned()),
                R_UW_HW_IF_STR => Ok("uniwill".to_owned()),
                _ => Err(invalid()),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// API haut niveau
// ---------------------------------------------------------------------------

/// Interface matérielle détectée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interface {
    Clevo,
    Uniwill,
}

impl Interface {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clevo => "clevo",
            Self::Uniwill => "uniwill",
        }
    }

    /// Nombre d'interfaces ventilateur exposées.
    pub const fn fan_count(self) -> u8 {
        match self {
            Self::Clevo => MAX_CLEVO_FANS as u8,
            Self::Uniwill => MAX_UNIWILL_FANS as u8,
        }
    }
}

/// API typée de `/dev/tuxedo_io`.
pub struct TuxedoIo {
    transport: Box<dyn TuxedoIoTransport>,
    interface: Interface,
    module_version: Option<String>,
    model_id: Option<i32>,
}

impl std::fmt::Debug for TuxedoIo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TuxedoIo")
            .field("interface", &self.interface)
            .field("module_version", &self.module_version)
            .field("model_id", &self.model_id)
            .finish_non_exhaustive()
    }
}

impl TuxedoIo {
    /// Ouvre le périphérique réel et détecte l'interface.
    pub fn open() -> Result<Self> {
        Self::with_transport(Box::new(real::RealTransport::open()?))
    }

    /// Construit l'API sur un transport fourni (tests, simulation).
    pub fn with_transport(transport: Box<dyn TuxedoIoTransport>) -> Result<Self> {
        // La détection utilise les mêmes requêtes que l'original : le module
        // répond 1 pour l'interface qu'il expose.
        let clevo = transport.ioctl_read_i32(R_HWCHECK_CL).unwrap_or(0) == 1;
        let uniwill = transport.ioctl_read_i32(R_HWCHECK_UW).unwrap_or(0) == 1;
        let interface = match (clevo, uniwill) {
            (true, _) => Interface::Clevo,
            (false, true) => Interface::Uniwill,
            (false, false) => return Err(HwError::UnsupportedDevice),
        };
        let module_version = transport.ioctl_read_string(R_MOD_VERSION).ok();
        if let Some(version) = module_version.as_deref() {
            if !version_at_least(version, MOD_API_MIN_VERSION) {
                tracing::warn!(
                    version,
                    minimum = MOD_API_MIN_VERSION,
                    "module tuxedo_io plus ancien que l'API attendue"
                );
            }
        }
        let model_id = if interface == Interface::Uniwill {
            transport.ioctl_read_i32(R_UW_MODEL_ID).ok()
        } else {
            None
        };
        Ok(Self {
            transport,
            interface,
            module_version,
            model_id,
        })
    }

    /// Interface détectée.
    pub fn interface(&self) -> Interface {
        self.interface
    }

    /// Version annoncée par le module, si l'ioctl a réussi.
    pub fn module_version(&self) -> Option<&str> {
        self.module_version.as_deref()
    }

    /// Vrai si la version du module respecte l'API minimale.
    pub fn api_compatible(&self) -> bool {
        self.module_version
            .as_deref()
            .is_some_and(|version| version_at_least(version, MOD_API_MIN_VERSION))
    }

    /// Identifiant de modèle Uniwill (`R_UW_MODEL_ID`).
    pub fn model_id(&self) -> Option<i32> {
        self.model_id
    }

    /// Nombre de ventilateurs pilotables.
    pub fn fan_count(&self) -> u8 {
        self.interface.fan_count()
    }

    /// Vitesse d'un ventilateur en pourcentage (0..=100).
    pub fn fan_speed_percent(&self, fan: u8) -> Result<u8> {
        let fan = self.validate_fan(fan)?;
        match self.interface {
            Interface::Clevo => {
                let info = self.read_i32("R_CL_FANINFO", CL_FANINFO[fan])?;
                Ok(raw_to_percent((info & 0xFF) as u8, CLEVO_MAX_FAN_RAW))
            }
            Interface::Uniwill => {
                let raw = self.read_i32("R_UW_FANSPEED", UW_FANSPEED[fan])?;
                Ok(raw_to_percent(raw.clamp(0, 255) as u8, UNIWILL_MAX_FAN_RAW))
            }
        }
    }

    /// Fixe la vitesse d'un ventilateur en pourcentage (0..=100).
    pub fn set_fan_speed_percent(&self, fan: u8, percent: u8) -> Result<()> {
        let fan = self.validate_fan(fan)?;
        if percent > 100 {
            return Err(HwError::OutOfRange {
                context: "vitesse ventilateur",
                value: i64::from(percent),
                min: 0,
                max: 100,
            });
        }
        match self.interface {
            Interface::Clevo => {
                // L'interface Clevo transmet les trois vitesses dans un seul
                // mot de 32 bits : il faut relire les autres ventilateurs.
                let mut raw = [0u8; MAX_CLEVO_FANS];
                for (index, slot) in raw.iter_mut().enumerate() {
                    if index == fan {
                        *slot = percent_to_raw(percent, CLEVO_MAX_FAN_RAW);
                    } else {
                        let info = self.read_i32("R_CL_FANINFO", CL_FANINFO[index])?;
                        *slot = (info & 0xFF) as u8;
                    }
                }
                let packed =
                    i32::from(raw[0]) | (i32::from(raw[1]) << 8) | (i32::from(raw[2]) << 16);
                self.write_i32("W_CL_FANSPEED", W_CL_FANSPEED, packed)
            }
            Interface::Uniwill => {
                let value = percent_to_raw(percent, UNIWILL_MAX_FAN_RAW);
                self.write_i32("W_UW_FANSPEED", UW_FANSPEED_WRITE[fan], i32::from(value))
            }
        }
    }

    /// Température (°C) associée à un ventilateur.
    pub fn fan_temperature(&self, fan: u8) -> Result<i32> {
        let fan = self.validate_fan(fan)?;
        match self.interface {
            Interface::Clevo => {
                let info = self.read_i32("R_CL_FANINFO", CL_FANINFO[fan])?;
                // L'original utilise explicitement temp2 (bits 16-23) :
                // temp1 est moins souvent implémentée par les EC.
                let temperature = ((info >> 16) & 0xFF) as u8 as i8;
                if temperature <= 1 {
                    return Err(HwError::SensorUnavailable {
                        sensor: "ventilateur Clevo",
                    });
                }
                Ok(i32::from(temperature))
            }
            Interface::Uniwill => {
                let temperature = self.read_i32("R_UW_FAN_TEMP", UW_FAN_TEMP[fan])?;
                if temperature <= 0 {
                    return Err(HwError::SensorUnavailable {
                        sensor: "ventilateur Uniwill",
                    });
                }
                Ok(temperature)
            }
        }
    }

    /// Remet tous les ventilateurs en mode automatique.
    pub fn set_fans_auto(&self) -> Result<()> {
        match self.interface {
            Interface::Clevo => {
                // 1 | 1<<1 | 1<<2 | 1<<3, comme l'original.
                self.write_i32("W_CL_FANAUTO", W_CL_FANAUTO, 0b1111)
            }
            Interface::Uniwill => self.write_none("W_UW_FANAUTO", W_UW_FANAUTO),
        }
    }

    /// Active/désactive le mode manuel Uniwill (no-op sur Clevo).
    pub fn set_mode_enable(&self, enabled: bool) -> Result<()> {
        match self.interface {
            Interface::Clevo => Ok(()),
            Interface::Uniwill => {
                self.write_i32("W_UW_MODE_ENABLE", W_UW_MODE_ENABLE, i32::from(enabled))
            }
        }
    }

    /// Vitesse minimale des ventilateurs en pourcentage.
    pub fn fans_min_speed(&self) -> Result<u8> {
        match self.interface {
            // Constante de l'original (`ClevoDevice::GetFansMinSpeed`).
            Interface::Clevo => Ok(20),
            Interface::Uniwill => {
                let value = self.read_i32("R_UW_FANS_MIN_SPEED", R_UW_FANS_MIN_SPEED)?;
                if !(0..=100).contains(&value) {
                    return Err(HwError::OutOfRange {
                        context: "vitesse minimale ventilateurs",
                        value: i64::from(value),
                        min: 0,
                        max: 100,
                    });
                }
                Ok(value as u8)
            }
        }
    }

    /// Vrai si l'arrêt complet des ventilateurs est supporté.
    pub fn fans_off_available(&self) -> Result<bool> {
        match self.interface {
            Interface::Clevo => Ok(true),
            Interface::Uniwill => {
                Ok(self.read_i32("R_UW_FANS_OFF_AVAILABLE", R_UW_FANS_OFF_AVAILABLE)? == 1)
            }
        }
    }

    /// État du commutateur webcam (Clevo uniquement).
    pub fn webcam_enabled(&self) -> Result<bool> {
        match self.interface {
            Interface::Clevo => Ok(self.read_i32("R_CL_WEBCAM_SW", R_CL_WEBCAM_SW)? == 1),
            Interface::Uniwill => Err(HwError::NotSupported("R_CL_WEBCAM_SW")),
        }
    }

    /// Active/désactive la webcam (Clevo uniquement).
    pub fn set_webcam_enabled(&self, enabled: bool) -> Result<()> {
        match self.interface {
            Interface::Clevo => {
                self.write_i32("W_CL_WEBCAM_SW", W_CL_WEBCAM_SW, i32::from(enabled))
            }
            Interface::Uniwill => Err(HwError::NotSupported("W_CL_WEBCAM_SW")),
        }
    }

    /// Profils ODM disponibles.
    pub fn odm_profiles(&self) -> Result<Vec<String>> {
        match self.interface {
            Interface::Clevo => Ok(CLEVO_ODM_PROFILES
                .iter()
                .map(|(name, _)| (*name).to_owned())
                .collect()),
            Interface::Uniwill => {
                let count = self.read_i32("R_UW_PROFS_AVAILABLE", R_UW_PROFS_AVAILABLE)?;
                let mut profiles = Vec::new();
                if count >= 2 {
                    profiles.push("power_save".to_owned());
                    profiles.push("enthusiast".to_owned());
                }
                if count >= 3 {
                    profiles.push("overboost".to_owned());
                }
                Ok(profiles)
            }
        }
    }

    /// Profil ODM par défaut, selon la logique de l'original.
    pub fn default_odm_profile(&self) -> Result<Option<String>> {
        match self.interface {
            Interface::Clevo => Ok(Some("performance".to_owned())),
            Interface::Uniwill => {
                let count = self.read_i32("R_UW_PROFS_AVAILABLE", R_UW_PROFS_AVAILABLE)?;
                if count <= 0 {
                    return Ok(None);
                }
                let tdp_count = self.tdp_count()?;
                let profile = if tdp_count > 0 {
                    // Cas « LEDs seules » : l'original désactive l'overboost.
                    "overboost"
                } else if count > 2 {
                    "overboost"
                } else {
                    "enthusiast"
                };
                Ok(Some(profile.to_owned()))
            }
        }
    }

    /// Applique un profil ODM.
    pub fn set_odm_profile(&self, profile: &str) -> Result<()> {
        match self.interface {
            Interface::Clevo => {
                let value = CLEVO_ODM_PROFILES
                    .iter()
                    .find(|(name, _)| *name == profile)
                    .map(|(_, value)| *value)
                    .ok_or_else(|| HwError::InvalidValue {
                        context: "profil ODM Clevo",
                        value: profile.to_owned(),
                    })?;
                self.write_i32("W_CL_PERF_PROFILE", W_CL_PERF_PROFILE, value)
            }
            Interface::Uniwill => {
                let value = UNIWILL_ODM_PROFILES
                    .iter()
                    .find(|(name, _)| *name == profile)
                    .map(|(_, value)| *value)
                    .ok_or_else(|| HwError::InvalidValue {
                        context: "profil ODM Uniwill",
                        value: profile.to_owned(),
                    })?;
                self.write_i32("W_UW_PERF_PROF", W_UW_PERF_PROF, value)
            }
        }
    }

    /// Nombre de TDP configurables (0 sur Clevo).
    pub fn tdp_count(&self) -> Result<u8> {
        match self.interface {
            Interface::Clevo => Ok(0),
            Interface::Uniwill => {
                for index in (0..MAX_TDP).rev() {
                    if let Ok(value) = self.read_i32("R_UW_TDP*_MIN", UW_TDP_MIN[index]) {
                        if value >= 0 {
                            return Ok((index + 1) as u8);
                        }
                    }
                }
                Ok(0)
            }
        }
    }

    /// Descripteurs TDP (`pl1`, `pl2`, `pl4`).
    pub fn tdp_descriptors(&self) -> Result<Vec<&'static str>> {
        let count = self.tdp_count()? as usize;
        Ok(["pl1", "pl2", "pl4"].into_iter().take(count).collect())
    }

    /// Valeur TDP courante en watts.
    pub fn tdp(&self, index: u8) -> Result<i32> {
        let index = self.validate_tdp(index)?;
        self.read_i32("R_UW_TDP", UW_TDP_READ[index])
    }

    /// Borne minimale TDP en watts.
    pub fn tdp_min(&self, index: u8) -> Result<i32> {
        let index = self.validate_tdp(index)?;
        self.read_i32("R_UW_TDP_MIN", UW_TDP_MIN[index])
    }

    /// Borne maximale TDP en watts.
    pub fn tdp_max(&self, index: u8) -> Result<i32> {
        let index = self.validate_tdp(index)?;
        self.read_i32("R_UW_TDP_MAX", UW_TDP_MAX[index])
    }

    /// Applique un TDP en watts après validation des bornes du firmware.
    pub fn set_tdp(&self, index: u8, watts: i32) -> Result<()> {
        let index = self.validate_tdp(index)?;
        let min = self.read_i32("R_UW_TDP_MIN", UW_TDP_MIN[index])?;
        let max = self.read_i32("R_UW_TDP_MAX", UW_TDP_MAX[index])?;
        if watts < min || watts > max {
            return Err(HwError::OutOfRange {
                context: "TDP",
                value: i64::from(watts),
                min: i64::from(min),
                max: i64::from(max),
            });
        }
        self.write_i32("W_UW_TDP", UW_TDP_WRITE[index], watts)
    }

    /// Remet les ventilateurs en automatique avant de rendre la main.
    ///
    /// Best-effort : l'arrêt du daemon ne doit pas échouer si le module a déjà
    /// disparu (suspension, déchargement).
    pub fn shutdown(&self) {
        if let Err(err) = self.set_fans_auto() {
            tracing::warn!(error = %err, "tuxedo-io : retour en mode auto impossible");
        }
        if let Err(err) = self.set_mode_enable(false) {
            tracing::debug!(error = %err, "tuxedo-io : désactivation du mode manuel impossible");
        }
    }

    fn validate_fan(&self, fan: u8) -> Result<usize> {
        if usize::from(fan) >= usize::from(self.fan_count()) {
            return Err(HwError::OutOfRange {
                context: "index ventilateur",
                value: i64::from(fan),
                min: 0,
                max: i64::from(self.fan_count()) - 1,
            });
        }
        Ok(usize::from(fan))
    }

    fn validate_tdp(&self, index: u8) -> Result<usize> {
        if self.interface != Interface::Uniwill {
            return Err(HwError::NotSupported("TDP"));
        }
        if usize::from(index) >= MAX_TDP {
            return Err(HwError::OutOfRange {
                context: "index TDP",
                value: i64::from(index),
                min: 0,
                max: (MAX_TDP - 1) as i64,
            });
        }
        Ok(usize::from(index))
    }

    fn read_i32(&self, name: &'static str, request: u64) -> Result<i32> {
        self.transport
            .ioctl_read_i32(request)
            .map_err(|source| HwError::Ioctl {
                name,
                request,
                source,
            })
    }

    fn write_i32(&self, name: &'static str, request: u64, value: i32) -> Result<()> {
        self.transport
            .ioctl_write_i32(request, value)
            .map_err(|source| HwError::Ioctl {
                name,
                request,
                source,
            })
    }

    fn write_none(&self, name: &'static str, request: u64) -> Result<()> {
        self.transport
            .ioctl_write_none(request)
            .map_err(|source| HwError::Ioctl {
                name,
                request,
                source,
            })
    }
}

fn percent_to_raw(percent: u8, max_raw: u32) -> u8 {
    let percent = u32::from(percent.min(100));
    ((percent * max_raw + 50) / 100) as u8
}

fn raw_to_percent(raw: u8, max_raw: u32) -> u8 {
    if max_raw == 0 {
        return 0;
    }
    ((u32::from(raw) * 100 + max_raw / 2) / max_raw).min(100) as u8
}

/// Parse une version `X.Y.Z`. Les suffixes éventuels sont ignorés.
pub fn parse_version(value: &str) -> Option<(u32, u32, u32)> {
    let mut parts = value.trim().split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    let patch_raw = parts.next().unwrap_or("0");
    let patch: String = patch_raw.chars().take_while(char::is_ascii_digit).collect();
    let patch = patch.parse::<u32>().ok()?;
    Some((major, minor, patch))
}

/// Vrai si `found` est au moins `minimum`. Une version illisible est refusée.
pub fn version_at_least(found: &str, minimum: &str) -> bool {
    match (parse_version(found), parse_version(minimum)) {
        (Some(found), Some(minimum)) => found >= minimum,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeTransport;
    use super::*;

    fn clevo() -> TuxedoIo {
        TuxedoIo::with_transport(Box::new(FakeTransport::clevo())).unwrap()
    }

    fn uniwill() -> TuxedoIo {
        TuxedoIo::with_transport(Box::new(FakeTransport::uniwill())).unwrap()
    }

    // Valeurs vérifiées avec le header C compilé sur x86_64 (sizeof(ptr)=8).
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn ioctl_numbers_match_c_header() {
        assert_eq!(R_MOD_VERSION, 0x8008_EC00);
        assert_eq!(R_HWCHECK_CL, 0x8008_EC05);
        assert_eq!(R_HWCHECK_UW, 0x8008_EC06);
        assert_eq!(R_CL_HW_IF_STR, 0x8008_ED00);
        assert_eq!(R_CL_FANINFO1, 0x8008_ED10);
        assert_eq!(W_CL_FANSPEED, 0x4008_EE10);
        assert_eq!(W_CL_FANAUTO, 0x4008_EE11);
        assert_eq!(W_CL_WEBCAM_SW, 0x4008_EE12);
        assert_eq!(R_CL_WEBCAM_SW, 0x8008_ED13);
        assert_eq!(W_CL_PERF_PROFILE, 0x4008_EE15);
        assert_eq!(R_UW_MODEL_ID, 0x8008_EF01);
        assert_eq!(R_UW_FANSPEED, 0x8008_EF10);
        assert_eq!(R_UW_FAN_TEMP, 0x8008_EF12);
        assert_eq!(W_UW_FANAUTO, 0x0000_F014);
        assert_eq!(R_UW_TDP0, 0x8008_EF18);
        assert_eq!(W_UW_TDP0, 0x4008_F015);
        assert_eq!(R_UW_PROFS_AVAILABLE, 0x8008_EF21);
        assert_eq!(W_UW_MODE_ENABLE, 0x4008_F013);
    }

    #[test]
    fn detects_clevo_and_uniwill() {
        assert_eq!(clevo().interface(), Interface::Clevo);
        assert_eq!(clevo().fan_count(), 3);
        assert_eq!(clevo().module_version(), Some("0.2.6"));
        assert!(clevo().api_compatible());

        assert_eq!(uniwill().interface(), Interface::Uniwill);
        assert_eq!(uniwill().fan_count(), 2);
    }

    #[test]
    fn unsupported_device_is_reported() {
        let result = TuxedoIo::with_transport(Box::new(FakeTransport::unsupported()));
        assert!(matches!(result, Err(HwError::UnsupportedDevice)));
    }

    #[test]
    fn old_api_version_is_flagged() {
        let io = TuxedoIo::with_transport(Box::new(FakeTransport::clevo().with_version("0.2.5")))
            .unwrap();
        assert!(!io.api_compatible());
    }

    #[test]
    fn clevo_fan_speed_roundtrip_and_range() {
        let io = clevo();
        io.set_fan_speed_percent(0, 50).unwrap();
        assert_eq!(io.fan_speed_percent(0).unwrap(), 50);
        // Les autres ventilateurs sont préservés.
        io.set_fan_speed_percent(1, 100).unwrap();
        assert_eq!(io.fan_speed_percent(0).unwrap(), 50);
        assert_eq!(io.fan_speed_percent(1).unwrap(), 100);
        assert!(io.set_fan_speed_percent(3, 50).is_err());
        assert!(io.set_fan_speed_percent(0, 101).is_err());
    }

    #[test]
    fn clevo_temperature_decoding() {
        let transport = FakeTransport::clevo();
        transport.set_clevo_temperature(0, 42);
        let io = TuxedoIo::with_transport(Box::new(transport)).unwrap();
        assert_eq!(io.fan_temperature(0).unwrap(), 42);
        assert!(io.fan_temperature(9).is_err());
    }

    #[test]
    fn uniwill_fan_speed_uses_200_scale() {
        let io = uniwill();
        io.set_fan_speed_percent(0, 50).unwrap();
        assert_eq!(io.fan_speed_percent(0).unwrap(), 50);
        io.set_fan_speed_percent(1, 100).unwrap();
        assert_eq!(io.fan_speed_percent(1).unwrap(), 100);
        assert!(io.fan_speed_percent(2).is_err());
    }

    #[test]
    fn fans_auto_and_mode_enable() {
        let io = uniwill();
        io.set_mode_enable(true).unwrap();
        io.set_fan_speed_percent(0, 80).unwrap();
        io.set_fans_auto().unwrap();
        assert_eq!(io.fan_speed_percent(0).unwrap(), 0);
        io.set_mode_enable(false).unwrap();
        io.shutdown();
    }

    #[test]
    fn webcam_only_on_clevo() {
        let io = clevo();
        assert!(io.webcam_enabled().unwrap());
        io.set_webcam_enabled(false).unwrap();
        assert!(!io.webcam_enabled().unwrap());

        assert!(matches!(
            uniwill().webcam_enabled(),
            Err(HwError::NotSupported(_))
        ));
        assert!(matches!(
            uniwill().set_webcam_enabled(true),
            Err(HwError::NotSupported(_))
        ));
    }

    #[test]
    fn odm_profiles_and_selection() {
        let clevo = clevo();
        let profiles = clevo.odm_profiles().unwrap();
        assert!(profiles.contains(&"quiet".to_owned()));
        assert!(clevo.set_odm_profile("entertainment").is_ok());
        assert!(clevo.set_odm_profile("nope").is_err());
        assert_eq!(
            clevo.default_odm_profile().unwrap().as_deref(),
            Some("performance")
        );

        let uniwill = uniwill();
        let profiles = uniwill.odm_profiles().unwrap();
        assert_eq!(profiles, vec!["power_save", "enthusiast", "overboost"]);
        assert!(uniwill.set_odm_profile("enthusiast").is_ok());
        assert_eq!(
            uniwill.default_odm_profile().unwrap().as_deref(),
            Some("overboost")
        );
    }

    #[test]
    fn tdp_is_uniwill_only_and_validated() {
        let io = uniwill();
        assert_eq!(io.tdp_count().unwrap(), 3);
        assert_eq!(io.tdp_descriptors().unwrap(), vec!["pl1", "pl2", "pl4"]);
        assert_eq!(io.tdp(0).unwrap(), 35);
        assert_eq!(io.tdp_min(0).unwrap(), 10);
        assert_eq!(io.tdp_max(0).unwrap(), 80);
        io.set_tdp(0, 55).unwrap();
        assert_eq!(io.tdp(0).unwrap(), 55);

        let err = io.set_tdp(0, 5).unwrap_err();
        assert!(matches!(err, HwError::OutOfRange { .. }));
        let err = io.set_tdp(0, 200).unwrap_err();
        assert!(matches!(err, HwError::OutOfRange { .. }));
        assert!(io.set_tdp(3, 50).is_err());

        let clevo = clevo();
        assert_eq!(clevo.tdp_count().unwrap(), 0);
        assert!(matches!(clevo.tdp(0), Err(HwError::NotSupported(_))));
    }

    #[test]
    fn odm_profiles_follow_firmware_count() {
        let no_profiles =
            TuxedoIo::with_transport(Box::new(FakeTransport::uniwill().with_profiles(1)))
                .expect("uniwill");
        assert!(no_profiles.odm_profiles().unwrap().is_empty());

        let two = TuxedoIo::with_transport(Box::new(FakeTransport::uniwill().with_profiles(2)))
            .expect("uniwill");
        assert_eq!(
            two.odm_profiles().unwrap(),
            vec!["power_save", "enthusiast"]
        );

        let three = TuxedoIo::with_transport(Box::new(FakeTransport::uniwill().with_profiles(3)))
            .expect("uniwill");
        assert_eq!(
            three.odm_profiles().unwrap(),
            vec!["power_save", "enthusiast", "overboost"]
        );
    }

    #[test]
    fn tdp_count_stops_at_last_exposed_limit() {
        let transport = FakeTransport::uniwill().with_tdp_limits([10, -1, -1], [80, 0, 0]);
        let io = TuxedoIo::with_transport(Box::new(transport)).expect("uniwill");
        assert_eq!(io.tdp_count().unwrap(), 1);
        assert_eq!(io.tdp_descriptors().unwrap(), vec!["pl1"]);
    }

    #[test]
    fn tdp_count_zero_when_firmware_has_no_limits() {
        let transport = FakeTransport::uniwill().with_tdp_limits([-1, -1, -1], [0, 0, 0]);
        let io = TuxedoIo::with_transport(Box::new(transport)).unwrap();
        assert_eq!(io.tdp_count().unwrap(), 0);
    }

    #[test]
    fn fan_min_and_off_availability() {
        let io = uniwill();
        assert_eq!(io.fans_min_speed().unwrap(), 20);
        assert!(io.fans_off_available().unwrap());
        let clevo = clevo();
        assert_eq!(clevo.fans_min_speed().unwrap(), 20);
        assert!(clevo.fans_off_available().unwrap());
    }

    #[test]
    fn version_parsing() {
        assert_eq!(parse_version("0.2.6"), Some((0, 2, 6)));
        assert_eq!(parse_version("1.0.0-rc1"), Some((1, 0, 0)));
        assert!(version_at_least("0.2.6", "0.2.6"));
        assert!(version_at_least("0.3.0", "0.2.6"));
        assert!(!version_at_least("0.2.5", "0.2.6"));
        assert!(!version_at_least("garbage", "0.2.6"));
    }

    #[test]
    fn percent_conversions_are_monotonic() {
        for percent in 0..=100u8 {
            let raw = percent_to_raw(percent, 0xFF);
            let back = raw_to_percent(raw, 0xFF);
            assert!(back.abs_diff(percent) <= 1, "{percent} -> {raw} -> {back}");
        }
    }

    #[test]
    fn real_open_without_device_is_unavailable() {
        // /dev/tuxedo_io n'existe pas en CI : l'erreur doit être propre.
        if std::path::Path::new(DEVICE_PATH).exists() {
            return;
        }
        assert!(matches!(TuxedoIo::open(), Err(HwError::Unavailable { .. })));
    }
}
