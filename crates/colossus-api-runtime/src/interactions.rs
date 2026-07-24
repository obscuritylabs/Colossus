use crate::writer::RunWriter;
use async_trait::async_trait;
use colossus_api::{
    ApprovalRisk, Interaction, InteractionKind, InteractionResponse, InteractionStatus, RunStatus,
    RunUpdateKind,
};
use colossus_contracts::{
    ApprovalProof, AutomaticApprovalNotice, EffectRequest, PolicyDecision,
    RiskReviewFallbackNotice, UserPromptRequest, UserPromptResponse,
};
use colossus_policy::AllowApproval;
use colossus_ports::{ApprovalProvider, PolicyError, ToolError, UserPromptProvider};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::sync::oneshot;
use uuid::Uuid;

const DEFAULT_INTERACTION_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const PUBLIC_APPROVAL_PROMPT: &str = "An effect requires explicit approval";
const MAX_PUBLIC_ORIGIN_BYTES: usize = 512;

struct ApprovalContext {
    public_binding: String,
    action: String,
    display_resource: String,
    risk: Option<ApprovalRisk>,
}

#[derive(Clone)]
struct ActivePublicRun {
    writer: Arc<RunWriter>,
    pending: Arc<PendingResponses>,
    timeout: Duration,
}

tokio::task_local! {
    static ACTIVE_PUBLIC_RUN: ActivePublicRun;
}

type PendingSender = oneshot::Sender<Result<InteractionResponse, ()>>;

pub(crate) enum PendingResponse {
    Delivered(InteractionResponse),
    Cancelled,
    Elapsed(oneshot::Receiver<Result<InteractionResponse, ()>>),
}

#[derive(Default)]
pub(crate) struct PendingResponses {
    senders: Mutex<BTreeMap<String, (String, PendingSender)>>,
}

impl PendingResponses {
    pub(crate) fn insert(&self, run_id: &str, interaction_id: &str, sender: PendingSender) -> bool {
        lock(&self.senders)
            .insert(interaction_id.into(), (run_id.into(), sender))
            .is_none()
    }

    pub(crate) fn remove(&self, interaction_id: &str) -> Option<PendingSender> {
        lock(&self.senders)
            .remove(interaction_id)
            .map(|(_, sender)| sender)
    }

    fn deliver(&self, run_id: &str, interaction: &Interaction) -> bool {
        let sender = {
            let mut senders = lock(&self.senders);
            let Some((bound_run_id, _)) = senders.get(&interaction.id) else {
                return false;
            };
            if bound_run_id != run_id {
                return false;
            }
            senders.remove(&interaction.id).map(|(_, sender)| sender)
        };
        let Some(sender) = sender else {
            return false;
        };
        interaction
            .response
            .clone()
            .is_some_and(|response| sender.send(Ok(response)).is_ok())
    }

    fn cancel_run(&self, run_id: &str) {
        let cancelled = {
            let mut senders = lock(&self.senders);
            let ids = senders
                .iter()
                .filter_map(|(id, (bound_run_id, _))| {
                    (bound_run_id == run_id).then_some(id.clone())
                })
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| senders.remove(&id).map(|(_, sender)| sender))
                .collect::<Vec<_>>()
        };
        for sender in cancelled {
            let _ = sender.send(Err(()));
        }
    }
}

/// Routes public-run prompts into durable interactions and falls back for private clients.
pub struct PublicInteractionRouter {
    pending: Arc<PendingResponses>,
    fallback_approvals: Arc<dyn ApprovalProvider>,
    fallback_prompts: Option<Arc<dyn UserPromptProvider>>,
    timeout: Duration,
}

impl PublicInteractionRouter {
    /// Compose a router with existing private-interface providers.
    pub fn new(
        fallback_approvals: Arc<dyn ApprovalProvider>,
        fallback_prompts: Option<Arc<dyn UserPromptProvider>>,
    ) -> Self {
        Self {
            pending: Arc::new(PendingResponses::default()),
            fallback_approvals,
            fallback_prompts,
            timeout: DEFAULT_INTERACTION_TIMEOUT,
        }
    }

    pub(super) async fn scope<T>(
        &self,
        writer: Arc<RunWriter>,
        future: impl std::future::Future<Output = T>,
    ) -> T {
        ACTIVE_PUBLIC_RUN
            .scope(
                ActivePublicRun {
                    writer,
                    pending: Arc::clone(&self.pending),
                    timeout: self.timeout,
                },
                future,
            )
            .await
    }

