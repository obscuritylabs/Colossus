use super::*;

/// Active model-visible tool catalog with strict schema validation.
pub trait ToolRegistry: Send + Sync {
    /// Stable sorted active specifications.
    fn list_specs(&self) -> Vec<ToolSpec>;

    /// Resolve and validate one call before policy evaluation.
    fn validate(&self, call: &ToolCall) -> Result<ToolSpec, ToolError>;
}

/// Execute a previously validated tool call.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute with full run/session provenance.
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError>;
}

/// Optional interactive interface used only when a surface can safely ask the user.
#[async_trait]
pub trait UserPromptProvider: Send + Sync {
    /// Present one bounded question and return the user's bounded answer.
    async fn prompt(&self, request: UserPromptRequest) -> Result<UserPromptResponse, ToolError>;
}
