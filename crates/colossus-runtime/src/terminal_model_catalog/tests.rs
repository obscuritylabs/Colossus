use super::*;
use crate::{RuntimeConfig, RuntimeOpenOptions};
use colossus_contracts::{TerminalModelProfile, WorkflowAgentOptions};
use colossus_policy::DenyApproval;
use std::sync::atomic::{AtomicUsize, Ordering};

fn card(id: &str) -> ProviderModelInfo {
    ProviderModelInfo {
        id: id.into(),
        ..ProviderModelInfo::default()
    }
}

#[tokio::test]
async fn concurrent_catalog_requests_share_refresh_results_and_keep_the_last_success() {
    let cache = TerminalModelCatalog::default();
    let calls = AtomicUsize::new(0);
    let fetch = || async {
        calls.fetch_add(1, Ordering::Relaxed);
        tokio::task::yield_now().await;
        Ok(vec![card("original")])
    };
    let (first, second) = tokio::join!(
        cache.get_or_fetch("provider", false, fetch),
        cache.get_or_fetch("provider", false, fetch),
    );
    assert_eq!(first.unwrap(), second.unwrap());
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    let fail = || async {
        calls.fetch_add(1, Ordering::Relaxed);
        tokio::task::yield_now().await;
        Err(RuntimeError::Config("catalog offline".into()))
    };
    let (first, second) = tokio::join!(
        cache.get_or_fetch("provider", true, fail),
        cache.get_or_fetch("provider", true, fail),
    );
    assert!(first.unwrap_err().to_string().contains("catalog offline"));
    assert!(second.unwrap_err().to_string().contains("catalog offline"));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(
        cache.get_or_fetch("provider", false, fail).await.unwrap(),
        vec![card("original")],
    );
    assert_eq!(calls.load(Ordering::Relaxed), 2);

    cache
        .get_or_fetch("provider", true, || async { Ok(vec![card("updated")]) })
        .await
        .unwrap();
    assert_eq!(cache.current("provider").unwrap().unwrap()[0].id, "updated");
}

#[tokio::test]
async fn terminal_catalog_keeps_a_bounded_number_of_recent_providers() {
    let cache = TerminalModelCatalog::default();
    for index in 0..=MAX_CACHED_CATALOGS {
        cache
            .get_or_fetch(&index.to_string(), false, || async {
                Ok(vec![card("model")])
            })
            .await
            .unwrap();
    }
    assert_eq!(cache.entries.lock().unwrap().len(), MAX_CACHED_CATALOGS);
    assert!(cache.current("0").unwrap().is_none());
    assert!(
        cache
            .current(&MAX_CACHED_CATALOGS.to_string())
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn terminal_model_uses_released_limits_only_for_the_scoped_agent_operation() {
    let workspace = crate::test_support::private_tempdir();
    let mut config = RuntimeConfig::controlled_chat_template(workspace.path().join("state.redb"));
    config.use_ephemeral_storage();
    let options = RuntimeOpenOptions::for_controlled_chat(workspace.path())
        .unwrap()
        .with_unavailable_model_roles(["blocked".to_owned()].into_iter().collect());
    let runtime =
        Runtime::open_with_options(&config, Arc::new(DenyApproval), None, options).unwrap();
    let saved = runtime.provider_route("primary").unwrap();
    let session = runtime.create_session(Some("Temporary model")).unwrap();
    let profile = TerminalModelProfile {
        provider_profile: "echo".into(),
        model: "echo".into(),
        context_window_tokens: 16_384,
        max_output_tokens: 1_024,
    };
    assert!(
        runtime
            .terminal_model_route("primary", &session.id, &profile, None)
            .is_err()
    );
    let mut catalog = runtime.terminal_model_catalog("echo", false).await.unwrap();
    let echo = catalog.iter_mut().find(|card| card.id == "echo").unwrap();
    echo.context_window_tokens = Some(profile.context_window_tokens);
    echo.max_output_tokens = Some(profile.max_output_tokens);
    runtime
        .terminal_model_catalog
        .get_or_fetch("echo", true, || async { Ok(catalog) })
        .await
        .unwrap();
    let route = runtime
        .terminal_model_route("primary", &session.id, &profile, None)
        .unwrap();
    assert_eq!(
        route.limits.context_window_tokens,
        profile.context_window_tokens
    );
    assert_eq!(route.limits.max_output_tokens, profile.max_output_tokens);
    assert!(
        runtime
            .terminal_model_route("blocked", &session.id, &profile, None)
            .is_err()
    );
    for invalid in [
        TerminalModelProfile {
            model: "absent".into(),
            ..profile.clone()
        },
        TerminalModelProfile {
            context_window_tokens: profile.context_window_tokens + 1,
            ..profile.clone()
        },
        TerminalModelProfile {
            max_output_tokens: profile.max_output_tokens + 1,
            ..profile.clone()
        },
        TerminalModelProfile {
            context_window_tokens: 1_024,
            ..profile.clone()
        },
    ] {
        assert!(
            runtime
                .terminal_model_route("primary", &session.id, &invalid, None)
                .is_err()
        );
    }

    // Simulate a successful server confirmation whose reply never reaches the client.
    let unacknowledged = TerminalModelProfile {
        max_output_tokens: profile.max_output_tokens - 1,
        ..profile.clone()
    };
    runtime
        .terminal_model_route("primary", &session.id, &unacknowledged, None)
        .unwrap();
    assert!(
        runtime
            .pinned_terminal_model_route("primary", "other-session", &profile, None)
            .is_err()
    );
    assert!(
        runtime
            .with_terminal_model_selection(
                "primary",
                "other-session",
                &profile,
                None,
                std::future::ready(()),
            )
            .is_err()
    );

    runtime
        .terminal_model_catalog
        .get_or_fetch("echo", true, || async { Ok(vec![card("replacement")]) })
        .await
        .unwrap();
    assert!(
        runtime
            .terminal_model_route("primary", &session.id, &profile, None,)
            .is_err()
    );
    for index in 0..MAX_CACHED_CATALOGS {
        runtime
            .terminal_model_catalog
            .get_or_fetch(&index.to_string(), false, || async {
                Ok(vec![card("model")])
            })
            .await
            .unwrap();
    }
    assert!(
        runtime
            .terminal_model_catalog
            .current("echo")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        runtime
            .pinned_terminal_model_route("primary", &session.id, &profile, None)
            .unwrap(),
        route
    );

    let chosen = runtime
        .with_terminal_model_selection(
            "primary",
            &session.id,
            &profile,
            None,
            runtime.run_model_in_session(
                "primary",
                "test model selection",
                "transient echo",
                None,
                &session.id,
            ),
        )
        .unwrap()
        .await
        .unwrap();
    assert_eq!(chosen.model_profile, route.model_profile);
    assert_eq!(chosen.provider_profile, "echo");
    assert!(chosen.output.contains("transient echo"));

    let public_options = WorkflowAgentOptions {
        model_profile: Some(route.model_profile),
        reasoning_effort: None,
    };
    assert!(
        runtime
            .model_route_with_options("primary", &public_options)
            .is_err()
    );
    let ordinary = runtime
        .run_model("primary", "test model selection", "saved echo")
        .await
        .unwrap();
    assert_eq!(ordinary.model_profile, saved.model_profile);
}
