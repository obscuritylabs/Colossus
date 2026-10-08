//! Native settings columns and explicit enum conversion.
use super::{DomainRow, Metadata, mismatch, set, signed, unsigned};
use colossus_cloud::{
    CloudError, CloudResult,
    settings::{
        ClassificationBanner, ClassificationPosition, ClassificationTone, ControlPlaneSettings,
        ProjectPolicyExpectation,
    },
    storage::{BootstrapMarker, EntityValue},
};
use colossus_ports::StoreError;
use diesel::{
    QueryableByName,
    sql_types::{Array, BigInt, Bool, Nullable, Text},
};

row!(SettingsRow {
    classification_enabled: Option<bool> => Nullable<Bool>,
    classification_text: Option<String> => Nullable<Text>,
    classification_tone: Option<String> => Nullable<Text>,
    classification_position: Option<String> => Nullable<Text>,
    required_sandbox_profile: Option<String> => Nullable<Text>,
    allowed_approval_modes: Option<Vec<String>> => Nullable<Array<Text>>,
    allowed_tools: Option<Vec<String>> => Nullable<Array<Text>>,
    completed: Option<bool> => Nullable<Bool>,
    bootstrap_version: Option<i64> => Nullable<BigInt>,
});

fn required<T>(value: Option<T>) -> CloudResult<T> {
    value.ok_or(CloudError::Storage)
}
fn tone_name(value: ClassificationTone) -> &'static str {
    match value {
        ClassificationTone::Neutral => "neutral",
        ClassificationTone::Info => "info",
        ClassificationTone::Warning => "warning",
        ClassificationTone::Danger => "danger",
    }
}
fn tone(value: &str) -> CloudResult<ClassificationTone> {
    match value {
        "neutral" => Ok(ClassificationTone::Neutral),
        "info" => Ok(ClassificationTone::Info),
        "warning" => Ok(ClassificationTone::Warning),
        "danger" => Ok(ClassificationTone::Danger),
        _ => Err(CloudError::Storage),
    }
}
fn position_name(value: ClassificationPosition) -> &'static str {
    match value {
        ClassificationPosition::Top => "top",
        ClassificationPosition::TopAndBottom => "top_and_bottom",
    }
}
fn position(value: &str) -> CloudResult<ClassificationPosition> {
    match value {
        "top" => Ok(ClassificationPosition::Top),
        "top_and_bottom" => Ok(ClassificationPosition::TopAndBottom),
        _ => Err(CloudError::Storage),
    }
}

impl DomainRow for SettingsRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let mut row = Self {
            metadata,
            classification_enabled: None,
            classification_text: None,
            classification_tone: None,
            classification_position: None,
            required_sandbox_profile: None,
            allowed_approval_modes: None,
            allowed_tools: None,
            completed: None,
            bootstrap_version: None,
        };
        match value {
            EntityValue::DisplaySettings(value) => {
                row.classification_enabled = Some(value.classification.enabled);
                row.classification_text = Some(value.classification.text.clone());
                row.classification_tone = Some(tone_name(value.classification.tone).into());
                row.classification_position =
                    Some(position_name(value.classification.position).into());
            }
            EntityValue::PolicyExpectation(value) => {
                row.required_sandbox_profile = value.required_sandbox_profile.clone();
                row.allowed_approval_modes =
                    Some(value.allowed_approval_modes.iter().cloned().collect());
                row.allowed_tools = value
                    .allowed_tools
                    .as_ref()
                    .map(|values| values.iter().cloned().collect());
            }
            EntityValue::BootstrapMarker(value) => {
                row.completed = Some(value.completed);
                row.bootstrap_version = Some(signed(value.version)?);
            }
            _ => return Err(mismatch()),
        }
        Ok(row)
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        match self.metadata.id.as_str() {
            "display" => Ok(ControlPlaneSettings {
                revision: self.metadata.revision()?,
                classification: ClassificationBanner {
                    enabled: required(self.classification_enabled)?,
                    text: required(self.classification_text)?,
                    tone: tone(&required(self.classification_tone)?)?,
                    position: position(&required(self.classification_position)?)?,
                },
            }
            .into()),
            "policy-expectation" => Ok(ProjectPolicyExpectation {
                revision: self.metadata.revision()?,
                required_sandbox_profile: self.required_sandbox_profile,
                allowed_approval_modes: set(required(self.allowed_approval_modes)?)?,
                allowed_tools: self.allowed_tools.map(set).transpose()?,
            }
            .into()),
            "identity-bootstrap-v3" | "administrator-bootstrap-v3" => Ok(BootstrapMarker {
                completed: required(self.completed)?,
                version: unsigned(required(self.bootstrap_version)?)?,
            }
            .into()),
            _ => Err(CloudError::Storage),
        }
    }
}
