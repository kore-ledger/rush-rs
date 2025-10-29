

//! # SQLite database backend.
//!
//! This module contains the SQLite database backend implementation.
//!

//use crate::error::NodeError;

use store::{
    Error,
    database::{Collection, DbManager, State},
};

use rusqlite::{Connection, OpenFlags, Result as SQLiteResult, params};
use tracing::info;

use std::sync::{Arc, Mutex};
use std::{fs, path::Path};

/// SQLite database manager for persistent actor storage.
/// Manages SQLite database connections and provides factory methods
/// for creating collections (event storage) and state storage (snapshots).
///
/// # Storage Model
///
/// - **Collections**: SQLite tables with (prefix, sn, value) schema
/// - **State**: SQLite tables with (prefix, value) schema
/// - **Connection**: Thread-safe shared connection using Arc<Mutex<Connection>>
///
#[derive(Clone)]
pub struct SqliteManager {
    /// Thread-safe shared SQLite connection.
    conn: Arc<Mutex<Connection>>,
}

impl SqliteManager {
    /// Creates a new SQLite database manager.
    /// Opens or creates a SQLite database file at the specified path.
    ///
    /// # Arguments
    ///
    /// * `path` - Directory path where the database file will be created.
    ///            The database file will be named "database.db" within this directory.
    ///
    /// # Returns
    ///
    /// Returns a new SqliteManager instance.
    ///
    /// # Errors
    ///
    /// Returns Error::CreateStore if:
    /// - The directory cannot be created
    /// - The SQLite connection cannot be opened
    ///
    pub fn new(path: &str) -> Result<Self, Error> {
        info!("Creating SQLite database manager");
        if !Path::new(&path).exists() {
            info!("Path does not exist, creating it");
            fs::create_dir_all(path).map_err(|e| {
                Error::CreateStore(format!(
                    "fail SQLite create directory: {}",
                    e
                ))
            })?;
        }

        info!("Opening SQLite connection");
        let conn = open(format!("{}/database.db", path)).map_err(|e| {
            Error::CreateStore(format!("fail SQLite open connection: {}", e))
        })?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

impl DbManager<SqliteCollection, SqliteCollection> for SqliteManager {
    fn create_state(
        &self,
        identifier: &str,
        prefix: &str,
    ) -> Result<SqliteCollection, Error> {
        // Create statement to create a table.
        let stmt = format!(
            "CREATE TABLE IF NOT EXISTS {} (prefix TEXT NOT NULL, value \
            BLOB NOT NULL, PRIMARY KEY (prefix))",
            identifier
        );

        {
            let conn = self.conn.lock().expect("open connection");

            conn.execute(stmt.as_str(), ()).map_err(|e| {
                Error::CreateStore(format!("fail SQLite create table: {}", e))
            })?;
        }

        Ok(SqliteCollection::new(self.conn.clone(), identifier, prefix))
    }

    fn create_collection(
        &self,
        identifier: &str,
        prefix: &str,
    ) -> Result<SqliteCollection, Error> {
        // Create statement to create a table.
        let stmt = format!(
            "CREATE TABLE IF NOT EXISTS {} (prefix TEXT NOT NULL, sn TEXT NOT NULL, value \
            BLOB NOT NULL, PRIMARY KEY (prefix, sn))",
            identifier
        );

        {
            let conn = self.conn.lock().expect("open connection");

            conn.execute(stmt.as_str(), ()).map_err(|e| {
                Error::CreateStore(format!("fail SQLite create table: {}", e))
            })?;
        }

        Ok(SqliteCollection::new(self.conn.clone(), identifier, prefix))
    }
}

/// SQLite collection that implements both Collection and State traits.
/// Stores key-value pairs in a SQLite table with prefix-based namespacing.
///
/// # Schema
///
/// **For Collections**: (prefix TEXT, sn TEXT, value BLOB, PRIMARY KEY (prefix, sn))
/// **For State**: (prefix TEXT, value BLOB, PRIMARY KEY (prefix))
///
/// where:
/// - `prefix` is the actor's namespace identifier
/// - `sn` is the sequence number (for events)
/// - `value` is the serialized data
///
pub struct SqliteCollection {
    /// Shared SQLite connection.
    conn: Arc<Mutex<Connection>>,
    /// Table name in the database.
    table: String,
    /// Prefix for filtering rows (actor namespace).
    prefix: String,
}

impl SqliteCollection {
    /// Creates a new SQLite collection.
    ///
    /// # Arguments
    ///
    /// * `conn` - Shared SQLite connection.
    /// * `table` - Name of the table in the database.
    /// * `prefix` - Prefix for namespacing this collection's data.
    ///
    /// # Returns
    ///
    /// Returns a new SqliteCollection instance.
    ///
    pub fn new(
        conn: Arc<Mutex<Connection>>,
        table: &str,
        prefix: &str,
    ) -> Self {
        Self {
            conn,
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

impl State for SqliteCollection {
    fn get(&self) -> Result<Vec<u8>, Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("sqlite open connection: {}", e))
        })?;
        let query =
            format!("SELECT value FROM {} WHERE prefix = ?1", &self.table);
        let row: Vec<u8> = conn
            .query_row(&query, params![self.prefix], |row| row.get(0))
            .map_err(|e| Error::EntryNotFound(e.to_string()))?;

        Ok(row)
    }

    fn put(&mut self, data: &[u8]) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("sqlite open connection: {}", e))
        })?;
        let stmt = format!(
            "INSERT OR REPLACE INTO {} (prefix, value) VALUES (?1, ?2)",
            &self.table
        );
        conn.execute(&stmt, params![self.prefix, data])
            .map_err(|e| Error::Store(format!("sqlite insert error: {}", e)))?;
        Ok(())
    }

    fn del(&mut self) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("SQLITE open connection: {}", e))
        })?;
        let stmt = format!("DELETE FROM {} WHERE prefix = ?1", &self.table);
        conn.execute(&stmt, params![self.prefix,])
            .map_err(|e| Error::EntryNotFound(e.to_string()))?;
        Ok(())
    }

    fn purge(&mut self) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("SQLITE open connection: {}", e))
        })?;
        let stmt = format!("DELETE FROM {} WHERE prefix = ?1", &self.table);
        conn.execute(&stmt, params![self.prefix])
            .map_err(|e| Error::Store(format!("SQLITE purge error: {}", e)))?;
        Ok(())
    }

    fn name(&self) -> &str {
        self.table.as_str()
    }
}

