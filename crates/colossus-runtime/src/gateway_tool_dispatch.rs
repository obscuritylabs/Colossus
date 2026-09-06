use super::*;

mod extensions;
mod filesystem;
mod network;
mod process;
mod repository;
mod work;

// Keep each tool family in its own async state machine. Polling one tool must
// not reserve stack for every unrelated filesystem, MCP, and workflow path.
#[async_trait]
impl ToolExecutor for GatewayToolExecutor {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        match call.name.as_str() {
            "echo" | "filesystem.list" | "filesystem.read" | "filesystem.search"
            | "filesystem.write" | "filesystem.replace" => {
                Box::pin(self.execute_filesystem(call, context)).await
            }
            "git.status" | "git.diff" | "git.show" | "repo.map" | "repo.symbol_search"
            | "repo.references" | "repo.file_summary" | "patch.preview" | "patch.apply"
            | "patch.reverse" => Box::pin(self.execute_repository(call, context)).await,
            "shell.run" => Box::pin(self.execute_process(call, context)).await,
            "task.create"
            | "task.update"
            | "task.list"
            | "decision.create"
            | "decision.update"
            | "decision.list"
            | "decision.archive"
            | "decision.supersede"
            | "agent.delegate"
            | "agent.result"
            | "agent.list"
            | "goal.show"
            | "goal.update"
            | "plan.create"
            | "plan.update"
            | "plan.show"
            | "plan.approve_request"
            | "memory.create"
            | "memory.update"
            | "memory.list"
            | "memory.search"
            | "memory.archive"
            | "memory.supersede" => Box::pin(self.execute_work(call, context)).await,
            "plugin.list"
            | "plugin.inspect"
            | "plugin.skill.read"
            | "plugin.resource.list"
            | "plugin.resource.read"
            | "mcp.servers"
            | "mcp.tools"
            | "mcp.call" => Box::pin(self.execute_extensions(call, context)).await,
            "web.search" | "network.http" | "web.fetch" | "docs.fetch" => {
                Box::pin(self.execute_network(call, context)).await
            }
            _ => {
                let output = self
                    .execute_integration_tool(&call, context)
                    .await?
                    .ok_or_else(|| ToolError::Unknown(call.name.clone()))?;
                Ok(ToolResult {
                    call_id: call.call_id,
                    name: call.name,
                    output,
                    exit_code: 0,
                })
            }
        }
    }
}