    pub(super) fn deliver(&self, run_id: &str, interaction: &Interaction) -> bool {
        self.pending.deliver(run_id, interaction)
    }

    pub(super) fn cancel_run(&self, run_id: &str) {
        self.pending.cancel_run(run_id);
    }

    #[cfg(test)]
    pub(crate) fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[async_trait]
impl UserPromptProvider for PublicInteractionRouter {
    async fn prompt(&self, request: UserPromptRequest) -> Result<UserPromptResponse, ToolError> {
        let active = match ACTIVE_PUBLIC_RUN.try_with(Clone::clone) {
            Ok(active) => active,
            Err(_) => {
                return match &self.fallback_prompts {
                    Some(provider) => provider.prompt(request).await,
                    None => Err(ToolError::Failed(
                        "no interactive application is attached".into(),
                    )),
                };
            }
        };
        let response = request_interaction(
            &active,
            InteractionKind::Prompt,
            request.question,
            request.choices.clone(),
            request.allow_free_form,
            None,
        )
        .await
        .map_err(|_| ToolError::Failed("the public prompt was not answered".into()))?;
        match response {
            InteractionResponse::Prompt {
                answer,
                selected_index,
            } => Ok(UserPromptResponse {
                answer,
                selected_index: selected_index.and_then(|index| usize::try_from(index).ok()),
            }),
            InteractionResponse::Approval { .. } => Err(ToolError::Failed(
                "the public prompt response type was invalid".into(),
            )),
        }
    }
}

#[async_trait]
impl ApprovalProvider for PublicInteractionRouter {
    fn risk_auto_enabled(&self) -> bool {
        ACTIVE_PUBLIC_RUN
            .try_with(|_| false)
            .unwrap_or_else(|_| self.fallback_approvals.risk_auto_enabled())
    }

    async fn automatic_approval_granted(&self, notice: AutomaticApprovalNotice) {
        self.fallback_approvals
            .automatic_approval_granted(notice)
            .await;
    }

    async fn risk_review_fallback(&self, notice: RiskReviewFallbackNotice) {
        self.fallback_approvals.risk_review_fallback(notice).await;
    }

