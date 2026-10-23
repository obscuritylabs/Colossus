//! Released configuration availability never grants Hosted runtime authority.
use serde::Serialize;

pub(crate) const OPERATOR_SETUP_REQUIRED: &str = "Hosted execution is not enabled for this project. A deployment operator must configure its runtime and allowed origins before project overrides or connection credentials can be used.";

#[derive(Clone, Serialize)]
pub(crate) struct ExecutionAvailability {
    enabled: bool,
    settings_editable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
}

impl ExecutionAvailability {
    pub(crate) fn enabled() -> Self {
        Self {
            enabled: true,
            settings_editable: true,
            reason: None,
        }
    }

    pub(crate) fn dormant() -> Self {
        Self {
            enabled: false,
            settings_editable: false,
            reason: Some(OPERATOR_SETUP_REQUIRED),
        }
    }
}
