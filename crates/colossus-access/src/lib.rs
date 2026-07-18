//! Metadata-driven tool exposure and built-in policy profile resolution.

mod config;
mod descriptor;
mod diagnostics;
mod registry;
mod resolver;

pub use config::{
    AccessConfig, AccessError, AccessProfile, ActionAccessConfig, ToolAccessConfig, validate_config,
};
pub use descriptor::{
    AccessContext, ActionClass, ActionDescriptor, CapabilitySource, ToolDescriptor,
    ToolPrerequisite,
};
pub use diagnostics::{
    AccessDecision, AccessResolution, ResolvedAction, ResolvedTool, ToolAvailability,
};
pub use registry::{builtin_action_descriptors, builtin_tool_descriptor};
pub use resolver::resolve_access;

#[cfg(test)]
mod tests;