impl Collection for SqliteCollection {
    fn get(&self, key: &str) -> Result<Vec<u8>, Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("sqlite open connection: {}", e))
        })?;
        let query = format!(
            "SELECT value FROM {} WHERE prefix = ?1 AND sn = ?2",
            &self.table
        );
        let row: Vec<u8> = conn
            .query_row(&query, params![self.prefix, key], |row| row.get(0))
            .map_err(|e| Error::EntryNotFound(e.to_string()))?;

        Ok(row)
    }

    fn put(&mut self, key: &str, data: &[u8]) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("sqlite open connection: {}", e))
        })?;
        let stmt = format!(
            "INSERT OR REPLACE INTO {} (prefix, sn, value) VALUES (?1, ?2, ?3)",
            &self.table
        );
        conn.execute(&stmt, params![self.prefix, key, data])
            .map_err(|e| Error::Store(format!("sqlite insert error: {}", e)))?;
        Ok(())
    }

    fn del(&mut self, key: &str) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("SQLITE open connection: {}", e))
        })?;
        let stmt = format!(
            "DELETE FROM {} WHERE prefix = ?1 AND sn = ?2",
            &self.table
        );
        conn.execute(&stmt, params![self.prefix, key])
            .map_err(|e| Error::EntryNotFound(e.to_string()))?;
        Ok(())
    }

    fn purge(&mut self) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|e| {
            Error::Store(format!("SQLITE open connection: {}", e))
        })?;
        let stmt = format!("DELETE FROM {} WHERE prefix = ?1", &self.table);
        conn.execute(&stmt, params![self.prefix])
            .map_err(|e| Error::Store(format!("SQLITE purge error: {}", e)))?;
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
    let flags =
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE;
    let conn = Connection::open_with_flags(path, flags).map_err(|e| {
        Error::Store(format!("SQLite failed to open connection: {}", e))
    })?;

    conn.execute_batch(
        "
        PRAGMA journal_mode=WAL;
        PRAGMA synchronous=NORMAL;
        ",
    )
    .map_err(|e| {
        Error::Store(format!("SQLite failed to execute batch: {}", e))
    })?;

    Ok(conn)
}

#[cfg(test)]
mod tests {
    pub fn create_temp_dir() -> String {
        let path = temp_dir();

        if fs::metadata(&path).is_err() {
            fs::create_dir_all(&path).unwrap();
        }
        path
    }

    fn temp_dir() -> String {
        let dir =
            tempfile::tempdir().expect("Can not create temporal directory.");
        dir.path().to_str().unwrap().to_owned()
    }

    impl Default for SqliteManager {
        fn default() -> Self {
            let path = format!("{}/database.db", create_temp_dir());
            let conn = open(&path)
                .map_err(|e| {
                    Error::CreateStore(format!(
                        "fail SQLite open connection: {}",
                        e
                    ))
                })
                .expect("Cannot open the database ");

            Self {
                conn: Arc::new(Mutex::new(conn)),
            }
        }
    }

    use super::*;
    use store::{
        database::{Collection, DbManager},
        test_store_trait,
    };

    test_store_trait! {
        unit_test_sqlite_manager:SqliteManager:SqliteCollection
    }
}
