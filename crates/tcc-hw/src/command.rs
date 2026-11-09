// SPDX-License-Identifier: GPL-3.0-or-later
//! Exécution de commandes externes allowlistées, sans shell.
//!
//! Trois outils externes sont indispensables à la parité fonctionnelle :
//! `nvidia-smi` (capteurs et limites NVIDIA), `prime-select` (état PRIME) et,
//! en repli documenté, `kscreen-doctor` (agent de session, hors de ce crate).
//!
//! Règles appliquées ici :
//!
//! - **Chemin absolu** issu d'une liste compile-time ([`NVIDIA_SMI`],
//!   [`PRIME_SELECT`]) : aucun `PATH`, aucun nom nu.
//! - **Arguments séparés** : `Command::new(program).args(args)`, jamais de
//!   chaîne passée à un shell.
//! - **Environnement minimal** : `env_clear` puis `PATH`, `LC_ALL` et `LANG`
//!   fixes. Un enfant n'hérite ni de secrets, ni de variables de session
//!   (`HOME`, `PWD`, `BASH_ENV`, `IFS`, ...).
//! - **Délai maximal** : un `nvidia-smi` bloqué (GPU en veille, driver coincé)
//!   ne doit pas geler le worker. Les tubes sont lus dans des threads dédiés
//!   pour éviter un interblocage si la sortie dépasse le tampon du noyau.
//! - **Lecture bornée dans le temps** : après la fin (ou le kill) du processus,
//!   les lecteurs disposent d'une échéance courte. Un descendant qui garderait
//!   `stdout`/`stderr` ouvert ne peut donc pas bloquer le worker : le thread
//!   lecteur est détaché et une erreur typée est retournée.
//! - **Sortie bornée** : 64 Kio maximum par flux ; au-delà, la commande est
//!   considérée comme invalide.
//! - **Aucune donnée sensible** : ces commandes ne reçoivent ni mot de passe,
//!   ni identifiant. Les messages d'erreur sont nettoyés avant d'être
//!   journalisés.
//!
//! Toute nouvelle commande doit être ajoutée à une constante de ce module et
//! revue par `security` : la surface d'attaque est volontairement fermée.

use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Taille maximale acceptée sur `stdout` et sur `stderr`.
const MAX_OUTPUT: u64 = 64 * 1024;
/// Taille maximale conservée dans un message d'erreur.
const MAX_ERROR_TEXT: usize = 512;
/// Marge accordée aux lecteurs après la fin (ou le kill) du processus.
///
/// Elle est volontairement courte : la sortie restante est déjà dans le tampon
/// du tube, et un descendant qui retient le descripteur ne doit pas immobiliser
/// le worker matériel.
const READ_GRACE: Duration = Duration::from_millis(100);
/// `PATH` minimal des enfants (les outils peuvent appeler des sous-commandes).
const CHILD_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";

/// Chemins absolus acceptés pour `nvidia-smi`, dans l'ordre de préférence.
pub const NVIDIA_SMI: [&str; 3] = [
    "/usr/bin/nvidia-smi",
    "/usr/sbin/nvidia-smi",
    "/bin/nvidia-smi",
];

/// Chemins absolus acceptés pour `prime-select`.
pub const PRIME_SELECT: [&str; 3] = [
    "/usr/bin/prime-select",
    "/usr/sbin/prime-select",
    "/sbin/prime-select",
];

/// Chemins absolus acceptés pour `dpkg-query` (lecture de la version de
/// `nvidia-prime`, utilisée pour identifier le `prime-select` TUXEDO).
pub const DPKG_QUERY: [&str; 3] = [
    "/usr/bin/dpkg-query",
    "/bin/dpkg-query",
    "/usr/sbin/dpkg-query",
];

/// Sortie d'une commande allowlistée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// `stdout` complet (borné).
    pub stdout: String,
    /// `stderr` complet (borné).
    pub stderr: String,
    /// Code de sortie, `None` si le processus a été tué par un signal.
    pub code: Option<i32>,
}

impl CommandOutput {
    /// Vrai si le code de sortie est `0`.
    #[must_use]
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// Première ligne non vide de `stdout`, sans espaces de bord.
    #[must_use]
    pub fn first_line(&self) -> Option<&str> {
        self.stdout
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
    }
}

