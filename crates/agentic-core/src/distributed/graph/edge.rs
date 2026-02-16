//! Edge Types - Connections between executors in the workflow graph.
//!
//! # 5 Edge Types
//!
//! ```text
//! 1. Direct        A ───────────────────▶ B
//! 2. Conditional   A ─── if(cond) ──────▶ B
//! 3. Switch-Case   A ─── match ────┬────▶ B (case1)
//!                                  ├────▶ C (case2)
//!                                  └────▶ D (default)
//! 4. Fan-Out       A ──────────────┬────▶ B
//!                                  ├────▶ C (parallel)
//!                                  └────▶ D
//! 5. Fan-In        A ───┐
//!                  B ───┼──────────────▶ D (aggregation)
//!                  C ───┘
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_core::distributed::graph::{Edge, EdgeKind, ExecutorId};
//!
//! // Direct edge
//! let edge = Edge::direct("code_gen", "compiler");
//!
//! // Conditional edge
//! let edge = Edge::conditional("compiler", "tests", |result: &CompileResult| result.success);
//!
//! // Fan-out edge
//! let edge = Edge::fan_out("splitter", vec!["worker1", "worker2", "worker3"]);
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

use super::types::ExecutorId;

// ============================================================================
// EDGE ID
// ============================================================================

/// Unique identifier for an edge.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeId(String);

impl EdgeId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Generate an edge ID from source and target.
    pub fn from_endpoints(source: &ExecutorId, target: &ExecutorId) -> Self {
        Self(format!("{}→{}", source.as_str(), target.as_str()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EdgeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for EdgeId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

// ============================================================================
// EDGE KIND
// ============================================================================

/// The type of edge connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeKind {
    /// Direct connection: always routes to target.
    Direct,
    /// Conditional: routes if condition returns true.
    Conditional,
    /// Switch-case: routes to one of multiple targets based on match.
    Switch,
    /// Fan-out: routes to multiple targets in parallel.
    FanOut,
    /// Fan-in: aggregates from multiple sources.
    FanIn,
}

impl fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EdgeKind::Direct => write!(f, "Direct"),
            EdgeKind::Conditional => write!(f, "Conditional"),
            EdgeKind::Switch => write!(f, "Switch"),
            EdgeKind::FanOut => write!(f, "FanOut"),
            EdgeKind::FanIn => write!(f, "FanIn"),
        }
    }
}

// ============================================================================
// EDGE CONDITION
// ============================================================================

/// A condition function for conditional edges.
///
/// Returns true if the message should be routed through this edge.
pub type ConditionFn<T> = Arc<dyn Fn(&T) -> bool + Send + Sync>;

/// A switch function that returns the index of the target to route to.
///
/// Returns None if no target matches (uses default if available).
pub type SwitchFn<T> = Arc<dyn Fn(&T) -> Option<usize> + Send + Sync>;

/// A selection function for fan-out edges.
///
/// Returns indices of targets to activate (empty = all targets).
pub type SelectionFn<T> = Arc<dyn Fn(&T, usize) -> Vec<usize> + Send + Sync>;

/// An aggregation function for fan-in edges.
///
/// Combines multiple inputs into a single output.
pub type AggregationFn<T> = Arc<dyn Fn(Vec<T>) -> T + Send + Sync>;

// ============================================================================
// EDGE TARGET
// ============================================================================

/// Target(s) of an edge.
#[derive(Debug, Clone)]
pub enum EdgeTarget {
    /// Single target executor.
    Single(ExecutorId),
    /// Multiple targets (for fan-out/switch).
    Multiple(Vec<ExecutorId>),
}

impl EdgeTarget {
    /// Get all target IDs.
    pub fn targets(&self) -> Vec<&ExecutorId> {
        match self {
            EdgeTarget::Single(id) => vec![id],
            EdgeTarget::Multiple(ids) => ids.iter().collect(),
        }
    }

    /// Check if this target contains the given executor.
    pub fn contains(&self, id: &ExecutorId) -> bool {
        match self {
            EdgeTarget::Single(target) => target == id,
            EdgeTarget::Multiple(targets) => targets.contains(id),
        }
    }

    /// Get the number of targets.
    pub fn len(&self) -> usize {
        match self {
            EdgeTarget::Single(_) => 1,
            EdgeTarget::Multiple(ids) => ids.len(),
        }
    }

    /// Check if there are no targets.
    pub fn is_empty(&self) -> bool {
        match self {
            EdgeTarget::Single(_) => false,
            EdgeTarget::Multiple(ids) => ids.is_empty(),
        }
    }
}

