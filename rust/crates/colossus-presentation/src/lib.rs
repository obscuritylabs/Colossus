//! Event-sourced presentation preferences and pure semantic terminal rendering.

use colossus_contracts::{
    Actor, ContextStatus, EventClassification, ExecutionContext, NewEvent, ProviderEvent,
    WorkStateSnapshot,
};
pub use colossus_contracts::{
    EventDisplayMode, ReplPreferences, StreamDisplayMode, ThemeName, TranscriptDensity,
};
use colossus_ports::{EventJournal, PresentationRepository, StoreError};
use serde_json::{Value, json};
use std::sync::Arc;
use thiserror::Error;

const PREFERENCES_STREAM: &str = "presentation:repl";
const PREFERENCES_UPDATED: &str = "presentation.preferences.updated.v1";

/// Presentation rendering failure.
#[derive(Debug, Error)]
pub enum PresentationError {
    /// Released content could not be rendered safely.
    #[error("presentation rendering failed: {0}")]
    Invalid(String),
}

fn validate_preferences(preferences: &ReplPreferences) -> Result<(), StoreError> {
    if preferences.schema_version != 1 {
        return Err(StoreError::Adapter("schema_version must be 1".into()));
    }
    Ok(())
}

/// Immutable-journal implementation of the presentation preference port.
pub struct EventSourcedPresentationRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedPresentationRepository {
    /// Bind the global REPL presentation profile to the authoritative journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }
}

impl PresentationRepository for EventSourcedPresentationRepository {
    fn load(&self) -> Result<ReplPreferences, StoreError> {
        let events = self.journal.read_stream(PREFERENCES_STREAM)?;
        let Some(event) = events.last() else {
            return Ok(ReplPreferences::default());
        };
        if event.event_type != PREFERENCES_UPDATED {
            return Err(StoreError::Verification(
                "presentation stream contains an unknown event".into(),
            ));
        }
        let payload = self.journal.decrypt_payload(event)?;
        let preferences: ReplPreferences = serde_json::from_value(
            payload
                .get("preferences")
                .cloned()
                .ok_or_else(|| StoreError::Verification("preferences payload is absent".into()))?,
        )
        .map_err(|error| StoreError::Verification(error.to_string()))?;
        validate_preferences(&preferences)?;
        Ok(preferences)
    }

    fn save(
        &self,
        preferences: ReplPreferences,
        actor: Actor,
    ) -> Result<ReplPreferences, StoreError> {
        validate_preferences(&preferences)?;
        let expected_stream_version =
            u64::try_from(self.journal.read_stream(PREFERENCES_STREAM)?.len())
                .map_err(|error| StoreError::Adapter(error.to_string()))?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: PREFERENCES_STREAM.into(),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: PREFERENCES_UPDATED.into(),
            actor,
            context: ExecutionContext {
                correlation_id: PREFERENCES_STREAM.into(),
                ..ExecutionContext::default()
            },
            payload: json!({"preferences": &preferences}),
        })?;
        Ok(preferences)
    }
}

/// Pure semantic renderer over already released contracts.
pub struct SemanticRenderer {
    preferences: ReplPreferences,
}

impl SemanticRenderer {
    /// Create a renderer for one immutable preference snapshot.
    pub fn new(preferences: ReplPreferences) -> Self {
        Self { preferences }
    }

    fn label(&self, name: &str) -> String {
        match self.preferences.theme {
            ThemeName::Default => format!("[{name}]"),
            ThemeName::HighContrast => format!("{}:", name.to_ascii_uppercase()),
            ThemeName::Plain => format!("{name}:"),
        }
    }

