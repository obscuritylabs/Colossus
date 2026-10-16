//! Immutable, task-scoped preferences for user-facing agent invocations.

use colossus_contracts::WorkflowAgentOptions;
use std::future::Future;

tokio::task_local! {
    static MODEL_SELECTION: WorkflowAgentOptions;
}

/// Apply configured model preferences to root agent invocations in this future.
///
/// The scope follows sequential Goal iterations, but is never inherited by spawned
/// tasks. Workflows and delegated children retain their own explicit routes. This
/// changes no role, tool ceiling, policy, credential, or execution authority.
pub fn scope_model_selection<F: Future>(
    options: WorkflowAgentOptions,
    future: F,
) -> impl Future<Output = F::Output> {
    MODEL_SELECTION.scope(options, future)
}

pub(super) fn current_model_selection() -> Option<WorkflowAgentOptions> {
    MODEL_SELECTION.try_with(Clone::clone).ok()
}
