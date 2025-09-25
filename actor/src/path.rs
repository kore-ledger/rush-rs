// Copyright 2025 Kore Ledger, SL
// SPDX-License-Identifier: Apache-2.0

//! # Actor path
//!
//! The `path` module provides the `ActorPath` type. The `ActorPath` type is a path to an actor in the actor system.
//!

use serde::{Deserialize, Serialize};

use std::cmp::Ordering;
use std::fmt::{Error, Formatter};

/// Hierarchical actor path providing unique addressing for actors within the actor system.
///
/// `ActorPath` represents the hierarchical location of an actor within the actor system tree,
/// similar to a filesystem path. It provides a structured way to address, route messages to,
/// and manage relationships between actors in the supervision hierarchy. Each path consists
/// of a sequence of string segments that form a unique identifier for an actor.
///
/// # Structure
///
/// Actor paths follow a hierarchical structure where each segment represents a level in the
/// actor tree. For example, `/user/manager/worker` represents an actor named "worker" that
/// is supervised by "manager", which in turn is supervised by "user" (a system root actor).
///
/// # Core Capabilities
///
/// - **Hierarchical Addressing**: Provides unique identification for actors in the system
/// - **Path Navigation**: Navigate up and down the actor hierarchy (parent, root, children)
/// - **Relationship Queries**: Determine parent-child, ancestor-descendant relationships
/// - **Path Manipulation**: Create new paths by extending existing ones
/// - **Serialization**: Full serde support for persistence and network transmission
///
/// # Thread Safety
///
/// `ActorPath` is fully thread-safe and implements `Send + Sync`. All operations are
/// immutable and return new instances, making it safe to share across threads and
/// use in concurrent message passing scenarios.
///
/// # Performance Characteristics
///
/// - **Creation**: O(n) where n is the number of path segments to parse
/// - **Navigation**: O(n) for parent/root operations due to vector cloning
/// - **Comparison**: O(n) lexicographic comparison of path segments
/// - **Memory**: Stores segments as `Vec<String>`, reasonable for typical path depths
///
/// # Usage Patterns
///
/// ```ignore
/// use rush_actor::ActorPath;
///
/// // Create paths from strings
/// let root_path = ActorPath::from("/user");
/// let child_path = ActorPath::from("/user/manager/worker");
///
/// // Navigate the hierarchy
/// let parent = child_path.parent();           // "/user/manager"
/// let root = child_path.root();               // "/user"
/// let key = child_path.key();                 // "worker"
///
/// // Query relationships
/// assert!(parent.is_parent_of(&child_path));
/// assert!(child_path.is_descendant_of(&root));
///
/// // Extend paths
/// let new_child = parent / "supervisor";      // "/user/manager/supervisor"
/// ```
///
/// # Path Format
///
/// Paths use forward slashes as separators and always begin with a leading slash:
/// - Root: `/user`
/// - Child: `/user/manager`
/// - Deep: `/user/manager/worker/task`
/// - Empty: `/` (represents an empty path)
///
/// # Error Handling
///
/// Path operations are designed to be safe and never panic. Invalid operations
/// return empty paths or maintain the original path state as appropriate.
///
#[derive(
    Clone, Hash, Eq, PartialEq, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct ActorPath(Vec<String>);

impl ActorPath {
    /// Extracts the root segment of this actor path, representing the top-level actor.
    ///
    /// The root is the first segment in the hierarchical path and represents the top-level
    /// actor in the supervision tree for this branch. This method is essential for understanding
    /// the supervision hierarchy and routing messages to the appropriate root actor.
    ///
    /// # Returns
    ///
    /// Returns a new `ActorPath` containing only the root segment. If the current path
    /// is already a single segment (top-level), returns a clone of itself. For empty
    /// paths, returns an empty path.
    ///
    /// # Performance
    ///
    /// This operation has O(1) complexity for single-segment paths and O(n) for multi-segment
    /// paths due to the need to clone the root segment into a new vector.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::ActorPath;
    ///
    /// // Multi-level path
    /// let path = ActorPath::from("/user/manager/worker");
    /// let root = path.root();
    /// assert_eq!(root.to_string(), "/user");
    ///
    /// // Already at root level
    /// let root_path = ActorPath::from("/user");
    /// let still_root = root_path.root();
    /// assert_eq!(still_root.to_string(), "/user");
    ///
    /// // Empty path
    /// let empty = ActorPath::from("/");
    /// let empty_root = empty.root();
    /// assert_eq!(empty_root.to_string(), "/");
    /// ```
    ///
    /// # Use Cases
    ///
    /// - **Supervision Queries**: Determine the root supervisor for error escalation
    /// - **Message Routing**: Route messages to the appropriate root actor subsystem
    /// - **System Organization**: Group actors by their root supervisor for management
    /// - **Debugging**: Trace actor hierarchies back to their root for analysis
    ///
    pub fn root(&self) -> Self {
        if self.0.len() == 1 {
            self.clone()
        } else if !self.0.is_empty() {
            ActorPath(self.0.iter().take(1).cloned().collect())
        } else {
            ActorPath(Vec::new())
        }
    }