/// Résout le premier chemin existant et exécutable d'une allowlist.
///
/// Les chemins sont des constantes de compilation : suivre un lien symbolique
/// (usrmerge, wrappers de distribution) est acceptable et n'ouvre pas de
/// surface d'attaque contrôlable par un utilisateur local.
#[must_use]
pub fn resolve(candidates: &[&'static str]) -> Option<&'static str> {
    candidates.iter().copied().find(|candidate| {
        let path = Path::new(candidate);
        match std::fs::metadata(path) {
            Ok(metadata) => metadata.is_file() && is_executable(&metadata),
            Err(_) => false,
        }
    })
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    true
}

/// Exécute une commande allowlistée avec un délai maximal.
///
/// # Errors
///
/// - [`crate::HwError::NotSupported`] si `program` est vide ;
/// - [`crate::HwError::External`] en cas d'échec de lancement, de dépassement du
///   délai, de délai de lecture dépassé, de sortie trop volumineuse ou de code
///   de sortie non nul.
pub fn run(
    program: &'static str,
    args: &[&str],
    timeout: Duration,
) -> crate::error::Result<CommandOutput> {
    if program.is_empty() || !program.starts_with('/') {
        return Err(crate::error::HwError::PolicyDenied {
            resource: "commande externe",
            reason: "chemin non absolu",
        });
    }
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .env("PATH", CHILD_PATH)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        // Les scripts `prime-select` peuvent lire `$HOME` ; on fixe une valeur
        // déterministe plutôt que d'hériter de la session de l'appelant.
        .env("HOME", "/root")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|source| crate::error::HwError::External {
            program,
            reason: format!("lancement impossible ({})", source.kind()),
        })?;

    let stdout_reader = child.stdout.take().map(spawn_reader);
    let stderr_reader = child.stderr.take().map(spawn_reader);

    let status = match wait_with_timeout(&mut child, started, timeout) {
        Ok(status) => status,
        Err(failure) => {
            // Laisse une chance aux lecteurs de récupérer les dernières
            // données, sans jamais dépasser une courte marge.
            let drain_deadline = Instant::now() + READ_GRACE;
            let _ = join_reader(stdout_reader, drain_deadline);
            let _ = join_reader(stderr_reader, drain_deadline);
            return Err(crate::error::HwError::External {
                program,
                reason: failure,
            });
        }
    };

    // La sortie restante est déjà dans le tube : la marge est suffisante pour
    // la lire même si le processus s'est terminé juste avant l'échéance.
    let read_deadline = (started + timeout).max(Instant::now() + READ_GRACE);
    let stdout = join_reader(stdout_reader, read_deadline)
        .map_err(|reason| crate::error::HwError::External { program, reason })?;
    let stderr = join_reader(stderr_reader, read_deadline)
        .map_err(|reason| crate::error::HwError::External { program, reason })?;

    let output = CommandOutput {
        stdout,
        stderr,
        code: status.code(),
    };
    if !output.success() {
        return Err(crate::error::HwError::External {
            program,
            reason: format!(
                "code {:?} : {}",
                output.code,
                sanitize(output.stderr.trim())
            ),
        });
    }
    Ok(output)
}

/// Lecteur de flux borné, avec canal pour ne jamais bloquer sur `join`.
#[derive(Debug)]
struct Reader {
    receiver: Receiver<io::Result<String>>,
    handle: JoinHandle<()>,
}

fn spawn_reader(mut stream: impl Read + Send + 'static) -> Reader {
    let (sender, receiver) = mpsc::sync_channel(1);
    let handle = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let result = stream
            .by_ref()
            .take(MAX_OUTPUT + 1)
            .read_to_end(&mut buffer)
            .and_then(|_| {
                if buffer.len() as u64 > MAX_OUTPUT {
                    Err(io::Error::other("sortie trop volumineuse"))
                } else {
                    Ok(String::from_utf8_lossy(&buffer).into_owned())
                }
            });
        // Le récepteur peut avoir été abandonné après un délai : ignorer.
        let _ = sender.send(result);
    });
    Reader { receiver, handle }
}

