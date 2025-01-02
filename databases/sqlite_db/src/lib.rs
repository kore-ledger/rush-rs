// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # SQLite database module.
//!
//! This module contains the SQLite database backend implementation.
//!

//pub mod sqlite;

mod sqlite;

pub use sqlite::{SqliteCollection, SqliteManager};
