//! Caller-owned one-attempt generation with durable spend deduplication.
use super::*;
use colossus_api::{NextPromptPrediction, PredictNextPromptRequest};

impl RuntimeAgentRunApi {
    pub(super) async fn next_prompt_prediction(
        &self,
        caller: &CallerContext,
        request: PredictNextPromptRequest,
    ) -> ApiResult<NextPromptPrediction> {
        caller.require_scope(scopes::RUNS_READ)?;
        caller.require_scope(scopes::RUNS_EXECUTE)?;
        if request.source_run_id.is_empty()
            || request.source_run_id.len() > 128
            || request.source_run_id.chars().any(char::is_control)
        {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "source_run_id",
                "a bounded source run identifier is required",
            ));
        }
        // Shared visibility never conveys private context or provider spend authority.
        let source = self
            .repository
            .get_run(caller, &request.source_run_id)?
            .ok_or_else(|| missing_run(caller))?;
        caller.require_role(&source.role)?;
        let empty = || NextPromptPrediction::unavailable(source.id.clone());
        if source.status != RunStatus::Completed || source.archived {
            return Ok(empty());
        }
        let Some(result) = &source.result else {
            return Ok(empty());
        };
        let _permit = self
            .lists
            .acquire(caller.principal().application_id())
            .map_err(|AdmissionLimitReached| capacity_error(caller))?;
        // Runtime shares the durable append-before-effect claim with CLI worker
        // adapters. Remounts and process restarts never repeat a purchased request.
        let mut response = empty();
        if let Ok(prediction) = self
            .runtime
            .predict_next_prompt(
                &source.session_id,
                &source.id,
                &source.role,
                &result.model_profile,
                caller.actor(),
            )
            .await
        {
            response.text = prediction.text;
            response.usage = prediction.usage.map(|usage| TokenUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                total_tokens: usage.total_tokens,
                cached_input_tokens: usage.cached_input_tokens,
                reasoning_tokens: usage.reasoning_tokens,
            });
        }
        Ok(response)
    }
}
