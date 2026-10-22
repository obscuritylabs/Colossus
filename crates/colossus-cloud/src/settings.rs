//! Audited display configuration and project policy expectations.
use crate::storage::{CloudTransaction, EntityKey, EntityKind, EntityMutation};
use crate::{CloudCaller, CloudError, CloudPermission, CloudRepository, CloudResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Closed visual severity choices backed by shared theme tokens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationTone {
    /// Neutral display marking.
    #[default]
    Neutral,
    /// Informational display marking.
    Info,
    /// Warning display marking.
    Warning,
    /// High-priority display marking.
    Danger,
}
/// Classification marking placement around the application chrome.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationPosition {
    /// Show the marking above the application chrome.
    Top,
    /// Show the marking above and below the application chrome.
    #[default]
    TopAndBottom,
}
/// Literal deployment banner colors; CSS expressions and low-contrast pairs are rejected.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassificationColors {
    /// Background color in the exact `#RRGGBB` format.
    pub background: String,
    /// Text color in the exact `#RRGGBB` format, with at least 4.5:1 contrast.
    pub text: String,
}
impl ClassificationColors {
    /// Require a pair of opaque sRGB colors readable at the banner's ordinary text size.
    pub fn validate(&self) -> CloudResult<()> {
        let background = color_luminance(&self.background).ok_or(CloudError::InvalidArgument)?;
        let text = color_luminance(&self.text).ok_or(CloudError::InvalidArgument)?;
        if (background.max(text) + 0.05) / (background.min(text) + 0.05) < 4.5 {
            return Err(CloudError::InvalidArgument);
        }
        Ok(())
    }
}
fn color_luminance(color: &str) -> Option<f64> {
    if color.len() != 7
        || !color.starts_with('#')
        || !color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
    {
        return None;
    }
    let channel = |index| {
        let value = f64::from(u8::from_str_radix(&color[index..index + 2], 16).ok()?) / 255.0;
        Some(if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        })
    };
    Some(0.2126 * channel(1)? + 0.7152 * channel(3)? + 0.0722 * channel(5)?)
}
/// Plain-text deployment marking, rendered in the application chrome and sign-in view.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassificationBanner {
    /// Whether the deployment marking is visible.
    pub enabled: bool,
    /// Bounded plain text; rendered without HTML interpretation.
    pub text: String,
    /// Visual severity selected from shared design tokens.
    pub tone: ClassificationTone,
    /// Placement around the application chrome.
    pub position: ClassificationPosition,
    /// Optional administrator-published colors; absent preserves existing tone colors and audit bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<ClassificationColors>,
}
impl ClassificationBanner {
    /// Validate bounded plain text; the tone and position are closed enums.
    pub fn validate(&self) -> CloudResult<()> {
        if self.text.chars().count() > 160
            || self.text.chars().any(char::is_control)
            || (self.enabled && self.text.trim().is_empty())
        {
            return Err(CloudError::InvalidArgument);
        }
        if let Some(colors) = &self.colors {
            colors.validate()?;
        }
        Ok(())
    }
}
/// Public deployment display settings with an optimistic database revision.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPlaneSettings {
    /// Exact optimistic database revision; zero creates the initial setting.
    pub revision: u64,
    /// Public deployment marking configuration.
    pub classification: ClassificationBanner,
}
/// Display baseline only; setting it never changes native execution policy or grants.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPolicyExpectation {
    /// Exact optimistic database revision; zero creates the initial setting.
    pub revision: u64,
    /// Expected reported sandbox profile; absent means no profile baseline.
    pub required_sandbox_profile: Option<String>,
    /// Allowed reported modes; empty means no mode baseline.
    pub allowed_approval_modes: BTreeSet<String>,
    /// Expected tool ceiling; absent means no tool baseline, empty permits none.
    pub allowed_tools: Option<BTreeSet<String>>,
}
/// Monitoring state based on fresh, authenticated runtime-reported configuration.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAlignment {
    /// No baseline, no observation, or the observation is stale.
    Unknown,
    /// Fresh reported metadata agrees with the configured monitoring baseline.
    Aligned,
    /// Fresh reported metadata differs from the configured monitoring baseline.
    Drift,
}
/// A display-only monitoring difference; never an execution denial or attestation.
#[derive(Clone, Debug, Serialize)]
pub struct PolicyDifference {
    /// Stable metadata-only reason code.
    pub code: &'static str,
    /// Human-readable bounded difference description.
    pub description: &'static str,
}
/// Monitoring comparison that cannot change local authority.
#[derive(Clone, Debug, Serialize)]
pub struct PolicyEvaluation {
    /// Current monitoring conclusion, preserving uncertainty.
    pub status: PolicyAlignment,
    /// Differences in the retained observation, which may be stale.
    pub findings: Vec<PolicyDifference>,
}
impl ProjectPolicyExpectation {
    /// Compare safe released metadata; absent, stale or unconfigured evidence stays unknown.
    pub fn evaluate(
        &self,
        posture: Option<&colossus_sdk::RuntimePolicyPosture>,
        stale: bool,
    ) -> PolicyEvaluation {
        let mut findings = Vec::new();
        if let Some(posture) = posture {
            if self
                .required_sandbox_profile
                .as_ref()
                .is_some_and(|profile| profile != &posture.sandbox_profile)
            {
                findings.push(PolicyDifference{code:"sandbox_profile_drift",description:"Reported sandbox profile differs from the project's monitoring baseline."});
            }
            let mode = match posture.approval_mode {
                colossus_sdk::PolicyApprovalMode::Ask => "ask",
                colossus_sdk::PolicyApprovalMode::Deny => "deny",
                colossus_sdk::PolicyApprovalMode::RiskAuto => "risk_auto",
                colossus_sdk::PolicyApprovalMode::DangerAuto => "danger_auto",
                colossus_sdk::PolicyApprovalMode::Unknown => "unknown",
            };
            if !self.allowed_approval_modes.is_empty()
                && !self.allowed_approval_modes.contains(mode)
            {
                findings.push(PolicyDifference{code:"approval_mode_drift",description:"Reported approval mode falls outside the project's monitoring baseline."});
            }
            if self.allowed_tools.as_ref().is_some_and(|tools| {
                posture
                    .allowed_tools
                    .iter()
                    .any(|tool| !tools.contains(tool))
            }) {
                findings.push(PolicyDifference{code:"tool_ceiling_drift",description:"The reported tool ceiling includes tools outside the project's monitoring baseline."});
            }
        }
        let configured = self.required_sandbox_profile.is_some()
            || !self.allowed_approval_modes.is_empty()
            || self.allowed_tools.is_some();
        let status = if stale || posture.is_none() || !configured {
            PolicyAlignment::Unknown
        } else if findings.is_empty() {
            PolicyAlignment::Aligned
        } else {
            PolicyAlignment::Drift
        };
        PolicyEvaluation { status, findings }
    }
}
fn setting_key(project: &str, id: &str) -> EntityKey {
    EntityKey {
        kind: EntityKind::Setting,
        project_id: project.into(),
        parent_id: None,
        id: id.into(),
    }
}
impl CloudRepository {
    /// Public display settings contain no operational data, user identities or secrets.
    pub async fn display_settings(&self) -> CloudResult<ControlPlaneSettings> {
        match self.store.read(&setting_key("__identity", "display")).await {
            Ok(record) => record.value.try_into(),
            Err(CloudError::NotFound) => Ok(ControlPlaneSettings::default()),
            Err(error) => Err(error),
        }
    }
    /// The host must authenticate a global administrator before entering this operation.
    pub async fn replace_display_settings(
        &self,
        actor: &str,
        mut settings: ControlPlaneSettings,
    ) -> CloudResult<ControlPlaneSettings> {
        if actor.is_empty() || actor.len() > 256 || actor.chars().any(char::is_control) {
            return Err(CloudError::InvalidArgument);
        }
        settings.classification.validate()?;
        settings.classification.text = settings.classification.text.trim().into();
        if let Some(colors) = &mut settings.classification.colors {
            colors.background.make_ascii_lowercase();
            colors.text.make_ascii_lowercase();
        }
        let expected_revision = settings.revision;
        settings.revision = expected_revision
            .checked_add(1)
            .ok_or(CloudError::InvalidArgument)?;
        self.store
            .commit(CloudTransaction {
                entities: vec![EntityMutation {
                    key: setting_key("__identity", "display"),
                    expected_revision,
                    value: settings.clone().into(),
                    actor: actor.into(),
                    operation: "control_plane.display.updated.v1".into(),
                }],
                ..Default::default()
            })
            .await?;
        Ok(settings)
    }
    /// Read the exact project baseline beneath project visibility.
    pub async fn project_policy(
        &self,
        caller: &CloudCaller,
    ) -> CloudResult<ProjectPolicyExpectation> {
        caller.require(CloudPermission::Read)?;
        match self
            .store
            .read(&setting_key(caller.project_id(), "policy-expectation"))
            .await
        {
            Ok(record) => record.value.try_into(),
            Err(CloudError::NotFound) => Ok(ProjectPolicyExpectation::default()),
            Err(error) => Err(error),
        }
    }
    /// Update an audited monitoring baseline without changing runtime enforcement.
    pub async fn replace_project_policy(
        &self,
        caller: &CloudCaller,
        mut policy: ProjectPolicyExpectation,
    ) -> CloudResult<ProjectPolicyExpectation> {
        caller.require(CloudPermission::Administer)?;
        if policy
            .required_sandbox_profile
            .as_ref()
            .is_some_and(|profile| !label(profile))
            || policy.allowed_approval_modes.iter().any(|mode| {
                !matches!(
                    mode.as_str(),
                    "ask" | "risk_auto" | "danger_auto" | "deny" | "unknown"
                )
            })
            || policy
                .allowed_tools
                .as_ref()
                .is_some_and(|tools| tools.len() > 512 || tools.iter().any(|tool| !label(tool)))
        {
            return Err(CloudError::InvalidArgument);
        }
        let expected_revision = policy.revision;
        policy.revision = expected_revision
            .checked_add(1)
            .ok_or(CloudError::InvalidArgument)?;
        self.store
            .commit(CloudTransaction {
                entities: vec![EntityMutation {
                    key: setting_key(caller.project_id(), "policy-expectation"),
                    expected_revision,
                    value: policy.clone().into(),
                    actor: caller.subject().into(),
                    operation: "control_plane.policy_expectation.updated.v1".into(),
                }],
                ..Default::default()
            })
            .await?;
        Ok(policy)
    }
}
fn label(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryCloudStore;

    #[test]
    fn legacy_banner_serialization_remains_identical_without_color_overrides() {
        let legacy = serde_json::json!({
            "revision": 7,
            "classification": {
                "enabled": true,
                "text": "INTERNAL",
                "tone": "warning",
                "position": "top_and_bottom"
            }
        });
        let settings: super::ControlPlaneSettings = serde_json::from_value(legacy.clone()).unwrap();
        assert!(settings.classification.colors.is_none());
        assert_eq!(serde_json::to_value(settings).unwrap(), legacy);
    }

    #[test]
    fn classification_colors_require_literal_hex_and_readable_contrast() {
        for (background, text) in [
            ("#FFFFFF", "#000000"),
            ("#000000", "#ffffff"),
            ("#008000", "#ffffff"),
            ("#767676", "#ffffff"),
        ] {
            assert!(
                super::ClassificationColors {
                    background: background.into(),
                    text: text.into()
                }
                .validate()
                .is_ok()
            );
        }
        for (background, text) in [
            ("white", "#000000"),
            ("#fff", "#000000"),
            ("#ffffff00", "#000000"),
            ("url(https://example.test)", "#000000"),
            ("#ffffff", "var(--text)"),
            ("#ffffff", "#cccccc"),
            ("#000000", "#222222"),
            ("#777777", "#ffffff"),
            ("#１２３４５６", "#ffffff"),
        ] {
            assert!(
                super::ClassificationColors {
                    background: background.into(),
                    text: text.into()
                }
                .validate()
                .is_err()
            );
        }
    }
    #[test]
    fn stale_or_missing_policy_observations_never_establish_current_alignment() {
        let posture:colossus_sdk::RuntimePolicyPosture=serde_json::from_value(serde_json::json!({
            "schema_version":1,"provenance":"runtime_reported","fingerprint":"a".repeat(64),
            "configuration_revision":null,"access_profile":"minimal","sandbox_backend":"native",
            "sandbox_profile":"workspace","boundary_acknowledged":false,"approval_mode":"ask",
            "allowed_roles":[],"allowed_tools":["fs.read"],"capabilities":[],"models":[],"findings":[],
            "telemetry":{"provenance":"unavailable","denied_requests":null,"approval_requests":null,"outcome_unknown_runs":null}
        })).unwrap();
        assert!(posture.validate());
        let baseline = ProjectPolicyExpectation {
            required_sandbox_profile: Some("offline".into()),
            allowed_approval_modes: ["deny".into()].into(),
            allowed_tools: Some(Default::default()),
            ..Default::default()
        };
        let evaluation = baseline.evaluate(Some(&posture), false);
        assert!(matches!(evaluation.status, PolicyAlignment::Drift));
        assert_eq!(evaluation.findings.len(), 3);
        assert!(matches!(
            baseline.evaluate(Some(&posture), true).status,
            PolicyAlignment::Unknown
        ));
        assert!(matches!(
            baseline.evaluate(None, false).status,
            PolicyAlignment::Unknown
        ));
        assert!(matches!(
            ProjectPolicyExpectation::default()
                .evaluate(Some(&posture), false)
                .status,
            PolicyAlignment::Unknown
        ));
        let aligned = ProjectPolicyExpectation {
            required_sandbox_profile: Some("workspace".into()),
            ..Default::default()
        };
        assert!(matches!(
            aligned.evaluate(Some(&posture), false).status,
            PolicyAlignment::Aligned
        ));
    }
    #[tokio::test]
    async fn banner_is_audited_cas_plain_text_and_project_baseline_is_scoped() {
        let repo = CloudRepository::new(std::sync::Arc::new(MemoryCloudStore::default())).unwrap();
        let mut settings = repo.display_settings().await.unwrap();
        settings.classification.enabled = true;
        settings.classification.text = " INTERNAL <restricted> ".into();
        settings.classification.colors = Some(ClassificationColors {
            background: "#008000".into(),
            text: "#FFFFFF".into(),
        });
        let saved = repo
            .replace_display_settings("administrator", settings.clone())
            .await
            .unwrap();
        assert_eq!(saved.classification.text, "INTERNAL <restricted>");
        assert_eq!(saved.revision, 1);
        assert_eq!(
            saved.classification.colors.as_ref().unwrap().text,
            "#ffffff"
        );
        assert_eq!(repo.display_settings().await.unwrap(), saved);
        let mut invalid = saved.clone();
        invalid.classification.colors = Some(ClassificationColors {
            background: "#ffffff".into(),
            text: "#eeeeee".into(),
        });
        assert_eq!(
            repo.replace_display_settings("administrator", invalid)
                .await
                .unwrap_err(),
            CloudError::InvalidArgument
        );
        assert_eq!(repo.display_settings().await.unwrap(), saved);
        assert_eq!(
            repo.replace_display_settings("administrator", settings)
                .await
                .unwrap_err(),
            CloudError::Conflict
        );
        let reader = CloudCaller::new(
            "viewer".into(),
            "project-a".into(),
            [CloudPermission::Read].into(),
        )
        .unwrap();
        assert_eq!(
            repo.replace_project_policy(&reader, ProjectPolicyExpectation::default())
                .await
                .unwrap_err(),
            CloudError::PermissionDenied
        );
        let administrator = CloudCaller::new(
            "admin".into(),
            "project-a".into(),
            [CloudPermission::Read, CloudPermission::Administer].into(),
        )
        .unwrap();
        let baseline = ProjectPolicyExpectation {
            required_sandbox_profile: Some("offline".into()),
            ..Default::default()
        };
        assert_eq!(
            repo.replace_project_policy(&administrator, baseline)
                .await
                .unwrap()
                .revision,
            1
        );
        assert_eq!(
            repo.project_policy(&reader)
                .await
                .unwrap()
                .required_sandbox_profile
                .as_deref(),
            Some("offline")
        );
        let other = CloudCaller::new(
            "viewer".into(),
            "project-b".into(),
            [CloudPermission::Read].into(),
        )
        .unwrap();
        assert!(
            repo.project_policy(&other)
                .await
                .unwrap()
                .required_sandbox_profile
                .is_none()
        );
    }
}
