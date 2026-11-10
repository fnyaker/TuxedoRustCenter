// SPDX-License-Identifier: GPL-3.0-or-later
//
// Ce fichier fait partie de TuxedoRustCenter.

//! Client non privilégié de TuxedoRustCenter : appels D-Bus génériques,
//! statistiques locales et opérations sur profils, partagés par `tcc-cli` et
//! les interfaces graphiques alternatives.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod bus;
pub mod jsonpath;
pub mod ops;
pub mod stats;