// ============================================================================
// EDGE SOURCE
// ============================================================================

/// Source(s) of an edge.
#[derive(Debug, Clone)]
pub enum EdgeSource {
    /// Single source executor.
    Single(ExecutorId),
    /// Multiple sources (for fan-in).
    Multiple(Vec<ExecutorId>),
}

impl EdgeSource {
    /// Get all source IDs.
    pub fn sources(&self) -> Vec<&ExecutorId> {
        match self {
            EdgeSource::Single(id) => vec![id],
            EdgeSource::Multiple(ids) => ids.iter().collect(),
        }
    }

    /// Check if this source contains the given executor.
    pub fn contains(&self, id: &ExecutorId) -> bool {
        match self {
            EdgeSource::Single(source) => source == id,
            EdgeSource::Multiple(sources) => sources.contains(id),
        }
    }
}

// ============================================================================
// EDGE
// ============================================================================

/// An edge connecting executors in the workflow graph.
///
/// Edges transport messages between executors and can have conditions
/// that determine when they activate.
pub struct Edge<T: Send + Sync + 'static> {
    /// Unique identifier.
    pub id: EdgeId,
    /// Human-readable name.
    pub name: Option<String>,
    /// Source executor(s).
    pub source: EdgeSource,
    /// Target executor(s).
    pub target: EdgeTarget,
    /// Edge type.
    pub kind: EdgeKind,
    /// Condition function (for Conditional edges).
    condition: Option<ConditionFn<T>>,
    /// Switch function (for Switch edges).
    switch_fn: Option<SwitchFn<T>>,
    /// Selection function (for FanOut edges).
    selection_fn: Option<SelectionFn<T>>,
    /// Default target index (for Switch edges).
    default_target: Option<usize>,
}

