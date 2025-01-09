// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! RocksDB store implementation.
//!

use store::{
    database::{Collection, DbManager},
    Error,
};

use rocksdb::{
    ColumnFamilyDescriptor, DBIteratorWithThreadMode, IteratorMode, Options, DB,
};

use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};

/// RocksDb manager.
#[derive(Clone)]
pub struct RocksDbManager {
    opts: Options,
    db: Arc<RwLock<DB>>,
}

impl RocksDbManager {
    /// Create a new RocksDb manager.
    ///
    /// # Returns
    ///
    /// The RocksDb manager.
    ///
    pub fn new(path: &str) -> Self {
        let mut options = Options::default();
        options.create_if_missing(true);

        let cfs = match DB::list_cf(&options, path) {
            Ok(cf_names) => cf_names,
            Err(_) => vec!["default".to_string()], // Si la base de datos no existe, usamos solo `default`
        };

        // Crear descriptores para cada column family
        let cf_descriptors: Vec<_> = cfs
            .iter()
            .map(|cf| ColumnFamilyDescriptor::new(cf, Options::default()))
            .collect();

        // Abrir la base de datos con las column families existentes
        let db = DB::open_cf_descriptors(&options, path, cf_descriptors)
            .expect("Cannot open the database with existing column families.");

        Self {
            opts: options,
            db: Arc::new(RwLock::new(db)),
        }
    }
}

impl Default for RocksDbManager {
    fn default() -> Self {
        let dir =
            tempfile::tempdir().expect("Can not create temporal directory.");
        let db =
            DB::open_default(dir.path()).expect("Can not create the database.");
        Self {
            opts: Options::default(),
            db: Arc::new(RwLock::new(db)),
        }
    }
}

impl DbManager<RocksDbStore> for RocksDbManager {
    fn create_collection(
        &self,
        name: &str,
        prefix: &str,
    ) -> Result<RocksDbStore, Error> {
        let mut db = self
            .db
            .write()
            .map_err(|e| Error::CreateStore(format!("{:?}", e)))?;
        if db.cf_handle(name).is_none() {
            db.create_cf(name, &self.opts)
                .map_err(|e| Error::CreateStore(format!("{:?}", e)))?;
        }
        Ok(RocksDbStore {
            name: name.to_owned(),
            prefix: prefix.to_owned(),
            store: self.db.clone(),
        })
    }

    fn stop(self) -> Result<(), Error> {
        let db = self
            .db
            .read()
            .map_err(|e| Error::Store(format!("{:?}", e)))?;
        db.flush().map_err(|e| Error::Store(format!("{:?}", e)))
    }
}

/// RocksDb store.
pub struct RocksDbStore {
    name: String,
    prefix: String,
    store: Arc<RwLock<DB>>,
    //column: ColumnFamily,
}