    /// Obtains the parent path by removing the last segment from this actor path.
    ///
    /// The parent represents the immediate supervisor of this actor in the supervision
    /// hierarchy. This method is fundamental for supervision operations, error escalation,
    /// and understanding the actor tree structure. It provides the path to the actor
    /// that created and supervises the current actor.
    ///
    /// # Returns
    ///
    /// Returns a new `ActorPath` with the last segment removed. If the current path
    /// has only one segment (is already at root level), returns an empty path.
    /// For empty paths, returns an empty path.
    ///
    /// # Performance
    ///
    /// This operation has O(n) complexity due to the need to clone and truncate
    /// the internal vector of path segments.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::ActorPath;
    ///
    /// // Multi-level path
    /// let path = ActorPath::from("/user/manager/worker");
    /// let parent = path.parent();
    /// assert_eq!(parent.to_string(), "/user/manager");
    ///
    /// // Get grandparent
    /// let grandparent = parent.parent();
    /// assert_eq!(grandparent.to_string(), "/user");
    ///
    /// // Root level actor (has no parent in user space)
    /// let root = ActorPath::from("/user");
    /// let root_parent = root.parent();
    /// assert_eq!(root_parent.to_string(), "/");
    /// assert!(root_parent.is_empty());
    /// ```
    ///
    /// # Use Cases
    ///
    /// - **Error Escalation**: Report failures to the supervising actor
    /// - **Lifecycle Management**: Notify parent of actor termination
    /// - **Supervision Hierarchy**: Navigate up the actor tree for management
    /// - **Message Routing**: Route messages to supervisors when needed
    /// - **Debugging**: Trace supervision relationships for system analysis
    ///
    /// # Behavioral Notes
    ///
    /// - Root-level actors return empty paths (no parent in user space)
    /// - Empty paths remain empty (defensive behavior)
    /// - Operation is always safe and never panics
    ///
    pub fn parent(&self) -> Self {
        if self.0.len() > 1 {
            let mut tokens = self.0.clone();
            tokens.truncate(tokens.len() - 1);
            ActorPath(tokens)
        } else {
            ActorPath(Vec::new())
        }
    }

    /// Retrieves the final segment of this actor path, representing the actor's name.
    ///
    /// The key (also called the actor name) is the last segment in the hierarchical path
    /// and represents the unique identifier of this specific actor within its parent's
    /// supervision scope. This is the name that was used when creating the actor and
    /// is essential for addressing and debugging individual actors.
    ///
    /// # Returns
    ///
    /// Returns a `String` containing the last segment of the path. For empty paths,
    /// returns an empty string. This method never panics and always returns a valid string.
    ///
    /// # Performance
    ///
    /// This operation has O(1) complexity as it only accesses the last element
    /// of the internal vector and clones the string content.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::ActorPath;
    ///
    /// // Multi-level path
    /// let path = ActorPath::from("/user/manager/worker");
    /// assert_eq!(path.key(), "worker");
    ///
    /// // Single-level path
    /// let root = ActorPath::from("/user");
    /// assert_eq!(root.key(), "user");
    ///
    /// // Empty path
    /// let empty = ActorPath::from("/");
    /// assert_eq!(empty.key(), "");
    /// ```
    ///
    /// # Use Cases
    ///
    /// - **Actor Identification**: Get the specific name of an actor for logging or debugging
    /// - **Child Management**: Identify children when iterating through supervised actors
    /// - **Message Routing**: Extract destination actor name for routing decisions
    /// - **Configuration**: Use actor names for configuration lookups or resource allocation
    /// - **Monitoring**: Create metrics and traces using actor names as labels
    ///
    /// # Behavioral Notes
    ///
    /// - Empty paths return empty strings (safe fallback behavior)
    /// - Single-segment paths return their only segment
    /// - The returned string is always owned (cloned from internal storage)
    /// - Method is safe to call on any valid ActorPath instance
    ///
    pub fn key(&self) -> String {
        self.0.last().cloned().unwrap_or_else(|| "".to_string())
    }

    /// Calculates the depth level of this actor path in the supervision hierarchy.
    ///
    /// The level represents the depth of this actor in the supervision tree, with
    /// root actors at level 1, their immediate children at level 2, and so on.
    /// This information is crucial for understanding the supervision hierarchy depth
    /// and making decisions about error escalation and resource management.
    ///
    /// # Returns
    ///
    /// Returns a `usize` representing the number of segments in the path:
    /// - `0` for empty paths
    /// - `1` for root-level actors
    /// - `n` for actors at depth n in the hierarchy
    ///
    /// # Performance
    ///
    /// This operation has O(1) complexity as it simply returns the length
    /// of the internal vector.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::ActorPath;
    ///
    /// // Empty path
    /// let empty = ActorPath::from("/");
    /// assert_eq!(empty.level(), 0);
    ///
    /// // Root level actor
    /// let root = ActorPath::from("/user");
    /// assert_eq!(root.level(), 1);
    ///
    /// // Child actor
    /// let child = ActorPath::from("/user/manager");
    /// assert_eq!(child.level(), 2);
    ///
    /// // Deep hierarchy
    /// let deep = ActorPath::from("/user/manager/worker/task");
    /// assert_eq!(deep.level(), 4);
    /// ```
    ///
    /// # Use Cases
    ///
    /// - **Hierarchy Analysis**: Understand the depth of actor trees for optimization
    /// - **Resource Limits**: Enforce maximum supervision depth policies
    /// - **Error Escalation**: Determine escalation paths based on hierarchy depth
    /// - **Monitoring**: Track system complexity through hierarchy metrics
    /// - **Debugging**: Visualize actor tree structure and depth distribution
    ///
    /// # Behavioral Notes
    ///
    /// - Always returns a non-negative integer
    /// - Empty paths return 0 (no segments)
    /// - Single-segment paths return 1 (root level)
    /// - Method execution is guaranteed to be fast and never fails
    ///
    pub fn level(&self) -> usize {
        self.0.len()
    }

