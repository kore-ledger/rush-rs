// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! RocksDB store implementation.
//!

use store::{
    database::{Collection, DbManager},
    Error,
};

use rocksdb::{DBIteratorWithThreadMode, IteratorMode, Options, DB};

use std::sync::{Arc, RwLock, RwLockReadGuard};

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
        let db = DB::open_default(path).expect("Can not create the database.");
        Self {
            opts: Options::default(),
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
    fn create_collection(&self, name: &str) -> Result<RocksDbStore, Error> {
        let mut db = self
            .db
            .write()
            .map_err(|e| Error::CreateStore(format!("{:?}", e)))?;
        db.create_cf(name, &self.opts)
            .map_err(|e| Error::CreateStore(format!("{:?}", e)))?;
        Ok(RocksDbStore {
            name: name.to_owned(),
            store: self.db.clone(),
        })
    }
}

/// RocksDb store.
pub struct RocksDbStore {
    name: String,
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
            Ok(db
                .put_cf(handle, key, &data)
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
            Ok(db
                .delete_cf(handle, key)
                .map_err(|e| Error::Get(format!("{:?}", e)))?)
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
        Box::new(RocksDbIterator::new(&self.store, &self.name, reverse))
    }
}

type GuardIter<'a> = (
    Arc<RwLockReadGuard<'a, DB>>,
    DBIteratorWithThreadMode<'a, DB>,
);

pub struct RocksDbIterator<'a> {
    store: &'a Arc<RwLock<DB>>,
    name: String,
    mode: IteratorMode<'a>,
    current: Option<GuardIter<'a>>,
}

impl<'a> RocksDbIterator<'a> {
    pub fn new(store: &'a Arc<RwLock<DB>>, name: &str, reverse: bool) -> Self {
        let mode = if reverse {
            IteratorMode::End
        } else {
            IteratorMode::Start
        };
        Self {
            store,
            name: name.to_owned(),
            mode,
            current: None,
        }
    }
}

impl<'a> Iterator for RocksDbIterator<'a> {
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
            self.current = Some((Arc::new(guard), iter));
            &mut self.current.as_mut().unwrap().1
        };
        if let Some(Ok((key, value))) = iter.next() {
            let key = String::from_utf8(key.to_vec())
                .expect("Can not convert key to string.");
            Some((key, value.to_vec()))
        } else {
            self.current = None;
            None
        }
    }
}

unsafe fn change_lifetime_const<'a, 'b, T>(x: &'a T) -> &'b T {
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
