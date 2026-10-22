//! Generate an owner-private hosted-chat deployment from one reviewed primary provider.
//!
//! This explicitly selects that provider as a project service connection. It copies
//! configuration references only; no provider credential is loaded or exported.
use colossus_cloud_chat::ChatAuthentication;
use colossus_cloud_server::config::{ChatConfig, ChatProjectConfig};
use colossus_home::ColossusHome;
use colossus_runtime::{KeyConfig, RuntimeConfig};
use std::{
    collections::BTreeMap,
    io::Write as _,
    path::{Path, PathBuf},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let directory =
        PathBuf::from(arguments.next().ok_or(
            "usage: init_chat ABSOLUTE_PRIVATE_DIR PROJECT RUNTIME_CONFIG [CODEX_AUTH_FILE]",
        )?);
    let project = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or("project identifier required")?;
    colossus_cloud::validate_identifier(&project)?;
    let source = RuntimeConfig::from_path(PathBuf::from(
        arguments.next().ok_or("runtime configuration required")?,
    ))?;
    let codex_auth_path = arguments.next().map(PathBuf::from);
    if arguments.next().is_some() {
        return Err("unexpected argument".into());
    }
    let home = ColossusHome::ensure_at(directory)?;
    let root = home.confined_root();
    if root.path().join("chat.json").exists() {
        return Err("hosted chat configuration already exists".into());
    }
    let selected_model = source
        .models
        .roles
        .get("primary")
        .ok_or("source requires a primary model")?;
    let primary = source
        .models
        .profiles
        .get(selected_model)
        .ok_or("primary model is unavailable")?;
    let provider_id = &primary.provider_profile;
    let provider = source
        .providers
        .profiles
        .get(provider_id)
        .ok_or("primary provider is unavailable")?;
    let mut runtime = RuntimeConfig::controlled_chat_template(root.path().join("state.redb"));
    runtime.storage.keys = KeyConfig::Environment {
        journal_variable: "COLOSSUS_CHAT_JOURNAL_KEY".into(),
        journal_key_id: "hosted-chat".into(),
        signing_variable: "COLOSSUS_CHAT_SIGNING_KEY".into(),
        anchor_path: root.path().join("journal.anchor"),
    };
    runtime.providers.profiles = BTreeMap::from([(provider_id.clone(), provider.clone())]);
    runtime.models.profiles = source
        .models
        .profiles
        .iter()
        .filter(|(_, model)| model.provider_profile == *provider_id)
        .map(|(id, model)| (id.clone(), model.clone()))
        .collect();
    runtime.models.roles = source
        .models
        .roles
        .iter()
        .filter(|(_, profile)| runtime.models.profiles.contains_key(*profile))
        .map(|(role, profile)| (role.clone(), profile.clone()))
        .collect();
    runtime.network = source.network;
    // These are explicit operator declarations; controlled composition independently
    // rejects wildcard origins, grants and unreviewed tool activation.
    runtime.sandbox.network_destinations = source.sandbox.network_destinations;
    RuntimeConfig::from_yaml(&runtime.to_yaml()?)?;
    let authentication = if provider.kind.generation_action() == "provider.echo" {
        ChatAuthentication::None
    } else {
        ChatAuthentication::Shared
    };
    if provider.kind.generation_action() == "provider.openai.codex" && codex_auth_path.is_none() {
        return Err("the selected Codex provider requires its explicit native account file".into());
    }
    let project_config = ChatProjectConfig {
        labels: Default::default(),
        codex_auth_shared: false,
        runtime,
        mcp_authentication: BTreeMap::new(),
        provider_authentication: BTreeMap::from([(provider_id.clone(), authentication)]),
        provider_generations: BTreeMap::new(),
        mcp_generations: BTreeMap::new(),
        revision: 1,
        codex_auth_path,
    };
    let configuration = ChatConfig {
        shared: None,
        home: root.path().join("host"),
        wrapping_key_variable: "COLOSSUS_CHAT_WRAPPING_KEY".into(),
        max_active_runtimes: 16,
        idle_seconds: 300,
        projects: BTreeMap::from([(project, project_config)]),
    };
    let encoded = serde_json::to_vec_pretty(&configuration)?;
    let file = root.open_file(Path::new("chat.json"))?;
    file.file().write_all(&encoded)?;
    file.file().sync_all()?;
    root.sync_directory()?;
    println!(
        "Hosted chat configuration prepared at {}",
        file.path().display()
    );
    Ok(())
}
