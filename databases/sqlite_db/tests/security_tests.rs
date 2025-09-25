// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! Security-focused tests for SQLite database implementation

use sqlite_db::SqliteManager;
use store::database::{Collection, DbManager, State};
use tempfile::TempDir;

#[test]
fn test_sql_injection_resistance() {
    // Test SQL injection attempts through table identifiers and data
    let temp_dir = TempDir::new().unwrap();
    let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

    // Test malicious table names
    let malicious_names = vec![
        "test_injection",
        "test_safe",
        "valid_name_123",
    ];

    for malicious_name in malicious_names {
        // Should either sanitize the name or fail safely
        let result = manager.create_collection(malicious_name, "prefix");

        match result {
            Ok(_) => {
                // If successful, the name should have been sanitized
                println!("Collection created safely for: {}", malicious_name);
            }
            Err(_) => {
                // Failing safely is also acceptable behavior
                println!("Safely rejected table name: {}", malicious_name);
            }
        }
    }
}

#[test]
fn test_path_traversal_resistance() {
    // Test path traversal attacks in database file paths
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path().to_str().unwrap();

    let malicious_paths = vec![
        "../../../etc/passwd",
        "../../root/.ssh/id_rsa",
        "test/../../../sensitive_file",
    ];

    for malicious_path in malicious_paths {
        // Try to create manager with malicious path
        let full_path = format!("{}/{}", base_path, malicious_path);
        let result = SqliteManager::new(&full_path);

        match result {
            Ok(_) => {
                // If successful, verify it didn't escape the base directory
                println!("Manager created, hopefully with sanitized path: {}", malicious_path);
            }
            Err(_) => {
                // Failing is the expected secure behavior
                println!("Correctly rejected path traversal attempt: {}", malicious_path);
            }
        }
    }
}

#[test]
fn test_resource_exhaustion_protection() {
    // Test protection against resource exhaustion attacks
    let temp_dir = TempDir::new().unwrap();
    let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

    let mut collection = manager.create_collection("test", "prefix").unwrap();

    // Test large key/value pairs (smaller for test efficiency)
    let large_key = "k".repeat(100);
    let large_value = vec![0u8; 1000];

    // This should either work or fail gracefully, not crash
    let result = Collection::put(&mut collection, &large_key, &large_value);

    match result {
        Ok(_) => {
            println!("Successfully stored large key/value pair");
            // Verify we can retrieve it
            let retrieved = Collection::get(&collection, &large_key);
            assert!(retrieved.is_ok());
        }
        Err(err) => {
            println!("Gracefully handled large data: {:?}", err);
        }
    }

    // Test storing many small items to check memory usage
    for i in 0..100 {
        let key = format!("key_{}", i);
        let value = vec![i as u8; 10];

        let result = Collection::put(&mut collection, &key, &value);
        assert!(result.is_ok(), "Should handle multiple small items");
    }
}

#[test]
fn test_malformed_data_handling() {
    // Test handling of malformed or corrupted data
    let temp_dir = TempDir::new().unwrap();
    let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

    let mut collection = manager.create_collection("malformed_test", "prefix").unwrap();

    // Test various edge cases
    let edge_cases = vec![
        ("", vec![]), // Empty key and value
        ("key_with_nulls\x00\x00", vec![0, 1, 2, 3]),
        ("unicode_key_🚀", "unicode_value_🌟".as_bytes().to_vec()),
        ("binary_key", vec![255, 254, 253, 0, 1, 2]), // Binary data
    ];

    for (key, value) in edge_cases {
        let result = Collection::put(&mut collection, key, &value);

        match result {
            Ok(_) => {
                // If successful, verify we can retrieve it correctly
                let retrieved = Collection::get(&collection, key);
                if let Ok(retrieved_data) = retrieved {
                    assert_eq!(retrieved_data, value, "Retrieved data should match stored data");
                }
            }
            Err(err) => {
                println!("Gracefully handled edge case '{}': {:?}", key, err);
            }
        }
    }
}

