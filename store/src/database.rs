

//! Store manager

use crate::error::Error;

use tracing::debug;

/// A trait representing a database manager that creates collections and state storage.
/// Implementations of this trait provide the factory methods for creating
/// persistent storage backends used by actors for event sourcing and state snapshots.
///
/// # Type Parameters
///
/// * `C` - The collection type that stores key-value pairs (events).
/// * `S` - The state type that stores single values (snapshots).
///
pub trait DbManager<C, S>: Sync + Send + Clone
where
    C: Collection + 'static,
    S: State + 'static,
{
    /// Creates a new collection for storing key-value pairs (typically events).
    /// Collections are used for event sourcing where multiple events
    /// are stored with unique keys (usually sequence numbers).
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the collection (e.g., table name, column family).
    /// * `prefix` - A prefix for filtering/namespacing values within the collection.
    ///
    /// # Returns
    ///
    /// Returns a collection instance if successful.
    ///
    /// # Errors
    ///
    /// Returns an error if the collection could not be created.
    ///
    fn create_collection(&self, name: &str, prefix: &str) -> Result<C, Error>;

    /// Creates a new state storage for storing single values (typically snapshots).
    /// State storage is used for persisting actor state snapshots,
    /// where only the latest value matters.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the state storage (e.g., table name, column family).
    /// * `prefix` - A prefix for filtering/namespacing the state value.
    ///
    /// # Returns
    ///
    /// Returns a state storage instance if successful.
    ///
    /// # Errors
    ///
    /// Returns an error if the state storage could not be created.
    ///
    fn create_state(&self, name: &str, prefix: &str) -> Result<S, Error>;

    /// Stops the database manager and performs cleanup.
    /// Default implementation does nothing. Override this to implement
    /// connection closing, flushing, or other cleanup operations.
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if cleanup was successful.
    ///
    /// # Errors
    ///
    /// Returns an error if cleanup failed.
    ///
    fn stop(self) -> Result<(), Error> {
        Ok(())
    }
}