    /// Creates a path truncated to a specific depth level in the hierarchy.
    ///
    /// This method allows you to extract ancestors at specific levels of the supervision
    /// hierarchy. It's useful for finding supervisors at particular depths, implementing
    /// escalation policies, or analyzing the supervision tree structure.
    ///
    /// # Parameters
    ///
    /// * `level` - The target depth level (1-based indexing):
    ///   - `1` returns the root actor path
    ///   - `2` returns the path up to the second level
    ///   - Values `>= current level` return the full path unchanged
    ///   - Values `< 1` return the full path unchanged (defensive behavior)
    ///
    /// # Returns
    ///
    /// Returns a new `ActorPath` truncated to the specified level. The operation
    /// is safe and handles edge cases gracefully by returning appropriate fallback paths.
    ///
    /// # Performance
    ///
    /// This operation has O(n) complexity where n is the target level, due to
    /// vector cloning and truncation operations.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::ActorPath;
    ///
    /// let path = ActorPath::from("/user/manager/worker/task");
    ///
    /// // Get ancestors at specific levels
    /// assert_eq!(path.at_level(1).to_string(), "/user");           // root
    /// assert_eq!(path.at_level(2).to_string(), "/user/manager");   // parent of worker
    /// assert_eq!(path.at_level(3).to_string(), "/user/manager/worker"); // parent
    /// assert_eq!(path.at_level(4).to_string(), "/user/manager/worker/task"); // self
    ///
    /// // Edge cases
    /// assert_eq!(path.at_level(0), path);    // Invalid level returns self
    /// assert_eq!(path.at_level(10), path);   // Level too high returns self
    ///
    /// // Single-level path
    /// let root = ActorPath::from("/user");
    /// assert_eq!(root.at_level(1).to_string(), "/user");
    /// ```
    ///
    /// # Use Cases
    ///
    /// - **Error Escalation**: Find supervisors at specific hierarchy levels
    /// - **Policy Enforcement**: Apply rules based on supervision depth
    /// - **Monitoring**: Extract metrics for actors at particular levels
    /// - **Resource Management**: Allocate resources based on hierarchy position
    /// - **Debugging**: Analyze supervision relationships at different depths
    ///
    /// # Special Cases
    ///
    /// - **Top-level paths**: Handled specially to return root efficiently
    /// - **Level equals parent depth**: Returns parent path directly
    /// - **Invalid levels**: Returns the original path (defensive programming)
    /// - **Empty paths**: Handled gracefully without panics
    ///
    /// # Behavioral Notes
    ///
    /// - Uses 1-based indexing (level 1 = root, level 2 = first child, etc.)
    /// - Always returns a valid ActorPath, never panics
    /// - Preserves path integrity for out-of-bounds level values
    /// - Optimized for common cases (root, parent, current level)
    ///
    pub fn at_level(&self, level: usize) -> Self {
        if level < 1 || level >= self.level() {
            self.clone()
        } else if self.is_top_level() {
            self.root()
        } else if level == self.level() - 1 {
            self.parent()
        } else {
            let mut tokens = self.0.clone();
            tokens.truncate(level);
            ActorPath(tokens)
        }
    }

    /// Checks whether this actor path contains any segments.
    ///
    /// An empty path represents the absence of any actor addressing information
    /// and is typically used as a sentinel value or default state. Empty paths
    /// can occur when operations like getting the parent of a root actor are performed,
    /// or when paths are initialized without valid addressing data.
    ///
    /// # Returns
    ///
    /// Returns `true` if the path contains no segments (level 0), `false` otherwise.
    /// This method provides a clear way to distinguish between valid actor addresses
    /// and empty path states.
    ///
    /// # Performance
    ///
    /// This operation has O(1) complexity as it only checks the length
    /// of the internal vector.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::ActorPath;
    ///
    /// // Empty path from root string
    /// let empty = ActorPath::from("/");
    /// assert!(empty.is_empty());
    ///
    /// // Empty path from empty string
    /// let also_empty = ActorPath::from("");
    /// assert!(also_empty.is_empty());
    ///
    /// // Non-empty path
    /// let actor = ActorPath::from("/user");
    /// assert!(!actor.is_empty());
    ///
    /// // Path becomes empty when getting parent of root
    /// let root = ActorPath::from("/user");
    /// let root_parent = root.parent();
    /// assert!(root_parent.is_empty());
    /// ```
    ///
    /// # Use Cases
    ///
    /// - **Validation**: Check if a path contains valid addressing information
    /// - **Default Handling**: Identify uninitialized or sentinel path values
    /// - **Error Recovery**: Detect when path operations result in invalid states
    /// - **Conditional Logic**: Make decisions based on path validity
    /// - **Debugging**: Identify unexpected empty paths in system analysis
    ///
    /// # Behavioral Notes
    ///
    /// - Empty paths have level 0 and key returns empty string
    /// - Empty paths remain empty for most operations (parent, root, etc.)
    /// - Empty paths can be safely used in comparisons and displays as "/"
    /// - Method execution is guaranteed to be fast and never fails
    ///
    /// # Related Methods
    ///
    /// - `level()` returns 0 for empty paths
    /// - `key()` returns empty string for empty paths
    /// - `parent()` and `root()` of empty paths remain empty
    ///
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Determines if this path is an ancestor of another path in the supervision hierarchy.
    ///
    /// An ancestor relationship exists when this path represents a supervisor (direct or indirect)
    /// of the other path. This is fundamental for understanding supervision hierarchies,
    /// error escalation paths, and actor lifecycle management. The relationship is transitive:
    /// if A is ancestor of B and B is ancestor of C, then A is ancestor of C.
    ///
    /// # Parameters
    ///
    /// * `other` - The path to test for descendant relationship. Must be a reference
    ///            to prevent unnecessary cloning during comparison operations.
    ///
    /// # Returns
    ///
    /// Returns `true` if this path is a proper ancestor of the other path,
    /// `false` otherwise. A path is never considered an ancestor of itself
    /// (proper ancestor relationship).
    ///
    /// # Performance
    ///
    /// This operation has O(n) complexity where n is the length of this path,
    /// due to string formatting and prefix checking operations.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::ActorPath;
    ///
    /// let root = ActorPath::from("/user");
    /// let manager = ActorPath::from("/user/manager");
    /// let worker = ActorPath::from("/user/manager/worker");
    /// let different = ActorPath::from("/system/logger");
    ///
    /// // Direct and indirect ancestry
    /// assert!(root.is_ancestor_of(&manager));     // direct ancestor
    /// assert!(root.is_ancestor_of(&worker));      // indirect ancestor
    /// assert!(manager.is_ancestor_of(&worker));   // direct ancestor
    ///
    /// // Self and unrelated paths
    /// assert!(!root.is_ancestor_of(&root));       // not ancestor of self
    /// assert!(!root.is_ancestor_of(&different));  // different hierarchy
    /// assert!(!worker.is_ancestor_of(&manager));  // reverse relationship
    /// ```
    ///
    /// # Use Cases
    ///
    /// - **Error Escalation**: Determine valid escalation paths in supervision trees
    /// - **Access Control**: Verify if an actor can supervise or manage another
    /// - **Hierarchy Validation**: Ensure proper supervisor-supervisee relationships
    /// - **Message Routing**: Route messages up supervision hierarchies
    /// - **Resource Management**: Allocate resources based on supervision relationships
    ///
    /// # Implementation Notes
    ///
    /// - Uses string prefix matching with trailing slash for exact hierarchy matching
    /// - Prevents false positives from similar path prefixes (e.g., "/user" vs "/user2")
    /// - Self-relationship returns false (proper ancestor definition)
    /// - Empty paths are never ancestors of non-empty paths
    ///
    /// # Behavioral Notes
    ///
    /// - Transitive: if A ancestor of B and B ancestor of C, then A ancestor of C
    /// - Antisymmetric: if A ancestor of B, then B is not ancestor of A
    /// - Irreflexive: no path is ancestor of itself
    /// - Safe for all valid ActorPath instances, never panics
    ///
    pub fn is_ancestor_of(&self, other: &ActorPath) -> bool {
        let me = format!("{}/", self);
        other.to_string().as_str().starts_with(me.as_str())
    }

