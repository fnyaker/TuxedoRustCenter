// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Runtime Tokio borné commun aux binaires admin.
//!
//! Les services `tccd-admin` et `tccd-admin-hw` sont courts, activés à la
//! demande et soumis à `TasksMax=16` dans systemd. Le runtime ne doit donc
//! **jamais** créer un worker par cœur : il utilise deux workers async et un
//! pool bloquant plafonné pour `spawn_blocking` (transactions disque, ioctl,
//! sysfs).
//!
//! Total borné : 1 thread principal + 2 workers + 4 bloqueurs = 7 threads,
//! indépendant du nombre de cœurs.

use std::io;

/// Threads async du runtime admin.
pub const RUNTIME_WORKER_THREADS: usize = 2;
/// Plafond du pool bloquant (`spawn_blocking`).
pub const RUNTIME_MAX_BLOCKING_THREADS: usize = 4;

// Garde-fous de dimensionnement : total borné (thread principal + workers +
// pool bloquant) strictement sous `TasksMax=16`.
const _: () = assert!(RUNTIME_WORKER_THREADS == 2);
const _: () = assert!(RUNTIME_MAX_BLOCKING_THREADS <= 4);
const _: () = assert!(1 + RUNTIME_WORKER_THREADS + RUNTIME_MAX_BLOCKING_THREADS < 16);

/// Construit le runtime borné d'un service admin.
///
/// # Errors
///
/// Erreur d'entrée/sortie du runtime Tokio.
pub fn build_runtime() -> io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(RUNTIME_WORKER_THREADS)
        .max_blocking_threads(RUNTIME_MAX_BLOCKING_THREADS)
        .enable_all()
        .build()
}

#[cfg(test)]
mod tests {
    use super::{RUNTIME_WORKER_THREADS, build_runtime};

    #[test]
    fn runtime_is_bounded_and_spawn_blocking_works() {
        let runtime = build_runtime().expect("runtime admin");
        assert_eq!(
            runtime.metrics().num_workers(),
            RUNTIME_WORKER_THREADS,
            "le runtime ne doit pas créer un worker par cœur"
        );

        // Les transactions admin utilisent `spawn_blocking` : il doit rester
        // fonctionnel avec ce runtime borné.
        let value = runtime.block_on(async {
            tokio::task::spawn_blocking(|| 42u8)
                .await
                .expect("spawn_blocking")
        });
        assert_eq!(value, 42);
    }
}
