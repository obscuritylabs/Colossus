//! Canonical operator model-card settings for the local terminal.

use super::*;
use colossus_contracts::{
    ModelFeatureDeclarations, ModelFeatureMode, ModelFeatureSettings, ReasoningEffort,
    SavedTerminalModelRef, TerminalModelCard, TerminalModelCardUpdate, TerminalModelOverrides,
    TerminalModelTarget,
};
use colossus_provider::ResolvedModel;
use serde::{Deserialize, Serialize};
use std::future::Future;

const CARDS_STREAM: &str = "terminal:model-cards-v1";
const CARDS_SAVED: &str = "terminal.model_cards.saved.v1";
const MAX_CARDS: usize = 256;

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TerminalModelCardIndex {
    cards: Vec<StoredCard>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredCard {
    target: TerminalModelTarget,
    binding: String,
    revision: u64,
    overrides: TerminalModelOverrides,
    advertised: Option<ProviderModelInfo>,
}

struct CardConnection {
    binding: String,
    provider_profile: String,
    kind: ProviderKind,
    model: String,
    configured: Option<ModelProfile>,
}

impl TerminalModelCardIndex {
    fn card(&self, target: &TerminalModelTarget) -> Option<&StoredCard> {
        self.cards.iter().find(|card| card.target == *target)
    }

    fn bound(
        &self,
        target: &TerminalModelTarget,
        connection: &CardConnection,
    ) -> Option<&StoredCard> {
        self.card(target).filter(|card| card.applies_to(connection))
    }
}

impl StoredCard {
    fn applies_to(&self, connection: &CardConnection) -> bool {
        if self.binding != connection.binding {
            return false;
        }
        // A configured model can keep its identity while its current limits shrink.
        // Incompatible older edits stay in the journal and can be replaced from the
        // current defaults without hiding the other saved cards.
        match &connection.configured {
            Some(defaults) => {
                apply_card_overrides(connection.kind, defaults, &self.overrides, None).is_ok()
            }
            None => self.advertised.as_ref().is_some_and(|advertised| {
                discovered_defaults(connection, advertised)
                    .and_then(|defaults| {
                        apply_card_overrides(connection.kind, &defaults, &self.overrides, None)
                    })
                    .is_ok()
            }),
        }
    }
}

impl Runtime {
    /// List the saved cards whose exact model and configured connection still match.
    pub fn terminal_model_cards(&self) -> Result<Vec<TerminalModelCard>, RuntimeError> {
        let (_, index) = self.load_terminal_model_cards()?;
        let mut cards = Vec::new();
        for stored in &index.cards {
            let Some(connection) = self.terminal_card_connection(&stored.target)? else {
                continue;
            };
            if stored.applies_to(&connection) {
                cards.push(
                    self.build_terminal_card(
                        "primary",
                        &stored.target,
                        &connection,
                        Some(stored),
                        None,
                    )?
                    .0,
                );
            }
        }
        Ok(cards)
    }

    /// Open an editable card from configured defaults, an audited catalog, or saved state.
    pub fn terminal_model_card(
        &self,
        target: &TerminalModelTarget,
    ) -> Result<TerminalModelCard, RuntimeError> {
        let (_, index) = self.load_terminal_model_cards()?;
        let connection = self.require_terminal_card_connection(target)?;
        let stored = index.bound(target, &connection);
        Ok(self
            .build_terminal_card("primary", target, &connection, stored, None)?
            .0)
    }

    /// Replace one card at its observed revision without creating a conversation.
    /// The local operator channel authorizes this metadata change. Actual model
    /// execution still resolves the configured connection and requires normal policy.
    pub async fn save_terminal_model_card(
        &self,
        session_id: &str,
        update: TerminalModelCardUpdate,
        control: &RunControl,
    ) -> Result<TerminalModelCard, RuntimeError> {
        validate_card_session(session_id)?;
        let (version, mut index) = self.load_terminal_model_cards()?;
        let connection = self.require_terminal_card_connection(&update.target)?;
        if update.expected_binding != connection.binding {
            return Err(RuntimeError::Config(
                "The model or its configured connection changed. Reopen the card and try again."
                    .into(),
            ));
        }
        let current = index.bound(&update.target, &connection);
        let actual = current.map_or(0, |card| card.revision);
        if actual != update.expected_revision {
            return Err(StoreError::Conflict {
                stream_id: CARDS_STREAM.into(),
                expected: update.expected_revision,
                actual,
            }
            .into());
        }
        let revision = version
            .checked_add(1)
            .ok_or_else(|| StoreError::Adapter("terminal model card revision exhausted".into()))?;
        let mut staged = StoredCard {
            target: update.target,
            binding: connection.binding.clone(),
            revision,
            overrides: update.overrides,
            advertised: current.and_then(|card| card.advertised.clone()),
        };
        let (card, _) =
            self.build_terminal_card("primary", &staged.target, &connection, Some(&staged), None)?;
        staged.advertised = card.advertised.clone();
        index
            .cards
            .retain(|existing| existing.target != staged.target);
        if index.cards.len() == MAX_CARDS {
            // At capacity, reclaim cards whose configured connection has disappeared
            // or changed. Keep edits that can still be repaired under the same binding.
            let mut retained = Vec::with_capacity(MAX_CARDS);
            for existing in index.cards {
                if self
                    .terminal_card_connection(&existing.target)?
                    .is_some_and(|current| current.binding == existing.binding)
                {
                    retained.push(existing);
                }
            }
            index.cards = retained;
            if index.cards.len() == MAX_CARDS {
                return Err(RuntimeError::Config(
                    "The terminal model card limit has been reached.".into(),
                ));
            }
        }
        index.cards.push(staged);
        index
            .cards
            .sort_by(|left, right| left.target.cmp(&right.target));
        if control.is_cancelled() {
            return Err(RuntimeError::Config(
                "Model settings save was cancelled.".into(),
            ));
        }
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: CARDS_STREAM.into(),
            expected_stream_version: version,
            classification: EventClassification::Domain,
            event_type: CARDS_SAVED.into(),
            actor: terminal_actor(),
            context: ExecutionContext {
                correlation_id: CARDS_STREAM.into(),
                session_id: Some(session_id.into()),
                ..ExecutionContext::default()
            },
            payload: serde_json::to_value(index)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        })?;
        Ok(card)
    }

    /// Resolve an explicit terminal choice without loading a provider catalog.
    pub fn saved_terminal_model_route(
        &self,
        role: &str,
        model: &SavedTerminalModelRef,
        reasoning_effort: Option<ReasoningEffort>,
    ) -> Result<ModelRoute, RuntimeError> {
        self.resolve_saved_terminal_model(role, model, reasoning_effort)
            .map(|model| model.route())
    }

    /// Snapshot the saved route for one root terminal operation.
    pub fn with_saved_terminal_model<F: Future>(
        &self,
        role: &str,
        model: &SavedTerminalModelRef,
        reasoning_effort: Option<ReasoningEffort>,
        future: F,
    ) -> Result<impl Future<Output = F::Output>, RuntimeError> {
        let model = self.resolve_saved_terminal_model(role, model, reasoning_effort)?;
        Ok(self.with_resolved_terminal_model(role, model, future))
    }

    fn resolve_saved_terminal_model(
        &self,
        role: &str,
        model: &SavedTerminalModelRef,
        reasoning_effort: Option<ReasoningEffort>,
    ) -> Result<ResolvedModel, RuntimeError> {
        let target = &model.target;
        let connection = self.require_terminal_card_connection(target)?;
        if model.binding != connection.binding {
            return Err(RuntimeError::Config(
                "The selected model or its connection changed. Reopen /models and select it again."
                    .into(),
            ));
        }
        let (_, index) = self.load_terminal_model_cards()?;
        let stored = index.card(target);
        if stored.is_some_and(|card| !card.applies_to(&connection)) {
            return Err(RuntimeError::Config("The saved model card no longer matches the configured connection or limits. Reopen and save the current card.".into()));
        }
        if stored.is_none() && connection.configured.is_none() {
            return Err(RuntimeError::Config(
                "Save this discovered model before choosing it as a saved card.".into(),
            ));
        }
        Ok(self
            .build_terminal_card(role, target, &connection, stored, reasoning_effort)?
            .1)
    }

    fn require_terminal_card_connection(
        &self,
        target: &TerminalModelTarget,
    ) -> Result<CardConnection, RuntimeError> {
        self.terminal_card_connection(target)?.ok_or_else(|| {
            RuntimeError::Config("The model card's configured route is unavailable.".into())
        })
    }

    fn terminal_card_connection(
        &self,
        target: &TerminalModelTarget,
    ) -> Result<Option<CardConnection>, RuntimeError> {
        let (provider, model, configured) = match target {
            TerminalModelTarget::Configured { profile } => {
                if profile.is_empty() || profile.len() > 128 {
                    return Err(RuntimeError::Config(
                        "Invalid configured model profile.".into(),
                    ));
                }
                let Ok(resolved) = self.providers.model(profile) else {
                    return Ok(None);
                };
                let model = resolved.model_profile().clone();
                (
                    Arc::clone(resolved.provider()),
                    model.model.clone(),
                    Some(model),
                )
            }
            TerminalModelTarget::Discovered {
                provider_profile,
                model,
            } => {
                if provider_profile.is_empty()
                    || provider_profile.len() > 128
                    || model.is_empty()
                    || model.len() > 512
                    || model
                        .chars()
                        .any(|ch| ch.is_whitespace() || ch.is_control())
                {
                    return Err(RuntimeError::Config(
                        "Invalid discovered model identity.".into(),
                    ));
                }
                let Ok(provider) = self.providers.profile(provider_profile) else {
                    return Ok(None);
                };
                (provider, model.clone(), None)
            }
        };
        let profile = provider.profile();
        let credential_reference = provider
            .credential_reference()
            .map(|reference| reference.reference);
        let identity = serde_json::to_vec(&(
            target,
            profile.kind.as_str(),
            &profile.name,
            profile.generation_endpoint()?,
            credential_reference,
            &model,
        ))
        .map_err(|error| RuntimeError::Config(error.to_string()))?;
        Ok(Some(CardConnection {
            binding: hex::encode(Sha256::digest(identity)),
            provider_profile: profile.name.clone(),
            kind: profile.kind,
            model,
            configured,
        }))
    }

    fn build_terminal_card(
        &self,
        role: &str,
        target: &TerminalModelTarget,
        connection: &CardConnection,
        stored: Option<&StoredCard>,
        reasoning_effort: Option<ReasoningEffort>,
    ) -> Result<(TerminalModelCard, ResolvedModel), RuntimeError> {
        let saved_advertised = stored.and_then(|card| card.advertised.clone());
        let advertised = if connection.configured.is_some() || saved_advertised.is_none() {
            self.terminal_model_catalog
                .released_model(&connection.provider_profile, &connection.model)?
                .or(saved_advertised)
        } else {
            saved_advertised
        };
        let defaults = match &connection.configured {
            Some(model) => model.clone(),
            None => discovered_defaults(
                connection,
                advertised.as_ref().ok_or_else(|| {
                    RuntimeError::Config(
                        "Open this provider in /models to load the model's catalog card.".into(),
                    )
                })?,
            )?,
        };
        let overrides = stored
            .map(|card| card.overrides.clone())
            .unwrap_or_default();
        let profile =
            apply_card_overrides(connection.kind, &defaults, &overrides, reasoning_effort)?;
        let model = self.providers.resolve_terminal_model(role, profile)?;
        let card = TerminalModelCard {
            target: target.clone(),
            binding: connection.binding.clone(),
            route: model.route(),
            default_limits: defaults.limits,
            default_features: defaults.feature_settings,
            default_reasoning_effort: defaults.reasoning_effort,
            advertised,
            overrides,
            revision: stored.map_or(0, |card| card.revision),
        };
        Ok((card, model))
    }

    fn load_terminal_model_cards(&self) -> Result<(u64, TerminalModelCardIndex), RuntimeError> {
        let Some(event) = self
            .journal
            .read_stream_backwards(CARDS_STREAM, None, 1)?
            .pop()
        else {
            return Ok((0, TerminalModelCardIndex::default()));
        };
        if event.event_type != CARDS_SAVED || event.event_version != 1 {
            return Err(StoreError::Verification(
                "unknown terminal model card snapshot event".into(),
            )
            .into());
        }
        let index: TerminalModelCardIndex =
            serde_json::from_value(self.journal.decrypt_payload(&event)?)
                .map_err(|error| StoreError::Verification(error.to_string()))?;
        if index.cards.len() > MAX_CARDS {
            return Err(StoreError::Verification(
                "terminal model card snapshot exceeds the card limit".into(),
            )
            .into());
        }
        let mut targets = BTreeSet::new();
        for card in &index.cards {
            if !targets.insert(&card.target) {
                return Err(StoreError::Verification(
                    "duplicate target in terminal model card snapshot".into(),
                )
                .into());
            }
            if !(1..=event.stream_version).contains(&card.revision) {
                return Err(StoreError::Verification(
                    "invalid revision in terminal model card snapshot".into(),
                )
                .into());
            }
            if card.binding.len() != 64
                || !card.binding.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(StoreError::Verification(
                    "invalid binding in terminal model card snapshot".into(),
                )
                .into());
            }
            if let TerminalModelTarget::Discovered { model, .. } = &card.target
                && card
                    .advertised
                    .as_ref()
                    .is_none_or(|advertised| &advertised.id != model)
            {
                return Err(StoreError::Verification(
                    "missing or mismatched catalog evidence in terminal model card snapshot".into(),
                )
                .into());
            }
        }
        Ok((event.stream_version, index))
    }
}

