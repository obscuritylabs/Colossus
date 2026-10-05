//! Released workflow structure and execution facts, excluding effect payloads.
use super::*;

/// A bounded read-only projection of an exact registered definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowLogic {
    /// Ordered main steps, including nested control flow.
    pub steps: Vec<WorkflowLogicStep>,
    /// Recovery steps, shown separately from the successful path.
    pub compensation: Vec<WorkflowLogicStep>,
}

impl WorkflowLogic {
    /// Bound untrusted remote projections before rendering or recursive translation.
    pub fn within_bounds(&self) -> bool {
        fn bounded(
            steps: &[WorkflowLogicStep],
            depth: usize,
            count: &mut usize,
            ids: &mut std::collections::BTreeSet<String>,
        ) -> bool {
            // Each nested branch adds four JSON levels. Keep the projection
            // within the existing transport's 32-level object bound.
            if depth > 7 {
                return false;
            }
            for step in steps {
                *count += 1;
                if *count > 512
                    || step.id.is_empty()
                    || step.id.len() > 128
                    || step.id.chars().any(char::is_control)
                    || !ids.insert(step.id.clone())
                    || step.summary.len() > 4096
                    || step.branches.len() > 512
                    || match step.kind {
                        WorkflowLogicKind::Condition => {
                            step.branches.len() != 2
                                || step.branches[0].label != "True"
                                || step.branches[1].label != "False"
                        }
                        WorkflowLogicKind::Parallel => {
                            step.branches.is_empty()
                                || step.branches.iter().enumerate().any(|(index, branch)| {
                                    branch.label != format!("Branch {}", index + 1)
                                        || branch.steps.is_empty()
                                })
                        }
                        WorkflowLogicKind::Foreach => {
                            step.branches.len() != 1
                                || step.branches[0].label != "Each item"
                                || step.branches[0].steps.is_empty()
                        }
                        _ => !step.branches.is_empty(),
                    }
                    || step.branches.iter().any(|branch| {
                        branch.label.len() > 128 || !bounded(&branch.steps, depth + 1, count, ids)
                    })
                {
                    return false;
                }
            }
            true
        }
        let mut count = 0;
        let mut ids = std::collections::BTreeSet::new();
        !self.steps.is_empty()
            && bounded(&self.steps, 0, &mut count, &mut ids)
            && bounded(&self.compensation, 0, &mut count, &mut ids)
            && serde_json::to_vec(self).is_ok_and(|value| value.len() <= 256 * 1024)
    }
}

/// One structural step with no prompt, tool arguments, inputs, or emitted value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowLogicStep {
    /// Definition-local stable identity.
    pub id: String,
    /// Declarative step category.
    pub kind: WorkflowLogicKind,
    /// Released condition, bound, tool name, or child definition reference.
    pub summary: String,
    /// Labeled nested sequences; empty for leaf steps.
    pub branches: Vec<WorkflowLogicBranch>,
}

/// A labeled conditional, concurrent, or iterative sequence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowLogicBranch {
    /// Structural label, such as True, False, or Each item.
    pub label: String,
    /// Steps executed in order within this branch.
    pub steps: Vec<WorkflowLogicStep>,
}

/// Closed set of supported declarative step categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowLogicKind {
    /// Model-backed work, with the prompt withheld.
    Agent,
    /// Registered tool, with its arguments withheld.
    Tool,
    /// Exact child workflow reference.
    Workflow,
    /// Operator approval gate.
    Approval,
    /// Restricted expression with two paths.
    Condition,
    /// Bounded concurrent branches.
    Parallel,
    /// Bounded iteration over an input array.
    Foreach,
    /// Structured operator input gate.
    WaitForInput,
    /// Pure output, with its value withheld.
    Emit,
}

/// Aggregate recorded executions for a definition-local step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowStepState {
    /// Exact definition step identity; never an output or filesystem path.
    pub step_id: String,
    /// Most urgent recorded execution state, without inferring unvisited branches.
    pub status: WorkflowStepStatus,
    /// Distinct completed execution identities, including bounded loop iterations.
    pub completed_executions: u32,
}

/// Released recorded execution state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepStatus {
    /// Execution has started.
    Running,
    /// An operator or dependency is needed.
    Waiting,
    /// Recorded executions completed.
    Completed,
    /// An explicit failure was recorded.
    Failed,
    /// Execution stopped without completion evidence.
    Interrupted,
    /// An unfinished execution was cancelled with the run.
    Cancelled,
}
