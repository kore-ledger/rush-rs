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
    fn create_collection(&self, name: &str) -> Result<MemoryStore, Error> {
        Ok(MemoryStore {
            name: name.to_owned(),
            data: BTreeMap::new(),
        })
    }
}

/// A store implementation that stores data in memory.
///
pub struct MemoryStore {
    name: String,
    data: BTreeMap<String, Vec<u8>>,
}

impl Collection for MemoryStore {
    fn name(&self) -> &str {
        &self.name
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, Error> {
        match self.data.get(key) {
            Some(value) => Ok(value.clone()),
            None => Err(Error::EntryNotFound),
        }
    }

    fn put(&mut self, key: &str, data: &[u8]) -> Result<(), Error> {
        self.data.insert(key.to_string(), data.to_vec());
        Ok(())
    }

    fn del(&mut self, key: &str) -> Result<(), Error> {
        match self.data.remove(key) {
            Some(_) => Ok(()),
            None => Err(Error::EntryNotFound),
        }
    }

    fn iter<'a>(
        &'a self,
        reverse: bool,
    ) -> Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a> {
        //let iter = self.data.iter().filter(move |(key, _)| key.starts_with(prefix));
        if reverse {
            Box::new(
                self.data
                    .iter()
                    .rev()
                    .map(|(key, value)| (key.clone(), value.clone())),
            )
        } else {
            Box::new(
                self.data
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_store_trait;
    test_store_trait!(unit_test_memory_manager:crate::memory::MemoryManager:MemoryStore);
}