fn validate_card_session(session_id: &str) -> Result<(), RuntimeError> {
    if session_id.is_empty()
        || session_id.len() > 128
        || !session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(RuntimeError::Config(
            "Invalid terminal model session ID.".into(),
        ));
    }
    Ok(())
}

fn discovered_defaults(
    connection: &CardConnection,
    advertised: &ProviderModelInfo,
) -> Result<ModelProfile, RuntimeError> {
    let context = advertised
        .context_window_tokens
        .filter(|limit| *limit >= 1_024)
        .unwrap_or(32_768);
    let available = context - context.div_ceil(10).max(512) - 1;
    let output = advertised
        .max_output_tokens
        .unwrap_or(4_096)
        .min(context / 2)
        .min(available);
    let features = ModelFeatureSettings {
        declared: ModelFeatureDeclarations {
            tool_calls: advertised.tool_calls,
            streaming: advertised.streaming,
            image_inputs: if connection.kind == ProviderKind::Echo {
                Some(false)
            } else {
                advertised.image_inputs
            },
            server_compaction: advertised.server_compaction,
        },
        ..ModelFeatureSettings::default()
    };
    Ok(ModelProfile::new(
        format!("terminal-saved-{}", connection.binding),
        &connection.provider_profile,
        &connection.model,
        context,
        output,
        features.capabilities(),
        None,
    )?
    .with_feature_settings(features))
}

