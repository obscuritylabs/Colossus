use super::*;

fn open(config: &RuntimeConfig, workspace: &Path) -> Runtime {
    Runtime::open_with_options(
        config,
        Arc::new(colossus_policy::DenyApproval),
        None,
        RuntimeOpenOptions::for_controlled_chat(workspace).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn saved_discovered_card_restores_and_routes_without_a_catalog_after_reopen() {
    let workspace = crate::test_support::private_tempdir();
    let config = RuntimeConfig::controlled_chat_template(workspace.path().join("state.redb"));
    let draft_id = Uuid::now_v7().to_string();
    let target = TerminalModelTarget::Discovered {
        provider_profile: "echo".into(),
        model: "echo".into(),
    };
    let runtime = open(&config, workspace.path());
    assert!(runtime.terminal_model_cards().unwrap().is_empty());
    assert!(runtime.terminal_model_card(&target).is_err());
    runtime.terminal_model_catalog("echo", false).await.unwrap();
    let unsaved = runtime.terminal_model_card(&target).unwrap();
    assert_eq!(unsaved.revision, 0);
    let edit = TerminalModelCardUpdate {
        target: target.clone(),
        expected_binding: unsaved.binding,
        expected_revision: 0,
        overrides: TerminalModelOverrides {
            context_window_tokens: Some(16_384),
            input_budget_tokens: Some(6_000),
            max_output_tokens: Some(512),
            streaming: Some(ModelFeatureMode::Off),
            image_inputs: Some(ModelFeatureMode::Off),
            ..Default::default()
        },
    };
    let cancelled = RunControl::default();
    cancelled.cancel();
    assert!(
        runtime
            .save_terminal_model_card(&draft_id, edit.clone(), &cancelled)
            .await
            .is_err()
    );
    assert!(runtime.terminal_model_cards().unwrap().is_empty());
    let saved = runtime
        .save_terminal_model_card(&draft_id, edit.clone(), &RunControl::default())
        .await
        .unwrap();
    assert_eq!(saved.revision, 1);
    assert!(runtime.get_session(&draft_id).unwrap().is_none());
    assert!(matches!(
        runtime
            .save_terminal_model_card(&draft_id, edit, &RunControl::default())
            .await,
        Err(RuntimeError::Store(StoreError::Conflict {
            expected: 0,
            actual: 1,
            ..
        }))
    ));
    drop(runtime);

    let runtime = open(&config, workspace.path());
    assert!(
        runtime
            .terminal_model_catalog
            .released_model("echo", "echo")
            .unwrap()
            .is_none()
    );
    assert_eq!(runtime.terminal_model_cards().unwrap(), vec![saved.clone()]);
    let selected = saved.saved_ref();
    let route = runtime
        .saved_terminal_model_route("primary", &selected, None)
        .unwrap();
    assert_eq!(route.limits.input_budget_tokens, 6_000);
    assert_eq!(route.limits.max_output_tokens, 512);
    assert!(!route.capabilities.streaming);
    assert!(!route.capabilities.image_inputs);
    let run = runtime
        .with_saved_terminal_model(
            "primary",
            &selected,
            None,
            runtime.run_model("primary", "use saved model", "persisted echo settings"),
        )
        .unwrap()
        .await
        .unwrap();
    assert_eq!(run.model_profile, route.model_profile);
    assert!(run.output.contains("persisted echo settings"));
    assert_eq!(runtime.model_profiles()[0].model_profile, "echo");
}

#[tokio::test]
async fn rebinding_a_configured_model_does_not_apply_its_old_override() {
    let workspace = crate::test_support::private_tempdir();
    let mut config = RuntimeConfig::controlled_chat_template(workspace.path().join("state.redb"));
    let draft = Uuid::now_v7().to_string();
    let target = TerminalModelTarget::Configured {
        profile: "echo".into(),
    };
    let runtime = open(&config, workspace.path());
    let before = runtime.terminal_model_card(&target).unwrap();
    let saved = runtime
        .save_terminal_model_card(
            &draft,
            TerminalModelCardUpdate {
                target: target.clone(),
                expected_binding: before.binding.clone(),
                expected_revision: 0,
                overrides: TerminalModelOverrides {
                    max_output_tokens: Some(73),
                    ..Default::default()
                },
            },
            &RunControl::default(),
        )
        .await
        .unwrap();
    assert_eq!(saved.route.limits.max_output_tokens, 73);
    drop(runtime);
    config.models.profiles.get_mut("echo").unwrap().model = "different-echo-model".into();
    let runtime = open(&config, workspace.path());
    assert!(runtime.terminal_model_cards().unwrap().is_empty());
    let current = runtime.terminal_model_card(&target).unwrap();
    assert_eq!(current.revision, 0);
    assert_eq!(current.route.model, "different-echo-model");
    assert_eq!(current.route.limits, current.default_limits);
    assert_ne!(current.binding, before.binding);
    assert!(
        runtime
            .saved_terminal_model_route("primary", &saved.saved_ref(), None)
            .is_err()
    );
    let rejected = runtime
        .save_terminal_model_card(
            &draft,
            TerminalModelCardUpdate {
                target,
                expected_binding: before.binding,
                expected_revision: before.revision,
                overrides: saved.overrides,
            },
            &RunControl::default(),
        )
        .await
        .unwrap_err();
    assert!(
        rejected
            .to_string()
            .contains("configured connection changed")
    );
    assert!(runtime.terminal_model_cards().unwrap().is_empty());
}

#[tokio::test]
async fn replacing_a_credential_reference_at_the_same_endpoint_fences_old_selections() {
    let workspace = crate::test_support::private_tempdir();
    let mut config = RuntimeConfig::controlled_chat_template(workspace.path().join("state.redb"));
    let provider = config.providers.profiles.get_mut("echo").unwrap();
    provider.kind = ProviderKind::OpenAiCompatible;
    provider.base_url = Some("https://provider.invalid/v1".into());
    provider.credential_reference = Some("env:COLOSSUS_MODEL_BINDING_A".into());
    config.sandbox.network_destinations = vec!["https://provider.invalid".into()];
    let draft = Uuid::now_v7().to_string();
    let target = TerminalModelTarget::Configured {
        profile: "echo".into(),
    };
    let control = RunControl::default();
    let runtime = open(&config, workspace.path());
    let before = runtime.terminal_model_card(&target).unwrap();
    let selected_a = before.saved_ref();
    let saved_a = runtime
        .save_terminal_model_card(
            &draft,
            TerminalModelCardUpdate {
                target: target.clone(),
                expected_binding: before.binding,
                expected_revision: before.revision,
                overrides: TerminalModelOverrides {
                    max_output_tokens: Some(321),
                    ..Default::default()
                },
            },
            &control,
        )
        .await
        .unwrap();
    assert_eq!(saved_a.saved_ref(), selected_a);
    drop(runtime);

    config
        .providers
        .profiles
        .get_mut("echo")
        .unwrap()
        .credential_reference = Some("env:COLOSSUS_MODEL_BINDING_B".into());
    let runtime = open(&config, workspace.path());
    let after = runtime.terminal_model_card(&target).unwrap();
    assert_eq!(after.revision, 0);
    assert_ne!(after.binding, selected_a.binding);
    assert!(runtime.terminal_model_cards().unwrap().is_empty());
    assert!(
        runtime
            .saved_terminal_model_route("primary", &selected_a, None)
            .is_err()
    );
    let selected_b = after.saved_ref();
    let saved_b = runtime
        .save_terminal_model_card(
            &draft,
            TerminalModelCardUpdate {
                target: target.clone(),
                expected_binding: after.binding,
                expected_revision: after.revision,
                overrides: TerminalModelOverrides {
                    max_output_tokens: Some(654),
                    ..Default::default()
                },
            },
            &control,
        )
        .await
        .unwrap();
    assert_eq!(saved_b.saved_ref(), selected_b);
    assert!(
        runtime
            .saved_terminal_model_route("primary", &selected_a, None)
            .is_err()
    );
    assert_eq!(
        runtime
            .saved_terminal_model_route("primary", &selected_b, None)
            .unwrap()
            .limits
            .max_output_tokens,
        654
    );
    runtime
        .save_terminal_model_card(
            &draft,
            TerminalModelCardUpdate {
                target,
                expected_binding: saved_b.binding,
                expected_revision: saved_b.revision,
                overrides: TerminalModelOverrides {
                    max_output_tokens: Some(987),
                    ..Default::default()
                },
            },
            &control,
        )
        .await
        .unwrap();
    assert_eq!(
        runtime
            .saved_terminal_model_route("primary", &selected_b, None)
            .unwrap()
            .limits
            .max_output_tokens,
        987
    );
}

#[tokio::test]
async fn incompatible_config_baseline_keeps_other_cards_and_allows_repair() {
    let workspace = crate::test_support::private_tempdir();
    let mut config = RuntimeConfig::controlled_chat_template(workspace.path().join("state.redb"));
    let draft = Uuid::now_v7().to_string();
    let configured = TerminalModelTarget::Configured {
        profile: "echo".into(),
    };
    let discovered = TerminalModelTarget::Discovered {
        provider_profile: "echo".into(),
        model: "echo".into(),
    };
    let runtime = open(&config, workspace.path());
    runtime.terminal_model_catalog("echo", false).await.unwrap();
    let control = RunControl::default();
    for (target, input_budget_tokens) in [(&configured, Some(20_000)), (&discovered, None)] {
        let card = runtime.terminal_model_card(target).unwrap();
        runtime
            .save_terminal_model_card(
                &draft,
                TerminalModelCardUpdate {
                    target: target.clone(),
                    expected_binding: card.binding,
                    expected_revision: card.revision,
                    overrides: TerminalModelOverrides {
                        input_budget_tokens,
                        ..Default::default()
                    },
                },
                &control,
            )
            .await
            .unwrap();
    }
    drop(runtime);

    config
        .models
        .profiles
        .get_mut("echo")
        .unwrap()
        .context_window_tokens = 8_192;
    let runtime = open(&config, workspace.path());
    let saved = runtime.terminal_model_cards().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].target, discovered);
    let current = runtime.terminal_model_card(&configured).unwrap();
    assert_eq!(current.revision, 0);
    assert_eq!(current.route.limits, current.default_limits);
    let repaired = runtime
        .save_terminal_model_card(
            &draft,
            TerminalModelCardUpdate {
                target: configured,
                expected_binding: current.binding,
                expected_revision: current.revision,
                overrides: TerminalModelOverrides {
                    input_budget_tokens: Some(1_024),
                    ..Default::default()
                },
            },
            &control,
        )
        .await
        .unwrap();
    assert_eq!(repaired.revision, 3);
    assert_eq!(repaired.route.limits.input_budget_tokens, 1_024);
    assert_eq!(runtime.terminal_model_cards().unwrap().len(), 2);
}