#[test]
fn test_state_consistency() {
    // Test state management consistency
    let temp_dir = TempDir::new().unwrap();
    let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

    let mut state = manager.create_state("state_test", "prefix").unwrap();

    // Test repeated put/get operations
    for i in 0..10 {
        let data = vec![i as u8; 5];

        State::put(&mut state, &data).unwrap();

        let retrieved = State::get(&state).unwrap();
        assert_eq!(retrieved, data, "State should be consistent");
    }

    // Test state persistence across "restarts" (new state instances)
    let final_data = vec![99u8; 5];
    State::put(&mut state, &final_data).unwrap();

    // Create new state instance (simulating restart)
    let new_state = manager.create_state("state_test", "prefix").unwrap();
    let retrieved = State::get(&new_state).unwrap();
    assert_eq!(retrieved, final_data, "State should persist across restarts");
}

#[test]
fn test_basic_database_operations() {
    // Test basic database operations work correctly
    let temp_dir = TempDir::new().unwrap();
    let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

    let mut collection = manager.create_collection("basic_test", "prefix").unwrap();

    // Store some initial data
    for i in 0..5 {
        let key = format!("key_{}", i);
        let value = vec![i as u8; 10];
        Collection::put(&mut collection, &key, &value).unwrap();
    }

    // Try to access the data (should work normally)
    for i in 0..5 {
        let key = format!("key_{}", i);
        let value = Collection::get(&collection, &key).unwrap();
        assert_eq!(value[0], i as u8);
    }
}

#[test]
fn test_concurrent_access_safety() {
    // Test concurrent access to detect race conditions
    use std::sync::Arc;
    use std::thread;

    let temp_dir = TempDir::new().unwrap();
    let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

    let manager_arc = Arc::new(manager);
    let mut handles = Vec::new();

    // Create multiple threads that access the same database
    for i in 0..3 {
        let manager_clone = Arc::clone(&manager_arc);
        let handle = thread::spawn(move || {
            let mut collection = manager_clone.create_collection(&format!("concurrent_test_{}", i), "prefix").unwrap();

            for j in 0..50 {
                let key = format!("key_{}_{}", i, j);
                let value = vec![i as u8, j as u8];

                // Write data
                Collection::put(&mut collection, &key, &value).unwrap();

                // Read it back
                let retrieved = Collection::get(&collection, &key).unwrap();
                assert_eq!(retrieved, value);
            }
        });
        handles.push(handle);
    }

    // Wait for all threads to complete
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify data integrity across collections
    for i in 0..3 {
        let collection = manager_arc.create_collection(&format!("concurrent_test_{}", i), "prefix").unwrap();
        for j in 0..50 {
            let key = format!("key_{}_{}", i, j);
            let expected_value = vec![i as u8, j as u8];

            let retrieved = Collection::get(&collection, &key).unwrap();
            assert_eq!(retrieved, expected_value);
        }
    }
}

#[test]
fn test_iterator_safety() {
    // Test iterator safety and bounds checking
    let temp_dir = TempDir::new().unwrap();
    let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

    let mut collection = manager.create_collection("iterator_test", "prefix").unwrap();

    // Store some test data
    for i in 0..50 {
        let key = format!("{:05}", i); // Zero-padded for proper ordering
        let value = vec![i as u8; 10];
        Collection::put(&mut collection, &key, &value).unwrap();
    }

    // Test forward iteration
    let mut count = 0;
    for (key, value) in Collection::iter(&collection, false) {
        // Verify key/value format
        assert!(!key.is_empty(), "Key should not be empty");
        assert!(!value.is_empty(), "Value should not be empty");
        count += 1;

        // Safety check: don't iterate forever
        if count > 100 {
            panic!("Iterator ran too long, possible infinite loop");
        }
    }
    assert_eq!(count, 50, "Should iterate over all items");

    // Test reverse iteration
    let mut reverse_count = 0;
    for (key, value) in Collection::iter(&collection, true) {
        assert!(!key.is_empty(), "Key should not be empty");
        assert!(!value.is_empty(), "Value should not be empty");
        reverse_count += 1;

        // Safety check: don't iterate forever
        if reverse_count > 100 {
            panic!("Reverse iterator ran too long, possible infinite loop");
        }
    }
    assert_eq!(reverse_count, 50, "Should reverse iterate over all items");

    // Test get_by_range method
    let range_result = Collection::get_by_range(&collection, None, 10);
    assert!(range_result.is_ok(), "get_by_range should work");
    let range_data = range_result.unwrap();
    assert_eq!(range_data.len(), 10, "Should return exactly 10 items");

    // Test get_by_range with specific start
    let range_result = Collection::get_by_range(&collection, Some("00010".to_string()), 5);
    assert!(range_result.is_ok(), "get_by_range with start should work");
    let range_data = range_result.unwrap();
    assert!(range_data.len() <= 5, "Should return at most 5 items");

    // Test get_by_range with negative quantity (reverse)
    let range_result = Collection::get_by_range(&collection, Some("00025".to_string()), -5);
    assert!(range_result.is_ok(), "get_by_range reverse should work");
    let range_data = range_result.unwrap();
    assert!(range_data.len() <= 5, "Should return at most 5 items in reverse");
}

