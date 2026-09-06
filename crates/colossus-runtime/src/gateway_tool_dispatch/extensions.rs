use super::*;

impl GatewayToolExecutor {
    pub(super) async fn execute_extensions(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let mut exit_code = 0;
        let output = match call.name.as_str() {
            "plugin.list" => {
                self.execute_plugin_tool(&call, context, PluginOperation::List)
                    .await?
            }
            "plugin.inspect" => {
                self.execute_plugin_tool(
                    &call,
                    context,
                    PluginOperation::Inspect {
                        plugin_name: required_tool_string(&call, "plugin")?.into(),
                    },
                )
                .await?
            }
            "plugin.skill.read" => {
                self.execute_plugin_tool(
                    &call,
                    context,
                    PluginOperation::SkillRead {
                        skill_id: required_tool_string(&call, "skill")?.into(),
                    },
                )
                .await?
            }
            "plugin.resource.list" => {
                self.execute_plugin_tool(
                    &call,
                    context,
                    PluginOperation::ListResources {
                        skill_id: required_tool_string(&call, "skill")?.into(),
                    },
                )
                .await?
            }
            "plugin.resource.read" => {
                self.execute_plugin_tool(
                    &call,
                    context,
                    PluginOperation::ReadResource {
                        skill_id: required_tool_string(&call, "skill")?.into(),
                        path: required_tool_string(&call, "path")?.into(),
                    },
                )
                .await?
            }
            "mcp.servers" => {
                let catalog = active_plugin_catalog();
                let snapshot_mcp = catalog
                    .as_ref()
                    .and_then(|catalog| catalog.mcp.as_ref())
                    .or(self.mcp.as_ref());
                let servers = snapshot_mcp
                    .ok_or_else(|| ToolError::Failed("MCP adapter is unavailable".into()))?
                    .servers();
                serde_json::to_string(&servers)
                    .map_err(|error| ToolError::Failed(error.to_string()))?
            }
            "mcp.tools" => {
                self.discover_mcp_tool_output(
                    &call,
                    context,
                    optional_tool_string(&call, "server")?,
                )
                .await?
            }
            "mcp.call" => {
                let server = required_tool_string(&call, "server")?.to_owned();
                let tool = required_tool_string(&call, "tool")?.to_owned();
                let arguments = call.arguments.get("arguments").cloned().ok_or_else(|| {
                    ToolError::InvalidArguments {
                        tool: call.name.clone(),
                        message: "arguments must be an object".into(),
                    }
                })?;
                let result = self
                    .execute_mcp_tool(&call, context, &server, &tool, arguments)
                    .await?;
                exit_code = result.exit_code;
                result.output
            }
            name => return Err(ToolError::Unknown(name.into())),
        };
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output,
            exit_code,
        })
    }
}
