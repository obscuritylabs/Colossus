//! Terminal input and output for shared provider setup services.

use super::*;
use colossus_contracts::{ProviderModelInfo, provider_presets};
use colossus_runtime::{ProviderSetupConnection, ProviderSetupModel, SETUP_PROVIDER_PROFILE};

pub(super) async fn dispatch(
    cli: &Cli,
    home: &ColossusHome,
    home_workspace: &Path,
    runtime_options: &RuntimeOpenOptions,
) -> Result<bool, Box<dyn Error>> {
    let Command::Provider(command) = &cli.command else {
        return Ok(false);
    };
    match &command.command {
        ProviderAction::Presets => print_json(&provider_presets())?,
        ProviderAction::Discover(args) => {
            let connection = connection_input(args)?;
            let models = discover(&connection, home_workspace, runtime_options.clone()).await?;
            print_json(&models)?;
        }
        ProviderAction::Setup(args) => {
            validate_config_init_scope(cli.config.as_deref(), args.local)?;
            let target = config_init_target(
                cli.config.as_deref(),
                args.local,
                &runtime_options.workspace,
                home,
                home_workspace,
                false,
            );
            if fs::symlink_metadata(&target.config_path).is_ok() {
                return Err("configuration already exists; use --config NEW_PATH for a new provider configuration, or `provider discover` to inspect models".into());
            }
            let connection = connection_input(&args.connection)?;
            let (id, card) = if let Some(id) = &args.model {
                (id.trim().to_owned(), None)
            } else {
                if !io::stdin().is_terminal() {
                    return Err("non-interactive setup requires --model; use `provider discover --preset ID` to load available models first".into());
                }
                let models = discover(&connection, home_workspace, runtime_options.clone()).await
                    .map_err(|error| format!("model discovery failed: {error}. Retry `provider discover` after checking the connection, or use --model for manual setup."))?;
                let card = choose_model(&models)?;
                (card.id.clone(), Some(card))
            };
            let yaml = config_init_yaml(&target, None, None, None, None)?;
            let config = RuntimeConfig::from_yaml(&yaml)?
                .with_setup_provider(&connection)?
                .with_setup_model(
                    &ProviderSetupModel {
                        id,
                        context_window_tokens: args.context_window_tokens,
                        max_output_tokens: args.max_output_tokens,
                        tool_calls: args.tool_calls,
                        streaming: args.streaming,
                        image_inputs: args.image_inputs,
                    },
                    card.as_ref(),
                )?;
            let encoded = config.to_yaml()?;
            RuntimeConfig::from_yaml(&encoded)?;
            persist_new_configuration(&target, &encoded)?;
            print_json(
                &json!({"created": true, "config_path": target.config_path, "provider": connection.preset, "model": config.models.profiles["primary"]}),
            )?;
            emit_security_posture_warning(&config.security_posture())?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn connection_input(
    args: &ProviderConnectionArgs,
) -> Result<ProviderSetupConnection, Box<dyn Error>> {
    let preset_id = if let Some(preset) = &args.preset {
        preset.clone()
    } else {
        if !io::stdin().is_terminal() {
            return Err(
                "--preset is required outside a terminal; use `provider presets` to list choices"
                    .into(),
            );
        }
        eprintln!("Choose a provider:");
        for (index, preset) in provider_presets().iter().enumerate() {
            eprintln!("  {}. {} ({})", index + 1, preset.label, preset.id);
        }
        let choice = prompt("Provider number or ID")?;
        if let Ok(index) = choice.parse::<usize>() {
            provider_presets()
                .get(index.wrapping_sub(1))
                .ok_or("invalid provider selection")?
                .id
                .into()
        } else {
            choice
        }
    };
    let preset = provider_presets()
        .iter()
        .find(|preset| preset.id == preset_id)
        .ok_or("unknown provider preset; use `provider presets`")?;
    let mut base_url = args.base_url.clone();
    if preset.base_url.is_none()
        && preset.protocol != colossus_contracts::ProviderSetupProtocol::Codex
        && base_url.is_none()
    {
        if !io::stdin().is_terminal() {
            return Err("custom providers require --base-url".into());
        }
        base_url = Some(prompt(
            "API base URL (including the version path, such as /v1)",
        )?);
    }
    let mut credential_env = args.credential_env.clone();
    if preset.id.starts_with("custom-")
        && credential_env.is_none()
        && !args.no_credential
        && io::stdin().is_terminal()
    {
        let name = prompt("API key environment variable name (blank for no authentication)")?;
        if !name.is_empty() {
            credential_env = Some(name);
        }
    }
    Ok(ProviderSetupConnection {
        preset: preset_id,
        base_url,
        credential_env,
        no_credential: args.no_credential,
    })
}

async fn discover(
    connection: &ProviderSetupConnection,
    home_workspace: &Path,
    runtime_options: RuntimeOpenOptions,
) -> Result<Vec<ProviderModelInfo>, Box<dyn Error>> {
    // A dedicated durable journal keeps catalog evidence separate from the user's runs.
    // Echo stays primary: no model ID or generation is needed to list a connection.
    let mut config = RuntimeConfig::offline_template("provider-discovery.redb")
        .with_setup_provider(connection)?;
    config.storage.location = StorageLocation::HomeWorkspace;
    let config = config.resolve_storage_paths(&runtime_options.workspace, home_workspace)?;
    let runtime =
        Runtime::open_with_options(&config, Arc::new(DenyApproval), None, runtime_options)?;
    let result = runtime.provider_models(Some(SETUP_PROVIDER_PROFILE)).await;
    let mut models = Vec::new();
    finalize_runtime_command(
        result.map_err(Into::into),
        || runtime.checkpoint(),
        |output| {
            models = output;
            Ok(())
        },
    )?;
    Ok(models)
}

fn choose_model(models: &[ProviderModelInfo]) -> Result<ProviderModelInfo, Box<dyn Error>> {
    eprintln!(
        "Loaded {} models. Unknown capabilities stay disabled unless overridden with setup flags.",
        models.len()
    );
    let filter = if models.len() > 40 {
        prompt("Filter models by ID or name")?
    } else {
        String::new()
    };
    let matching = matching_models(models, &filter);
    for (index, card) in matching.iter().take(40).enumerate() {
        eprintln!(
            "  {}. {}{} · context {} · output {}",
            index + 1,
            card.id,
            card.display_name
                .as_ref()
                .map(|name| format!(" ({name})"))
                .unwrap_or_default(),
            card.context_window_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            card.max_output_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into())
        );
    }
    if matching.len() > 40 {
        eprintln!(
            "Showing 40 of {} matches; enter an exact model ID to select any result.",
            matching.len()
        );
    }
    let choice = prompt("Model number or exact model ID")?;
    let card = if let Ok(index) = choice.parse::<usize>() {
        matching
            .get(index.wrapping_sub(1))
            .filter(|_| index <= 40)
            .copied()
    } else {
        models.iter().find(|card| card.id == choice)
    };
    card.cloned()
        .ok_or_else(|| "model was not in the catalog; retry or use --model for manual setup".into())
}

fn matching_models<'a>(
    models: &'a [ProviderModelInfo],
    filter: &str,
) -> Vec<&'a ProviderModelInfo> {
    let filter = filter.to_lowercase();
    models
        .iter()
        .filter(|card| {
            card.id.to_lowercase().contains(&filter)
                || card
                    .display_name
                    .as_ref()
                    .is_some_and(|name| name.to_lowercase().contains(&filter))
        })
        .collect()
}