    async fn request_approval(
        &self,
        request: &EffectRequest,
        request_hash: &str,
        decision: &PolicyDecision,
    ) -> Result<Option<ApprovalProof>, PolicyError> {
        let active = match ACTIVE_PUBLIC_RUN.try_with(Clone::clone) {
            Ok(active) => active,
            Err(_) => {
                return self
                    .fallback_approvals
                    .request_approval(request, request_hash, decision)
                    .await;
            }
        };
        let public_binding = new_public_approval_binding(request_hash).map_err(|_| {
            PolicyError::Unavailable("the public approval binding could not be created".into())
        })?;
        let response = request_interaction(
            &active,
            InteractionKind::Approval,
            PUBLIC_APPROVAL_PROMPT.into(),
            Vec::new(),
            false,
            Some(ApprovalContext {
                public_binding: public_binding.clone(),
                action: public_approval_action(request),
                display_resource: public_approval_resource(request),
                risk: match request.risk.level.as_deref() {
                    Some("low") => Some(ApprovalRisk::Low),
                    Some("medium") => Some(ApprovalRisk::Medium),
                    Some("high") => Some(ApprovalRisk::High),
                    _ => None,
                },
            }),
        )
        .await
        .map_err(|_| PolicyError::Unavailable("the public approval was not answered".into()))?;
        let InteractionResponse::Approval {
            approved,
            request_hash: returned_hash,
        } = response
        else {
            return Err(PolicyError::InvalidDecision(
                "the public approval response type was invalid".into(),
            ));
        };
        if returned_hash != public_binding || !approved {
            return Ok(None);
        }
        ApprovalProvider::request_approval(
            &AllowApproval {
                approved_by: active.writer.caller().principal().application_id().into(),
            },
            request,
            request_hash,
            decision,
        )
        .await
    }
}

async fn request_interaction(
    active: &ActivePublicRun,
    kind: InteractionKind,
    prompt: String,
    choices: Vec<String>,
    allow_free_form: bool,
    approval: Option<ApprovalContext>,
) -> Result<InteractionResponse, ()> {
    let interaction_id = Uuid::now_v7().to_string();
    let now = OffsetDateTime::now_utc();
    let created_at = now.format(&Rfc3339).map_err(|_| ())?;
    let expires_at = (now + time::Duration::try_from(active.timeout).map_err(|_| ())?)
        .format(&Rfc3339)
        .map_err(|_| ())?;
    let interaction = Interaction {
        id: interaction_id.clone(),
        kind,
        status: InteractionStatus::Pending,
        application_id: active.writer.caller().principal().application_id().into(),
        created_at,
        prompt,
        choices,
        allow_free_form,
        request_hash: approval
            .as_ref()
            .map(|details| details.public_binding.clone()),
        action: approval.as_ref().map(|details| details.action.clone()),
        resource: approval
            .as_ref()
            .map(|details| details.display_resource.clone()),
        risk: approval.and_then(|details| details.risk),
        expires_at,
        response: None,
        responded_at: None,
    };
    let (sender, receiver) = oneshot::channel();
    if !active
        .pending
        .insert(active.writer.run_id(), &interaction_id, sender)
    {
        return Err(());
    }
    if active
        .writer
        .append(RunUpdateKind::Interaction {
            interaction: interaction.clone(),
        })
        .is_err()
    {
        active.pending.remove(&interaction_id);
        return Err(());
    }
    let response = match await_pending_response(receiver, active.timeout).await {
        PendingResponse::Delivered(response) => response,
        PendingResponse::Cancelled => return Err(()),
        PendingResponse::Elapsed(receiver) => {
            let mut expired = interaction;
            expired.status = InteractionStatus::Expired;
            let append = active.writer.append(RunUpdateKind::Interaction {
                interaction: expired,
            });
            match append {
                Ok(_) => {
                    // The durable expiry won the writer's sequence mutex. Only now
                    // remove the sender, preventing a later response from being
                    // delivered to this runtime turn.
                    active.pending.remove(&interaction_id);
                    return Err(());
                }
                Err(error)
                    if matches!(
                        error.reason,
                        colossus_api::ApiErrorReason::ConcurrentModification
                            | colossus_api::ApiErrorReason::InvalidRunTransition
                    ) =>
                {
                    // A response or terminal cancellation held the same writer mutex
                    // first and changed the durable run. Its sender may be between
                    // releasing the writer and completing delivery, so honor it.
                    match receiver.await {
                        Ok(Ok(response)) => response,
                        Ok(Err(())) | Err(_) => return Err(()),
                    }
                }
                Err(_) => {
                    // Storage/capacity failures are not evidence that delivery won.
                    // Close this waiter rather than hanging past its deadline.
                    active.pending.remove(&interaction_id);
                    return Err(());
                }
            }
        }
    };
    let run = active.writer.current_run().map_err(|_| ())?.ok_or(())?;
    if run.status == RunStatus::Waiting {
        active
            .writer
            .append(RunUpdateKind::State {
                status: RunStatus::Running,
            })
            .map_err(|_| ())?;
    }
    Ok(response)
}

async fn await_pending_response(
    mut receiver: oneshot::Receiver<Result<InteractionResponse, ()>>,
    timeout: Duration,
) -> PendingResponse {
    match tokio::time::timeout(timeout, &mut receiver).await {
        Ok(Ok(Ok(response))) => PendingResponse::Delivered(response),
        Ok(Ok(Err(()))) | Ok(Err(_)) => PendingResponse::Cancelled,
        Err(_) => PendingResponse::Elapsed(receiver),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn new_public_approval_binding(private_request_hash: &str) -> Result<String, getrandom::Error> {
    loop {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes)?;
        let binding = lowercase_hex(&bytes);
        if binding != private_request_hash {
            return Ok(binding);
        }
    }
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn public_approval_resource(request: &EffectRequest) -> String {
    if let Ok(url) = url::Url::parse(&request.resource)
        && matches!(url.scheme(), "http" | "https")
    {
        let origin = url.origin().ascii_serialization();
        if origin != "null"
            && !origin.is_empty()
            && origin.len() <= MAX_PUBLIC_ORIGIN_BYTES
            && !origin.chars().any(char::is_control)
        {
            return origin;
        }
    }

    let action = request.action.as_str();
    if action.starts_with("filesystem.")
        || action.starts_with("patch.")
        || action.starts_with("git.")
        || action.starts_with("repo.")
        || action.starts_with("repository.")
    {
        "workspace resource".into()
    } else if action.starts_with("process.") || action.starts_with("shell.") {
        "configured executable".into()
    } else if action.starts_with("provider.") {
        "configured model provider".into()
    } else if action.starts_with("network.")
        || action.starts_with("web.")
        || action.starts_with("search.")
    {
        "configured network destination".into()
    } else if action.starts_with("mcp.")
        || action.starts_with("integration.")
        || action.starts_with("openapi.")
    {
        "configured integration".into()
    } else if action.starts_with("task.")
        || action.starts_with("decision.")
        || action.starts_with("plan.")
        || action.starts_with("goal.")
        || action.starts_with("session.")
        || action.starts_with("memory.")
    {
        "Colossus record".into()
    } else {
        "protected resource".into()
    }
}

fn public_approval_action(request: &EffectRequest) -> String {
    let action = request.action.as_str();
    if action.starts_with("filesystem.")
        || action.starts_with("patch.")
        || action.starts_with("git.")
        || action.starts_with("repo.")
        || action.starts_with("repository.")
    {
        "workspace.modify".into()
    } else if action.starts_with("process.") || action.starts_with("shell.") {
        "process.execute".into()
    } else if action.starts_with("provider.") {
        "model.invoke".into()
    } else if action.starts_with("network.")
        || action.starts_with("web.")
        || action.starts_with("search.")
    {
        "network.access".into()
    } else if action.starts_with("mcp.")
        || action.starts_with("integration.")
        || action.starts_with("openapi.")
    {
        "integration.invoke".into()
    } else if action.starts_with("task.")
        || action.starts_with("decision.")
        || action.starts_with("plan.")
        || action.starts_with("goal.")
        || action.starts_with("session.")
        || action.starts_with("memory.")
    {
        "colossus.record".into()
    } else {
        "protected.effect".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::{Actor, ActorType};
    use colossus_policy::effect_request;
    use serde_json::json;

    fn request(action: &str, resource: &str) -> EffectRequest {
        effect_request(
            Actor {
                actor_type: ActorType::Application,
                id: "app:approval-test".into(),
            },
            action,
            resource,
            json!({}),
        )
    }

    #[test]
    fn approval_display_never_releases_absolute_paths_or_executables() {
        let private_path = "/Users/alex/private/customer-secret.txt";
        let display = public_approval_resource(&request("filesystem.write", private_path));
        assert_eq!(display, "workspace resource");
        assert!(!display.contains("alex"));
        assert!(!display.contains("customer-secret"));

        let executable = r"C:\Users\alex\private\secret-tool.exe";
        let display = public_approval_resource(&request("process.run", executable));
        assert_eq!(display, "configured executable");
        assert!(!display.contains("secret-tool"));
    }

    #[test]
    fn approval_display_releases_only_network_origin() {
        let private_url =
            "https://user:password@example.com/private/path?signature=super-secret#fragment";
        let display = public_approval_resource(&request("network.http", private_url));
        assert_eq!(display, "https://example.com");
        for private in [
            "user",
            "password",
            "private",
            "signature",
            "super-secret",
            "fragment",
        ] {
            assert!(!display.contains(private), "{private}");
        }
    }

    #[test]
    fn approval_display_falls_back_to_an_opaque_label_for_secret_like_resources() {
        let private = "Bearer sk-super-secret-private-resource";
        let display = public_approval_resource(&request("custom.effect", private));
        assert_eq!(display, "protected resource");
        assert!(!display.contains(private));
        assert_eq!(
            PUBLIC_APPROVAL_PROMPT,
            "An effect requires explicit approval"
        );
    }

    #[test]
    fn approval_action_is_a_fixed_public_category() {
        let private_action = "custom.provider.sk-super-secret.internal-operation";
        let display = public_approval_action(&request(private_action, "private-resource"));
        assert_eq!(display, "protected.effect");
        assert!(!display.contains("secret"));
        assert!(!display.contains("internal"));

        assert_eq!(
            public_approval_action(&request("filesystem.write", "/private/path")),
            "workspace.modify"
        );
        assert_eq!(
            public_approval_action(&request("network.http", "https://example.com/private")),
            "network.access"
        );
    }

    #[test]
    fn approval_binding_is_randomized_and_not_the_private_request_hash() {
        let private_request_hash =
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let first = new_public_approval_binding(private_request_hash).expect("first binding");
        let second = new_public_approval_binding(private_request_hash).expect("second binding");

        for binding in [&first, &second] {
            assert_eq!(binding.len(), 64);
            assert!(
                binding
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            );
            assert_ne!(binding, private_request_hash);
        }
        assert_ne!(first, second);
    }
}
