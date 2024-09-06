// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: AGPL-3.0-or-later

//! # SQLite database backend.
//!
//! This module contains the SQLite database backend implementation.
//!

//use crate::error::NodeError;

use store::{
    database::{Collection, DbManager},
    Error,
};

use rusqlite::{params, Connection, OpenFlags, Result as SQLiteResult};

use std::path::Path;
use std::sync::{Arc, Mutex};

/// SQLite database manager.
#[derive(Clone)]
pub struct SqliteManager {
    path: String,
}

impl SqliteManager {
    /// Create a new SQLite database manager.
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_owned(),
        }
    }
}

impl Default for SqliteManager {
    fn default() -> Self {
        Self::new(":memory:")
    }
}

impl DbManager<SqliteCollection> for SqliteManager {
    fn create_collection(
        &self,
        identifier: &str,
        prefix: &str,
    ) -> Result<SqliteCollection, Error> {
        // Open a connection to the database.
        let conn = open(&self.path).map_err(|_| {
            Error::CreateStore("fail SQLite open connection".to_owned())
        })?;

        // Create statement to create a table.
        let stmt = format!(
            "CREATE TABLE IF NOT EXISTS {} (prefix TEXT NOT NULL, sn TEXT NOT NULL, value \
            BLOB NOT NULL, PRIMARY KEY (prefix, sn))",
            identifier
        );
        conn.execute(stmt.as_str(), ()).map_err(|_| {
            Error::CreateStore("fail SQLite create table".to_owned())
        })?;

        Ok(SqliteCollection::new(conn, identifier, prefix))
    }
}

/// SQLite collection
pub struct SqliteCollection {
    conn: Arc<Mutex<Connection>>,
    table: String,
    prefix: String,
}

impl SqliteCollection {
    /// Create a new SQLite collection.
    pub fn new(conn: Connection, table: &str, prefix: &str) -> Self {
        Self {
            conn: Arc::new(Mutex::new(conn)),
            table: table.to_owned(),
            prefix: prefix.to_owned(),
        }
    }

    /// Create a new iterartor filtering by prefix.
    fn make_iter<'a>(
        &'a self,
        reverse: bool,
    ) -> SQLiteResult<Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a>> {
        let order = if reverse { "DESC" } else { "ASC" };
        let conn = self.conn.lock().expect("open connection");
        let query = format!(
            "SELECT sn, value FROM {} WHERE prefix = ?1 ORDER BY sn {}",
            self.table, order,
        );
        let mut stmt = conn.prepare(&query)?;
        let mut rows = stmt.query(params![self.prefix])?;
        let mut values = Vec::new();
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            values.push((key, row.get(1)?));
        }
        Ok(Box::new(values.into_iter()))
    }
}

impl Collection for SqliteCollection {
    fn get(&self, key: &str) -> Result<Vec<u8>, Error> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| Error::Store("sqlite open connection".to_owned()))?;
        let query = format!(
            "SELECT value FROM {} WHERE prefix = ?1 AND sn = ?2",
            &self.table
        );
        let row: Vec<u8> = conn
            .query_row(&query, params![self.prefix, key], |row| row.get(0))
            .map_err(|_| Error::EntryNotFound)?;

        Ok(row)
    }

    fn put(&mut self, key: &str, data: &[u8]) -> Result<(), Error> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| Error::Store("sqlite open connection".to_owned()))?;
        let stmt = format!(
            "INSERT OR REPLACE INTO {} (prefix, sn, value) VALUES (?1, ?2, ?3)",
            &self.table
        );
        conn.execute(&stmt, params![self.prefix, key, data])
            .map_err(|_| Error::Store("sqlite insert error".to_owned()))?;
        Ok(())
    }

    fn del(&mut self, key: &str) -> Result<(), Error> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| Error::Store("SQLITE open connection".to_owned()))?;
        let stmt = format!(
            "DELETE FROM {} WHERE prefix = ?1 AND sn = ?2",
            &self.table
        );
        conn.execute(&stmt, params![self.prefix, key])
            .map_err(|_| Error::EntryNotFound)?;
        Ok(())
    }

    fn purge(&mut self) -> Result<(), Error> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| Error::Store("SQLITE open connection".to_owned()))?;
        let stmt = format!("DELETE FROM {} WHERE prefix = ?1", &self.table);
        conn.execute(&stmt, params![self.prefix])
            .map_err(|_| Error::Store("SQLITE purge error".to_owned()))?;
        Ok(())
    }

    fn iter<'a>(
        &'a self,
        reverse: bool,
    ) -> Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a> {
        match self.make_iter(reverse) {
            Ok(iter) => {
                let iterator = SQLiteIterator { iter };
                Box::new(iterator)
            }
            Err(_) => Box::new(std::iter::empty()),
        }
    }

    fn name(&self) -> &str {
        self.table.as_str()
    }
}

pub struct SQLiteIterator<'a> {
    pub iter: Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a>,
}

impl Iterator for SQLiteIterator<'_> {
    type Item = (String, Vec<u8>);

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }
}

/// Open a SQLite database connection.
pub fn open<P: AsRef<Path>>(path: P) -> Result<Connection, Error> {
    let path = path.as_ref();
    let mut flags = OpenFlags::default();
    flags.insert(OpenFlags::SQLITE_OPEN_READ_WRITE);
    flags.insert(OpenFlags::SQLITE_OPEN_CREATE);
    let conn = Connection::open_with_flags(path, flags)
        .map_err(|_| Error::Store("SQLite fail open connection".to_owned()))?;
    conn.execute_batch(
        "
        PRAGMA journal_mode=WAL;
        PRAGMA synchronous=NORMAL;
        ",
    )
    .map_err(|_| Error::Store("SQListe fail execute batch".to_owned()))?;
    Ok(conn)
}

#[cfg(test)]
mod tests {

    use super::*;
    use store::{
        database::{Collection, DbManager},
        test_store_trait,
    };

    test_store_trait! {
        unit_test_sqlite_manager:SqliteManager:SqliteCollection
    }
}