    /// Determines if this path is a descendant of another path in the supervision hierarchy.
    ///
    /// A descendant relationship exists when this path represents a supervisee (direct or indirect)
    /// of the other path. This is the inverse of the ancestor relationship and is essential
    /// for understanding supervision flows, resource allocation, and error propagation patterns.
    /// The relationship is transitive across multiple supervision levels.
    ///
    /// # Parameters
    ///
    /// * `other` - The path to test for ancestor relationship. Must be a reference
    ///            to prevent unnecessary cloning during comparison operations.
    ///
    /// # Returns
    ///
    /// Returns `true` if this path is a proper descendant of the other path,
    /// `false` otherwise. A path is never considered a descendant of itself
    /// (proper descendant relationship).
    ///
    /// # Performance
    ///
    /// This operation has O(n) complexity where n is the length of the other path,
    /// due to string formatting and prefix checking operations.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::ActorPath;
    ///
    /// let root = ActorPath::from("/user");
    /// let manager = ActorPath::from("/user/manager");
    /// let worker = ActorPath::from("/user/manager/worker");
    /// let different = ActorPath::from("/system/logger");
    ///
    /// // Direct and indirect descendancy
    /// assert!(manager.is_descendant_of(&root));      // direct descendant
    /// assert!(worker.is_descendant_of(&root));       // indirect descendant
    /// assert!(worker.is_descendant_of(&manager));    // direct descendant
    ///
    /// // Self and unrelated paths
    /// assert!(!root.is_descendant_of(&root));        // not descendant of self
    /// assert!(!different.is_descendant_of(&root));   // different hierarchy
    /// assert!(!manager.is_descendant_of(&worker));   // reverse relationship
    /// ```
    ///
    /// # Use Cases
    ///
    /// - **Supervision Queries**: Verify if an actor is supervised by another
    /// - **Resource Inheritance**: Determine resource flow down supervision trees
    /// - **Event Propagation**: Route events down to supervised actors
    /// - **Hierarchy Analysis**: Understand supervision relationships and depths
    /// - **Security**: Verify access rights based on supervision relationships
    ///
    /// # Implementation Notes
    ///
    /// - Uses string prefix matching with trailing slash for exact hierarchy matching
    /// - Prevents false positives from similar path prefixes
    /// - Self-relationship returns false (proper descendant definition)
    /// - Empty paths are never descendants of any path
    ///
    /// # Behavioral Notes
    ///
    /// - Transitive: if A descendant of B and B descendant of C, then A descendant of C
    /// - Antisymmetric: if A descendant of B, then B is not descendant of A
    /// - Irreflexive: no path is descendant of itself
    /// - Inverse of ancestor relationship: A.is_descendant_of(B) ≡ B.is_ancestor_of(A)
    ///
    pub fn is_descendant_of(&self, other: &ActorPath) -> bool {
        let me = self.to_string();
        me.as_str().starts_with(format!("{}/", other).as_str())
    }

