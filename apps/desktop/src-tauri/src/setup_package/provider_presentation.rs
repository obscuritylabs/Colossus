//! Desktop-only provider artwork and instructions; never part of runtime YAML.
use super::types::invalid;
use crate::{desktop_settings::DesktopSettings, dto::CommandErrorDto};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderPresentation {
    #[serde(default)]
    pub(crate) description_markdown: String,
    #[serde(default)]
    pub(crate) icon: Option<String>,
    #[serde(default)]
    pub(crate) dark_icon: Option<String>,
}

impl ProviderPresentation {
    pub(crate) fn normalized(mut self) -> Result<Self, CommandErrorDto> {
        if self.description_markdown.len() > 16_384 {
            return Err(invalid("Provider instructions must be 16 KiB or smaller."));
        }
        for data in [&mut self.icon, &mut self.dark_icon].into_iter().flatten() {
            if data.len() > 90_000 {
                return Err(invalid(
                    "Provider icons must be PNG files no larger than 64 KiB.",
                ));
            }
            let encoded = data
                .strip_prefix("data:image/png;base64,")
                .ok_or_else(|| invalid("Provider icons must be embedded PNG images."))?;
            let bytes = STANDARD
                .decode(encoded)
                .map_err(|_| invalid("The provider icon is invalid."))?;
            *data = super::archive::normalize_png(&bytes)?;
        }
        Ok(self)
    }
}

pub(crate) fn provider_presentation(
    settings: &DesktopSettings,
    resource_id: &str,
) -> Option<ProviderPresentation> {
    let entry = settings
        .global_configuration
        .providers
        .iter()
        .find(|entry| entry.id == resource_id && !entry.archived)?;
    let current = super::export::current(entry)?;
    if let Some(presentation) = settings
        .global_configuration
        .provider_presentations
        .get(resource_id)
    {
        return Some(presentation.clone());
    }
    settings.setup_packages.iter().find_map(|package| {
        let key = format!("provider:{}", current.profile);
        if package.catalog_resources.as_ref()?.get(&key)? != resource_id
            || !package
                .providers
                .iter()
                .any(|provider| provider.connection == *current)
        {
            return None;
        }
        let value = package.manifest.providers.get(&current.profile)?;
        Some(ProviderPresentation {
            description_markdown: value.description_markdown.clone(),
            icon: value
                .icon
                .as_ref()
                .and_then(|path| package.icons.get(path))
                .cloned(),
            dark_icon: value
                .dark_icon
                .as_ref()
                .and_then(|path| package.icons.get(path))
                .cloned(),
        })
    })
}

pub(crate) fn provider_presentations(
    settings: &DesktopSettings,
) -> BTreeMap<String, ProviderPresentation> {
    settings
        .global_configuration
        .providers
        .iter()
        .filter_map(|entry| {
            provider_presentation(settings, &entry.id).map(|value| (entry.id.clone(), value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn icon(size: u32) -> String {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            size,
            size,
            image::Rgba([20, 90, 180, 255]),
        ))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
        format!(
            "data:image/png;base64,{}",
            STANDARD.encode(bytes.into_inner())
        )
    }

    #[test]
    fn authored_presentation_is_bounded_and_normalizes_only_embedded_pngs() {
        let valid = ProviderPresentation {
            description_markdown: "## Token access".into(),
            icon: Some(icon(8)),
            dark_icon: Some(icon(8)),
        };
        assert_eq!(valid.clone().normalized().unwrap(), valid);
        for invalid_icon in [
            "https://example.test/icon.png".into(),
            "data:image/svg+xml;base64,PHN2Zz4=".into(),
            icon(513),
            format!("data:image/png;base64,{}", "A".repeat(90_000)),
        ] {
            assert!(
                ProviderPresentation {
                    icon: Some(invalid_icon),
                    ..ProviderPresentation::default()
                }
                .normalized()
                .is_err()
            );
        }
        assert!(
            ProviderPresentation {
                description_markdown: "é".repeat(8193),
                ..ProviderPresentation::default()
            }
            .normalized()
            .is_err()
        );
    }

    #[test]
    fn provider_presentation_survives_storage_and_offline_export_without_runtime_fields() {
        let mut settings = DesktopSettings::default();
        let mut package = super::super::tests::saved();
        super::super::catalog::import_catalog(&mut settings, &mut package, None).unwrap();
        let id = package.catalog_resources.as_ref().unwrap()["provider:company"].clone();
        settings.setup_packages.push(package.clone());
        assert!(
            provider_presentation(&settings, &id)
                .unwrap()
                .description_markdown
                .contains("administrator")
        );
        let authored = ProviderPresentation {
            description_markdown:
                "### Team instructions\n[Get a token](https://team.example.test/token)".into(),
            icon: Some(icon(8)),
            dark_icon: Some(icon(16)),
        };
        settings
            .global_configuration
            .provider_presentations
            .insert(id.clone(), authored.clone());
        let restored: DesktopSettings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        assert_eq!(
            provider_presentation(&restored, &id),
            Some(authored.clone())
        );
        let exported = super::super::commands::export_current(&restored).unwrap();
        assert!(!exported.config_yaml.contains("descriptionMarkdown"));
        assert!(!exported.config_yaml.contains("data:image"));
        let bytes = super::super::archive::write(&exported).unwrap();
        let source = super::super::archive::read(&bytes).unwrap();
        assert_eq!(
            source.manifest.providers["company"].description_markdown,
            authored.description_markdown
        );
        assert_eq!(source.icons.len(), 2);
        let canonical: serde_json::Value = serde_saphyr::from_str(&exported.config_yaml).unwrap();
        let mut imported =
            super::super::configuration::inspected(source, &canonical, &bytes).unwrap();
        let mut recipient = DesktopSettings::default();
        super::super::catalog::import_catalog(&mut recipient, &mut imported, None).unwrap();
        let imported_id = imported.catalog_resources.as_ref().unwrap()["provider:company"].clone();
        recipient.setup_packages.push(imported);
        assert_eq!(
            provider_presentation(&recipient, &imported_id),
            Some(authored)
        );
        // An explicit clear must not rediscover old artwork from the original package.
        settings
            .global_configuration
            .provider_presentations
            .insert(id.clone(), ProviderPresentation::default());
        assert_eq!(
            provider_presentation(&settings, &id),
            Some(ProviderPresentation::default())
        );
        let cleared = super::super::commands::export_current(&settings).unwrap();
        assert!(cleared.icons.is_empty());
        assert!(
            cleared.manifest.providers["company"]
                .description_markdown
                .is_empty()
        );
        // Replacing the same setup package preserves user-authored overrides.
        super::super::catalog::import_catalog(&mut settings, &mut package.clone(), Some(&package))
            .unwrap();
        assert_eq!(
            provider_presentation(&settings, &id),
            Some(ProviderPresentation::default())
        );
    }
}
