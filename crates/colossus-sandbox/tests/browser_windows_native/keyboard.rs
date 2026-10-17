//! Enter must finish one admitted gesture even when the page navigates during it.
use colossus_contracts::{BrowserAction, BrowserKey, BrowserTarget, BrowserUrl};
use colossus_ports::{
    BrowserDriver, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
};

pub async fn check(
    driver: &dyn BrowserDriver,
    request: &BrowserDriverOpenRequest,
    target: &mut BrowserTarget,
    control: &BrowserDriverControl,
    origin: &str,
) {
    for (case, path) in [
        "/keyboard-keydown",
        "/keyboard-form",
        "/keyboard-keyup",
        "/keyboard-mousedown",
    ]
    .into_iter()
    .enumerate()
    {
        let index = 500 + case as u32 * 100;
        eprintln!("CHECK input navigation {path} at step {index}");
        let opened = driver
            .execute(
                super::exercise::command(
                    request,
                    target,
                    BrowserAction::Navigate {
                        url: BrowserUrl::parse(format!("{origin}{path}")).unwrap(),
                    },
                    index,
                ),
                control,
            )
            .await
            .expect("keyboard fixture navigation");
        target.document_id = opened.tab.document_id;
        let snapshot = super::exercise::snapshot(driver, request, target, control, index + 1).await;
        target.document_id = snapshot.tab.document_id;
        let field = snapshot
            .snapshot
            .unwrap()
            .nodes
            .into_iter()
            .find(|node| node.name == "Keyboard query")
            .unwrap()
            .element;
        let filled = driver
            .execute(
                super::exercise::command(
                    request,
                    target,
                    BrowserAction::Fill {
                        element: field,
                        text: "owned keyboard query".into(),
                    },
                    index + 2,
                ),
                control,
            )
            .await
            .expect("fill keyboard fixture");
        target.document_id = filled.tab.document_id;
        if path == "/keyboard-form" {
            driver
                .execute(
                    super::exercise::command(
                        request,
                        target,
                        BrowserAction::Press {
                            key: BrowserKey::Space,
                        },
                        index + 45,
                    ),
                    control,
                )
                .await
                .expect("Space character input");
        }
        let snapshot = super::exercise::snapshot(driver, request, target, control, index + 3).await;
        target.document_id = snapshot.tab.document_id;
        if path == "/keyboard-form" {
            let field = snapshot
                .snapshot
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .find(|node| node.name == "Keyboard query")
                .unwrap();
            assert_eq!(
                field.value.as_deref(),
                Some("owned keyboard query "),
                "Space must insert its character"
            );
        }
        let prior_target = target.clone();
        let action = if path == "/keyboard-mousedown" {
            BrowserAction::Click {
                element: snapshot
                    .snapshot
                    .unwrap()
                    .nodes
                    .into_iter()
                    .find(|node| node.name == "Keyboard query")
                    .unwrap()
                    .element,
            }
        } else {
            BrowserAction::Press {
                key: BrowserKey::Enter,
            }
        };
        let pressed = driver
            .execute(
                super::exercise::command(request, target, action, index + 4),
                control,
            )
            .await;
        assert!(
            pressed.is_ok(),
            "Input gesture must acknowledge {path}: {pressed:?}"
        );
        target.document_id = pressed.unwrap().tab.document_id;
        let expected = format!("Keyboard submission {}", case + 1);
        let mut submitted = false;
        for attempt in 0..40 {
            let snapshot =
                super::exercise::snapshot(driver, request, target, control, index + 5 + attempt)
                    .await;
            target.document_id = snapshot.tab.document_id;
            if snapshot.tab.title == expected {
                submitted = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(submitted, "Input gesture must submit {path} exactly once");
        assert_eq!(
            driver
                .execute(
                    super::exercise::command(
                        request,
                        &prior_target,
                        BrowserAction::Press {
                            key: BrowserKey::Enter,
                        },
                        index + 60
                    ),
                    control
                )
                .await,
            Err(BrowserDriverError::Stale),
            "the next action must not reuse the navigated document"
        );
        eprintln!("PASS input gesture submits {path} exactly once with acknowledged navigation");
    }
    let restored = driver
        .execute(
            super::exercise::command(
                request,
                target,
                BrowserAction::Navigate {
                    url: BrowserUrl::parse(format!("{origin}/fixture")).unwrap(),
                },
                850,
            ),
            control,
        )
        .await
        .expect("restore ordinary native fixture");
    target.document_id = restored.tab.document_id;
}