    /// Render current session work without exposing repository internals.
    pub fn work_state(&self, state: &WorkStateSnapshot) -> String {
        let summary = format!(
            "{} session={} tasks={}/{} decisions={} plans={} goals={} agents={}",
            self.label("work"),
            state.session_id,
            state.open_task_count,
            state.tasks.len(),
            state.active_decisions.len(),
            state.actionable_plans.len(),
            state.current_goals.len(),
            state.current_subagents.len()
        );
        if self.preferences.transcript_density == TranscriptDensity::Compact {
            return summary;
        }
        let mut lines = vec![summary];
        lines.extend(
            state
                .tasks
                .iter()
                .filter(|task| {
                    !matches!(
                        task.status,
                        colossus_contracts::TaskStatus::Completed
                            | colossus_contracts::TaskStatus::Cancelled
                    )
                })
                .map(|task| format!("  task [{}] {}", task.id, task.title)),
        );
        lines.extend(
            state
                .current_goals
                .iter()
                .map(|goal| format!("  goal [{}] {}", goal.id, goal.objective)),
        );
        lines.join("\n")
    }

    /// Render context budget and compaction state.
    pub fn context_status(&self, status: &ContextStatus) -> String {
        format!(
            "{} session={} messages={} tokens={}/{} compacted={} snapshot={}",
            self.label("context"),
            status.session_id,
            status.message_count,
            status.token_estimate,
            status.context_window_tokens,
            status.compacted,
            status.active_snapshot_id.as_deref().unwrap_or("none")
        )
    }

    /// Render one already policy-released provider event.
    ///
    /// Visible model deltas are streamed separately and final output is not repeated. Safe
    /// reasoning summaries remain independently configurable from tool/activity events.
    pub fn provider_event(
        &self,
        event: &ProviderEvent,
    ) -> Result<Option<String>, PresentationError> {
        if self.preferences.stream_mode == StreamDisplayMode::Raw {
            return Ok(None);
        }
        let rendered = match event {
            ProviderEvent::ModelDelta { .. } | ProviderEvent::FinalOutput { .. } => None,
            ProviderEvent::ReasoningSummary { summary } if self.preferences.show_reasoning => {
                Some(format!("{} {summary}", self.label("thinking")))
            }
            ProviderEvent::ReasoningSummary { .. } => None,
            ProviderEvent::ToolCallRequested {
                call_id,
                name,
                arguments,
            } => match self.preferences.events_mode {
                EventDisplayMode::Off => None,
                EventDisplayMode::Compact => Some(format!("{} {name}", self.label("tool"))),
                EventDisplayMode::Verbose => Some(format!(
                    "{} call_id={call_id} name={name} arguments={}",
                    self.label("tool"),
                    serde_json::to_string(arguments)
                        .map_err(|error| PresentationError::Invalid(error.to_string()))?
                )),
            },
            ProviderEvent::Usage { usage } => match self.preferences.events_mode {
                EventDisplayMode::Verbose => Some(format!(
                    "{} input={} output={} total={} cached={} reasoning={}",
                    self.label("usage"),
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.total_tokens,
                    usage
                        .cached_input_tokens
                        .map_or_else(|| "unknown".into(), |value| value.to_string()),
                    usage
                        .reasoning_tokens
                        .map_or_else(|| "unknown".into(), |value| value.to_string())
                )),
                EventDisplayMode::Compact | EventDisplayMode::Off => None,
            },
        };
        Ok(rendered)
    }

