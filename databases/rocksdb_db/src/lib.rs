// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! RocksDB database module.
//!
//! This module contains the RocksDB database implementation.
//!

pub mod rocksdb;

pub use rocksdb::{RocksDbManager, RocksDbStore};