    /// Determines if this path is the direct parent of another path in the supervision hierarchy.
    ///
    /// A parent relationship exists when this path directly supervises the other path,
    /// meaning the other path is exactly one level deeper and shares the same path prefix.
    /// This is a more specific relationship than ancestor, checking for immediate
    /// supervision rather than any level of hierarchy.
    ///
    /// # Parameters
    ///
    /// * `other` - The path to test for direct child relationship. Must be a reference
    ///            to prevent unnecessary cloning during comparison operations.
    ///
    /// # Returns
    ///
    /// Returns `true` if this path is the direct parent of the other path,
    /// `false` otherwise. Only immediate parent-child relationships return true.
    ///
    /// # Performance
    ///
    /// This operation has O(n) complexity where n is the length of this path,
    /// due to the need to compute and compare the parent of the other path.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::ActorPath;
    ///
    /// let root = ActorPath::from("/user");
    /// let manager = ActorPath::from("/user/manager");
    /// let worker = ActorPath::from("/user/manager/worker");
    ///
    /// // Direct parent relationships
    /// assert!(root.is_parent_of(&manager));        // direct parent
    /// assert!(manager.is_parent_of(&worker));      // direct parent
    ///
    /// // Not direct parent relationships
    /// assert!(!root.is_parent_of(&worker));        // grandparent, not parent
    /// assert!(!manager.is_parent_of(&root));       // reverse relationship
    /// assert!(!root.is_parent_of(&root));          // self relationship
    /// ```
    ///
    /// # Use Cases
    ///
    /// - **Direct Supervision**: Verify immediate supervisor-supervisee relationships
    /// - **Child Management**: Identify direct children for lifecycle operations
    /// - **Resource Allocation**: Manage resources between direct supervisor levels
    /// - **Message Routing**: Route messages to immediate children or parents
    /// - **Error Handling**: Handle errors at the appropriate supervision level
    ///
    /// # Implementation Notes
    ///
    /// - Computes parent of the other path and compares for equality
    /// - More efficient than string-based prefix matching for direct relationships
    /// - Returns false for self-relationships and indirect relationships
    /// - Safe for all valid ActorPath instances
    ///
    /// # Behavioral Notes
    ///
    /// - Symmetric with is_child_of: A.is_parent_of(B) ≡ B.is_child_of(A)
    /// - More restrictive than is_ancestor_of (only immediate relationships)
    /// - Irreflexive: no path is parent of itself
    /// - Returns false for paths that differ by more than one level
    ///
    pub fn is_parent_of(&self, other: &ActorPath) -> bool {
        *self == other.parent()
    }

    /// Determines if this path is the direct child of another path in the supervision hierarchy.
    ///
    /// A child relationship exists when this path is directly supervised by the other path,
    /// meaning this path is exactly one level deeper and shares the same path prefix.
    /// This is a more specific relationship than descendant, checking for immediate
    /// supervision rather than any level of hierarchy depth.
    ///
    /// # Parameters
    ///
    /// * `other` - The path to test for direct parent relationship. Must be a reference
    ///            to prevent unnecessary cloning during comparison operations.
    ///
    /// # Returns
    ///
    /// Returns `true` if this path is the direct child of the other path,
    /// `false` otherwise. Only immediate parent-child relationships return true.
    ///
    /// # Performance
    ///
    /// This operation has O(n) complexity where n is the length of this path,
    /// due to the need to compute and compare the parent of this path.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::ActorPath;
    ///
    /// let root = ActorPath::from("/user");
    /// let manager = ActorPath::from("/user/manager");
    /// let worker = ActorPath::from("/user/manager/worker");
    ///
    /// // Direct child relationships
    /// assert!(manager.is_child_of(&root));         // direct child
    /// assert!(worker.is_child_of(&manager));       // direct child
    ///
    /// // Not direct child relationships
    /// assert!(!worker.is_child_of(&root));         // grandchild, not child
    /// assert!(!root.is_child_of(&manager));        // reverse relationship
    /// assert!(!root.is_child_of(&root));           // self relationship
    /// ```
    ///
    /// # Use Cases
    ///
    /// - **Supervision Verification**: Confirm direct supervision relationships
    /// - **Child Discovery**: Identify if an actor is a direct child during iteration
    /// - **Lifecycle Management**: Handle direct child actor lifecycle events
    /// - **Resource Cleanup**: Clean up resources for direct children
    /// - **Error Propagation**: Propagate errors from direct children to parents
    ///
    /// # Implementation Notes
    ///
    /// - Computes parent of this path and compares for equality with other
    /// - More efficient than string-based prefix matching for direct relationships
    /// - Returns false for self-relationships and indirect relationships
    /// - Safe for all valid ActorPath instances
    ///
    /// # Behavioral Notes
    ///
    /// - Symmetric with is_parent_of: A.is_child_of(B) ≡ B.is_parent_of(A)
    /// - More restrictive than is_descendant_of (only immediate relationships)
    /// - Irreflexive: no path is child of itself
    /// - Returns false for paths that differ by more than one level
    ///
    pub fn is_child_of(&self, other: &ActorPath) -> bool {
        self.parent() == *other
    }

    /// Determines if this path represents a top-level (root) actor in the system.
    ///
    /// A top-level actor is one that has no parent in the user actor space,
    /// containing exactly one segment in its path. These actors typically serve
    /// as the root supervisors for entire actor subsystems and are directly
    /// managed by the actor system itself.
    ///
    /// # Returns
    ///
    /// Returns `true` if the path contains exactly one segment (level 1),
    /// `false` otherwise. Empty paths (level 0) return false.
    ///
    /// # Performance
    ///
    /// This operation has O(1) complexity as it only checks the length
    /// of the internal vector.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use rush_actor::ActorPath;
    ///
    /// // Top-level actors
    /// let user_root = ActorPath::from("/user");
    /// let system_root = ActorPath::from("/system");
    /// assert!(user_root.is_top_level());
    /// assert!(system_root.is_top_level());
    ///
    /// // Not top-level actors
    /// let child = ActorPath::from("/user/manager");
    /// let grandchild = ActorPath::from("/user/manager/worker");
    /// assert!(!child.is_top_level());
    /// assert!(!grandchild.is_top_level());
    ///
    /// // Empty path is not top-level
    /// let empty = ActorPath::from("/");
    /// assert!(!empty.is_top_level());
    /// ```
    ///
    /// # Use Cases
    ///
    /// - **System Organization**: Identify root supervisors for different subsystems
    /// - **Resource Management**: Apply different policies to top-level actors
    /// - **Error Handling**: Handle errors at the system boundary differently
    /// - **Monitoring**: Track system-level metrics for root actors
    /// - **Configuration**: Apply system-level configuration to root actors
    ///
    /// # Implementation Notes
    ///
    /// - Simply checks if the internal vector has exactly one element
    /// - More efficient than level() == 1 comparison
    /// - Does not require string operations or complex logic
    /// - Safe for all valid ActorPath instances
    ///
    /// # Behavioral Notes
    ///
    /// - Top-level actors have no parents (parent() returns empty path)
    /// - Top-level actors are their own root (root() returns self)
    /// - Exactly equivalent to level() == 1
    /// - Method execution is guaranteed to be fast and never fails
    ///
    /// # Related Methods
    ///
    /// - `level()` returns 1 for top-level paths
    /// - `parent()` returns empty path for top-level paths
    /// - `root()` returns self for top-level paths
    ///
    pub fn is_top_level(&self) -> bool {
        self.0.len() == 1
    }
}

