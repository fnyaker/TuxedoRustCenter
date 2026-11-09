// SPDX-License-Identifier: GPL-3.0-or-later
//! Accès sysfs contrôlé.
//!
//! Toutes les opérations passent par [`SysfsRoot`], qui porte la racine
//! (`/sys` en production, un répertoire de test en CI). Les chemins sont
//! toujours relatifs à cette racine et ne peuvent pas en sortir.
//!
//! Les lectures suivent les liens symboliques : c'est indispensable, car
//! `/sys/class/*` et `/sys/bus/*` sont des arbres de liens vers les
//! périphériques réels. Les écritures, en revanche, ouvrent le fichier final
//! avec `O_NOFOLLOW` et vérifient avec `fstat` que le descripteur désigne un
//! fichier régulier, possédé par root et non inscriptible par le monde lorsque
//! la racine est `/sys`. Cela empêche qu'un lien symbolique ajouté par un
//! acteur local transforme une écriture privilégiée en écriture arbitraire.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use crate::error::{HwError, Result};

/// Racine sysfs par défaut.
pub const DEFAULT_SYSFS_ROOT: &str = "/sys";
/// Variable d'environnement permettant d'injecter une racine de test.
///
/// Elle n'est lue que si la feature Cargo `dev-overrides` est active **et** que
/// le processus n'est pas root. Voir [`SysfsRoot::from_env`].
pub const SYSFS_ROOT_ENV: &str = "TCC_SYSFS_ROOT";

/// Taille maximale d'une valeur écrite dans sysfs (un seul attribut).
const MAX_WRITE_LEN: usize = 4096;

/// Racine sysfs injectable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysfsRoot {
    root: PathBuf,
    /// Exige que les fichiers écrits soient possédés par root et non
    /// accessibles en écriture au monde. Actif pour `/sys` uniquement ; les
    /// racines de test appartiennent à l'utilisateur de CI.
    strict_ownership: bool,
}

impl Default for SysfsRoot {
    fn default() -> Self {
        Self::system()
    }
}

