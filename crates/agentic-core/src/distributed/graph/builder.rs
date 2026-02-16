//! WorkflowBuilder - Fluent API for constructing workflow graphs.
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_core::distributed::graph::{WorkflowBuilder, Edge};
//!
//! let workflow = WorkflowBuilder::<String>::new("my-workflow")
//!     .set_start("code_gen")
//!     .add_executor_id("code_gen")
//!     .add_executor_id("compiler")
//!     .add_executor_id("tests")
//!     .add_edge(Edge::direct("code_gen", "compiler"))
//!     .add_conditional_edge("compiler", "tests", |r| r.success, "on_success")
//!     .add_conditional_edge("compiler", "code_gen", |r| !r.success, "on_failure")
//!     .build()?;
//! ```

use std::collections::{HashMap, HashSet};

use super::edge::{Edge, EdgeCollection};
use super::types::{ExecutorId, GraphError, GraphResult, WorkflowId};

// ============================================================================
// WORKFLOW DEFINITION
// ============================================================================

/// A workflow definition containing executors and edges.
///
/// This is the output of WorkflowBuilder and can be used to run the workflow.
#[derive(Debug)]
pub struct WorkflowDefinition<T: Send + Sync + 'static> {
    /// Unique workflow identifier.
    pub id: WorkflowId,
    /// Human-readable name.
    pub name: Option<String>,
    /// Starting executor ID.
    pub start: ExecutorId,
    /// All executor IDs in the workflow.
    pub executors: HashSet<ExecutorId>,
    /// All edges connecting executors.
    pub edges: EdgeCollection<T>,
    /// Executor metadata (optional descriptions).
    pub executor_metadata: HashMap<ExecutorId, ExecutorMetadata>,
}

impl<T: Send + Sync + 'static> WorkflowDefinition<T> {
    /// Get the starting executor ID.
    pub fn start_executor(&self) -> &ExecutorId {
        &self.start
    }

    /// Get all executor IDs.
    pub fn executor_ids(&self) -> impl Iterator<Item = &ExecutorId> {
        self.executors.iter()
    }

    /// Check if an executor exists in the workflow.
    pub fn has_executor(&self, id: &ExecutorId) -> bool {
        self.executors.contains(id)
    }

    /// Get edges from an executor.
    pub fn edges_from(&self, executor_id: &ExecutorId) -> Vec<&Edge<T>> {
        self.edges.from_executor(executor_id)
    }

    /// Get edges to an executor.
    pub fn edges_to(&self, executor_id: &ExecutorId) -> Vec<&Edge<T>> {
        self.edges.to_executor(executor_id)
    }

    /// Get successors of an executor (direct connections).
    pub fn successors(&self, executor_id: &ExecutorId) -> Vec<&ExecutorId> {
        self.edges_from(executor_id)
            .into_iter()
            .flat_map(|e| e.target.targets())
            .collect()
    }

    /// Get predecessors of an executor.
    pub fn predecessors(&self, executor_id: &ExecutorId) -> Vec<&ExecutorId> {
        self.edges_to(executor_id)
            .into_iter()
            .flat_map(|e| e.source.sources())
            .collect()
    }

    /// Check if this is a terminal executor (no outgoing edges).
    pub fn is_terminal(&self, executor_id: &ExecutorId) -> bool {
        self.edges_from(executor_id).is_empty()
    }

    /// Get all terminal executors.
    pub fn terminal_executors(&self) -> Vec<&ExecutorId> {
        self.executors
            .iter()
            .filter(|id| self.is_terminal(id))
            .collect()
    }
}

/// Metadata for an executor in the workflow.
#[derive(Debug, Clone, Default)]
pub struct ExecutorMetadata {
    /// Human-readable name.
    pub name: Option<String>,
    /// Description of what this executor does.
    pub description: Option<String>,
}

// ============================================================================
// WORKFLOW BUILDER
// ============================================================================

/// Builder for constructing workflow graphs with a fluent API.
///
/// # Type Parameter
///
/// - `T`: The message type that flows through edges.
pub struct WorkflowBuilder<T: Send + Sync + 'static> {
    id: WorkflowId,
    name: Option<String>,
    start: Option<ExecutorId>,
    executors: HashSet<ExecutorId>,
    edges: Vec<Edge<T>>,
    executor_metadata: HashMap<ExecutorId, ExecutorMetadata>,
}

