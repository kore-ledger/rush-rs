// Copyright 2024 Antonio Estévez
// SPDX-License-Identifier: Apache-2.0

//! Store manager

use crate::error::Error;

use tracing::debug;

/// A trait representing a database manager to create collections
pub trait DbManager<C>: Sync + Send + Clone
where
    C: Collection + 'static,
{
    /// Create collection.
    ///
    /// # Arguments
    ///
    /// - name: The name of the collection.
    /// - prefix: The prefix to filter the values by.
    ///
    /// # Returns
    ///
    /// The collection.
    ///
    fn create_collection(&self, name: &str, prefix: &str) -> Result<C, Error>;

    /// Stop manager.
    ///

    fn stop(self) -> Result<(), Error> {
        Ok(())
    }
}

/// A trait representing a collection of key-value pairs in a database.
///
pub trait Collection: Sync + Send + 'static {
    /// Retrieve the name of the collection.
    ///
    /// # Returns
    ///     
    /// The name of the collection.
    ///
    fn name(&self) -> &str;

    /// Retrieves the value associated with the given key.
    ///
    /// # Arguments
    ///
    /// - key: The key to retrieve the value for.
    ///
    /// # Returns
    ///
    /// The value associated with the given key.
    ///
    /// # Errors
    ///
    /// - If the operation failed.
    ///
    fn get(&self, key: &str) -> Result<Vec<u8>, Error>;

    /// Associates the given value with the given key.
    ///
    /// # Arguments
    ///
    /// - key: The key to associate the value with.
    /// - data: The value to associate with the key.
    ///
    /// # Returns
    ///
    /// An error if the operation failed.
    ///
    /// # Errors
    ///
    /// - If the operation failed.
    ///
    fn put(&mut self, key: &str, data: &[u8]) -> Result<(), Error>;

    /// Removes the value associated with the given key.
    ///
    /// # Arguments
    ///
    /// - key: The key to remove the value for.
    ///
    /// # Returns
    ///
    /// An error if the operation failed.
    ///
    fn del(&mut self, key: &str) -> Result<(), Error>;

    /// Returns the last value in the collection.
    ///
    /// # Returns
    ///
    /// The last key / value in the collection.
    ///
    /// # Errors
    ///
    /// - If the operation failed.
    ///
    fn last(&self) -> Option<(String, Vec<u8>)> {
        let mut iter = self.iter(true);
        let value = iter.next();
        debug!("Last value: {:?}", value);
        value
    }

    /// Returns an iterator over the key-value pairs in the collection.
    ///
    /// # Arguments
    ///
    /// - reverse: Whether to iterate in reverse order.
    ///
    /// # Returns
    ///
    /// An iterator over the key-value pairs in the collection.
    ///
    fn iter<'a>(
        &'a self,
        reverse: bool,
    ) -> Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a>;

    /// Flush collection.
    ///
    fn flush(&self) -> Result<(), Error> {
        Ok(())
    }

    /// Returns a vector of values in the collection that are in the given range.
    ///
    /// # Arguments
    ///
    /// - from: The key to start the range from. If None, the range starts from the beginning.
    /// - quantity: The number of values to return. If negative, the range is reversed.
    /// - prefix: The prefix to filter the values by.
    ///
    /// # Returns
    ///
    /// A vector of values in the collection that are in the given range.
    ///
    /// # Errors
    ///
    /// - If the range is invalid.
    /// - If the range is out of bounds.
    ///
    fn get_by_range(
        &self,
        from: Option<String>,
        quantity: isize,
    ) -> Result<Vec<Vec<u8>>, Error> {
        fn convert<'a>(
            iter: impl Iterator<Item = (String, Vec<u8>)> + 'a,
        ) -> Box<dyn Iterator<Item = (String, Vec<u8>)> + 'a> {
            Box::new(iter)
        }
        let (mut iter, quantity) = match from {
            Some(key) => {
                // Find the key
                let iter = if quantity >= 0 {
                    self.iter(false)
                } else {
                    self.iter(true)
                };
                let mut iter = iter.peekable();
                loop {
                    let Some((current_key, _)) = iter.peek() else {
                        return Err(Error::EntryNotFound);
                    };
                    if current_key == &key {
                        break;
                    }
                    iter.next();
                }
                iter.next(); // Exclusive From
                (convert(iter), quantity.abs())
            }
            None => {
                if quantity >= 0 {
                    (self.iter(false), quantity)
                } else {
                    (self.iter(true), quantity.abs())
                }
            }
        };
        let mut result = Vec::new();
        let mut counter = 0;
        while counter < quantity {
            let Some((_, event)) = iter.next() else {
                break;
            };
            result.push(event);
            counter += 1;
        }
        Ok(result)
    }
}

#[macro_export]
macro_rules! test_store_trait {
    ($name:ident: $type:ty: $type2:ty) => {
        #[cfg(test)]
        mod $name {
            use super::*;
            use $crate::error::Error;

            #[test]
            fn test_create() {
                let manager = <$type>::default();
                let store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                assert_eq!(store.name(), "test");
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_put_get() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                store.put("key", b"value").unwrap();
                assert_eq!(store.get("key").unwrap(), b"value");
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_del() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                store.put("key", b"value").unwrap();
                store.del("key").unwrap();
                assert_eq!(store.get("key"), Err(Error::EntryNotFound));
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_iter() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                store.put("key1", b"value1").unwrap();
                store.put("key2", b"value2").unwrap();
                store.put("key3", b"value3").unwrap();
                let mut iter = store.iter(false);
                assert_eq!(
                    iter.next(),
                    Some(("key1".to_string(), b"value1".to_vec()))
                );
                assert_eq!(
                    iter.next(),
                    Some(("key2".to_string(), b"value2".to_vec()))
                );
                assert_eq!(
                    iter.next(),
                    Some(("key3".to_string(), b"value3".to_vec()))
                );
                assert_eq!(iter.next(), None);
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_iter_reverse() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                store.put("test.key1", b"value1").unwrap();
                store.put("test.key2", b"value2").unwrap();
                store.put("test.key3", b"value3").unwrap();
                let mut iter = store.iter(true);
                assert_eq!(
                    iter.next(),
                    Some(("test.key3".to_string(), b"value3".to_vec()))
                );
                assert_eq!(
                    iter.next(),
                    Some(("test.key2".to_string(), b"value2".to_vec()))
                );
                assert_eq!(
                    iter.next(),
                    Some(("test.key1".to_string(), b"value1".to_vec()))
                );
                assert_eq!(iter.next(), None);
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_last() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                store.put("key1", b"value1").unwrap();
                store.put("key2", b"value2").unwrap();
                store.put("key3", b"value3").unwrap();
                let last = store.last();
                assert_eq!(
                    last,
                    Some(("key3".to_string(), b"value3".to_vec()))
                );
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_get_by_range() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                store.put("key1", b"value1").unwrap();
                store.put("key2", b"value2").unwrap();
                store.put("key3", b"value3").unwrap();
                let result = store.get_by_range(None, 2).unwrap();
                assert_eq!(
                    result,
                    vec![b"value1".to_vec(), b"value2".to_vec()]
                );
                let result =
                    store.get_by_range(Some("key3".to_string()), -2).unwrap();
                assert_eq!(
                    result,
                    vec![b"value2".to_vec(), b"value1".to_vec()]
                );
                assert!(manager.stop().is_ok())
            }
        }
    };
}
