use super::display::validate as validate_display;
use super::*;
use std::time::Duration;

fn display() -> ClientIdentityDisplay {
    ClientIdentityDisplay {
        origin: "https://accounts.example.test:8443".into(),
        candidates: vec![
            ClientIdentityDisplayCandidate {
                label: "Alice — Example issuing CA".into(),
                fingerprint_sha256: "a1".repeat(32),
            },
            ClientIdentityDisplayCandidate {
                label: "Alice — Other issuing CA".into(),
                fingerprint_sha256: "b2".repeat(32),
            },
        ],
        remaining_ms: 30_000,
    }
}

fn pending() -> (PendingReview, oneshot::Receiver<Option<String>>) {
    let display = display();
    let (response, receiver) = oneshot::channel();
    (
        PendingReview {
            details: ReviewDto {
                ui_token: "native-one-use-token".into(),
                origin: display.origin,
                candidates: display.candidates,
                appearance: ReviewAppearance::default(),
            },
            target_id: "workspace-native-only".into(),
            selection_epoch: 1,
            deadline: Instant::now() + Duration::from_secs(30),
            response: Some(response),
        },
        receiver,
    )
}

#[test]
fn only_bounded_unambiguous_metadata_can_enter_the_document() {
    assert!(validate_display(&display()).is_ok());
    let mut joined_name = display();
    joined_name.candidates[0].label = "نام\u{200c}کاربر".into();
    assert!(validate_display(&joined_name).is_ok());
    for origin in [
        "http://accounts.example.test",
        "https://user@accounts.example.test",
        "https://accounts.example.test/path",
        "https://accounts.example.test?key=secret",
        "https://accounts.example.test#fragment",
        "https://ACCOUNTS.example.test",
    ] {
        let mut request = display();
        request.origin = origin.into();
        assert!(validate_display(&request).is_err(), "{origin}");
    }
    for label in [
        "",
        "\t",
        "Alice\nIssuer",
        "Alice\u{202e}different",
        "Alice\u{200b}",
    ] {
        let mut request = display();
        request.candidates[0].label = label.into();
        assert!(validate_display(&request).is_err());
    }
    for fingerprint in ["00", "Z".repeat(64).as_str(), "A".repeat(64).as_str()] {
        let mut request = display();
        request.candidates[0].fingerprint_sha256 = fingerprint.into();
        assert!(validate_display(&request).is_err());
    }
    let mut duplicate = display();
    duplicate.candidates[1] = duplicate.candidates[0].clone();
    assert!(validate_display(&duplicate).is_err());
    let mut too_many = display();
    too_many.candidates = vec![too_many.candidates[0].clone(); 65];
    assert!(validate_display(&too_many).is_err());
    for remaining in [0, 120_001] {
        let mut request = display();
        request.remaining_ms = remaining;
        assert!(validate_display(&request).is_err());
    }
}

#[tokio::test]
async fn choice_is_exact_one_use_and_invalid_answers_cannot_consume_it() {
    let (mut pending, mut receiver) = pending();
    let fingerprint = "a1".repeat(32);
    for (token, selected, proposed) in [
        ("old-ui-token", true, Some(fingerprint.as_str())),
        ("native-one-use-token", false, Some(fingerprint.as_str())),
        ("native-one-use-token", true, Some("forged-fingerprint")),
    ] {
        assert!(
            pending
                .finish(token, proposed, Instant::now(), selected)
                .is_err()
        );
        assert!(matches!(
            receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
    }
    assert!(
        pending
            .finish(
                "native-one-use-token",
                Some(&fingerprint),
                pending.deadline,
                true
            )
            .is_err()
    );
    pending
        .finish(
            "native-one-use-token",
            Some(&fingerprint),
            Instant::now(),
            true,
        )
        .unwrap();
    assert_eq!(receiver.await.unwrap(), Some(fingerprint.clone()));
    assert!(
        pending
            .finish(
                "native-one-use-token",
                Some(&fingerprint),
                Instant::now(),
                true
            )
            .is_err()
    );
}

#[tokio::test]
async fn cancellation_is_one_use_and_ambiguity_never_selects_a_certificate() {
    let (mut pending, receiver) = pending();
    let duplicate = pending.details.candidates[0].clone();
    pending.details.candidates.push(duplicate.clone());
    assert!(
        pending
            .finish(
                "native-one-use-token",
                Some(&duplicate.fingerprint_sha256),
                Instant::now(),
                true
            )
            .is_err()
    );
    pending
        .finish("native-one-use-token", None, Instant::now(), true)
        .unwrap();
    assert_eq!(receiver.await.unwrap(), None);
    assert!(
        pending
            .finish("native-one-use-token", None, Instant::now(), true)
            .is_err()
    );
}

#[tokio::test]
async fn selection_epoch_revokes_even_when_the_same_workspace_is_reselected() {
    let state = AppState::default();
    let (pending, _receiver) = pending();
    state
        .select_target(Some("workspace-native-only".into()))
        .await;
    assert!(
        state
            .selected_target("workspace-native-only")
            .await
            .is_none()
    );
    assert_eq!(
        *state.subscribe_selection().borrow(),
        pending.selection_epoch
    );
    assert!(pending.is_current(&state, Instant::now()));
    state.select_target(Some("other-workspace".into())).await;
    assert!(!pending.is_current(&state, Instant::now()));
    state
        .select_target(Some("workspace-native-only".into()))
        .await;
    assert!(!pending.is_current(&state, Instant::now()));
}

#[tokio::test]
async fn cancellation_and_deadlines_win_over_a_racing_choice_without_hanging() {
    for cause in ["selection", "view", "timeout", "closed", "choice"] {
        let (sender, receiver) = oneshot::channel();
        if cause == "closed" {
            drop(sender);
        } else {
            sender.send(Some("a1".repeat(32))).unwrap();
        }
        let invalidated = async {
            if cause != "selection" {
                std::future::pending::<()>().await;
            }
        };
        let view_failed = async {
            if cause != "view" {
                std::future::pending::<()>().await;
            }
        };
        let deadline = if cause == "timeout" {
            Instant::now()
        } else {
            Instant::now() + Duration::from_secs(30)
        };
        let answer = tokio::time::timeout(
            Duration::from_secs(1),
            decision(receiver, invalidated, view_failed, deadline),
        )
        .await
        .unwrap();
        match cause {
            "selection" | "view" => assert!(answer.is_err()),
            "timeout" | "closed" => assert_eq!(answer.unwrap(), None),
            "choice" => assert_eq!(answer.unwrap(), Some("a1".repeat(32))),
            _ => unreachable!(),
        }
    }
}

#[test]
fn document_receives_no_native_key_scope_or_generation_and_appearance_is_closed() {
    let (pending, _receiver) = pending();
    let json = serde_json::to_value(pending.details).unwrap();
    let keys: Vec<_> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["appearance", "candidates", "origin", "uiToken"]);
    assert!(!json.to_string().contains("workspace-native-only"));
    for value in [
        serde_json::json!({"colorTheme":"dark","darkPalette":"hacker","textSize":"large","uiToken":"forged"}),
        serde_json::json!({"colorTheme":"dark","darkPalette":"#ff0000","textSize":"large"}),
        serde_json::json!({"colorTheme":"remote","darkPalette":"black","textSize":"large"}),
    ] {
        assert!(serde_json::from_value::<ReviewAppearance>(value).is_err());
    }
}