impl<T: Send + Sync + 'static> WorkflowBuilder<T> {
    /// Create a new workflow builder.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: WorkflowId::new(id),
            name: None,
            start: None,
            executors: HashSet::new(),
            edges: Vec::new(),
            executor_metadata: HashMap::new(),
        }
    }

    /// Set a human-readable name for the workflow.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    // ========================================================================
    // EXECUTOR METHODS
    // ========================================================================

    /// Set the starting executor.
    ///
    /// This executor receives the initial input when the workflow runs.
    pub fn set_start(mut self, executor_id: impl Into<ExecutorId>) -> Self {
        let id = executor_id.into();
        self.executors.insert(id.clone());
        self.start = Some(id);
        self
    }

    /// Add an executor by ID.
    ///
    /// The actual executor instance is provided when running the workflow.
    pub fn add_executor_id(mut self, executor_id: impl Into<ExecutorId>) -> Self {
        self.executors.insert(executor_id.into());
        self
    }

    /// Add an executor with metadata.
    pub fn add_executor_with_metadata(
        mut self,
        executor_id: impl Into<ExecutorId>,
        name: Option<String>,
        description: Option<String>,
    ) -> Self {
        let id = executor_id.into();
        self.executors.insert(id.clone());
        self.executor_metadata
            .insert(id, ExecutorMetadata { name, description });
        self
    }

    // ========================================================================
    // EDGE METHODS
    // ========================================================================

    /// Add a pre-built edge.
    pub fn add_edge(mut self, edge: Edge<T>) -> Self {
        // Auto-add executors referenced by the edge
        for source in edge.source.sources() {
            self.executors.insert(source.clone());
        }
        for target in edge.target.targets() {
            self.executors.insert(target.clone());
        }
        self.edges.push(edge);
        self
    }

    /// Add a direct edge between two executors.
    pub fn add_direct_edge(
        self,
        source: impl Into<ExecutorId>,
        target: impl Into<ExecutorId>,
    ) -> Self {
        self.add_edge(Edge::direct(source, target))
    }

    /// Add a conditional edge with a condition function.
    pub fn add_conditional_edge<F>(
        self,
        source: impl Into<ExecutorId>,
        target: impl Into<ExecutorId>,
        condition: F,
        name: impl Into<String>,
    ) -> Self
    where
        F: Fn(&T) -> bool + Send + Sync + 'static,
    {
        let edge = Edge::conditional(source, target, condition).with_name(name);
        self.add_edge(edge)
    }

    /// Add a switch edge (routes to one of multiple targets).
    pub fn add_switch_edge<F>(
        self,
        source: impl Into<ExecutorId>,
        targets: Vec<impl Into<ExecutorId>>,
        switch_fn: F,
    ) -> Self
    where
        F: Fn(&T) -> Option<usize> + Send + Sync + 'static,
    {
        self.add_edge(Edge::switch(source, targets, switch_fn))
    }

    /// Add a switch edge with a default target.
    pub fn add_switch_edge_with_default<F>(
        self,
        source: impl Into<ExecutorId>,
        targets: Vec<impl Into<ExecutorId>>,
        switch_fn: F,
        default_index: usize,
    ) -> Self
    where
        F: Fn(&T) -> Option<usize> + Send + Sync + 'static,
    {
        let edge = Edge::switch(source, targets, switch_fn).with_default(default_index);
        self.add_edge(edge)
    }

    /// Add a fan-out edge (routes to all targets).
    pub fn add_fan_out_edge(
        self,
        source: impl Into<ExecutorId>,
        targets: Vec<impl Into<ExecutorId>>,
    ) -> Self {
        self.add_edge(Edge::fan_out(source, targets))
    }

    /// Add a fan-out edge with dynamic selection.
    pub fn add_fan_out_edge_with_selection<F>(
        self,
        source: impl Into<ExecutorId>,
        targets: Vec<impl Into<ExecutorId>>,
        selection_fn: F,
    ) -> Self
    where
        F: Fn(&T, usize) -> Vec<usize> + Send + Sync + 'static,
    {
        self.add_edge(Edge::fan_out_with_selection(source, targets, selection_fn))
    }

    /// Add a fan-in edge (aggregates from multiple sources).
    pub fn add_fan_in_edge(
        self,
        sources: Vec<impl Into<ExecutorId>>,
        target: impl Into<ExecutorId>,
    ) -> Self {
        self.add_edge(Edge::fan_in(sources, target))
    }

    // ========================================================================
    // BUILD & VALIDATION
    // ========================================================================

    /// Build the workflow definition.
    ///
    /// Performs validation to ensure the graph is well-formed:
    /// - Start executor is set
    /// - All edge references point to valid executors
    /// - Graph is connected (all executors reachable from start)
    ///
    /// # Errors
    ///
    /// Returns an error if validation fails.
    pub fn build(self) -> GraphResult<WorkflowDefinition<T>> {
        // Check start executor is set
        let start = self
            .start
            .ok_or_else(|| GraphError::ValidationError("Start executor not set".into()))?;

        // Check start executor exists
        if !self.executors.contains(&start) {
            return Err(GraphError::ValidationError(format!(
                "Start executor '{}' not found in executor list",
                start
            )));
        }

        // Validate edge references first (before moving edges)
        for edge in &self.edges {
            for source in edge.source.sources() {
                if !self.executors.contains(source) {
                    return Err(GraphError::ValidationError(format!(
                        "Edge '{}' references unknown source executor '{}'",
                        edge.id, source
                    )));
                }
            }
            for target in edge.target.targets() {
                if !self.executors.contains(target) {
                    return Err(GraphError::ValidationError(format!(
                        "Edge '{}' references unknown target executor '{}'",
                        edge.id, target
                    )));
                }
            }
        }

        // Build edge collection (now we can move edges)
        let mut edge_collection = EdgeCollection::new();
        for edge in self.edges {
            edge_collection.add(edge);
        }

        // Check connectivity (all executors reachable from start)
        let reachable = compute_reachable(&start, &edge_collection);
        let unreachable: Vec<_> = self
            .executors
            .iter()
            .filter(|id| !reachable.contains(*id))
            .collect();

        if !unreachable.is_empty() {
            return Err(GraphError::ValidationError(format!(
                "Unreachable executors from start: {:?}",
                unreachable.iter().map(|id| id.as_str()).collect::<Vec<_>>()
            )));
        }

        Ok(WorkflowDefinition {
            id: self.id,
            name: self.name,
            start,
            executors: self.executors,
            edges: edge_collection,
            executor_metadata: self.executor_metadata,
        })
    }
}