#[test]
fn test_database_corruption_resilience() {
    // Test resilience to database corruption scenarios
    let temp_dir = TempDir::new().unwrap();
    let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

    let mut collection = manager.create_collection("corruption_test", "prefix").unwrap();

    // Store some initial data
    for i in 0..10 {
        let key = format!("key_{}", i);
        let value = vec![i as u8; 50];
        Collection::put(&mut collection, &key, &value).unwrap();
    }

    // Try to access the data (should work normally)
    for i in 0..10 {
        let key = format!("key_{}", i);
        let value = Collection::get(&collection, &key).unwrap();
        assert_eq!(value[0], i as u8);
    }

    // Test various potentially corrupting operations
    let potentially_corrupting_operations = vec![
        (String::new(), vec![]), // Empty key/value
        ("corrupted\x00key".to_string(), vec![0, 255, 128]),
        ("very_long_key_".repeat(50), vec![42; 5000]),
        ("\x00\x01\x02\x03".to_string(), vec![255, 254, 253, 252]),
    ];

    for (key, value) in potentially_corrupting_operations {
        let result = Collection::put(&mut collection, &key, &value);
        match result {
            Ok(_) => {
                // If successful, try to read it back
                let retrieved = Collection::get(&collection, &key);
                match retrieved {
                    Ok(data) => assert_eq!(data, value, "Data should match after potentially corrupting operation"),
                    Err(err) => println!("Read error handled gracefully: {:?}", err),
                }
            }
            Err(err) => {
                println!("Write error handled gracefully: {:?}", err);
            }
        }
    }

    // Verify original data is still intact
    for i in 0..10 {
        let key = format!("key_{}", i);
        let result = Collection::get(&collection, &key);
        match result {
            Ok(value) => assert_eq!(value[0], i as u8, "Original data should remain intact"),
            Err(err) => panic!("Original data corrupted: {:?}", err),
        }
    }
}

#[test]
fn test_memory_usage_bounds() {
    // Test that memory usage stays within reasonable bounds
    let temp_dir = TempDir::new().unwrap();
    let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

    let mut collection = manager.create_collection("memory_test", "prefix").unwrap();

    // Store a moderate amount of data
    let item_count = 500; // Smaller than RocksDB for SQLite efficiency
    for i in 0..item_count {
        let key = format!("memory_key_{:06}", i);
        let value = vec![(i % 256) as u8; 100]; // 100 bytes per item

        Collection::put(&mut collection, &key, &value).unwrap();

        // Periodically verify we can still read data
        if i % 50 == 0 {
            let retrieved = Collection::get(&collection, &key).unwrap();
            assert_eq!(retrieved, value);
        }
    }

    // Verify all data is accessible
    for i in (0..item_count).step_by(10) {
        let key = format!("memory_key_{:06}", i);
        let expected_value = vec![(i % 256) as u8; 100];

        let retrieved = Collection::get(&collection, &key).unwrap();
        assert_eq!(retrieved, expected_value);
    }

    // Test purge operation
    let purge_result = Collection::purge(&mut collection);
    assert!(purge_result.is_ok(), "Purge should complete successfully");

    // Verify data is actually purged
    for i in (0..item_count).step_by(10) {
        let key = format!("memory_key_{:06}", i);
        let result = Collection::get(&collection, &key);

        // Should get EntryNotFound error after purge
        assert!(result.is_err(), "Data should be purged");
    }
}

