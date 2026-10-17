//! Negative authority and freshness cases for multi-target and pixel actions.
use super::*;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn annotations_preserve_snapshot_but_reject_foreign_targets_and_unbounded_marks() {
    let driver = Arc::new(TestDriver::default());
    let (coordinator, actor, session, lease) = setup(driver.clone()).await;
    let target = target(&session);
    let observation = snapshot(&coordinator, &actor, &lease, &target).await;
    let snapshot = observation.snapshot.unwrap();
    let element = snapshot.nodes[0].element.clone();
    let mark = BrowserAnnotation {
        target: BrowserAnnotationTarget::Element {
            element: element.clone(),
        },
        label: "Target".into(),
        color: BrowserAnnotationColor::Blue,
    };
    let annotation = BrowserAction::Annotate {
        snapshot_id: snapshot.snapshot_id.clone(),
        viewport: BrowserViewport {
            width: 800,
            height: 600,
        },
        annotations: vec![mark.clone()],
        ttl_ms: 1000,
    };
    let before = driver.executions.load(Ordering::SeqCst);
    let mut foreign = element.clone();
    foreign.document_id = BrowserDocumentId::parse("bd_ffffffffffffffffffffffffffffffff").unwrap();
    assert!(matches!(
        coordinator
            .execute(
                &actor,
                &lease,
                &target,
                BrowserAction::Drag {
                    element: element.clone(),
                    to: foreign
                },
                &RunControl::default()
            )
            .await,
        Err(BrowserError::StaleDocument)
    ));
    let mut bad = mark.clone();
    bad.label = "x".repeat(161);
    for annotations in [
        vec![],
        vec![mark.clone(); 17],
        vec![bad],
        vec![BrowserAnnotation {
            target: BrowserAnnotationTarget::Rectangle {
                rect: BrowserRectangle {
                    x: 790,
                    y: 0,
                    width: 11,
                    height: 20,
                },
            },
            ..mark.clone()
        }],
    ] {
        assert!(matches!(
            coordinator
                .execute(
                    &actor,
                    &lease,
                    &target,
                    BrowserAction::Annotate {
                        snapshot_id: snapshot.snapshot_id.clone(),
                        viewport: BrowserViewport {
                            width: 800,
                            height: 600
                        },
                        annotations,
                        ttl_ms: 1000
                    },
                    &RunControl::default()
                )
                .await,
            Err(BrowserError::InvalidArguments)
        ));
    }
    assert_eq!(driver.executions.load(Ordering::SeqCst), before);
    coordinator
        .execute(&actor, &lease, &target, annotation, &RunControl::default())
        .await
        .unwrap();
    coordinator
        .execute(
            &actor,
            &lease,
            &target,
            BrowserAction::AnnotationClear {},
            &RunControl::default(),
        )
        .await
        .unwrap();
    coordinator
        .execute(
            &actor,
            &lease,
            &target,
            BrowserAction::Hover {
                element: element.clone(),
            },
            &RunControl::default(),
        )
        .await
        .unwrap();
    assert!(matches!(
        coordinator
            .execute(
                &actor,
                &lease,
                &target,
                BrowserAction::DoubleClick { element },
                &RunControl::default()
            )
            .await,
        Err(BrowserError::StaleDocument)
    ));
}

#[tokio::test]
async fn pixel_input_requires_current_snapshot_and_every_point_inside_the_viewport() {
    let driver = Arc::new(TestDriver::default());
    let (coordinator, actor, session, lease) = setup(driver.clone()).await;
    let target = target(&session);
    let old = snapshot(&coordinator, &actor, &lease, &target)
        .await
        .snapshot
        .unwrap()
        .snapshot_id;
    let current = snapshot(&coordinator, &actor, &lease, &target)
        .await
        .snapshot
        .unwrap()
        .snapshot_id;
    let viewport = BrowserViewport {
        width: 800,
        height: 600,
    };
    let before = driver.executions.load(Ordering::SeqCst);
    assert!(matches!(
        coordinator
            .execute(
                &actor,
                &lease,
                &target,
                BrowserAction::ClickAt {
                    snapshot_id: old,
                    viewport,
                    point: BrowserPoint { x: 10, y: 10 },
                    button: BrowserMouseButton::Left,
                    click_count: 1
                },
                &RunControl::default()
            )
            .await,
        Err(BrowserError::StaleDocument)
    ));
    for path in [
        vec![BrowserPoint { x: 10, y: 10 }],
        vec![BrowserPoint { x: 10, y: 10 }; 65],
        vec![
            BrowserPoint { x: 10, y: 10 },
            BrowserPoint { x: 800, y: 10 },
        ],
    ] {
        assert!(matches!(
            coordinator
                .execute(
                    &actor,
                    &lease,
                    &target,
                    BrowserAction::DragAt {
                        snapshot_id: current.clone(),
                        viewport,
                        path
                    },
                    &RunControl::default()
                )
                .await,
            Err(BrowserError::InvalidArguments)
        ));
    }
    assert_eq!(driver.executions.load(Ordering::SeqCst), before);
    coordinator
        .execute(
            &actor,
            &lease,
            &target,
            BrowserAction::ClickAt {
                snapshot_id: current.clone(),
                viewport,
                point: BrowserPoint { x: 799, y: 599 },
                button: BrowserMouseButton::Right,
                click_count: 2,
            },
            &RunControl::default(),
        )
        .await
        .unwrap();
    assert!(matches!(
        coordinator
            .execute(
                &actor,
                &lease,
                &target,
                BrowserAction::DragAt {
                    snapshot_id: current,
                    viewport,
                    path: vec![BrowserPoint { x: 10, y: 10 }; 2]
                },
                &RunControl::default()
            )
            .await,
        Err(BrowserError::StaleDocument)
    ));
}
