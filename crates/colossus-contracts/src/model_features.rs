use super::*;

/// Saved operator intent for one optional model feature.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFeatureMode {
    /// Omit the feature.
    Off,
    /// Use declarations and bounded observations, attempting unknown support.
    #[default]
    Auto,
    /// Attempt the feature despite negative capability evidence.
    On,
}

impl<'de> Deserialize<'de> for ModelFeatureMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Saved {
            Legacy(bool),
            Mode(String),
        }
        match Saved::deserialize(deserializer)? {
            Saved::Legacy(value) => Ok(value.into()),
            Saved::Mode(value) => match value.as_str() {
                "off" => Ok(Self::Off),
                "auto" => Ok(Self::Auto),
                "on" => Ok(Self::On),
                _ => Err(serde::de::Error::unknown_variant(
                    &value,
                    &["off", "auto", "on"],
                )),
            },
        }
    }
}

impl From<bool> for ModelFeatureMode {
    fn from(value: bool) -> Self {
        if value { Self::On } else { Self::Off }
    }
}

impl std::str::FromStr for ModelFeatureMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "off" | "false" => Ok(Self::Off),
            "auto" => Ok(Self::Auto),
            "on" | "true" => Ok(Self::On),
            _ => Err("expected off, auto, or on".into()),
        }
    }
}

impl ModelFeatureMode {
    /// Resolve request eligibility without granting any runtime authority.
    pub fn eligible(self, declared: Option<bool>, unsupported: bool) -> bool {
        match self {
            Self::Off => false,
            Self::Auto => declared != Some(false) && !unsupported,
            Self::On => true,
        }
    }
}

/// Provider-card declarations, kept separate from operator intent.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelFeatureDeclarations {
    /// Advertised structured tool support; absence means unknown.
    pub tool_calls: Option<bool>,
    /// Advertised streaming support; absence means unknown.
    pub streaming: Option<bool>,
    /// Advertised image support; absence means unknown.
    pub image_inputs: Option<bool>,
    /// Advertised Responses compaction support; absence means unknown.
    pub server_compaction: Option<bool>,
}

/// Saved request preferences. Legacy booleans retain explicit On/Off behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFeatureSettings {
    /// Structured tool preference.
    pub tool_calls: ModelFeatureMode,
    /// Streaming preference.
    pub streaming: ModelFeatureMode,
    /// Image-input preference.
    pub image_inputs: ModelFeatureMode,
    /// Responses server compaction preference.
    pub server_compaction: ModelFeatureMode,
    /// Card metadata bound to this selected model, never an authority grant.
    pub declared: ModelFeatureDeclarations,
}

impl<'de> Deserialize<'de> for ModelFeatureSettings {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Default, Deserialize)]
        #[serde(default, rename_all = "camelCase", deny_unknown_fields)]
        struct Fields {
            tool_calls: ModelFeatureMode,
            streaming: ModelFeatureMode,
            image_inputs: ModelFeatureMode,
            server_compaction: ModelFeatureMode,
            declared: ModelFeatureDeclarations,
        }
        let value = serde_json::Value::deserialize(deserializer)?;
        let legacy_missing_image = value.as_object().is_some_and(|fields| {
            !fields.contains_key("imageInputs")
                && ["toolCalls", "streaming"]
                    .iter()
                    .any(|key| fields.get(*key).is_some_and(serde_json::Value::is_boolean))
        });
        let fields: Fields = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            tool_calls: fields.tool_calls,
            streaming: fields.streaming,
            image_inputs: if legacy_missing_image {
                ModelFeatureMode::Off
            } else {
                fields.image_inputs
            },
            server_compaction: fields.server_compaction,
            declared: fields.declared,
        })
    }
}

impl ModelFeatureSettings {
    /// Resolve the three provider-neutral request capabilities.
    pub fn capabilities(self) -> ModelCapabilities {
        ModelCapabilities {
            tool_calls: self.tool_calls.eligible(self.declared.tool_calls, false),
            streaming: self.streaming.eligible(self.declared.streaming, false),
            image_inputs: self
                .image_inputs
                .eligible(self.declared.image_inputs, false),
        }
    }
}

impl From<ModelCapabilities> for ModelFeatureSettings {
    fn from(value: ModelCapabilities) -> Self {
        Self {
            tool_calls: value.tool_calls.into(),
            streaming: value.streaming.into(),
            image_inputs: value.image_inputs.into(),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests;