impl<T: Send + Sync + 'static> Edge<T> {
    // ========================================================================
    // CONSTRUCTORS
    // ========================================================================

    /// Create a direct edge (always routes).
    pub fn direct(source: impl Into<ExecutorId>, target: impl Into<ExecutorId>) -> Self {
        let source_id = source.into();
        let target_id = target.into();
        Self {
            id: EdgeId::from_endpoints(&source_id, &target_id),
            name: None,
            source: EdgeSource::Single(source_id),
            target: EdgeTarget::Single(target_id),
            kind: EdgeKind::Direct,
            condition: None,
            switch_fn: None,
            selection_fn: None,
            default_target: None,
        }
    }

    /// Create a conditional edge (routes if condition is true).
    pub fn conditional<F>(
        source: impl Into<ExecutorId>,
        target: impl Into<ExecutorId>,
        condition: F,
    ) -> Self
    where
        F: Fn(&T) -> bool + Send + Sync + 'static,
    {
        let source_id = source.into();
        let target_id = target.into();
        Self {
            id: EdgeId::from_endpoints(&source_id, &target_id),
            name: None,
            source: EdgeSource::Single(source_id),
            target: EdgeTarget::Single(target_id),
            kind: EdgeKind::Conditional,
            condition: Some(Arc::new(condition)),
            switch_fn: None,
            selection_fn: None,
            default_target: None,
        }
    }

    /// Create a switch edge (routes to one of multiple targets).
    pub fn switch<F>(
        source: impl Into<ExecutorId>,
        targets: Vec<impl Into<ExecutorId>>,
        switch_fn: F,
    ) -> Self
    where
        F: Fn(&T) -> Option<usize> + Send + Sync + 'static,
    {
        let source_id = source.into();
        let target_ids: Vec<ExecutorId> = targets.into_iter().map(Into::into).collect();
        Self {
            id: EdgeId::new(format!("{}→[switch]", source_id.as_str())),
            name: None,
            source: EdgeSource::Single(source_id),
            target: EdgeTarget::Multiple(target_ids),
            kind: EdgeKind::Switch,
            condition: None,
            switch_fn: Some(Arc::new(switch_fn)),
            selection_fn: None,
            default_target: None,
        }
    }

    /// Create a fan-out edge (routes to multiple targets in parallel).
    pub fn fan_out(source: impl Into<ExecutorId>, targets: Vec<impl Into<ExecutorId>>) -> Self {
        let source_id = source.into();
        let target_ids: Vec<ExecutorId> = targets.into_iter().map(Into::into).collect();
        Self {
            id: EdgeId::new(format!("{}→[fan-out]", source_id.as_str())),
            name: None,
            source: EdgeSource::Single(source_id),
            target: EdgeTarget::Multiple(target_ids),
            kind: EdgeKind::FanOut,
            condition: None,
            switch_fn: None,
            selection_fn: None,
            default_target: None,
        }
    }

    /// Create a fan-out edge with dynamic selection.
    pub fn fan_out_with_selection<F>(
        source: impl Into<ExecutorId>,
        targets: Vec<impl Into<ExecutorId>>,
        selection_fn: F,
    ) -> Self
    where
        F: Fn(&T, usize) -> Vec<usize> + Send + Sync + 'static,
    {
        let source_id = source.into();
        let target_ids: Vec<ExecutorId> = targets.into_iter().map(Into::into).collect();
        Self {
            id: EdgeId::new(format!("{}→[fan-out-select]", source_id.as_str())),
            name: None,
            source: EdgeSource::Single(source_id),
            target: EdgeTarget::Multiple(target_ids),
            kind: EdgeKind::FanOut,
            condition: None,
            switch_fn: None,
            selection_fn: Some(Arc::new(selection_fn)),
            default_target: None,
        }
    }

    /// Create a fan-in edge (aggregates from multiple sources).
    pub fn fan_in(sources: Vec<impl Into<ExecutorId>>, target: impl Into<ExecutorId>) -> Self {
        let source_ids: Vec<ExecutorId> = sources.into_iter().map(Into::into).collect();
        let target_id = target.into();
        Self {
            id: EdgeId::new(format!("[fan-in]→{}", target_id.as_str())),
            name: None,
            source: EdgeSource::Multiple(source_ids),
            target: EdgeTarget::Single(target_id),
            kind: EdgeKind::FanIn,
            condition: None,
            switch_fn: None,
            selection_fn: None,
            default_target: None,
        }
    }

    // ========================================================================
    // BUILDER METHODS
    // ========================================================================

    /// Set a human-readable name for the edge.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set a custom edge ID.
    pub fn with_id(mut self, id: impl Into<EdgeId>) -> Self {
        self.id = id.into();
        self
    }

    /// Set the default target for switch edges.
    pub fn with_default(mut self, index: usize) -> Self {
        self.default_target = Some(index);
        self
    }

    // ========================================================================
    // ROUTING METHODS
    // ========================================================================

    /// Check if this edge should activate for the given message.
    pub fn should_activate(&self, message: &T) -> bool {
        match self.kind {
            EdgeKind::Direct => true,
            EdgeKind::Conditional => self.condition.as_ref().map_or(true, |cond| cond(message)),
            EdgeKind::Switch => true, // Switch always activates, target selection happens in route()
            EdgeKind::FanOut => true,
            EdgeKind::FanIn => true,
        }
    }

    /// Get the target executor(s) for the given message.
    ///
    /// Returns the executor IDs that should receive this message.
    pub fn route(&self, message: &T) -> Vec<ExecutorId> {
        match &self.kind {
            EdgeKind::Direct => match &self.target {
                EdgeTarget::Single(id) => vec![id.clone()],
                EdgeTarget::Multiple(ids) => ids.clone(),
            },

            EdgeKind::Conditional => {
                if self.should_activate(message) {
                    match &self.target {
                        EdgeTarget::Single(id) => vec![id.clone()],
                        EdgeTarget::Multiple(ids) => ids.clone(),
                    }
                } else {
                    vec![]
                }
            }

            EdgeKind::Switch => {
                if let EdgeTarget::Multiple(targets) = &self.target {
                    let index = self
                        .switch_fn
                        .as_ref()
                        .and_then(|f| f(message))
                        .or(self.default_target);

                    match index {
                        Some(i) if i < targets.len() => vec![targets[i].clone()],
                        _ => vec![],
                    }
                } else {
                    vec![]
                }
            }

            EdgeKind::FanOut => {
                if let EdgeTarget::Multiple(targets) = &self.target {
                    if let Some(selection_fn) = &self.selection_fn {
                        let indices = selection_fn(message, targets.len());
                        if indices.is_empty() {
                            // Empty selection = all targets
                            targets.clone()
                        } else {
                            indices
                                .into_iter()
                                .filter_map(|i| targets.get(i).cloned())
                                .collect()
                        }
                    } else {
                        // No selection function = all targets
                        targets.clone()
                    }
                } else {
                    match &self.target {
                        EdgeTarget::Single(id) => vec![id.clone()],
                        EdgeTarget::Multiple(ids) => ids.clone(),
                    }
                }
            }

            EdgeKind::FanIn => match &self.target {
                EdgeTarget::Single(id) => vec![id.clone()],
                EdgeTarget::Multiple(ids) => ids.clone(),
            },
        }
    }

    /// Check if this edge originates from the given executor.
    pub fn is_from(&self, executor_id: &ExecutorId) -> bool {
        self.source.contains(executor_id)
    }

    /// Check if this edge targets the given executor.
    pub fn is_to(&self, executor_id: &ExecutorId) -> bool {
        self.target.contains(executor_id)
    }
}

