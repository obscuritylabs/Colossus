//! Dependency-free domain vocabulary for Colossus.

use std::{fmt, str::FromStr};

/// The provenance class for an actor requesting work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActorType {
    /// A human operator.
    User,
    /// A model-controlled agent.
    Model,
    /// A durable workflow.
    Workflow,
    /// A delegated child agent.
    Subagent,
    /// A trusted internal service.
    System,
}

/// The event's security and product classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventClassification {
    /// A domain lifecycle event.
    Domain,
    /// An effect lifecycle event.
    Effect,
    /// A policy decision event.
    Policy,
    /// An approval event.
    Approval,
    /// A workflow lifecycle event.
    Workflow,
    /// A trusted runtime event.
    System,
}

/// Whether authorization is happening before execution or before content release.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EffectPhase {
    /// Authorization before the adapter executes.
    PreEffect,
    /// Authorization before quarantined output is released.
    PostEffect,
}

/// A strict policy outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionOutcome {
    /// Execution or release is allowed with the returned obligations.
    Allow,
    /// Execution or release is denied.
    Deny,
    /// A human approval proof is required before re-evaluation.
    RequireApproval,
}

/// Availability of optional risk-model input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RiskStatus {
    /// The risk model returned a usable assessment.
    Available,
    /// No usable risk result exists.
    Unavailable,
}

/// Durable workflow run states.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkflowStatus {
    /// Accepted but not yet started.
    Queued,
    /// Actively executing.
    Running,
    /// Waiting for input or approval.
    Waiting,
    /// Finished successfully.
    Completed,
    /// Finished unsuccessfully.
    Failed,
    /// Cancelled by an actor.
    Cancelled,
    /// Abandoned by a stopped worker or process.
    Interrupted,
}

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                let value = match self { $(Self::$variant => $wire),+ };
                formatter.write_str(value)
            }
        }

        impl FromStr for $name {
            type Err = &'static str;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($wire => Ok(Self::$variant),)+
                    _ => Err(concat!("unknown ", stringify!($name))),
                }
            }
        }
    };
}

string_enum!(ActorType {
    User => "user",
    Model => "model",
    Workflow => "workflow",
    Subagent => "subagent",
    System => "system",
});
string_enum!(EventClassification {
    Domain => "domain",
    Effect => "effect",
    Policy => "policy",
    Approval => "approval",
    Workflow => "workflow",
    System => "system",
});
string_enum!(EffectPhase {
    PreEffect => "pre_effect",
    PostEffect => "post_effect",
});
string_enum!(DecisionOutcome {
    Allow => "allow",
    Deny => "deny",
    RequireApproval => "require_approval",
});
string_enum!(RiskStatus {
    Available => "available",
    Unavailable => "unavailable",
});
string_enum!(WorkflowStatus {
    Queued => "queued",
    Running => "running",
    Waiting => "waiting",
    Completed => "completed",
    Failed => "failed",
    Cancelled => "cancelled",
    Interrupted => "interrupted",
});

#[cfg(test)]
mod tests;