/// Trait for storing a single state value (typically actor snapshots).
/// State storage maintains only the most recent value, unlike Collections
/// which store multiple key-value pairs. This is used for persisting
/// actor state snapshots in event sourcing patterns.
///
pub trait State: Sync + Send + 'static {
    /// Retrieves the name of this state storage.
    ///
    /// # Returns
    ///
    /// The name identifier of this state storage.
    ///
    fn name(&self) -> &str;

    /// Retrieves the current state value.
    ///
    /// # Returns
    ///
    /// The serialized state data as bytes.
    ///
    /// # Errors
    ///
    /// Returns Error::EntryNotFound if no state has been stored yet.
    ///
    fn get(&self) -> Result<Vec<u8>, Error>;

    /// Stores or updates the state value.
    ///
    /// # Arguments
    ///
    /// * `data` - The serialized state data to store.
    ///
    /// # Returns
    ///
    /// Ok(()) if the state was successfully stored.
    ///
    /// # Errors
    ///
    /// Returns an error if the storage operation failed.
    ///
    fn put(&mut self, data: &[u8]) -> Result<(), Error>;

    /// Deletes the current state value.
    ///
    /// # Returns
    ///
    /// Ok(()) if the state was successfully deleted.
    ///
    /// # Errors
    ///
    /// Returns an error if the delete operation failed.
    ///
    fn del(&mut self) -> Result<(), Error>;

    /// Removes all data from the state storage.
    /// This is equivalent to del() but provides semantic clarity
    /// for complete cleanup operations.
    ///
    /// # Returns
    ///
    /// Ok(()) if the purge was successful.
    ///
    /// # Errors
    ///
    /// Returns an error if the purge operation failed.
    ///
    fn purge(&mut self) -> Result<(), Error>;

    /// Flushes any pending writes to persistent storage.
    /// Default implementation does nothing. Override this for databases
    /// that buffer writes.
    ///
    /// # Returns
    ///
    /// Ok(()) if the flush was successful.
    ///
    /// # Errors
    ///
    /// Returns an error if the flush operation failed.
    ///
    fn flush(&self) -> Result<(), Error> {
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

    /// Removes all values from the collection.
    ///
    /// # Returns
    ///
    /// An error if the operation failed.
    ///
    fn purge(&mut self) -> Result<(), Error>;

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
                        return Err(Error::EntryNotFound("None".to_owned()));
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
            fn test_create_collection() {
                let manager = <$type>::default();
                let store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                assert_eq!(Collection::name(&store), "test");
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_create_state() {
                let manager = <$type>::default();
                let store: $type2 =
                    manager.create_state("test", "test").unwrap();
                assert_eq!(State::name(&store), "test");
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_put_get_collection() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                Collection::put(&mut store, "key", b"value").unwrap();
                assert_eq!(Collection::get(&store, "key").unwrap(), b"value");
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_put_get_state() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_state("test", "test").unwrap();
                State::put(&mut store, b"value").unwrap();
                assert_eq!(State::get(&store).unwrap(), b"value");
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_del_collection() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                Collection::put(&mut store, "key", b"value").unwrap();
                Collection::del(&mut store, "key").unwrap();
                assert_eq!(
                    Collection::get(&store, "key"),
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_owned()
                    ))
                );
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_del_state() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_state("test", "test").unwrap();
                State::put(&mut store, b"value").unwrap();
                State::del(&mut store).unwrap();
                assert_eq!(
                    State::get(&store),
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_owned()
                    ))
                );
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_iter() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                Collection::put(&mut store, "key1", b"value1").unwrap();
                Collection::put(&mut store, "key2", b"value2").unwrap();
                Collection::put(&mut store, "key3", b"value3").unwrap();
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
                Collection::put(&mut store, "key1", b"value1").unwrap();
                Collection::put(&mut store, "key2", b"value2").unwrap();
                Collection::put(&mut store, "key3", b"value3").unwrap();
                let mut iter = store.iter(true);
                assert_eq!(
                    iter.next(),
                    Some(("key3".to_string(), b"value3".to_vec()))
                );
                assert_eq!(
                    iter.next(),
                    Some(("key2".to_string(), b"value2".to_vec()))
                );
                assert_eq!(
                    iter.next(),
                    Some(("key1".to_string(), b"value1".to_vec()))
                );
                assert_eq!(iter.next(), None);
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_last() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                Collection::put(&mut store, "key1", b"value1").unwrap();
                Collection::put(&mut store, "key2", b"value2").unwrap();
                Collection::put(&mut store, "key3", b"value3").unwrap();
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
                Collection::put(&mut store, "key1", b"value1").unwrap();
                Collection::put(&mut store, "key2", b"value2").unwrap();
                Collection::put(&mut store, "key3", b"value3").unwrap();
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

            #[test]
            fn test_purge_collection() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_collection("test", "test").unwrap();
                Collection::put(&mut store, "key1", b"value1").unwrap();
                Collection::put(&mut store, "key2", b"value2").unwrap();
                Collection::put(&mut store, "key3", b"value3").unwrap();
                assert_eq!(
                    Collection::get(&store, "key1"),
                    Ok(b"value1".to_vec())
                );
                assert_eq!(
                    Collection::get(&store, "key2"),
                    Ok(b"value2".to_vec())
                );
                assert_eq!(
                    Collection::get(&store, "key3"),
                    Ok(b"value3".to_vec())
                );
                Collection::purge(&mut store).unwrap();
                assert_eq!(
                    Collection::get(&store, "key1"),
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_owned()
                    ))
                );
                assert_eq!(
                    Collection::get(&store, "key2"),
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_owned()
                    ))
                );
                assert_eq!(
                    Collection::get(&store, "key3"),
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_owned()
                    ))
                );
                assert!(manager.stop().is_ok())
            }

            #[test]
            fn test_purge_state() {
                let manager = <$type>::default();
                let mut store: $type2 =
                    manager.create_state("test", "test").unwrap();
                State::put(&mut store, b"value1").unwrap();
                assert_eq!(State::get(&store), Ok(b"value1".to_vec()));
                State::purge(&mut store).unwrap();
                assert_eq!(
                    State::get(&store),
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_owned()
                    ))
                );

                State::put(&mut store, b"value2").unwrap();
                assert_eq!(State::get(&store), Ok(b"value2".to_vec()));
                State::purge(&mut store).unwrap();
                assert_eq!(
                    State::get(&store),
                    Err(Error::EntryNotFound(
                        "Query returned no rows".to_owned()
                    ))
                );
                assert!(manager.stop().is_ok())
            }
        }
    };
}