impl SysfsRoot {
    /// Crée une racine à partir d'un chemin existant ou de test.
    ///
    /// Les vérifications de propriétaire/permissions ne sont pas appliquées :
    /// réservez cette constructor aux tests et aux overrides de développement
    /// (non root). Utilisez [`SysfsRoot::system`] en production.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            strict_ownership: false,
        }
    }

    /// Racine réelle du système, avec vérification stricte du propriétaire et
    /// des permissions des fichiers écrits.
    pub fn system() -> Self {
        Self {
            root: PathBuf::from(DEFAULT_SYSFS_ROOT),
            strict_ownership: true,
        }
    }

    /// Lit `TCC_SYSFS_ROOT` et retourne la racine correspondante.
    ///
    /// **Overrides de développement uniquement** (D7) :
    /// - désactivé si la feature Cargo `dev-overrides` n'est pas compilée ;
    /// - refusé si l'UID effectif est `0` ;
    /// - retourne `None` si la variable est absente, vide, ou ne désigne pas un
    ///   répertoire.
    ///
    /// En release, `from_env_or_system()` retourne donc toujours `/sys`.
    #[cfg(feature = "dev-overrides")]
    pub fn from_env() -> Option<Self> {
        if !dev_override_allowed() {
            return None;
        }
        let value = std::env::var_os(SYSFS_ROOT_ENV)?;
        Self::from_override_value(&value)
    }

    /// La feature `dev-overrides` n'est pas compilée : la variable est ignorée.
    #[cfg(not(feature = "dev-overrides"))]
    pub fn from_env() -> Option<Self> {
        None
    }

    /// Construit une racine depuis la valeur d'override, sans lire
    /// l'environnement (testable).
    #[cfg(feature = "dev-overrides")]
    fn from_override_value(value: &std::ffi::OsStr) -> Option<Self> {
        if value.is_empty() {
            return None;
        }
        let path = PathBuf::from(value);
        if path.is_dir() {
            Some(Self::new(path))
        } else {
            None
        }
    }

    /// Racine système, sauf si `TCC_SYSFS_ROOT` désigne un répertoire valide.
    pub fn from_env_or_system() -> Self {
        Self::from_env().unwrap_or_else(Self::system)
    }

    /// Chemin de la racine.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Résout un chemin relatif en refusant `..`, les chemins absolus et les
    /// préfixes Windows. `CurDir` est toléré et normalisé.
    pub fn resolve(&self, relative: impl AsRef<Path>) -> Result<PathBuf> {
        let relative = relative.as_ref();
        if relative.is_absolute() {
            return Err(HwError::InvalidPath {
                path: relative.to_path_buf(),
            });
        }
        let mut normalized = PathBuf::new();
        for component in relative.components() {
            match component {
                Component::Normal(part) => normalized.push(part),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(HwError::InvalidPath {
                        path: relative.to_path_buf(),
                    });
                }
            }
        }
        Ok(self.root.join(normalized))
    }

    /// Vrai si le chemin existe (lien symbolique résolu).
    pub fn exists(&self, relative: impl AsRef<Path>) -> bool {
        self.resolve(relative)
            .map(|path| path.exists())
            .unwrap_or(false)
    }

    /// Vrai si le chemin est un fichier régulier après résolution des liens.
    pub fn is_file(&self, relative: impl AsRef<Path>) -> bool {
        self.resolve(relative)
            .map(|path| path.is_file())
            .unwrap_or(false)
    }

    /// Liste les entrées d'un répertoire, triées.
    ///
    /// Un répertoire absent retourne une liste vide : les énumérations de
    /// classes sysfs (`/sys/class/backlight`, `/sys/class/power_supply`, ...)
    /// doivent dégrader proprement sur du matériel qui ne les expose pas.
    pub fn list_dir(&self, relative: impl AsRef<Path>) -> Result<Vec<String>> {
        let path = self.resolve(relative)?;
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(HwError::io(path, err)),
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|err| HwError::io(path.clone(), err))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            names.push(name);
        }
        names.sort();
        Ok(names)
    }

    /// Lit un attribut texte et supprime les blancs de fin (dont le `\n`).
    pub fn read_string(&self, relative: impl AsRef<Path>) -> Result<String> {
        let path = self.resolve(relative)?;
        read_trimmed(&path)
    }

    /// Comme [`SysfsRoot::read_string`], mais retourne `None` si absent.
    pub fn read_optional_string(&self, relative: impl AsRef<Path>) -> Result<Option<String>> {
        match self.read_string(relative) {
            Ok(value) => Ok(Some(value)),
            Err(HwError::NotFound { .. }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Lit un entier signé.
    pub fn read_i64(&self, relative: impl AsRef<Path>) -> Result<i64> {
        let path = self.resolve(relative)?;
        let value = read_trimmed(&path)?;
        value.parse::<i64>().map_err(|_| HwError::InvalidValue {
            context: "entier sysfs",
            value,
        })
    }

    /// Lit un entier non signé.
    pub fn read_u64(&self, relative: impl AsRef<Path>) -> Result<u64> {
        let path = self.resolve(relative)?;
        let value = read_trimmed(&path)?;
        value.parse::<u64>().map_err(|_| HwError::InvalidValue {
            context: "entier sysfs positif",
            value,
        })
    }

    /// Lit un booléen sysfs (`0`/`1`, `Y`/`N`, `true`/`false`).
    pub fn read_bool(&self, relative: impl AsRef<Path>) -> Result<bool> {
        let path = self.resolve(relative)?;
        let value = read_trimmed(&path)?;
        parse_bool(&value).ok_or(HwError::InvalidValue {
            context: "booléen sysfs",
            value,
        })
    }

    /// Lit une liste de mots séparés par des blancs.
    pub fn read_string_list(&self, relative: impl AsRef<Path>) -> Result<Vec<String>> {
        let value = self.read_string(relative)?;
        Ok(value.split_whitespace().map(str::to_owned).collect())
    }

    /// Lit une liste CPU au format noyau (`0-3`, `0,2-3`, `0 2-3`).
    pub fn read_cpu_list(&self, relative: impl AsRef<Path>) -> Result<Vec<u32>> {
        let value = self.read_string(relative)?;
        parse_cpu_list(&value)
    }

    /// Écrit une valeur texte dans un attribut sysfs.
    ///
    /// Le fichier est ouvert avec `O_NOFOLLOW|O_CLOEXEC`, son type est vérifié
    /// via le descripteur (`fstat`), puis un `write` unique est effectué. Un
    /// attribut sysfs attend une valeur atomique ; plusieurs `write` pourraient
    /// être interprétés comme plusieurs commandes par le noyau.
    pub fn write_string(&self, relative: impl AsRef<Path>, value: &str) -> Result<()> {
        let path = self.resolve(relative)?;
        validate_write_value(value)?;
        let mut file = open_for_write(&path, self.strict_ownership)?;
        file.write_all(value.as_bytes())
            .map_err(|err| HwError::io(&path, err))
    }

    /// Écrit un entier signé.
    pub fn write_i64(&self, relative: impl AsRef<Path>, value: i64) -> Result<()> {
        self.write_string(relative, &value.to_string())
    }

    /// Écrit un booléen sous la forme `1`/`0`.
    pub fn write_bool(&self, relative: impl AsRef<Path>, value: bool) -> Result<()> {
        self.write_string(relative, if value { "1" } else { "0" })
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "1" | "Y" | "y" | "true" | "TRUE" => Some(true),
        "0" | "N" | "n" | "false" | "FALSE" => Some(false),
        _ => None,
    }
}

/// Vrai si les overrides de développement sont autorisés.
///
/// Refus si l'UID effectif est root ou illisible (D7). On lit
/// `/proc/self/status` plutôt que d'appeler `geteuid()` : cela évite une
/// seconde zone `unsafe` hors du wrapper ioctl, et `ProtectProc=invisible` ne
/// masque pas le statut du processus courant.
#[cfg(feature = "dev-overrides")]
fn dev_override_allowed() -> bool {
    effective_uid().is_some_and(|uid| uid != 0)
}

#[cfg(feature = "dev-overrides")]
fn effective_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_effective_uid(&status)
}

#[cfg(feature = "dev-overrides")]
fn parse_effective_uid(status: &str) -> Option<u32> {
    // Ligne : `Uid:\t<real>\t<effective>\t<saved>\t<fs>`
    let line = status.lines().find(|line| line.starts_with("Uid:"))?;
    let mut fields = line.split_whitespace();
    let _label = fields.next()?;
    let _real = fields.next()?;
    fields.next()?.parse::<u32>().ok()
}

fn read_trimmed(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents.trim().to_owned()),
        Err(err) if err.kind() == io::ErrorKind::InvalidData => {
            // `read_to_string` peut échouer sur des octets non UTF-8.
            Err(HwError::NonUtf8 {
                path: path.to_path_buf(),
            })
        }
        Err(err) => Err(HwError::io_or_not_found(path, err)),
    }
}

