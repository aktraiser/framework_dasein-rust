//! Fan-in edges join their sources instead of forwarding each message alone.
use async_trait::async_trait;
use dasein_agentic_core::distributed::graph::*;
use std::sync::{Arc, Mutex};

type Log = Arc<Mutex<Vec<String>>>;

/// Logs `id:input`, then relays `input/id` or yields it as an output.
struct Step {
    id: ExecutorId,
    log: Log,
    relay: bool,
}

#[async_trait]
impl Executor for Step {
    type Input = String;
    type Message = String;
    type Output = String;
    fn id(&self) -> &ExecutorId {
        &self.id
    }
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }
    async fn handle<C>(&self, input: String, ctx: &mut C) -> Result<(), ExecutorError>
    where
        C: ExecutorContext<String, String> + Send,
    {
        self.log
            .lock()
            .unwrap()
            .push(format!("{}:{input}", self.id));
        if self.relay {
            ctx.send_message(format!("{input}/{}", self.id)).await?;
        } else {
            ctx.yield_output(input).await?;
        }
        Ok(())
    }
}

fn registry(log: &Log, relays: &[&str], sinks: &[&str]) -> ExecutorRegistry<String, String> {
    let mut registry = ExecutorRegistry::new();
    for (names, relay) in [(relays, true), (sinks, false)] {
        for name in names {
            registry.register(Step {
                id: (*name).into(),
                log: log.clone(),
                relay,
            });
        }
    }
    registry
}

/// start fans out to `a` and `b`; `b` goes through `b2`, so its branch is one
/// superstep longer than `a`'s. Both branches join into `agg`.
fn uneven_branches() -> WorkflowBuilder<String> {
    WorkflowBuilder::<String>::new("join")
        .set_start("start")
        .add_fan_out_edge("start", vec!["a", "b"])
        .add_edge(Edge::direct("b", "b2"))
}

#[tokio::test]
async fn fan_in_waits_for_every_source() {
    let log = Log::default();
    let definition = uneven_branches()
        .add_fan_in_edge(vec!["a", "b2"], "agg")
        .build()
        .unwrap();
    let result = Workflow::new(
        definition,
        registry(&log, &["start", "a", "b", "b2"], &["agg"]),
    )
    .run("in".to_string())
    .await
    .unwrap();

    assert!(result.success, "{:?}", result.error);
    // Delivered together, in source order, only after the slower branch.
    assert_eq!(result.outputs, ["in/start/a", "in/start/b/b2"]);
    let log = log.lock().unwrap();
    let first_agg = log.iter().position(|e| e.starts_with("agg:")).unwrap();
    let slow_branch = log.iter().position(|e| e.starts_with("b2:")).unwrap();
    assert!(slow_branch < first_agg, "join fired early: {log:?}");
}

#[tokio::test]
async fn fan_in_with_aggregation_runs_target_once() {
    let log = Log::default();
    let definition = uneven_branches()
        .add_fan_in_edge_with(vec!["a", "b2"], "agg", |parts: Vec<String>| {
            parts.join(" + ")
        })
        .build()
        .unwrap();
    let result = Workflow::new(
        definition,
        registry(&log, &["start", "a", "b", "b2"], &["agg"]),
    )
    .run("in".to_string())
    .await
    .unwrap();

    assert!(result.success, "{:?}", result.error);
    assert_eq!(result.outputs, ["in/start/a + in/start/b/b2"]);
    let calls = log
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.starts_with("agg:"))
        .count();
    assert_eq!(calls, 1);
}

#[tokio::test]
async fn fan_in_fires_when_a_source_can_no_longer_deliver() {
    let log = Log::default();
    // `never` is a source of the join, but its branch is not taken. An
    // unrelated chain x1 -> x2 -> x3 keeps the graph busy meanwhile.
    let definition = WorkflowBuilder::<String>::new("partial")
        .set_start("start")
        .add_edge(Edge::direct("start", "a"))
        .add_edge(Edge::conditional("start", "never", |_: &String| false))
        .add_edge(Edge::direct("start", "x1"))
        .add_edge(Edge::direct("x1", "x2"))
        .add_edge(Edge::direct("x2", "x3"))
        .add_fan_in_edge(vec!["a", "never"], "agg")
        .build()
        .unwrap();
    let result = Workflow::new(
        definition,
        registry(&log, &["start", "a", "never", "x1", "x2"], &["agg", "x3"]),
    )
    .run("in".to_string())
    .await
    .unwrap();

    assert!(result.success, "{:?}", result.error);
    assert!(result.outputs.contains(&"in/start/a".to_string()));
    // The join does not wait for the unrelated chain to finish.
    let log = log.lock().unwrap();
    let agg = log.iter().position(|e| e.starts_with("agg:")).unwrap();
    let chain_end = log.iter().position(|e| e.starts_with("x3:")).unwrap();
    assert!(agg < chain_end, "join waited for unrelated work: {log:?}");
}

#[tokio::test]
async fn two_joins_into_one_target_stay_separate() {
    let log = Log::default();
    // Both joins target `agg`. `d` is one superstep behind `c`, so the second
    // join completes after the first.
    let definition = WorkflowBuilder::<String>::new("two-joins")
        .set_start("start")
        .add_fan_out_edge("start", vec!["a", "b", "c", "d0"])
        .add_edge(Edge::direct("d0", "d"))
        .add_fan_in_edge_with(vec!["a", "b"], "agg", |parts: Vec<String>| {
            parts.join(" + ")
        })
        .add_fan_in_edge_with(vec!["c", "d"], "agg", |parts: Vec<String>| {
            parts.join(" & ")
        })
        .build()
        .unwrap();
    let result = Workflow::new(
        definition,
        registry(&log, &["start", "a", "b", "c", "d0", "d"], &["agg"]),
    )
    .run("in".to_string())
    .await
    .unwrap();

    assert!(result.success, "{:?}", result.error);
    assert_eq!(
        result.outputs,
        ["in/start/a + in/start/b", "in/start/c & in/start/d0/d"]
    );
}

#[test]
fn held_fan_in_messages_survive_a_checkpoint() {
    let mut state = SuperstepState::new();
    state
        .fan_in_buffers
        .entry("3:[fan-in]→agg".into())
        .or_default()
        .push(("a".into(), serde_json::json!("in/start/a")));
    let restored: SuperstepState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    assert_eq!(restored.fan_in_buffers, state.fan_in_buffers);

    // Checkpoints written before fan-in buffering have no such field.
    let mut old = serde_json::to_value(&state).unwrap();
    old.as_object_mut().unwrap().remove("fan_in_buffers");
    let restored: SuperstepState = serde_json::from_value(old).unwrap();
    assert!(restored.fan_in_buffers.is_empty());
}
