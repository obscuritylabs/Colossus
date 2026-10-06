//! Global connection bookmarks; never credentials, enrollment, or execution authority.
use crate::{cloud_connector, dto::CommandErrorDto, state::AppState};
use colossus_home::{ColossusHome, ConfinedRoot};
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::Path,
};
use tauri::State;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ControlPlaneProfile {
    id: String,
    label: String,
    endpoint: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Profiles {
    revision: u64,
    profiles: Vec<ControlPlaneProfile>,
    default_profile: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProfileSnapshot {
    #[serde(flatten)]
    catalog: Profiles,
    connections: Vec<cloud_connector::CloudStatus>,
}

fn failure() -> CommandErrorDto {
    CommandErrorDto::invalid(
        "control_plane_profiles",
        "Connection settings are unavailable or changed; reload and try again.",
    )
}

fn access(next: Option<Profiles>) -> Result<Profiles, CommandErrorDto> {
    access_with_home(next, ColossusHome::resolve_and_ensure)
}

fn access_with_home(
    next: Option<Profiles>,
    resolve_home: impl FnOnce() -> Result<ColossusHome, colossus_home::HomeError>,
) -> Result<Profiles, CommandErrorDto> {
    if let Some(next) = &next {
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        if next.profiles.len() > 32
            || next.profiles.iter().any(|profile| {
                let valid_id = !profile.id.is_empty()
                    && profile.id.len() <= 128
                    && profile
                        .id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c));
                let valid_label = !profile.label.trim().is_empty()
                    && profile.label.len() <= 128
                    && !profile.label.chars().any(char::is_control);
                let valid_url = url::Url::parse(&profile.endpoint).is_ok_and(|url| {
                    let local = matches!(
                        url.host_str(),
                        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
                    );
                    (url.scheme() == "https" || url.scheme() == "http" && local)
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none()
                        && profile.endpoint.len() <= 2048
                });
                !valid_id
                    || !valid_label
                    || !valid_url
                    || !ids.insert(&profile.id)
                    || !names.insert(profile.label.trim().to_lowercase())
            })
            || next
                .default_profile
                .as_ref()
                .is_some_and(|id| !ids.contains(id))
        {
            return Err(CommandErrorDto::invalid(
                "control_plane_profiles",
                "Use unique connection names and HTTPS endpoints, or HTTP on loopback. Endpoints cannot include credentials, query parameters, or fragments.",
            ));
        }
    }
    let home = resolve_home().map_err(|_| failure())?;
    let root = ConfinedRoot::bind(
        home.confined_root()
            .prepare_directory(Path::new("desktop-control-plane"))
            .map_err(|_| failure())?,
    )
    .map_err(|_| failure())?;
    let lock = root
        .open_file(Path::new("profiles.lock"))
        .map_err(|_| failure())?;
    lock.file().lock_exclusive().map_err(|_| failure())?;
    let retained = root
        .open_file(Path::new("profiles.json"))
        .map_err(|_| failure())?;
    let mut file = retained.file();
    let result = (|| {
        retained.revalidate(&root).map_err(|_| failure())?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(32769)
            .read_to_end(&mut bytes)
            .map_err(|_| failure())?;
        if bytes.len() > 32768 {
            return Err(failure());
        }
        let old: Profiles = if bytes.is_empty() {
            Profiles::default()
        } else {
            serde_json::from_slice(&bytes).map_err(|_| failure())?
        };
        let Some(mut next) = next else { return Ok(old) };
        if next.revision != old.revision {
            return Err(failure());
        }
        next.revision = old.revision.checked_add(1).ok_or_else(failure)?;
        let bytes = serde_json::to_vec(&next).map_err(|_| failure())?;
        if bytes.len() > 32768 {
            return Err(failure());
        }
        let temp_name = format!("profiles-{}.tmp", uuid::Uuid::new_v4());
        let temp = root
            .open_file(Path::new(&temp_name))
            .map_err(|_| failure())?;
        let mut output = temp.file();
        output.write_all(&bytes).map_err(|_| failure())?;
        output.sync_all().map_err(|_| failure())?;
        retained.revalidate(&root).map_err(|_| failure())?;
        temp.revalidate(&root).map_err(|_| failure())?;
        std::fs::rename(temp.path(), retained.path()).map_err(|_| failure())?;
        Ok(next)
    })();
    let _ = FileExt::unlock(lock.file());
    result
}

#[tauri::command]
pub(crate) async fn control_plane_profiles(
    state: State<'_, AppState>,
) -> Result<ProfileSnapshot, CommandErrorDto> {
    let catalog = tauri::async_runtime::spawn_blocking(|| access(None))
        .await
        .map_err(|_| failure())??;
    Ok(ProfileSnapshot {
        catalog,
        connections: cloud_connector::global_connections(&state).await,
    })
}

#[tauri::command]
pub(crate) async fn save_control_plane_profiles(
    state: State<'_, AppState>,
    catalog: Profiles,
) -> Result<ProfileSnapshot, CommandErrorDto> {
    let catalog = tauri::async_runtime::spawn_blocking(move || access(Some(catalog)))
        .await
        .map_err(|_| failure())??;
    Ok(ProfileSnapshot {
        catalog,
        connections: cloud_connector::global_connections(&state).await,
    })
}

#[cfg(test)]
mod tests;