/// Compute all executors reachable from a given start point.
fn compute_reachable<T: Send + Sync + 'static>(
    start: &ExecutorId,
    edges: &EdgeCollection<T>,
) -> HashSet<ExecutorId> {
    let mut reachable = HashSet::new();
    let mut queue = vec![start.clone()];

    while let Some(current) = queue.pop() {
        if reachable.contains(&current) {
            continue;
        }
        reachable.insert(current.clone());

        // Add all targets of edges from current
        for edge in edges.from_executor(&current) {
            for target in edge.target.targets() {
                if !reachable.contains(target) {
                    queue.push(target.clone());
                }
            }
        }
    }

    reachable
}

// ============================================================================
// VALIDATION HELPERS
// ============================================================================

/// Validation result for workflow graphs.
#[derive(Debug)]
pub struct ValidationReport {
    /// List of errors found.
    pub errors: Vec<String>,
    /// List of warnings (non-fatal issues).
    pub warnings: Vec<String>,
}

impl ValidationReport {
    /// Create a new empty report.
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Check if validation passed (no errors).
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// Add an error.
    pub fn add_error(&mut self, msg: impl Into<String>) {
        self.errors.push(msg.into());
    }

    /// Add a warning.
    pub fn add_warning(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }
}

impl Default for ValidationReport {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate a workflow definition.
pub fn validate_workflow<T: Send + Sync + 'static>(
    workflow: &WorkflowDefinition<T>,
) -> ValidationReport {
    let mut report = ValidationReport::new();

    // Check for executors with no incoming edges (except start)
    for executor_id in &workflow.executors {
        if executor_id != &workflow.start && workflow.predecessors(executor_id).is_empty() {
            report.add_warning(format!(
                "Executor '{}' has no incoming edges (unreachable except as start)",
                executor_id
            ));
        }
    }

    // Check for cycles (would cause infinite loops)
    if let Some(cycle) = detect_cycle(workflow) {
        report.add_warning(format!(
            "Potential cycle detected: {} (may be intentional for retry logic)",
            cycle
                .iter()
                .map(super::types::ExecutorId::as_str)
                .collect::<Vec<_>>()
                .join(" → ")
        ));
    }

    // Check for terminal executors
    let terminals = workflow.terminal_executors();
    if terminals.is_empty() {
        report.add_warning("No terminal executors found (workflow may never complete)");
    }

    report
}