fn prompt(label: &str) -> Result<String, Box<dyn Error>> {
    eprint!("{label}: ");
    io::stderr().flush()?;
    let mut line = String::new();
    if io::stdin().read_line(&mut line)? == 0 {
        return Err("provider setup cancelled".into());
    }
    Ok(line.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_display_names_and_exact_provider_ids() {
        let cards = vec![ProviderModelInfo {
            id: "org/model-one".into(),
            display_name: Some("Friendly Name".into()),
            ..Default::default()
        }];
        assert_eq!(matching_models(&cards, "FRIENDLY").len(), 1);
        assert_eq!(matching_models(&cards, "org/").len(), 1);
        assert!(matching_models(&cards, "missing").is_empty());
    }

    #[test]
    fn setup_accepts_custom_protocol_and_explicit_capability_flags() {
        let cli = Cli::try_parse_from([
            "colossus",
            "provider",
            "setup",
            "--preset",
            "custom-responses",
            "--base-url",
            "http://localhost:1234/v1",
            "--model",
            "local-model",
            "--no-credential",
            "--tool-calls",
            "true",
            "--streaming",
            "false",
        ])
        .unwrap();
        let Command::Provider(ProviderCommand {
            command: ProviderAction::Setup(args),
        }) = cli.command
        else {
            panic!("provider setup");
        };
        assert_eq!(args.connection.preset.as_deref(), Some("custom-responses"));
        assert_eq!(args.tool_calls, Some(true));
        assert_eq!(args.streaming, Some(false));
    }
}