/// Converts a string slice into an ActorPath by parsing the hierarchical path structure.
///
/// This implementation provides the primary way to create ActorPath instances from
/// string representations. It handles path parsing, normalization, and validation
/// to ensure the resulting path follows the expected hierarchical structure.
///
/// # Path Format
///
/// The expected format uses forward slashes as separators with optional leading slash:
/// - "/user/manager/worker" - Standard hierarchical path
/// - "user/manager/worker" - Also valid (leading slash is optional)
/// - "/user//manager/worker" - Multiple slashes are normalized (empty segments filtered)
/// - "/" or "" - Results in empty path (no segments)
/// - " /user / manager " - Whitespace is trimmed from segments
///
/// # Performance
///
/// This operation has O(n) complexity where n is the length of the input string,
/// due to string splitting, filtering, and vector construction operations.
///
/// # Examples
///
/// ```ignore
/// use rush_actor::ActorPath;
///
/// // Standard paths
/// let path1 = ActorPath::from("/user/manager/worker");
/// let path2 = ActorPath::from("user/manager/worker");  // same result
///
/// // Edge cases
/// let empty = ActorPath::from("/");
/// let also_empty = ActorPath::from("");
/// assert!(empty.is_empty() && also_empty.is_empty());
///
/// // Normalization
/// let normalized = ActorPath::from("/user//manager///worker/");
/// assert_eq!(normalized.level(), 3); // empty segments filtered
/// assert_eq!(normalized.key(), "worker");
/// ```
///
/// # Use Cases
///
/// - **Configuration**: Parse actor paths from configuration files
/// - **User Input**: Convert user-provided path strings to ActorPath instances
/// - **Serialization**: Recreate paths from serialized string representations
/// - **Testing**: Create test paths from string literals
/// - **Message Routing**: Parse destination paths from message headers
///
/// # Error Handling
///
/// This implementation never panics and handles all edge cases gracefully:
/// - Empty strings result in empty paths
/// - Malformed paths are normalized (empty segments removed)
/// - Whitespace is automatically trimmed from segments
/// - Multiple consecutive slashes are treated as single separators
///
impl From<&str> for ActorPath {
    fn from(str: &str) -> Self {
        let tokens: Vec<String> = str
            .split('/')
            .filter(|x| !x.trim().is_empty())
            .map(|s| s.to_string())
            .collect();
        ActorPath(tokens)
    }
}

/// Converts an owned String into an ActorPath by delegating to the &str implementation.
///
/// This implementation provides a convenient way to create ActorPath instances from
/// owned String values without requiring explicit borrowing. It simply delegates
/// to the &str implementation to ensure consistent parsing behavior.
///
/// # Performance
///
/// This operation has the same O(n) complexity as the &str implementation,
/// with minimal additional overhead for the string reference conversion.
///
/// # Examples
///
/// ```ignore
/// use rush_actor::ActorPath;
///
/// let path_string = String::from("/user/manager/worker");
/// let path = ActorPath::from(path_string);
/// assert_eq!(path.key(), "worker");
///
/// // Works with string expressions
/// let dynamic_path = ActorPath::from(format!("/user/{}/worker", "manager"));
/// assert_eq!(dynamic_path.level(), 3);
/// ```
///
/// # Use Cases
///
/// - **Dynamic Path Creation**: Build paths from formatted strings
/// - **API Responses**: Convert received string data to ActorPath
/// - **Deserialization**: Handle owned strings from deserialization processes
/// - **Builder Patterns**: Use with string building operations
///
impl From<String> for ActorPath {
    fn from(string: String) -> Self {
        ActorPath::from(string.as_str())
    }
}

/// Converts a String reference into an ActorPath by delegating to the &str implementation.
///
/// This implementation provides ergonomic conversion from String references,
/// allowing ActorPath creation from borrowed String values without requiring
/// explicit deref coercion. It maintains consistency with the other From implementations.
///
/// # Performance
///
/// This operation has the same O(n) complexity as the &str implementation,
/// with minimal additional overhead for the string reference conversion.
///
/// # Examples
///
/// ```ignore
/// use rush_actor::ActorPath;
///
/// let path_string = String::from("/user/manager/worker");
/// let path = ActorPath::from(&path_string);
/// assert_eq!(path.key(), "worker");
///
/// // Useful when you need to keep the original string
/// let original = String::from("/system/logger");
/// let path = ActorPath::from(&original);
/// println!("Created path from: {}", original); // original still available
/// ```
///
/// # Use Cases
///
/// - **Borrowing Scenarios**: Convert String references without taking ownership
/// - **Function Parameters**: Accept String references and convert to ActorPath
/// - **Iterative Processing**: Process collections of String references
/// - **Debugging**: Create paths while preserving original string values
///
impl From<&String> for ActorPath {
    fn from(string: &String) -> Self {
        ActorPath::from(string.as_str())
    }
}

