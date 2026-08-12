//! Pinned names from the OpenTelemetry GenAI semantic conventions.

/// Upstream semantic-conventions-genai revision reviewed for these constants.
///
/// Keep this value aligned with `docs/develop/observability.md` and the convention
/// compliance tests when updating the development-status upstream specification.
pub const GENAI_SEMCONV_REVISION: &str = "46d43c8949afb53765a202e89f4534eeb75ca3fa";

/// Standard GenAI operation and attribute names used by Colossus.
pub mod attributes {
    /// Name of the operation being performed.
    pub const OPERATION_NAME: &str = "gen_ai.operation.name";
    /// Provider selected by the client instrumentation.
    pub const PROVIDER_NAME: &str = "gen_ai.provider.name";
    /// Model requested by the application.
    pub const REQUEST_MODEL: &str = "gen_ai.request.model";
    /// Model reported by the provider response.
    pub const RESPONSE_MODEL: &str = "gen_ai.response.model";
    /// Provider response identifier.
    pub const RESPONSE_ID: &str = "gen_ai.response.id";
    /// Time from request issuance until the first streaming response chunk, in seconds.
    pub const RESPONSE_TIME_TO_FIRST_CHUNK: &str = "gen_ai.response.time_to_first_chunk";
    /// Number of tokens used in the GenAI input.
    pub const USAGE_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";
    /// Number of tokens used in the GenAI output.
    pub const USAGE_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";
    /// Conversation or thread identifier.
    pub const CONVERSATION_ID: &str = "gen_ai.conversation.id";
    /// Human-readable in-process agent name.
    pub const AGENT_NAME: &str = "gen_ai.agent.name";
    /// Human-readable workflow name.
    pub const WORKFLOW_NAME: &str = "gen_ai.workflow.name";
    /// Tool name.
    pub const TOOL_NAME: &str = "gen_ai.tool.name";
    /// Tool call identifier.
    pub const TOOL_CALL_ID: &str = "gen_ai.tool.call.id";
    /// Tool type.
    pub const TOOL_TYPE: &str = "gen_ai.tool.type";
    /// Low-cardinality terminal error class.
    pub const ERROR_TYPE: &str = "error.type";
}

/// GenAI operation values used by Colossus.
pub mod operations {
    /// Chat-style model inference.
    pub const CHAT: &str = "chat";
    /// In-process agent invocation.
    pub const INVOKE_AGENT: &str = "invoke_agent";
    /// In-process workflow invocation.
    pub const INVOKE_WORKFLOW: &str = "invoke_workflow";
    /// Tool execution.
    pub const EXECUTE_TOOL: &str = "execute_tool";
    /// Distinguishable planning execution.
    pub const PLAN: &str = "plan";
}

/// Standard GenAI metric instrument names.
pub mod instruments {
    /// Input or output token usage histogram.
    pub const CLIENT_TOKEN_USAGE: &str = "gen_ai.client.token.usage";
    /// Provider operation duration histogram.
    pub const CLIENT_OPERATION_DURATION: &str = "gen_ai.client.operation.duration";
    /// Time to the first streaming response chunk.
    pub const CLIENT_TIME_TO_FIRST_CHUNK: &str = "gen_ai.client.operation.time_to_first_chunk";
    /// Time between output chunks.
    pub const CLIENT_TIME_PER_OUTPUT_CHUNK: &str = "gen_ai.client.operation.time_per_output_chunk";
    /// Agent invocation duration.
    pub const INVOKE_AGENT_DURATION: &str = "gen_ai.invoke_agent.duration";
    /// Model calls made by an agent invocation.
    pub const INVOKE_AGENT_INFERENCE_CALLS: &str = "gen_ai.invoke_agent.inference_calls";
    /// Tool calls made by an agent invocation.
    pub const INVOKE_AGENT_TOOL_CALLS: &str = "gen_ai.invoke_agent.tool_calls";
    /// Tool execution duration.
    pub const EXECUTE_TOOL_DURATION: &str = "gen_ai.execute_tool.duration";
    /// Workflow invocation duration.
    pub const INVOKE_WORKFLOW_DURATION: &str = "gen_ai.invoke_workflow.duration";
}

/// Colossus-specific correlation attributes. These are forbidden on metric points.
pub mod colossus_attributes {
    /// Authenticated public application identity.
    pub const APPLICATION_ID: &str = "colossus.application.id";
    /// Durable agent run identifier.
    pub const RUN_ID: &str = "colossus.run.id";
    /// Durable workflow run identifier.
    pub const WORKFLOW_RUN_ID: &str = "colossus.workflow.run.id";
    /// Workflow step identifier.
    pub const WORKFLOW_STEP_ID: &str = "colossus.workflow.step.id";
    /// Durable subagent identifier.
    pub const SUBAGENT_ID: &str = "colossus.subagent.id";
    /// Session message sequence.
    pub const MESSAGE_SEQUENCE: &str = "colossus.message.sequence";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_names_match_reviewed_genai_conventions() {
        assert_eq!(
            GENAI_SEMCONV_REVISION,
            "46d43c8949afb53765a202e89f4534eeb75ca3fa"
        );
        assert_eq!(operations::INVOKE_AGENT, "invoke_agent");
        assert_eq!(operations::INVOKE_WORKFLOW, "invoke_workflow");
        assert_eq!(operations::EXECUTE_TOOL, "execute_tool");
        assert_eq!(
            attributes::RESPONSE_TIME_TO_FIRST_CHUNK,
            "gen_ai.response.time_to_first_chunk"
        );
        assert_eq!(attributes::USAGE_INPUT_TOKENS, "gen_ai.usage.input_tokens");
        assert_eq!(
            attributes::USAGE_OUTPUT_TOKENS,
            "gen_ai.usage.output_tokens"
        );
        assert_eq!(instruments::CLIENT_TOKEN_USAGE, "gen_ai.client.token.usage");
        assert_eq!(
            instruments::INVOKE_AGENT_DURATION,
            "gen_ai.invoke_agent.duration"
        );
        assert_eq!(
            instruments::EXECUTE_TOOL_DURATION,
            "gen_ai.execute_tool.duration"
        );
        assert_eq!(
            instruments::INVOKE_WORKFLOW_DURATION,
            "gen_ai.invoke_workflow.duration"
        );
    }
}