impl<T: Send + Sync + 'static> fmt::Debug for Edge<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Edge")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("source", &self.source)
            .field("target", &self.target)
            .field("has_condition", &self.condition.is_some())
            .field("has_switch_fn", &self.switch_fn.is_some())
            .field("has_selection_fn", &self.selection_fn.is_some())
            .field("default_target", &self.default_target)
            .finish()
    }
}

// ============================================================================
// EDGE COLLECTION
// ============================================================================

/// A collection of edges for a workflow.
#[derive(Debug)]
pub struct EdgeCollection<T: Send + Sync + 'static> {
    edges: Vec<Edge<T>>,
}

impl<T: Send + Sync + 'static> Default for EdgeCollection<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + Sync + 'static> EdgeCollection<T> {
    pub fn new() -> Self {
        Self { edges: Vec::new() }
    }

    /// Add an edge to the collection.
    pub fn add(&mut self, edge: Edge<T>) {
        self.edges.push(edge);
    }

    /// Get all edges originating from an executor.
    pub fn from_executor(&self, executor_id: &ExecutorId) -> Vec<&Edge<T>> {
        self.edges
            .iter()
            .filter(|e| e.is_from(executor_id))
            .collect()
    }

    /// Get all edges targeting an executor.
    pub fn to_executor(&self, executor_id: &ExecutorId) -> Vec<&Edge<T>> {
        self.edges.iter().filter(|e| e.is_to(executor_id)).collect()
    }

    /// Get an edge by ID.
    pub fn get(&self, id: &EdgeId) -> Option<&Edge<T>> {
        self.edges.iter().find(|e| &e.id == id)
    }

    /// Get all edges.
    pub fn all(&self) -> &[Edge<T>] {
        &self.edges
    }