fn apply_card_overrides(
    kind: ProviderKind,
    defaults: &ModelProfile,
    overrides: &TerminalModelOverrides,
    effort: Option<ReasoningEffort>,
) -> Result<ModelProfile, RuntimeError> {
    let mut features = defaults.feature_settings;
    features.tool_calls = overrides.tool_calls.unwrap_or(features.tool_calls);
    features.streaming = overrides.streaming.unwrap_or(features.streaming);
    features.image_inputs = overrides.image_inputs.unwrap_or(features.image_inputs);
    features.server_compaction = overrides
        .server_compaction
        .unwrap_or(features.server_compaction);
    let reasoning_effort = effort
        .or(overrides.reasoning_effort)
        .or(defaults.reasoning_effort);
    if kind == ProviderKind::Echo {
        if reasoning_effort.is_some() || overrides.image_inputs == Some(ModelFeatureMode::On) {
            return Err(RuntimeError::Config(
                "Echo does not support reasoning effort or image inputs.".into(),
            ));
        }
        features.image_inputs = ModelFeatureMode::Off;
    }
    let mut model = ModelProfile::new(
        &defaults.name,
        &defaults.provider_profile,
        &defaults.model,
        overrides
            .context_window_tokens
            .unwrap_or(defaults.limits.context_window_tokens),
        overrides
            .max_output_tokens
            .unwrap_or(defaults.limits.max_output_tokens),
        features.capabilities(),
        reasoning_effort,
    )?
    .with_feature_settings(features);
    if let Some(input) = overrides.input_budget_tokens {
        if input == 0 || input > model.limits.input_budget_tokens {
            return Err(RuntimeError::Config(format!(
                "The input budget must be between 1 and {} tokens after output and safety reservations.",
                model.limits.input_budget_tokens
            )));
        }
        model.limits.input_budget_tokens = input;
    }
    Ok(model)
}

#[cfg(test)]
mod tests;