/// Refuse les valeurs qui casseraient le protocole attribut/valeur de sysfs.
fn validate_write_value(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(HwError::InvalidValue {
            context: "valeur sysfs vide",
            value: String::new(),
        });
    }
    if value.len() > MAX_WRITE_LEN {
        return Err(HwError::InvalidValue {
            context: "valeur sysfs trop longue",
            value: format!("<{} octets>", value.len()),
        });
    }
    if value
        .bytes()
        .any(|byte| byte == 0 || byte.is_ascii_control())
    {
        return Err(HwError::InvalidValue {
            context: "valeur sysfs avec caractère de contrôle",
            value: value.escape_default().to_string(),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn open_for_write(path: &Path, strict_ownership: bool) -> Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let mut options = OpenOptions::new();
    // `O_TRUNC` est ignoré par kernfs/sysfs mais rend les fichiers de test
    // fidèles à un attribut qui contient une seule valeur. Il évite qu'une
    // valeur plus courte laisse un suffixe de l'ancienne valeur.
    options
        .write(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = match options.open(path) {
        Ok(file) => file,
        Err(err) if err.raw_os_error() == Some(libc::ELOOP) => {
            return Err(HwError::UnsafeTarget {
                path: path.to_path_buf(),
            });
        }
        Err(err) => return Err(HwError::io_or_not_found(path, err)),
    };
    // `File::metadata` utilise `fstat` sur le descripteur déjà ouvert : pas de
    // course entre la vérification et l'écriture.
    let metadata = file.metadata().map_err(|err| HwError::io(path, err))?;
    if !metadata.file_type().is_file() {
        return Err(HwError::UnsafeTarget {
            path: path.to_path_buf(),
        });
    }
    if strict_ownership && !ownership_is_safe(metadata.uid(), metadata.mode()) {
        return Err(HwError::UnsafePermissions {
            path: path.to_path_buf(),
        });
    }
    Ok(file)
}

/// Vérifie qu'un attribut est possédé par root et non inscriptible par le
/// monde. Les attributs sysfs standard sont `root:root 0644` ou `0600`.
///
/// Le bit d'écriture de groupe est toléré : certains pilotes exposent des
/// attributs `0664` à un groupe privilégié, et le daemon tourne de toute façon
/// en root. Le bit « other write » est en revanche toujours refusé.
#[cfg(unix)]
fn ownership_is_safe(uid: u32, mode: u32) -> bool {
    uid == 0 && (mode & 0o002) == 0
}

#[cfg(not(unix))]
fn open_for_write(path: &Path, _strict_ownership: bool) -> Result<File> {
    // La cible est Linux ; ce fallback permet seulement de compiler les tests
    // de logique pure sur une autre plateforme.
    let metadata = fs::metadata(path).map_err(|err| HwError::io_or_not_found(path, err))?;
    if !metadata.is_file() {
        return Err(HwError::UnsafeTarget {
            path: path.to_path_buf(),
        });
    }
    OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|err| HwError::io(path, err))
}

/// Parse une liste CPU au format noyau.
///
/// Exemples : `0-7`, `0,2-3`, `0 2-3`. Le résultat est trié et dédupliqué.
pub fn parse_cpu_list(value: &str) -> Result<Vec<u32>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    // Le noyau utilise la virgule ; certaines interfaces hwmon utilisent
    // l'espace. On accepte les deux.
    for token in value.split([',', ' ', '\t']) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some((start, end)) = token.split_once('-') {
            let start = parse_cpu_index(start, value)?;
            let end = parse_cpu_index(end, value)?;
            if start > end {
                return Err(HwError::InvalidValue {
                    context: "liste CPU (intervalle inversé)",
                    value: value.to_owned(),
                });
            }
            // Borne défensive : un noyau ne doit jamais annoncer des millions
            // de cœurs, et une entrée corrompue ne doit pas allouer sans fin.
            if end - start > 4096 {
                return Err(HwError::InvalidValue {
                    context: "liste CPU (intervalle trop grand)",
                    value: value.to_owned(),
                });
            }
            result.extend(start..=end);
        } else {
            result.push(parse_cpu_index(token, value)?);
        }
    }
    result.sort_unstable();
    result.dedup();
    Ok(result)
}

