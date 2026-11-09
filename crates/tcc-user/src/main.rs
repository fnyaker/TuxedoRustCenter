// SPDX-License-Identifier: GPL-3.0-or-later
//! Agent de session utilisateur `tccd-user`.
//!
//! Tant que le contrat D-Bus `tcc-ipc` n'est pas disponible, le binaire sert de
//! CLI de diagnostic et d'application manuelle. Le mode agent sans argument
//! sort proprement après un log explicite.

use std::process::ExitCode;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tcc_user::cli::run(std::env::args().skip(1).collect())
}