impl Collection for RocksDbStore {
    fn name(&self) -> &str {
        &self.name
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, Error> {
        let db = self
            .store
            .read()
            .map_err(|e| Error::Get(format!("{:?}", e)))?;
        if let Some(handle) = db.cf_handle(&self.name) {
            let key = format!("{}.{}", self.prefix, key);
            let result = db
                .get_cf(handle, key)
                .map_err(|e| Error::Get(format!("{:?}", e)))?;
            match result {
                Some(value) => Ok(value),
                _ => Err(Error::EntryNotFound),
            }
        } else {
            Err(Error::Store(
                "RocksDB column for the store does not exist.".to_owned(),
            ))
        }
    }

    fn put(&mut self, key: &str, data: &[u8]) -> Result<(), Error> {
        let db = self
            .store
            .read()
            .map_err(|e| Error::Get(format!("{:?}", e)))?;
        if let Some(handle) = db.cf_handle(&self.name) {
            let key = format!("{}.{}", self.prefix, key);
            Ok(db
                .put_cf(handle, key, data)
                .map_err(|e| Error::Get(format!("{:?}", e)))?)
        } else {
            Err(Error::Store(
                "RocksDB column for the store does not exist.".to_owned(),
            ))
        }
    }

    fn del(&mut self, key: &str) -> Result<(), Error> {
        let db = self
            .store
            .read()
            .map_err(|e| Error::Get(format!("{:?}", e)))?;
        if let Some(handle) = db.cf_handle(&self.name) {
            let key = format!("{}.{}", self.prefix, key);
            Ok(db
                .delete_cf(handle, key)
                .map_err(|e| Error::Get(format!("{:?}", e)))?)
        } else {
            Err(Error::Store(
                "RocksDB column for the store does not exist.".to_owned(),
            ))
        }
    }

    fn purge(&mut self) -> Result<(), Error> {
        let db = self
            .store
            .read()
            .map_err(|e| Error::Get(format!("{:?}", e)))?;
        if let Some(handle) = db.cf_handle(&self.name) {
            let iter = db.iterator_cf(handle, IteratorMode::Start);
            for (key, _) in iter.flatten() {
                let key = String::from_utf8(key.to_vec()).map_err(|_| {
                    Error::Store("Can not convert key to string.".to_owned())
                })?;
                if key.starts_with(&self.prefix) {
                    db.delete_cf(handle, key)
                        .map_err(|e| Error::Get(format!("{:?}", e)))?;
                }
            }
            Ok(())
        } else {
            Err(Error::Store(
                "RocksDB column for the store does not exist.".to_owned(),
            ))
        }
    }

    fn iter<'a>(
        &'a self,
        reverse: bool,
    ) -> Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a> {
        Box::new(RocksDbIterator::new(
            &self.store,
            &self.name,
            &self.prefix,
            reverse,
        ))
    }

    fn flush(&self) -> Result<(), Error> {
        let db = self
            .store
            .read()
            .map_err(|e| Error::Get(format!("{:?}", e)))?;
        if let Some(handle) = db.cf_handle(&self.name) {
            Ok(db
                .flush_cf(handle)
                .map_err(|e| Error::Get(format!("{:?}", e)))?)
        } else {
            Err(Error::Store(
                "RocksDB column for the store does not exist.".to_owned(),
            ))
        }
    }
}

type GuardIter<'a> = (
    Mutex<RwLockReadGuard<'a, DB>>,
    DBIteratorWithThreadMode<'a, DB>,
);

pub struct RocksDbIterator<'a> {
    store: &'a Arc<RwLock<DB>>,
    name: String,
    prefix: String,
    mode: IteratorMode<'a>,
    current: Option<GuardIter<'a>>,
}

impl<'a> RocksDbIterator<'a> {
    pub fn new(
        store: &'a Arc<RwLock<DB>>,
        name: &str,
        prefix: &str,
        reverse: bool,
    ) -> Self {
        let mode = if reverse {
            IteratorMode::End
        } else {
            IteratorMode::Start
        };
        Self {
            store,
            name: name.to_owned(),
            prefix: prefix.to_owned(),
            mode,
            current: None,
        }
    }
}

impl Iterator for RocksDbIterator<'_> {
    type Item = (String, Vec<u8>);

    fn next(&mut self) -> Option<Self::Item> {
        let iter = if let Some((_, iter)) = &mut self.current {
            iter
        } else {
            let guard = self
                .store
                .read()
                .expect("Can not get read lock for the store.");
            let sref = unsafe { change_lifetime_const(&*guard) };
            let handle = sref
                .cf_handle(&self.name)
                .expect("RocksDB column for the store does not exist.");
            let iter = sref.iterator_cf(handle, self.mode);
            self.current = Some((Mutex::new(guard), iter));
            &mut self.current.as_mut().unwrap().1
        };
        while let Some(Ok((key, value))) = iter.next() {
            let key = String::from_utf8(key.to_vec())
                .expect("Can not convert key to string.");
            if key.starts_with(&self.prefix) {
                let key = &key[self.prefix.len() + 1..];
                return Some((key.to_owned(), value.to_vec()));
            }
        }
        self.current = None;
        None
    }
}

unsafe fn change_lifetime_const<'b, T>(x: &T) -> &'b T {
    &*(x as *const T)
}

#[cfg(test)]
mod tests {
    use super::*;
    use store::test_store_trait;
    test_store_trait! {
        unit_test_rocksdb_manager:crate::rocksdb::RocksDbManager:RocksDbStore
    }
}