fn parse_cpu_index(token: &str, full: &str) -> Result<u32> {
    token
        .trim()
        .parse::<u32>()
        .map_err(|_| HwError::InvalidValue {
            context: "liste CPU",
            value: full.to_owned(),
        })
}

/// Formate une liste CPU triée au format noyau avec intervalles.
pub fn format_cpu_list(cpus: &[u32]) -> String {
    let mut sorted = cpus.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut parts: Vec<String> = Vec::new();
    let mut index = 0;
    while index < sorted.len() {
        let start = sorted[index];
        let mut end = start;
        while index + 1 < sorted.len() && sorted[index + 1] == end + 1 {
            index += 1;
            end = sorted[index];
        }
        if start == end {
            parts.push(start.to_string());
        } else {
            parts.push(format!("{start}-{end}"));
        }
        index += 1;
    }
    parts.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn write_file(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        let mut file = fs::File::create(path).expect("create");
        file.write_all(contents.as_bytes()).expect("write");
    }

    #[test]
    fn read_and_write_roundtrip() {
        let dir = fixture();
        write_file(dir.path(), "class/leds/test/brightness", "42\n");
        let sys = SysfsRoot::new(dir.path());

        assert_eq!(sys.read_i64("class/leds/test/brightness").unwrap(), 42);
        sys.write_i64("class/leds/test/brightness", 7).unwrap();
        assert_eq!(sys.read_i64("class/leds/test/brightness").unwrap(), 7);
    }

    #[test]
    fn missing_file_is_not_found() {
        let dir = fixture();
        let sys = SysfsRoot::new(dir.path());
        assert!(matches!(
            sys.read_string("missing"),
            Err(HwError::NotFound { .. })
        ));
    }

    #[test]
    fn path_traversal_is_rejected() {
        let dir = fixture();
        let sys = SysfsRoot::new(dir.path());
        assert!(matches!(
            sys.read_string("../../etc/passwd"),
            Err(HwError::InvalidPath { .. })
        ));
        assert!(matches!(
            sys.resolve("/etc/passwd"),
            Err(HwError::InvalidPath { .. })
        ));
    }

    #[test]
    fn boolean_parsing_accepts_kernel_forms() {
        let dir = fixture();
        let sys = SysfsRoot::new(dir.path());
        write_file(dir.path(), "a", "1\n");
        write_file(dir.path(), "b", "0\n");
        write_file(dir.path(), "c", "N\n");
        write_file(dir.path(), "d", "oops\n");
        assert!(sys.read_bool("a").unwrap());
        assert!(!sys.read_bool("b").unwrap());
        assert!(!sys.read_bool("c").unwrap());
        assert!(matches!(
            sys.read_bool("d"),
            Err(HwError::InvalidValue { .. })
        ));
    }

    #[test]
    fn write_rejects_control_characters() {
        let dir = fixture();
        write_file(dir.path(), "attr", "0\n");
        let sys = SysfsRoot::new(dir.path());
        assert!(matches!(
            sys.write_string("attr", "1\n2"),
            Err(HwError::InvalidValue { .. })
        ));
        assert!(matches!(
            sys.write_string("attr", ""),
            Err(HwError::InvalidValue { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn write_refuses_symlink() {
        let dir = fixture();
        write_file(dir.path(), "target", "0\n");
        std::os::unix::fs::symlink(dir.path().join("target"), dir.path().join("link")).unwrap();
        let sys = SysfsRoot::new(dir.path());
        assert!(matches!(
            sys.write_string("link", "1"),
            Err(HwError::UnsafeTarget { .. })
        ));
        // La cible n'a pas été modifiée.
        assert_eq!(
            fs::read_to_string(dir.path().join("target")).unwrap(),
            "0\n"
        );
    }

    #[test]
    fn write_refuses_directory() {
        let dir = fixture();
        fs::create_dir(dir.path().join("dir")).unwrap();
        let sys = SysfsRoot::new(dir.path());
        assert!(sys.write_string("dir", "1").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn ownership_policy_requires_root_and_no_world_write() {
        assert!(ownership_is_safe(0, 0o644));
        assert!(ownership_is_safe(0, 0o664));
        assert!(!ownership_is_safe(0, 0o646));
        assert!(!ownership_is_safe(0, 0o666));
        assert!(!ownership_is_safe(1000, 0o644));
    }

    #[cfg(unix)]
    #[test]
    fn strict_root_rejects_user_owned_test_file() {
        use std::os::unix::fs::MetadataExt;

        let dir = fixture();
        write_file(dir.path(), "attr", "0\n");
        let uid = fs::metadata(dir.path().join("attr")).unwrap().uid();
        if uid == 0 {
            // CI en root : le fichier appartient à root, le test de refus ne
            // s'applique pas.
            return;
        }
        let sys = SysfsRoot {
            root: dir.path().to_path_buf(),
            strict_ownership: true,
        };
        assert!(matches!(
            sys.write_string("attr", "1"),
            Err(HwError::UnsafePermissions { .. })
        ));
    }

    #[test]
    fn list_dir_sorted_and_missing_empty() {
        let dir = fixture();
        let sys = SysfsRoot::new(dir.path());
        write_file(dir.path(), "class/backlight/z/name", "z");
        write_file(dir.path(), "class/backlight/a/name", "a");
        assert_eq!(
            sys.list_dir("class/backlight").unwrap(),
            vec!["a".to_owned(), "z".to_owned()]
        );
        assert!(sys.list_dir("class/power_supply").unwrap().is_empty());
    }

    #[test]
    fn cpu_list_parsing_and_formatting() {
        assert_eq!(parse_cpu_list("0-3").unwrap(), vec![0, 1, 2, 3]);
        assert_eq!(parse_cpu_list("0,2-3").unwrap(), vec![0, 2, 3]);
        assert_eq!(parse_cpu_list("0 2-3").unwrap(), vec![0, 2, 3]);
        assert!(parse_cpu_list("3-2").is_err());
        assert!(parse_cpu_list("x").is_err());
        assert_eq!(format_cpu_list(&[3, 1, 2, 7]), "1-3,7");
    }

    #[test]
    fn from_env_is_ignored_when_missing_directory() {
        // On teste uniquement le helper de résolution, sans toucher à
        // l'environnement global du processus (tests parallèles).
        assert!(SysfsRoot::new("/nonexistent-tcc-root").resolve("a").is_ok());
    }

    #[cfg(feature = "dev-overrides")]
    #[test]
    fn dev_override_requires_existing_directory() {
        assert!(SysfsRoot::from_override_value(std::ffi::OsStr::new("")).is_none());
        assert!(
            SysfsRoot::from_override_value(std::ffi::OsStr::new("/nonexistent-tcc-root")).is_none()
        );
        let dir = fixture();
        assert_eq!(
            SysfsRoot::from_override_value(dir.path().as_os_str()),
            Some(SysfsRoot::new(dir.path()))
        );
    }

    #[cfg(feature = "dev-overrides")]
    #[test]
    fn effective_uid_parsing() {
        assert_eq!(
            parse_effective_uid("Name:\ttcc\nUid:\t1000\t1000\t1000\t1000\n"),
            Some(1000)
        );
        assert_eq!(parse_effective_uid("Uid:\t1000\t0\t0\t0\n"), Some(0));
        assert_eq!(parse_effective_uid("Name:\ttcc\n"), None);
    }
}