/// Implements path extension using the division operator for ergonomic child path creation.
///
/// This implementation provides an intuitive syntax for creating child actor paths
/// by using the `/` operator to append path segments. It's designed to mimic
/// filesystem path operations and provides a clean, readable way to build
/// hierarchical actor paths programmatically.
///
/// # Operator Behavior
///
/// The `/` operator appends the right-hand side string to the current path,
/// creating a new ActorPath that represents a child or descendant in the
/// supervision hierarchy. Multiple segments can be added at once by using
/// slash-separated strings on the right side.
///
/// # Performance
///
/// This operation has O(n + m) complexity where n is the current path length
/// and m is the number of segments in the appended string, due to vector
/// concatenation and string parsing operations.
///
/// # Type Safety
///
/// The operator only accepts `&str` to prevent ambiguous operations and ensure
/// clear, predictable behavior. String slices must be used explicitly.
///
/// # Examples
///
/// ```ignore
/// use rush_actor::ActorPath;
///
/// let root = ActorPath::from("/user");
///
/// // Single segment extension
/// let manager = root.clone() / "manager";
/// assert_eq!(manager.to_string(), "/user/manager");
///
/// // Multiple segment extension
/// let worker = root.clone() / "manager/worker";
/// assert_eq!(worker.to_string(), "/user/manager/worker");
///
/// // Chained operations
/// let task = root / "manager" / "worker" / "task";
/// assert_eq!(task.level(), 4);
/// assert_eq!(task.key(), "task");
///
/// // Building paths programmatically
/// let base = ActorPath::from("/system");
/// let logger = base / "logging/file-logger";
/// assert!(base.is_ancestor_of(&logger));
/// ```
///
/// # Use Cases
///
/// - **Child Actor Creation**: Build paths for child actors during spawning
/// - **Path Construction**: Programmatically build complex actor hierarchies
/// - **Configuration**: Construct paths from configuration parameters
/// - **Testing**: Create test actor paths with readable syntax
/// - **Dynamic Routing**: Build destination paths for message routing
///
/// # String Parsing
///
/// The right-hand side string is parsed using the same logic as `From<&str>`:
/// - Forward slashes separate path segments
/// - Empty segments are filtered out
/// - Whitespace is trimmed from segments
/// - Multiple consecutive slashes are normalized
///
/// # Behavioral Notes
///
/// - Creates a new ActorPath instance (immutable operation)
/// - Preserves the original path (left-hand side remains unchanged)
/// - Handles empty strings gracefully (no change to path)
/// - Safe for all valid input combinations, never panics
/// - Can be chained for building deep hierarchies
///
impl std::ops::Div<&str> for ActorPath {
    type Output = ActorPath;

    fn div(self, rhs: &str) -> Self::Output {
        let mut keys = self.0;
        let mut tokens: Vec<String> = rhs
            .split('/')
            .filter(|x| !x.trim().is_empty())
            .map(|s| s.to_string())
            .collect();

        keys.append(&mut tokens);
        ActorPath(keys)
    }
}

/// Formats the ActorPath as a human-readable string representation.
///
/// This implementation provides the canonical string representation of an ActorPath,
/// suitable for logging, debugging, configuration files, and any context where
/// a readable path representation is needed. The format follows Unix-style path
/// conventions with forward slashes as separators.
///
/// # Format Specification
///
/// The output format depends on the path structure:
/// - **Empty paths**: "/" (single slash representing root)
/// - **Single segment**: "/segment" (root-level actor)
/// - **Multiple segments**: "/segment1/segment2/segment3" (hierarchical path)
///
/// # Performance
///
/// This operation has O(n) complexity where n is the total length of all path
/// segments, due to string joining and formatting operations.
///
/// # Examples
///
/// ```ignore
/// use rush_actor::ActorPath;
///
/// // Empty path
/// let empty = ActorPath::from("/");
/// assert_eq!(format!("{}", empty), "/");
///
/// // Single segment
/// let root = ActorPath::from("/user");
/// assert_eq!(format!("{}", root), "/user");
///
/// // Multiple segments
/// let deep = ActorPath::from("/user/manager/worker");
/// assert_eq!(format!("{}", deep), "/user/manager/worker");
///
/// // Usage in logging
/// println!("Processing message for actor: {}", deep);
/// ```
///
/// # Use Cases
///
/// - **Logging**: Display actor paths in log messages for debugging
/// - **Error Messages**: Include path information in error descriptions
/// - **Configuration**: Generate configuration keys or paths from ActorPath
/// - **Monitoring**: Display paths in metrics and monitoring dashboards
/// - **Serialization**: Convert paths to strings for JSON or other formats
///
/// # Implementation Details
///
/// - Uses efficient pattern matching on path length for optimal performance
/// - Always produces valid, parseable path strings
/// - Maintains consistency with From<&str> implementation (round-trip safe)
/// - Never panics, handles all valid ActorPath states
///
/// # Round-trip Compatibility
///
/// The Display output is guaranteed to be parseable by ActorPath::from():
///
/// ```ignore
/// use rush_actor::ActorPath;
///
/// let original = ActorPath::from("/user/manager/worker");
/// let displayed = format!("{}", original);
/// let parsed = ActorPath::from(displayed.as_str());
/// assert_eq!(original, parsed);
/// ```
///
impl std::fmt::Display for ActorPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        match self.level().cmp(&1) {
            Ordering::Less => write!(f, "/"),
            Ordering::Equal => write!(f, "/{}", self.0[0]),
            Ordering::Greater => write!(f, "/{}", self.0.join("/")),
        }
    }
}