/// Attend le lecteur **au plus** jusqu'à `deadline`.
///
/// Un thread encore bloqué sur un tube retenu par un descendant est détaché :
/// il se terminera lorsque le descripteur sera fermé, mais le worker matériel
/// n'est plus immobilisé. Le risque résiduel (un thread détaché par invocation
/// pathologique) est documenté dans `docs/hardware/HARDWARE_MATRIX.md`.
fn join_reader(reader: Option<Reader>, deadline: Instant) -> std::result::Result<String, String> {
    let Some(reader) = reader else {
        return Ok(String::new());
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    match reader.receiver.recv_timeout(remaining) {
        Ok(Ok(text)) => {
            let _ = reader.handle.join();
            Ok(text)
        }
        Ok(Err(error)) => {
            let _ = reader.handle.join();
            Err(format!(
                "lecture de la sortie impossible ({})",
                error.kind()
            ))
        }
        Err(RecvTimeoutError::Timeout) => Err(format!(
            "délai de lecture dépassé ({} ms restants)",
            remaining.as_millis()
        )),
        Err(RecvTimeoutError::Disconnected) => Err("lecteur de sortie interrompu".to_owned()),
    }
}

/// Attend le processus en le tuant après `timeout`.
fn wait_with_timeout(
    child: &mut Child,
    started: Instant,
    timeout: Duration,
) -> std::result::Result<std::process::ExitStatus, String> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(error) => return Err(format!("attente impossible ({})", error.kind())),
        }
        if started.elapsed() >= timeout {
            // Best-effort : un `kill` échoué ne masque pas le dépassement de
            // délai, mais on récupère le zombie pour ne pas fuir de processus.
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("délai de {} ms dépassé", timeout.as_millis()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Nettoie un texte d'erreur : pas de caractère de contrôle, taille bornée.
fn sanitize(value: &str) -> String {
    let mut cleaned: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_ERROR_TEXT)
        .collect();
    if cleaned.trim().is_empty() {
        cleaned = "aucun détail".to_owned();
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_paths_only() {
        assert!(resolve(&["nvidia-smi"]).is_none());
        assert!(resolve(&["/nonexistent/tcc-nvidia-smi"]).is_none());
    }

    #[test]
    fn runs_a_bounded_allowed_command() {
        // `/bin/sh` n'est pas allowlisté en production, mais le helper accepte
        // tout chemin absolu fourni par une constante interne : ce test vérifie
        // le mécanisme d'exécution sans shell.
        let output = run("/bin/echo", &["hello", "world"], Duration::from_secs(2))
            .expect("echo doit fonctionner");
        assert_eq!(output.stdout.trim(), "hello world");
        assert!(output.success());
        assert_eq!(output.first_line(), Some("hello world"));
    }

    #[test]
    fn non_zero_exit_is_typed() {
        let error = run("/bin/false", &[], Duration::from_secs(2)).expect_err("échec attendu");
        assert!(matches!(error, crate::error::HwError::External { .. }));
    }

    #[test]
    fn relative_program_is_refused() {
        let error = run("echo", &["x"], Duration::from_secs(1)).expect_err("refus attendu");
        assert!(error.is_policy_denied());
    }

    #[test]
    fn timeout_kills_the_child() {
        let error =
            run("/bin/sleep", &["5"], Duration::from_millis(100)).expect_err("délai attendu");
        match error {
            crate::error::HwError::External { reason, .. } => {
                assert!(reason.contains("délai"), "{reason}");
            }
            other => panic!("erreur inattendue : {other:?}"),
        }
    }

    #[test]
    fn environment_is_scrubbed() {
        // `/usr/bin/env` est un chemin absolu fixe, exécutable sur Linux.
        if !Path::new("/usr/bin/env").exists() {
            return;
        }
        let output = run("/usr/bin/env", &[], Duration::from_secs(2)).expect("env");
        // Seul l'environnement minimal est transmis.
        assert!(output.stdout.contains("PATH=/usr/sbin:/usr/bin:/sbin:/bin"));
        assert!(output.stdout.contains("LC_ALL=C"));
        assert!(output.stdout.contains("HOME=/root"));
        assert!(!output.stdout.contains("PWD="), "{}", output.stdout);
        assert!(!output.stdout.contains("BASH_ENV="), "{}", output.stdout);
        assert!(!output.stdout.contains("IFS="), "{}", output.stdout);
    }

    #[test]
    fn descendant_holding_stdout_does_not_block_the_worker() {
        // Le shell lance un descendant qui hérite des tubes, puis se termine.
        // Sans échéance de lecture, `join_reader` bloquerait jusqu'à la fin du
        // descendant (2 s) ; ici la lecture expire peu après la commande.
        let started = Instant::now();
        let error = run(
            "/bin/sh",
            &["-c", "sleep 2 & exec /bin/true"],
            Duration::from_millis(100),
        )
        .expect_err("la lecture doit expirer");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(1),
            "blocage de {elapsed:?} malgré l'échéance"
        );
        match error {
            crate::error::HwError::External { reason, .. } => {
                assert!(
                    reason.contains("lecture") || reason.contains("délai"),
                    "{reason}"
                );
            }
            other => panic!("erreur inattendue : {other:?}"),
        }
    }

    #[test]
    fn sanitize_removes_control_characters() {
        assert_eq!(sanitize("a\nb\tc"), "abc");
        assert_eq!(sanitize("   "), "aucun détail");
        let long = "x".repeat(MAX_ERROR_TEXT + 10);
        assert_eq!(sanitize(&long).len(), MAX_ERROR_TEXT);
    }
}
