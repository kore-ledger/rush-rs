// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Errors module
//!

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error type for the actor system.
#[derive(Clone, Debug, Error, Serialize, Deserialize, PartialEq)]
pub enum Error {
    /// Create store error.
    #[error("Can't create store: {0}")]
    CreateStore(String),
    /// Get data error.
    #[error("Get error: {0}")]
    Get(String),
    /// Entry not found error.
    #[error("Entry not found: {0}")]
    EntryNotFound(String),
    /// Store  Error.
    #[error("Store error: {0}")]
    Store(String),
}
