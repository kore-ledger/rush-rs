// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! Memory store implementation.
//!

use crate::{
    database::{Collection, DbManager},
    error::Error,
};

use std::collections::BTreeMap;

#[derive(Default, Clone)]
pub struct MemoryManager;

impl DbManager<MemoryStore> for MemoryManager {
    fn create_collection(
        &self,
        name: &str,
        prefix: &str,
    ) -> Result<MemoryStore, Error> {
        Ok(MemoryStore {
            name: name.to_owned(),
            prefix: prefix.to_owned(),
            data: BTreeMap::new(),
        })
    }
}

/// A store implementation that stores data in memory.
///
pub struct MemoryStore {
    name: String,
    prefix: String,
    data: BTreeMap<String, Vec<u8>>,
}

impl Collection for MemoryStore {
    fn name(&self) -> &str {
        &self.name
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, Error> {
        let key = format!("{}.{}", self.prefix, key);
        match self.data.get(&key) {
            Some(value) => Ok(value.clone()),
            None => Err(Error::EntryNotFound),
        }
    }

    fn put(&mut self, key: &str, data: &[u8]) -> Result<(), Error> {
        let key = format!("{}.{}", self.prefix, key);
        self.data.insert(key, data.to_vec());
        Ok(())
    }

    fn del(&mut self, key: &str) -> Result<(), Error> {
        let key = format!("{}.{}", self.prefix, key);
        match self.data.remove(&key) {
            Some(_) => Ok(()),
            None => Err(Error::EntryNotFound),
        }
    }

    fn iter<'a>(
        &'a self,
        reverse: bool,
    ) -> Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a> {
        if reverse {
            Box::new(
                self.data
                    .iter()
                    .rev()
                    .filter(|(key, _)| key.starts_with(&self.prefix))
                    .map(|(key, value)| {
                        let key = &key[self.prefix.len() + 1..];
                        (key.to_owned(), value.clone())
                    }),
            )
        } else {
            Box::new(
                self.data
                    .iter()
                    .filter(|(key, _)| key.starts_with(&self.prefix))
                    .map(|(key, value)| {
                        let key = &key[self.prefix.len() + 1..];
                        (key.to_owned(), value.clone())
                    }),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_store_trait;
    test_store_trait! {
        unit_test_memory_manager:crate::memory::MemoryManager:MemoryStore
    }
}
