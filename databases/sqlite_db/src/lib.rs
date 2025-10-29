

//! # SQLite database module.
//!
//! This module contains the SQLite database backend implementation.
//!

//pub mod sqlite;

mod sqlite;

pub use sqlite::{SqliteCollection, SqliteManager};
