use super::*;
use colossus_contracts::ImageViewOutput;
use colossus_media::{MAX_COMBINED_IMAGE_BYTES, import_released_images};

impl GatewayToolExecutor {
    pub(super) async fn execute_image_view(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let owners = self
            .process_sessions
            .as_ref()
            .ok_or_else(|| ToolError::Denied("active run ownership is required".into()))?;
        let mut owner = owners.active_run_owner(&context)?;
        if owner.actor_type == ActorType::User {
            owner = Actor {
                actor_type: ActorType::Application,
                id: "app:colossus-cli".into(),
            };
        } else if owner.actor_type != ActorType::Application {
            return Err(ToolError::Denied(
                "image artifact ownership is unavailable".into(),
            ));
        }
        let paths = call
            .arguments
            .get("paths")
            .and_then(Value::as_array)
            .filter(|paths| !paths.is_empty() && paths.len() <= 16)
            .ok_or_else(|| {
                ToolError::Failed("image.view requires between 1 and 16 paths".into())
            })?;
        let mut released = Vec::with_capacity(paths.len());
        if context
            .image_input_budget
            .as_ref()
            .is_some_and(|budget| paths.len() > budget.remaining_count)
        {
            return Err(ToolError::Failed(
                "image view exceeds the remaining 16-image context limit".into(),
            ));
        }
        let byte_limit = context
            .image_input_budget
            .as_ref()
            .map_or(MAX_COMBINED_IMAGE_BYTES, |budget| {
                budget.remaining_bytes.min(MAX_COMBINED_IMAGE_BYTES)
            });
        let mut combined = 0_u64;
        let ambient = self.danger_full_access(&context);
        for path in paths {
            let path = self.model_read_path(
                path.as_str()
                    .ok_or_else(|| ToolError::Failed("image path must be text".into()))?,
                &context,
                ambient,
            )?;
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| ToolError::Failed("image file name is invalid".into()))?
                .to_owned();
            let mut request = effect_request(
                model_actor(&call, &context),
                "filesystem.read_image",
                path.display().to_string(),
                json!({"path": path, "encoding": "binary"}),
            );
            request.capabilities = vec!["filesystem.read_image".into()];
            request.context = context.clone();
            let result = self
                .gateway
                .execute(request, self.filesystem.as_ref())
                .await
                .map_err(tool_gateway_error)?;
            combined = combined
                .checked_add(u64::try_from(result.bytes.len()).unwrap_or(u64::MAX))
                .filter(|size| *size <= byte_limit)
                .ok_or_else(|| ToolError::Failed("viewed images exceed the 32 MiB bound".into()))?;
            released.push((file_name, result.bytes));
        }
        // Decoding and journal writes must not block the async runtime. Retain nothing
        // until every requested file has passed the read/release and image bounds.
        let journal = owners.journal().clone();
        let output = tokio::task::spawn_blocking(move || {
            let images = import_released_images(journal.as_ref(), owner, context, released)
                .map_err(|_| ToolError::Failed("image validation or storage failed; use PNG, JPEG, or static WebP within the image limits".into()))?;
            serde_json::to_string(&ImageViewOutput { images })
                .map_err(|_| ToolError::Failed("viewed image metadata could not be encoded".into()))
        })
        .await
        .map_err(|_| ToolError::Failed("image validation did not complete".into()))??;
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output,
            exit_code: 0,
            images: Vec::new(),
        })
    }
}