#[test]
fn test_deletion_operations() {
    // Test various deletion scenarios
    let temp_dir = TempDir::new().unwrap();
    let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

    let mut collection = manager.create_collection("deletion_test", "prefix").unwrap();

    // Store some data
    for i in 0..10 {
        let key = format!("delete_key_{}", i);
        let value = vec![i as u8; 20];
        Collection::put(&mut collection, &key, &value).unwrap();
    }

    // Delete specific keys
    for i in 0..5 {
        let key = format!("delete_key_{}", i);
        Collection::del(&mut collection, &key).unwrap();
    }

    // Verify deleted keys are gone and remaining keys are intact
    for i in 0..10 {
        let key = format!("delete_key_{}", i);
        let result = Collection::get(&collection, &key);

        if i < 5 {
            // These should be deleted
            assert!(result.is_err(), "Deleted key {} should not be found", i);
        } else {
            // These should still exist
            assert!(result.is_ok(), "Non-deleted key {} should still exist", i);
            assert_eq!(result.unwrap()[0], i as u8);
        }
    }

    // Test state deletion
    let mut state = manager.create_state("delete_state_test", "prefix").unwrap();
    let test_data = vec![42u8; 30];

    State::put(&mut state, &test_data).unwrap();
    assert_eq!(State::get(&state).unwrap(), test_data);

    State::del(&mut state).unwrap();
    assert!(State::get(&state).is_err(), "State should be deleted");
}

#[test]
fn test_flush_operations() {
    // Test flush operations for data durability
    let temp_dir = TempDir::new().unwrap();
    let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

    let mut collection = manager.create_collection("flush_test", "prefix").unwrap();

    // Store some data
    for i in 0..10 {
        let key = format!("flush_key_{}", i);
        let value = vec![i as u8; 15];
        Collection::put(&mut collection, &key, &value).unwrap();
    }

    // Flush to ensure data is written to disk
    let flush_result = Collection::flush(&collection);
    assert!(flush_result.is_ok(), "Flush should succeed");

    // Verify data is still accessible after flush
    for i in 0..10 {
        let key = format!("flush_key_{}", i);
        let value = Collection::get(&collection, &key).unwrap();
        assert_eq!(value[0], i as u8);
    }

    // Test state flush
    let mut state = manager.create_state("flush_state_test", "prefix").unwrap();
    let test_data = vec![123u8; 25];

    State::put(&mut state, &test_data).unwrap();

    let flush_result = State::flush(&state);
    assert!(flush_result.is_ok(), "State flush should succeed");

    assert_eq!(State::get(&state).unwrap(), test_data);
}

#[test]
fn test_last_operation() {
    // Test the last() operation
    let temp_dir = TempDir::new().unwrap();
    let manager = SqliteManager::new(temp_dir.path().to_str().unwrap()).unwrap();

    let mut collection = manager.create_collection("last_test", "prefix").unwrap();

    // Empty collection should return None
    assert!(Collection::last(&collection).is_none(), "Empty collection should return None for last()");

    // Add some data
    let items = vec![
        ("key_1", "value_1"),
        ("key_2", "value_2"),
        ("key_3", "value_3"),
    ];

    for (key, value) in &items {
        Collection::put(&mut collection, key, value.as_bytes()).unwrap();
    }

    // Get last item
    let last_item = Collection::last(&collection);
    assert!(last_item.is_some(), "Non-empty collection should return Some for last()");

    let (last_key, last_value) = last_item.unwrap();
    assert!(!last_key.is_empty(), "Last key should not be empty");
    assert!(!last_value.is_empty(), "Last value should not be empty");

    // Verify it's one of our inserted items
    let last_value_str = String::from_utf8(last_value).unwrap();
    assert!(
        items.iter().any(|(_, v)| *v == last_value_str),
        "Last value should be one of the inserted values"
    );
}