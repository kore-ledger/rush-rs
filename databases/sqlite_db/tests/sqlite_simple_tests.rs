

//! Simple edge case tests for SQLite database to increase coverage

use sqlite_db::SqliteManager;
use store::database::{Collection, DbManager, State};
use tempfile::tempdir;

#[test]
fn test_sqlite_manager_edge_cases() {
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test");
    let manager = SqliteManager::new(&db_path.to_string_lossy()).unwrap();

    // Test collection operations
    let mut collection = manager.create_collection("test", "prefix").unwrap();

    // Test empty key/value
    Collection::put(&mut collection, "", b"").unwrap();
    let result = Collection::get(&collection, "").unwrap();
    assert_eq!(result, b"");

    // Test overwrite
    Collection::put(&mut collection, "key", b"value1").unwrap();
    Collection::put(&mut collection, "key", b"value2").unwrap();
    let result = Collection::get(&collection, "key").unwrap();
    assert_eq!(result, b"value2");

    // Test delete
    Collection::del(&mut collection, "key").unwrap();
    let result = Collection::get(&collection, "key");
    assert!(result.is_err());

    // Test delete non-existent (may or may not fail depending on implementation)
    let _result = Collection::del(&mut collection, "non-existent");
    // Some implementations might return Ok() for non-existent deletes
}

#[test]
fn test_sqlite_state_operations() {
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("state_test");
    let manager = SqliteManager::new(&db_path.to_string_lossy()).unwrap();

    let mut state = manager.create_state("state", "prefix").unwrap();

    // Test get empty state
    let result = State::get(&state);
    assert!(result.is_err());

    // Test put/get
    State::put(&mut state, b"state_data").unwrap();
    let result = State::get(&state).unwrap();
    assert_eq!(result, b"state_data");

    // Test delete
    State::del(&mut state).unwrap();
    let result = State::get(&state);
    assert!(result.is_err());
}

#[test]
fn test_sqlite_iteration() {
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("iteration");
    let manager = SqliteManager::new(&db_path.to_string_lossy()).unwrap();

    let mut collection = manager.create_collection("iter", "prefix").unwrap();

    // Test empty iteration
    let items: Vec<_> = collection.iter(false).collect();
    assert_eq!(items.len(), 0);

    // Add items
    Collection::put(&mut collection, "a", b"1").unwrap();
    Collection::put(&mut collection, "b", b"2").unwrap();
    Collection::put(&mut collection, "c", b"3").unwrap();

    // Test forward iteration
    let items: Vec<_> = collection.iter(false).collect();
    assert_eq!(items.len(), 3);

    // Test reverse iteration
    let items: Vec<_> = collection.iter(true).collect();
    assert_eq!(items.len(), 3);
}

#[test]
fn test_sqlite_flush_and_name() {
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("flush_test");
    let manager = SqliteManager::new(&db_path.to_string_lossy()).unwrap();

    let collection = manager.create_collection("test_collection", "prefix").unwrap();
    let state = manager.create_state("test_state", "prefix").unwrap();

    // Test names
    assert_eq!(Collection::name(&collection), "test_collection");
    assert_eq!(State::name(&state), "test_state");

    // Test flush (should be no-op)
    assert!(Collection::flush(&collection).is_ok());
    assert!(State::flush(&state).is_ok());
}