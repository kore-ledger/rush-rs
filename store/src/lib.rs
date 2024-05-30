// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! Store module.
//!
//! This module contains the store implementation.
//!
//! # Example
//!
//! ```rust
//!
//! ```
//!

pub mod database;
pub mod error;
mod memory;
pub mod store;

pub use error::Error;