    /// Render generic released structured output according to transcript density.
    pub fn structured(&self, value: &Value) -> Result<String, PresentationError> {
        if self.preferences.transcript_density == TranscriptDensity::Compact {
            serde_json::to_string(value)
        } else {
            serde_json::to_string_pretty(value)
        }
        .map_err(|error| PresentationError::Invalid(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EventDisplayMode, EventSourcedPresentationRepository, ReplPreferences, SemanticRenderer,
        StreamDisplayMode, ThemeName, TranscriptDensity,
    };
    use colossus_contracts::{Actor, ActorType, ProviderEvent, ProviderUsage, WorkStateSnapshot};
    use colossus_ports::{EventJournal, PresentationRepository};
    use colossus_testkit::{InMemoryEventJournal, assert_presentation_repository_conformance};
    use std::sync::Arc;

    #[test]
    fn preferences_reconstruct_from_immutable_events_and_validate_schema() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedPresentationRepository::new(Arc::clone(&journal));
        assert_eq!(
            repository.load().expect("defaults"),
            ReplPreferences::default()
        );
        let preferences = ReplPreferences {
            theme: ThemeName::HighContrast,
            multiline: true,
            stream_mode: StreamDisplayMode::Off,
            events_mode: EventDisplayMode::Verbose,
            show_reasoning: false,
            transcript_density: TranscriptDensity::Compact,
            ..ReplPreferences::default()
        };
        repository
            .save(
                preferences.clone(),
                Actor {
                    actor_type: ActorType::User,
                    id: "terminal-user".into(),
                },
            )
            .expect("save");
        let restarted = EventSourcedPresentationRepository::new(Arc::clone(&journal));
        assert_eq!(restarted.load().expect("load"), preferences);
        let events = journal.read_stream("presentation:repl").expect("events");
        assert_eq!(events[0].event_type, "presentation.preferences.updated.v1");
        let invalid = ReplPreferences {
            schema_version: 2,
            ..ReplPreferences::default()
        };
        assert!(
            restarted
                .save(
                    invalid,
                    Actor {
                        actor_type: ActorType::User,
                        id: "terminal-user".into(),
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn event_sourced_repository_passes_shared_conformance() {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository = EventSourcedPresentationRepository::new(journal);
        assert_presentation_repository_conformance(&repository);
    }

    #[test]
    fn work_renderer_has_compact_and_comfortable_semantics() {
        let state = WorkStateSnapshot {
            session_id: "session-1".into(),
            tasks: Vec::new(),
            open_task_count: 0,
            active_decisions: Vec::new(),
            actionable_plans: Vec::new(),
            current_goals: Vec::new(),
            current_subagents: Vec::new(),
        };
        let compact = SemanticRenderer::new(ReplPreferences {
            transcript_density: TranscriptDensity::Compact,
            ..ReplPreferences::default()
        });
        assert_eq!(
            compact.work_state(&state),
            "[work] session=session-1 tasks=0/0 decisions=0 plans=0 goals=0 agents=0"
        );
        let comfortable = SemanticRenderer::new(ReplPreferences::default());
        assert!(
            comfortable
                .work_state(&state)
                .starts_with("[work] session=session-1")
        );
    }

    #[test]
    fn provider_events_respect_reasoning_events_and_theme_independently() {
        let renderer = SemanticRenderer::new(ReplPreferences {
            theme: ThemeName::HighContrast,
            events_mode: EventDisplayMode::Off,
            show_reasoning: true,
            ..ReplPreferences::default()
        });
        assert_eq!(
            renderer
                .provider_event(&ProviderEvent::ReasoningSummary {
                    summary: "safe summary".into(),
                })
                .expect("reasoning"),
            Some("THINKING: safe summary".into())
        );
        assert_eq!(
            renderer
                .provider_event(&ProviderEvent::ToolCallRequested {
                    call_id: "call-1".into(),
                    name: "filesystem.read".into(),
                    arguments: serde_json::json!({"path": "README.md"}),
                })
                .expect("tool"),
            None
        );

        let verbose = SemanticRenderer::new(ReplPreferences {
            theme: ThemeName::Plain,
            events_mode: EventDisplayMode::Verbose,
            ..ReplPreferences::default()
        });
        assert_eq!(
            verbose
                .provider_event(&ProviderEvent::Usage {
                    usage: ProviderUsage {
                        input_tokens: 4,
                        output_tokens: 2,
                        total_tokens: 6,
                        cached_input_tokens: Some(1),
                        reasoning_tokens: None,
                    },
                })
                .expect("usage"),
            Some("usage: input=4 output=2 total=6 cached=1 reasoning=unknown".into())
        );
    }
}