#[test]
fn model_card_input_limit_and_feature_edits_reach_the_resolved_profile() {
    let features = ModelFeatureSettings {
        declared: ModelFeatureDeclarations {
            image_inputs: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    let baseline = ModelProfile::new(
        "configured",
        "provider",
        "model",
        16_384,
        1_024,
        features.capabilities(),
        None,
    )
    .unwrap()
    .with_feature_settings(features);
    let edit = TerminalModelOverrides {
        context_window_tokens: Some(20_480),
        input_budget_tokens: Some(7_500),
        max_output_tokens: Some(2_048),
        image_inputs: Some(ModelFeatureMode::On),
        streaming: Some(ModelFeatureMode::Off),
        ..Default::default()
    };
    let profile =
        apply_card_overrides(ProviderKind::OpenAiCompatible, &baseline, &edit, None).unwrap();
    assert_eq!(profile.limits.input_budget_tokens, 7_500);
    assert_eq!(profile.limits.max_output_tokens, 2_048);
    assert!(profile.capabilities.image_inputs);
    assert!(!profile.capabilities.streaming);
    for invalid in [0, 20_480] {
        assert!(
            apply_card_overrides(
                ProviderKind::OpenAiCompatible,
                &baseline,
                &TerminalModelOverrides {
                    input_budget_tokens: Some(invalid),
                    ..edit.clone()
                },
                None
            )
            .is_err()
        );
    }
}

#[tokio::test]
async fn full_snapshot_reclaims_only_obsolete_connections_when_adding_a_card() {
    let workspace = crate::test_support::private_tempdir();
    let mut config = RuntimeConfig::controlled_chat_template(workspace.path().join("state.redb"));
    let echo_profile = config.models.profiles["echo"].clone();
    let profile_names = (0..MAX_CARDS)
        .map(|number| format!("saved-echo-{number}"))
        .collect::<Vec<_>>();
    for name in &profile_names {
        config
            .models
            .profiles
            .insert(name.clone(), echo_profile.clone());
    }
    let configured = |name: &str| TerminalModelTarget::Configured {
        profile: name.into(),
    };
    let runtime = open(&config, workspace.path());
    let cards = profile_names
        .iter()
        .map(|name| {
            let target = configured(name);
            let connection = runtime.require_terminal_card_connection(&target).unwrap();
            StoredCard {
                target,
                binding: connection.binding,
                revision: 1,
                overrides: TerminalModelOverrides {
                    max_output_tokens: Some(512),
                    ..Default::default()
                },
                advertised: None,
            }
        })
        .collect();
    runtime
        .journal
        .append(NewEvent {
            event_version: 1,
            stream_id: CARDS_STREAM.into(),
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: CARDS_SAVED.into(),
            actor: terminal_actor(),
            context: ExecutionContext::default(),
            payload: serde_json::to_value(TerminalModelCardIndex { cards }).unwrap(),
        })
        .unwrap();
    let draft = Uuid::now_v7().to_string();
    let control = RunControl::default();
    let new_card = runtime.terminal_model_card(&configured("echo")).unwrap();
    let update = TerminalModelCardUpdate {
        target: new_card.target,
        expected_binding: new_card.binding,
        expected_revision: 0,
        overrides: TerminalModelOverrides::default(),
    };
    let full = runtime
        .save_terminal_model_card(&draft, update.clone(), &control)
        .await
        .unwrap_err();
    assert!(full.to_string().contains("card limit has been reached"));
    assert_eq!(runtime.load_terminal_model_cards().unwrap().0, 1);
    drop(runtime);

    config.models.profiles.remove(&profile_names[0]);
    config
        .models
        .profiles
        .get_mut(&profile_names[1])
        .unwrap()
        .model = "reassigned-echo".into();
    let runtime = open(&config, workspace.path());
    let saved = runtime
        .save_terminal_model_card(&draft, update, &control)
        .await
        .unwrap();
    assert_eq!(saved.revision, 2);
    let (_, index) = runtime.load_terminal_model_cards().unwrap();
    assert_eq!(index.cards.len(), MAX_CARDS - 1);
    assert!(index.card(&configured(&profile_names[0])).is_none());
    assert!(index.card(&configured(&profile_names[1])).is_none());
    for name in &profile_names[2..] {
        let retained = index.card(&configured(name)).unwrap();
        assert_eq!(retained.revision, 1);
        assert_eq!(retained.overrides.max_output_tokens, Some(512));
    }
}