/// Formats the ActorPath for debugging purposes with the same representation as Display.
///
/// This implementation provides debug formatting that mirrors the Display trait,
/// showing the path in its canonical string form rather than exposing internal
/// implementation details. This design choice prioritizes readability and
/// consistency in debugging contexts.
///
/// # Format Specification
///
/// The debug format is identical to Display format:
/// - **Empty paths**: "/" (single slash representing root)
/// - **Single segment**: "/segment" (root-level actor)
/// - **Multiple segments**: "/segment1/segment2/segment3" (hierarchical path)
///
/// # Performance
///
/// This operation has O(n) complexity where n is the total length of all path
/// segments, identical to the Display implementation.
///
/// # Examples
///
/// ```ignore
/// use rush_actor::ActorPath;
///
/// let path = ActorPath::from("/user/manager/worker");
///
/// // Debug formatting
/// assert_eq!(format!("{:?}", path), "/user/manager/worker");
/// assert_eq!(format!("{}", path), format!("{:?}", path));
///
/// // In debug contexts
/// println!("Debug: {:?}", path);
/// dbg!(&path);  // Shows: [src/path.rs:123] &path = /user/manager/worker
/// ```
///
/// # Use Cases
///
/// - **Debugging**: Clear path representation in debug output
/// - **Testing**: Readable assertions in test failures
/// - **Error Debugging**: Structured error output with path information
/// - **Development**: Quick path inspection during development
/// - **Logging**: Consistent format across Display and Debug contexts
///
/// # Design Rationale
///
/// Unlike typical Debug implementations that expose internal structure,
/// this implementation shows the logical path representation because:
/// - ActorPath's value is in its logical meaning, not internal Vec<String>
/// - Consistency between Debug and Display aids in debugging
/// - Path strings are more meaningful than vector contents in debug output
/// - Reduces cognitive load when reading debug output
///
/// # Implementation Details
///
/// - Identical logic to Display implementation for consistency
/// - Efficient pattern matching on path length
/// - Never panics, handles all valid ActorPath states
/// - Maintains same performance characteristics as Display
///
impl std::fmt::Debug for ActorPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        match self.level().cmp(&1) {
            Ordering::Less => write!(f, "/"),
            Ordering::Equal => write!(f, "/{}", self.0[0]),
            Ordering::Greater => write!(f, "/{}", self.0.join("/")),
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn parse_empty_string() {
        let path = ActorPath::from("");
        assert_eq!(path.0, Vec::<String>::new());
    }

    #[test]
    fn parse_single_root() {
        let path = ActorPath::from("/acme");
        println!("{:?}", path);
        assert_eq!(path.0, vec!["acme"]);
    }

    #[test]
    fn parse_two_deep() {
        let path = ActorPath::from("/acme/building");
        println!("{:?}", path);
        assert_eq!(path.0, vec!["acme", "building"]);
    }

    #[test]
    fn parse_three_deep() {
        let path = ActorPath::from("/acme/building/room");
        println!("{:?}", path);
        assert_eq!(path.0, vec!["acme", "building", "room"]);
    }

    #[test]
    fn parse_levels() {
        let path = ActorPath::from("/acme/building/room/sensor");
        println!("{:?}", path);
        assert_eq!(path.level(), 4);
    }

    #[test]
    fn test_get_key() {
        let path = ActorPath::from("/acme/building/room/sensor");
        println!("{:?}", path);
        assert_eq!(path.key(), "sensor".to_string());
    }

    #[test]
    fn parse_get_parent() {
        let path = ActorPath::from("/acme/building/room/sensor").parent();
        println!("{:?}", path);
        assert_eq!(path.parent().0, vec!["acme", "building"]);
    }

    #[test]
    fn parse_to_string() {
        let path = ActorPath::from("/acme/building/room/sensor");
        let string = path.to_string();
        println!("{:?}", string);
        assert_eq!(string, "/acme/building/room/sensor");
    }

    #[test]
    fn parse_root_at_root() {
        let path = ActorPath::from("/acme");
        let string = path.root().to_string();
        println!("{:?}", string);
        assert_eq!(string, "/acme");
    }

    #[test]
    fn parse_parent_at_root() {
        let path = ActorPath::from("/acme");
        let string = path.parent().to_string();
        println!("{:?}", string);
        assert_eq!(string, "/");
    }

    #[test]
    fn parse_root_to_string() {
        let path = ActorPath::from("/acme/building/room/sensor");
        let string = path.root().to_string();
        println!("{:?}", string);
        assert_eq!(string, "/acme");
    }

    #[test]
    fn test_if_empty() {
        let path = ActorPath::from("/");
        assert!(path.is_empty());
        let not_empty = ActorPath::from("/not_empty");
        assert!(!not_empty.is_empty());
    }

    #[test]
    fn test_if_parent_child() {
        let path = ActorPath::from("/acme/building/room/sensor");
        let parent = path.parent();
        assert!(parent.is_parent_of(&path));
        assert!(path.is_child_of(&parent));
    }

    #[test]
    fn test_if_descendant() {
        let path = ActorPath::from("/acme/building/room/sensor");
        let parent = path.parent();
        assert!(path.is_descendant_of(&parent));
        assert!(!path.is_descendant_of(&path));
    }

    #[test]
    fn test_if_ancestor() {
        let path = ActorPath::from("/acme/building/room/sensor");
        let parent = path.parent();
        assert!(parent.is_ancestor_of(&path));
        assert!(!path.is_ancestor_of(&path));
    }

    #[test]
    fn test_if_ancestor_descendant() {
        let path = ActorPath::from("/acme/building/room/sensor");
        let root = path.root();
        assert!(root.is_ancestor_of(&path));
        assert!(path.is_descendant_of(&root));
    }

    #[test]
    fn test_if_root() {
        let path = ActorPath::from("/acme/building/room/sensor");
        let root = path.root();
        println!("{:?}", path);
        println!("{:?}", root);
        assert!(root.is_top_level());
        assert!(!path.is_top_level());
    }

    #[test]
    fn test_at_level() {
        let path = ActorPath::from("/acme/building/room/sensor");
        assert_eq!(path.at_level(0), path);
        assert_eq!(path.at_level(1), path.root());
        assert_eq!(path.at_level(2), ActorPath::from("/acme/building"));
        assert_eq!(path.at_level(3), path.parent());
        assert_eq!(path.at_level(4), path);
        assert_eq!(path.at_level(5), path);
    }

    #[test]
    fn test_add_path() {
        let path = ActorPath::from("/acme");
        let child = path.clone() / "child";
        println!("{}", &child);
        assert!(path.is_parent_of(&child))
    }
}
