use super::{arguments::BrowserInvocation, effect::BrowserEffect, service::RuntimeBrowserTools};
use crate::prelude::*;

pub(crate) struct BrowserToolExecutor {
    pub(crate) gateway: Arc<EffectGateway>,
    pub(crate) registry: Arc<dyn ToolRegistry>,
    pub(crate) browser: Arc<RuntimeBrowserTools>,
    pub(crate) journal: Arc<dyn EventJournal>,
    pub(crate) inner: Arc<dyn ToolExecutor>,
}

#[async_trait]
impl ToolExecutor for BrowserToolExecutor {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        if !call.name.starts_with("browser.") {
            return self.inner.execute(call, context).await;
        }
        self.registry.validate(&call)?;
        if !context.offered_tools.contains(&call.name) {
            return Err(ToolError::Denied(
                "browser operation exceeds the active tool ceiling".into(),
            ));
        }
        self.browser.run(&context)?;
        let invocation = BrowserInvocation::from_call(&call)?;
        if matches!(
            call.name.as_str(),
            "browser.screenshot" | "browser.upload" | "browser.download"
        ) && self
            .browser
            .artifacts
            .lock()
            .map_err(|_| ToolError::Failed("browser artifact publisher unavailable".into()))?
            .is_none()
        {
            return Err(ToolError::Failed(
                "browser artifact publisher unavailable".into(),
            ));
        }
        let screenshot = StdMutex::new(None);
        let download = StdMutex::new(None);
        let upload = super::transfer::prepare_upload(
            &self.browser,
            Arc::clone(&self.journal),
            &invocation,
            &context,
        )
        .await?;
        let content = super::transfer::policy_content(&invocation, upload.as_ref())?;
        let upload = StdMutex::new(upload);
        let mut request = effect_request(
            crate::model_actor(&call, &context),
            invocation.action(),
            invocation.resource(),
            content,
        );
        request.capabilities = vec![invocation.action().into()];
        request.context = context;
        let release_context = request.context.clone();
        let released = self
            .gateway
            .execute(
                request,
                &BrowserEffect {
                    browser: &self.browser,
                    screenshot: &screenshot,
                    upload: &upload,
                    download: &download,
                },
            )
            .await
            .map_err(crate::tool_gateway_error)?;
        if call.name == "browser.screenshot" {
            let receipt = screenshot
                .into_inner()
                .map_err(|_| ToolError::Failed("browser capture receipt unavailable".into()))?
                .ok_or_else(|| ToolError::Failed("browser capture receipt unavailable".into()))?;
            return super::screenshot::publish(
                self,
                call,
                &release_context,
                receipt,
                released.bytes,
            )
            .await;
        }
        if call.name == "browser.download" {
            let receipt = download
                .into_inner()
                .map_err(|_| super::transfer::unavailable())?
                .ok_or_else(super::transfer::unavailable)?;
            return super::download::publish(self, call, &release_context, receipt, released.bytes)
                .await;
        }
        let output = String::from_utf8(released.bytes)
            .map_err(|_| ToolError::Failed("browser returned invalid released output".into()))?;
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output,
            exit_code: 0,
            images: Vec::new(),
        })
    }
}
