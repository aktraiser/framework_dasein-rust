//! What a run does, as it does it.
//!
//! A run reports its progress as [`WorkflowStreamEvent`]s: supersteps and
//! executors starting and ending, outputs, the events executors add, and the
//! model and tool calls of the agents inside them. Nothing is reported, and
//! nothing is paid for it, unless someone listens.
//!
//! [`Workflow::run_stream`](super::Workflow::run_stream) listens to one run
//! of a workflow. [`with_events`] listens to any future, for instance an
//! agent run outside a workflow.
//!
//! Events of one run are in the order things happened in the process that
//! ran them. Steps may run side by side: an agent's events name the executor
//! and the input they belong to. A workflow run inside a step reports to
//! the same listener: its events carry its own workflow id.
//!
//! The events of a step run by a remote worker come back with its result
//! and are reported then, together, between that step's start and its end,
//! if the worker was asked to report them.

use super::types::ExecutorId;
use super::workflow::WorkflowStreamEvent;
use std::future::Future;
use tokio::sync::mpsc::UnboundedSender;

tokio::task_local! {
    /// Who listens to the run this task is part of.
    static LISTENER: UnboundedSender<WorkflowStreamEvent>;
}

/// Run `future` with `listener` receiving the events of what it runs.
///
/// Workflows, executors and agents are followed as long as they run on the
/// calling task: the workflow engine and `ChatAgent` do; work moved to
/// another task with `tokio::spawn` is not.
///
/// A listener that was dropped is ignored: reporting never fails a run.
pub async fn with_events<F: Future>(
    listener: UnboundedSender<WorkflowStreamEvent>,
    future: F,
) -> F::Output {
    LISTENER.scope(listener, future).await
}

/// The step of a workflow that the current work belongs to.
#[derive(Clone)]
pub(crate) struct Step {
    pub executor_id: ExecutorId,
    pub input_index: usize,
}

tokio::task_local! {
    /// The step being run, where the engine runs one.
    static STEP: Step;
}

/// Run `future` as the work of `step`: what it reports is attributed to it.
pub(crate) async fn in_step<F: Future>(step: Step, future: F) -> F::Output {
    STEP.scope(step, future).await
}

/// The step the current work belongs to, if it runs inside a workflow.
pub(crate) fn current_step() -> Option<Step> {
    STEP.try_with(Clone::clone).ok()
}

/// Whether anyone listens: lets callers skip building a costly event.
pub(crate) fn listening() -> bool {
    LISTENER.try_with(|_| ()).is_ok()
}

/// Report an event to the listener of this run, if any.
pub(crate) fn emit(event: WorkflowStreamEvent) {
    let _ = LISTENER.try_with(|listener| drop(listener.send(event)));
}

/// Report an event that is only built if someone listens.
pub(crate) fn emit_with(event: impl FnOnce() -> WorkflowStreamEvent) {
    let _ = LISTENER.try_with(|listener| drop(listener.send(event())));
}

/// At most `limit` characters of `text`, for values shown rather than used.
pub(crate) fn preview(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}