/// Detect cycles in the workflow graph.
///
/// Returns Some(path) if a cycle is found, None otherwise.
fn detect_cycle<T: Send + Sync + 'static>(
    workflow: &WorkflowDefinition<T>,
) -> Option<Vec<ExecutorId>> {
    let mut visited = HashSet::new();
    let mut rec_stack = HashSet::new();
    let mut path = Vec::new();

    for executor_id in &workflow.executors {
        if detect_cycle_util(
            workflow,
            executor_id,
            &mut visited,
            &mut rec_stack,
            &mut path,
        ) {
            return Some(path);
        }
    }

    None
}

fn detect_cycle_util<T: Send + Sync + 'static>(
    workflow: &WorkflowDefinition<T>,
    node: &ExecutorId,
    visited: &mut HashSet<ExecutorId>,
    rec_stack: &mut HashSet<ExecutorId>,
    path: &mut Vec<ExecutorId>,
) -> bool {
    if rec_stack.contains(node) {
        path.push(node.clone());
        return true;
    }

    if visited.contains(node) {
        return false;
    }

    visited.insert(node.clone());
    rec_stack.insert(node.clone());
    path.push(node.clone());

    for successor in workflow.successors(node) {
        if detect_cycle_util(workflow, successor, visited, rec_stack, path) {
            return true;
        }
    }

    path.pop();
    rec_stack.remove(node);
    false
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distributed::graph::EdgeKind;

    #[test]
    fn test_simple_linear_workflow() {
        let workflow = WorkflowBuilder::<String>::new("test-wf")
            .name("Test Workflow")
            .set_start("a")
            .add_executor_id("b")
            .add_executor_id("c")
            .add_direct_edge("a", "b")
            .add_direct_edge("b", "c")
            .build()
            .unwrap();

        assert_eq!(workflow.id.as_str(), "test-wf");
        assert_eq!(workflow.name, Some("Test Workflow".to_string()));
        assert_eq!(workflow.start.as_str(), "a");
        assert_eq!(workflow.executors.len(), 3);

        // Check connectivity
        assert!(workflow.has_executor(&ExecutorId::new("a")));
        assert!(workflow.has_executor(&ExecutorId::new("b")));
        assert!(workflow.has_executor(&ExecutorId::new("c")));

        // Check edges
        let edges_from_a = workflow.edges_from(&ExecutorId::new("a"));
        assert_eq!(edges_from_a.len(), 1);

        // Check terminal
        assert!(workflow.is_terminal(&ExecutorId::new("c")));
        assert!(!workflow.is_terminal(&ExecutorId::new("a")));
    }

    #[test]
    fn test_conditional_workflow() {
        let workflow = WorkflowBuilder::<bool>::new("conditional-wf")
            .set_start("validator")
            .add_executor_id("success_handler")
            .add_executor_id("error_handler")
            .add_conditional_edge(
                "validator",
                "success_handler",
                |passed| *passed,
                "on_success",
            )
            .add_conditional_edge("validator", "error_handler", |passed| !*passed, "on_error")
            .build()
            .unwrap();

        assert_eq!(workflow.executors.len(), 3);
        assert_eq!(workflow.edges.len(), 2);
    }

    #[test]
    fn test_fan_out_workflow() {
        let workflow = WorkflowBuilder::<String>::new("fan-out-wf")
            .set_start("splitter")
            .add_fan_out_edge("splitter", vec!["worker1", "worker2", "worker3"])
            .add_fan_in_edge(vec!["worker1", "worker2", "worker3"], "aggregator")
            .build()
            .unwrap();

        assert_eq!(workflow.executors.len(), 5); // splitter, 3 workers, aggregator

        // Splitter should have outgoing edges
        let edges_from_splitter = workflow.edges_from(&ExecutorId::new("splitter"));
        assert_eq!(edges_from_splitter.len(), 1);
        assert_eq!(edges_from_splitter[0].kind, EdgeKind::FanOut);

        // Aggregator should be terminal
        assert!(workflow.is_terminal(&ExecutorId::new("aggregator")));
    }

    #[test]
    fn test_switch_workflow() {
        #[derive(Clone)]
        struct Priority(u8);

        let workflow = WorkflowBuilder::<Priority>::new("switch-wf")
            .set_start("router")
            .add_switch_edge_with_default(
                "router",
                vec!["fast", "normal", "slow"],
                |p: &Priority| {
                    if p.0 > 8 {
                        Some(0)
                    } else if p.0 > 4 {
                        Some(1)
                    } else {
                        None // Use default
                    }
                },
                2, // default to "slow"
            )
            .build()
            .unwrap();

        assert_eq!(workflow.executors.len(), 4);
    }

    #[test]
    fn test_validation_no_start() {
        let result = WorkflowBuilder::<String>::new("invalid")
            .add_executor_id("a")
            .add_executor_id("b")
            .add_direct_edge("a", "b")
            .build();

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, GraphError::ValidationError(_)));
    }

    #[test]
    fn test_validation_unreachable_executor() {
        let result = WorkflowBuilder::<String>::new("invalid")
            .set_start("a")
            .add_executor_id("b")
            .add_executor_id("orphan") // No edges to/from this
            .add_direct_edge("a", "b")
            .build();

        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            GraphError::ValidationError(msg) => {
                assert!(msg.contains("Unreachable"));
            }
            _ => panic!("Expected ValidationError"),
        }
    }

    #[test]
    fn test_validation_unknown_edge_target() {
        let result = WorkflowBuilder::<String>::new("invalid")
            .set_start("a")
            .add_direct_edge("a", "unknown")
            .build();

        // This should actually succeed because add_edge auto-adds executors
        assert!(result.is_ok());
    }

    #[test]
    fn test_cycle_detection() {
        let workflow = WorkflowBuilder::<String>::new("cyclic")
            .set_start("a")
            .add_direct_edge("a", "b")
            .add_direct_edge("b", "c")
            .add_direct_edge("c", "a") // Cycle back to a
            .build()
            .unwrap();

        let report = validate_workflow(&workflow);
        // Cycle should be detected as warning (may be intentional)
        assert!(report.warnings.iter().any(|w| w.contains("cycle")));
    }

    #[test]
    fn test_validation_report() {
        let workflow = WorkflowBuilder::<String>::new("test")
            .set_start("a")
            .add_direct_edge("a", "b")
            .build()
            .unwrap();

        let report = validate_workflow(&workflow);
        assert!(report.is_valid());
    }

    #[test]
    fn test_successors_and_predecessors() {
        let workflow = WorkflowBuilder::<String>::new("test")
            .set_start("a")
            .add_direct_edge("a", "b")
            .add_direct_edge("a", "c")
            .add_direct_edge("b", "d")
            .add_direct_edge("c", "d")
            .build()
            .unwrap();

        // a has two successors
        let successors = workflow.successors(&ExecutorId::new("a"));
        assert_eq!(successors.len(), 2);

        // d has two predecessors
        let predecessors = workflow.predecessors(&ExecutorId::new("d"));
        assert_eq!(predecessors.len(), 2);
    }

    #[test]
    fn test_executor_metadata() {
        let workflow = WorkflowBuilder::<String>::new("test")
            .set_start("gen")
            .add_executor_with_metadata(
                "gen",
                Some("Code Generator".into()),
                Some("Generates code from prompts".into()),
            )
            .add_executor_id("validator")
            .add_direct_edge("gen", "validator")
            .build()
            .unwrap();

        let meta = workflow
            .executor_metadata
            .get(&ExecutorId::new("gen"))
            .unwrap();
        assert_eq!(meta.name, Some("Code Generator".to_string()));
        assert_eq!(
            meta.description,
            Some("Generates code from prompts".to_string())
        );
    }
}