    /// Get the number of edges.
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// Route a message from an executor through all applicable edges.
    pub fn route_from(&self, executor_id: &ExecutorId, message: &T) -> Vec<ExecutorId> {
        let mut targets = Vec::new();
        for edge in self.from_executor(executor_id) {
            if edge.should_activate(message) {
                targets.extend(edge.route(message));
            }
        }
        targets
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direct_edge() {
        let edge: Edge<String> = Edge::direct("a", "b");

        assert_eq!(edge.kind, EdgeKind::Direct);
        assert!(edge.should_activate(&"test".to_string()));

        let targets = edge.route(&"test".to_string());
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].as_str(), "b");
    }

    #[test]
    fn test_conditional_edge_pass() {
        let edge: Edge<i32> = Edge::conditional("a", "b", |n: &i32| *n > 10);

        assert!(edge.should_activate(&20));
        let targets = edge.route(&20);
        assert_eq!(targets.len(), 1);
    }

    #[test]
    fn test_conditional_edge_fail() {
        let edge: Edge<i32> = Edge::conditional("a", "b", |n: &i32| *n > 10);

        assert!(!edge.should_activate(&5));
        let targets = edge.route(&5);
        assert!(targets.is_empty());
    }

    #[test]
    fn test_switch_edge() {
        #[derive(Debug)]
        enum Priority {
            High,
            Medium,
            Low,
        }

        let edge: Edge<Priority> = Edge::switch(
            "router",
            vec!["fast", "normal", "slow"],
            |p: &Priority| match p {
                Priority::High => Some(0),
                Priority::Medium => Some(1),
                Priority::Low => Some(2),
            },
        );

        assert_eq!(edge.kind, EdgeKind::Switch);

        let targets = edge.route(&Priority::High);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].as_str(), "fast");

        let targets = edge.route(&Priority::Low);
        assert_eq!(targets[0].as_str(), "slow");
    }

    #[test]
    fn test_switch_edge_with_default() {
        let edge: Edge<i32> = Edge::switch("router", vec!["a", "b", "c"], |n: &i32| {
            if *n == 1 {
                Some(0)
            } else {
                None
            }
        })
        .with_default(2);

        // Explicit match
        let targets = edge.route(&1);
        assert_eq!(targets[0].as_str(), "a");

        // Default
        let targets = edge.route(&999);
        assert_eq!(targets[0].as_str(), "c");
    }

    #[test]
    fn test_fan_out_edge() {
        let edge: Edge<String> = Edge::fan_out("splitter", vec!["w1", "w2", "w3"]);

        assert_eq!(edge.kind, EdgeKind::FanOut);

        let targets = edge.route(&"test".to_string());
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0].as_str(), "w1");
        assert_eq!(targets[1].as_str(), "w2");
        assert_eq!(targets[2].as_str(), "w3");
    }

    #[test]
    fn test_fan_out_with_selection() {
        #[derive(Debug)]
        struct Task {
            priority: u8,
        }

        let edge: Edge<Task> = Edge::fan_out_with_selection(
            "router",
            vec!["fast", "medium", "slow"],
            |task: &Task, _count| {
                if task.priority > 8 {
                    vec![0] // Only fast
                } else if task.priority > 5 {
                    vec![0, 1] // Fast + medium
                } else {
                    vec![] // All (empty = all)
                }
            },
        );

        // High priority → only fast
        let targets = edge.route(&Task { priority: 10 });
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].as_str(), "fast");

        // Medium priority → fast + medium
        let targets = edge.route(&Task { priority: 7 });
        assert_eq!(targets.len(), 2);

        // Low priority → all
        let targets = edge.route(&Task { priority: 2 });
        assert_eq!(targets.len(), 3);
    }

    #[test]
    fn test_fan_in_edge() {
        let edge: Edge<String> = Edge::fan_in(vec!["w1", "w2", "w3"], "aggregator");

        assert_eq!(edge.kind, EdgeKind::FanIn);
        assert!(edge.is_from(&ExecutorId::new("w1")));
        assert!(edge.is_from(&ExecutorId::new("w2")));
        assert!(edge.is_from(&ExecutorId::new("w3")));
        assert!(edge.is_to(&ExecutorId::new("aggregator")));
    }

    #[test]
    fn test_edge_collection() {
        let mut collection: EdgeCollection<String> = EdgeCollection::new();

        collection.add(Edge::direct("a", "b"));
        collection.add(Edge::direct("a", "c"));
        collection.add(Edge::direct("b", "d"));

        assert_eq!(collection.len(), 3);

        // From executor
        let edges = collection.from_executor(&ExecutorId::new("a"));
        assert_eq!(edges.len(), 2);

        // To executor
        let edges = collection.to_executor(&ExecutorId::new("d"));
        assert_eq!(edges.len(), 1);
    }

    #[test]
    fn test_edge_collection_routing() {
        let mut collection: EdgeCollection<i32> = EdgeCollection::new();

        collection.add(Edge::conditional("router", "high", |n: &i32| *n > 100));
        collection.add(Edge::conditional("router", "medium", |n: &i32| {
            *n > 50 && *n <= 100
        }));
        collection.add(Edge::conditional("router", "low", |n: &i32| *n <= 50));

        // High value
        let targets = collection.route_from(&ExecutorId::new("router"), &150);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].as_str(), "high");

        // Medium value
        let targets = collection.route_from(&ExecutorId::new("router"), &75);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].as_str(), "medium");

        // Low value
        let targets = collection.route_from(&ExecutorId::new("router"), &25);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].as_str(), "low");
    }

    #[test]
    fn test_edge_with_name() {
        let edge: Edge<String> = Edge::direct("a", "b").with_name("success_path");

        assert_eq!(edge.name, Some("success_path".to_string()));
    }

    #[test]
    fn test_edge_id() {
        let id = EdgeId::from_endpoints(&ExecutorId::new("a"), &ExecutorId::new("b"));
        assert_eq!(id.as_str(), "a→b");

        let id = EdgeId::new("custom-id");
        assert_eq!(id.as_str(), "custom-id");
    }

    #[test]
    fn test_edge_kind_display() {
        assert_eq!(format!("{}", EdgeKind::Direct), "Direct");
        assert_eq!(format!("{}", EdgeKind::Conditional), "Conditional");
        assert_eq!(format!("{}", EdgeKind::Switch), "Switch");
        assert_eq!(format!("{}", EdgeKind::FanOut), "FanOut");
        assert_eq!(format!("{}", EdgeKind::FanIn), "FanIn");
    }
}
