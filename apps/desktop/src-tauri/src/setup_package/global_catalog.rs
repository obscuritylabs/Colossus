use super::{
    catalog::retain_or_import,
    globals::{self, SetupResource},
    types::{SavedSetupPackage, invalid},
};
use crate::{
    desktop_settings::DesktopSettings, dto::CommandErrorDto,
    managed_configuration::CatalogEntrySetting,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn import(
    settings: &mut DesktopSettings,
    package: &mut SavedSetupPackage,
    previous: Option<&SavedSetupPackage>,
    shared: &BTreeSet<String>,
    resources: &mut BTreeMap<String, String>,
    changed: &mut BTreeSet<String>,
) -> Result<(), CommandErrorDto> {
    let mut next = globals::from_yaml(&package.config_yaml)?;
    next.validate()?;
    let mut old = previous
        .map(|p| globals::from_yaml(&p.config_yaml))
        .transpose()?
        .unwrap_or_default();
    // Changing a destination or any other definition requires fresh credentials.
    // A package cannot transfer an existing secret to a new endpoint by reusing a slot.
    let mut bindings = BTreeMap::new();
    for slot in next.credential_slots() {
        let reusable = (next.credential_consumers(&slot) == old.credential_consumers(&slot))
            .then_some(previous)
            .flatten();
        let id = reusable
            .and_then(|p| p.credential_bindings.get(&slot))
            .filter(|id| {
                settings
                    .global_configuration
                    .credentials
                    .iter()
                    .any(|c| &c.id == *id)
            })
            .cloned()
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        if !settings
            .global_configuration
            .credentials
            .iter()
            .any(|c| c.id == id)
        {
            settings
                .global_configuration
                .credentials
                .push(super::credential_metadata::missing(&next, &slot, &id));
        }
        bindings.insert(slot, id);
    }
    next.map_credentials(|slot| bindings[slot].clone());
    old.map_credentials(|slot| {
        previous
            .and_then(|p| p.credential_bindings.get(slot))
            .cloned()
            .unwrap_or_default()
    });
    let old_ids = previous
        .and_then(|p| p.catalog_resources.as_ref())
        .cloned()
        .unwrap_or_default();
    let mut imports = CatalogImport {
        old_ids: &old_ids,
        shared,
        resources,
        changed,
    };
    imports.resources(
        &mut settings.global_configuration.mcp_servers,
        "mcp",
        &next.mcp_servers,
        &old.mcp_servers,
    )?;
    imports.resources(
        &mut settings.global_configuration.search_providers,
        "search",
        &next.search_providers,
        &old.search_providers,
    )?;
    imports.resources(
        &mut settings.global_configuration.telemetry_profiles,
        "telemetry",
        &next.telemetry_profiles,
        &old.telemetry_profiles,
    )?;
    package.credential_bindings = bindings;
    Ok(())
}

struct CatalogImport<'a> {
    old_ids: &'a BTreeMap<String, String>,
    shared: &'a BTreeSet<String>,
    resources: &'a mut BTreeMap<String, String>,
    changed: &'a mut BTreeSet<String>,
}

impl CatalogImport<'_> {
    fn resources<T: Clone + PartialEq>(
        &mut self,
        catalog: &mut Vec<CatalogEntrySetting<T>>,
        prefix: &str,
        next: &[SetupResource<T>],
        old: &[SetupResource<T>],
    ) -> Result<(), CommandErrorDto> {
        for resource in next {
            let key = format!("{prefix}:{}", resource.id);
            let id = retain_or_import(
                catalog,
                self.old_ids
                    .get(&key)
                    .filter(|id| !self.shared.contains(*id)),
                old.iter()
                    .find(|r| r.id == resource.id)
                    .map(|r| &r.configuration),
                &resource.label,
                &resource.configuration,
                self.changed,
            )?;
            self.resources.insert(key, id);
        }
        Ok(())
    }
}

pub(super) fn validate_bindings(package: &SavedSetupPackage) -> Result<(), CommandErrorDto> {
    let slots = globals::from_yaml(&package.config_yaml)?.credential_slots();
    if package
        .credential_bindings
        .iter()
        .any(|(slot, id)| !slots.contains(slot) || uuid::Uuid::parse_str(id).is_err())
    {
        return Err(invalid("Saved setup credential bindings are invalid."));
    }
    Ok(())
}
