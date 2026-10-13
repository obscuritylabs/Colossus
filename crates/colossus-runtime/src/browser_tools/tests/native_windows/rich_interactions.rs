//! Real page event receipts and native annotation pixels at 200% scaling.
use super::*;

fn element_args(opened: &Value, observed: &BrowserObservation, name: &str) -> Value {
    let node = observed
        .snapshot
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|node| node.name == name)
        .unwrap();
    let mut args = observed_target(opened, observed);
    args["snapshot_id"] = json!(node.element.snapshot_id);
    args["element_id"] = json!(node.element.element_id);
    args
}
fn coordinates(opened: &Value, observed: &BrowserObservation) -> Value {
    let mut args = observed_target(opened, observed);
    args["snapshot_id"] = json!(observed.snapshot.as_ref().unwrap().snapshot_id);
    args["viewport"] = json!({"width":1280,"height":960});
    args
}
fn capture(publisher: &CapturePublisher) -> image::RgbaImage {
    image::load_from_memory_with_format(
        publisher.0.lock().unwrap().as_ref().unwrap().as_slice(),
        image::ImageFormat::Png,
    )
    .unwrap()
    .to_rgba8()
}
async fn viewport(
    presenter: &colossus_browser_presentation::PresentationClient,
    request: &BrowserDriverOpenRequest,
    opened: &Value,
    observed: &BrowserObservation,
    generation: u64,
) -> colossus_browser_presentation::Lease {
    presenter
        .configure(Configure {
            session: request.session_id.clone(),
            target: BrowserTarget {
                tab_id: observed.tab.tab_id.clone(),
                document_id: observed.tab.document_id.clone(),
            },
            control_generation: opened["control_generation"].as_u64().unwrap(),
            viewport_generation: generation,
            width: 640,
            height: 480,
            scale_milli: 2000,
            lease_ms: 1500,
        })
        .await
        .unwrap()
}
pub(super) async fn current_fixture_view(
    presenter: &colossus_browser_presentation::PresentationClient,
    request: &BrowserDriverOpenRequest,
    opened: &Value,
    observed: &BrowserObservation,
    digest: [u8; 32],
    generation: &mut u64,
) {
    let admitted = *generation;
    *generation += 1;
    let lease = viewport(presenter, request, opened, observed, admitted).await;
    let mut codec = FrameCodec::new(presenter.surface_key(), digest, lease).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        presenter.renew(lease, 1500).await.unwrap();
        if let Some(bytes) = presenter.next_frame(lease).await.unwrap() {
            codec.decode(bytes).unwrap();
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "current fixture document never painted"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    executor: &BrowserToolExecutor,
    context: &ExecutionContext,
    opened: &Value,
    mut observed: BrowserObservation,
    presenter: &colossus_browser_presentation::PresentationClient,
    digest: [u8; 32],
    request: &BrowserDriverOpenRequest,
    publisher: &CapturePublisher,
    generation: &mut u64,
) -> BrowserObservation {
    for (tool, name, result) in [
        ("browser.hover", "Hover target", "Hover received"),
        ("browser.double_click", "Double target", "Double received"),
        ("browser.right_click", "Context target", "Right received"),
    ] {
        current_fixture_view(presenter, request, opened, &observed, digest, generation).await;
        call(
            executor,
            context,
            tool,
            element_args(opened, &observed, name),
        )
        .await
        .unwrap();
        observed = observe(
            executor,
            context,
            observed_target(opened, &observed),
            result,
        )
        .await;
    }
    let mut coordinate = coordinates(opened, &observed);
    coordinate["point"] = json!({"x":300,"y":540});
    let mut resized = coordinate.clone();
    resized["viewport"]["width"] = json!(1279);
    assert!(
        call(executor, context, "browser.click_at", resized)
            .await
            .is_err()
    );
    current_fixture_view(presenter, request, opened, &observed, digest, generation).await;
    call(executor, context, "browser.click_at", coordinate.clone())
        .await
        .unwrap();
    assert!(
        call(executor, context, "browser.click_at", coordinate)
            .await
            .is_err()
    );
    observed = observe(
        executor,
        context,
        observed_target(opened, &observed),
        "Coordinate received",
    )
    .await;
    let mut drag = coordinates(opened, &observed);
    drag["path"] = json!([{"x":240,"y":660},{"x":300,"y":660},{"x":420,"y":680}]);
    current_fixture_view(presenter, request, opened, &observed, digest, generation).await;
    call(executor, context, "browser.drag_at", drag)
        .await
        .unwrap();
    observed = observe(
        executor,
        context,
        observed_target(opened, &observed),
        "Pointer drag received",
    )
    .await;
    let mut drag = element_args(opened, &observed, "HTML drag source");
    let to = observed
        .snapshot
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|n| n.name == "HTML drop target")
        .unwrap();
    drag["to"] = json!(to.element);
    current_fixture_view(presenter, request, opened, &observed, digest, generation).await;
    call(executor, context, "browser.drag", drag).await.unwrap();
    observed = observe(
        executor,
        context,
        observed_target(opened, &observed),
        "HTML drop received",
    )
    .await;
    // The background frame remains owned during the base viewer acceptance.
    // Broader inspection cannot claim one pause covers a separate child target.
    current_fixture_view(presenter, request, opened, &observed, digest, generation).await;
    observed = serde_json::from_value(
        call(
            executor,
            context,
            "browser.click",
            element_args(opened, &observed, "Add child frame"),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    observed = observe(
        executor,
        context,
        observed_target(opened, &observed),
        "Child frame added",
    )
    .await;
    let mut child_inspect = observed_target(opened, &observed);
    child_inspect["snapshot_id"] = json!(observed.snapshot.as_ref().unwrap().snapshot_id);
    child_inspect["expression"] = json!("document.title");
    assert!(
        call(executor, context, "browser.evaluate", child_inspect.clone())
            .await
            .is_err()
    );
    child_inspect.as_object_mut().unwrap().remove("expression");
    child_inspect["method"] = json!("DOM.getDocument");
    child_inspect["params"] = json!({});
    assert!(
        call(executor, context, "browser.cdp", child_inspect)
            .await
            .is_err()
    );
    current_fixture_view(presenter, request, opened, &observed, digest, generation).await;
    observed = serde_json::from_value(
        call(
            executor,
            context,
            "browser.click",
            element_args(opened, &observed, "Remove child frame"),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    observed = observe(
        executor,
        context,
        observed_target(opened, &observed),
        "Core Windows fixture",
    )
    .await;
    let mut mark = coordinates(opened, &observed);
    let mut inspect = observed_target(opened, &observed);
    inspect["snapshot_id"] = json!(observed.snapshot.as_ref().unwrap().snapshot_id);
    inspect["expression"] = json!("({title: document.title, answer: 6 * 7})");
    let evaluated = call(executor, context, "browser.evaluate", inspect.clone())
        .await
        .unwrap();
    assert_eq!(evaluated["inspection"]["value"]["answer"], 42);
    assert_eq!(
        evaluated["inspection"]["value"]["title"],
        "Core Windows fixture"
    );
    inspect["expression"] = json!("null");
    let null = call(executor, context, "browser.evaluate", inspect.clone())
        .await
        .unwrap();
    assert!(null["inspection"]["value"].is_null());
    inspect["expression"] = json!("document.body.setAttribute('data-escaped','yes')");
    assert!(
        call(executor, context, "browser.evaluate", inspect.clone())
            .await
            .is_err()
    );
    inspect.as_object_mut().unwrap().remove("expression");
    inspect["method"] = json!("Page.getLayoutMetrics");
    inspect["params"] = json!({});
    let metrics = call(executor, context, "browser.cdp", inspect.clone())
        .await
        .unwrap();
    assert!(
        metrics["inspection"]["value"]["cssLayoutViewport"]["clientWidth"]
            .as_u64()
            .unwrap()
            > 0
    );
    inspect["params"] = json!({"targetId":"foreign"});
    assert!(
        call(executor, context, "browser.cdp", inspect.clone())
            .await
            .is_err()
    );
    inspect["method"] = json!("DOM.getDocument");
    inspect["params"] = json!({"depth":0});
    let document = call(executor, context, "browser.cdp", inspect.clone())
        .await
        .unwrap();
    let root = document["inspection"]["value"]["root"]["nodeId"]
        .as_u64()
        .unwrap();
    inspect["method"] = json!("DOM.querySelector");
    inspect["params"] = json!({"nodeId":root,"selector":"#coordinate-button"});
    let node = call(executor, context, "browser.cdp", inspect.clone())
        .await
        .unwrap();
    let node = node["inspection"]["value"]["nodeId"].as_u64().unwrap();
    assert!(node > 0);
    for method in [
        "DOM.getAttributes",
        "DOM.getBoxModel",
        "DOM.getOuterHTML",
        "CSS.getComputedStyleForNode",
    ] {
        inspect["method"] = json!(method);
        inspect["params"] = json!({"nodeId":node});
        let value = call(executor, context, "browser.cdp", inspect.clone())
            .await
            .unwrap_or_else(|error| {
                panic!("real Windows inspection method {method} failed: {error}")
            });
        assert!(!value["inspection"]["value"].is_null());
    }
    inspect["method"] = json!("DOM.querySelectorAll");
    inspect["params"] = json!({"nodeId":root,"selector":"button"});
    let nodes = call(executor, context, "browser.cdp", inspect.clone())
        .await
        .unwrap();
    assert!(
        !nodes["inspection"]["value"]["nodeIds"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    for method in ["CSS.getMediaQueries", "Performance.getMetrics"] {
        inspect["method"] = json!(method);
        inspect["params"] = json!({});
        let value = call(executor, context, "browser.cdp", inspect.clone())
            .await
            .unwrap();
        assert!(!value["inspection"]["value"].is_null());
    }
    mark["annotations"] = json!([{"target":{"kind":"rectangle","rect":{"x":200,"y":500,"width":200,"height":80}},"label":"Owned target — annotation","color":"red"}]);
    mark["ttl_ms"] = json!(2000);
    call(executor, context, "browser.annotate", mark.clone())
        .await
        .unwrap();
    call(
        executor,
        context,
        "browser.screenshot",
        observed_target(opened, &observed),
    )
    .await
    .unwrap();
    let plain = capture(publisher);
    call(
        executor,
        context,
        "browser.screenshot.annotated",
        observed_target(opened, &observed),
    )
    .await
    .unwrap();
    let annotated = capture(publisher);
    assert_eq!(annotated.dimensions(), plain.dimensions());
    assert_ne!(annotated.get_pixel(200, 500), plain.get_pixel(200, 500));
    assert_eq!(annotated.get_pixel(200, 500).0, [200, 35, 45, 255]);
    if let Some(path) = std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ANNOTATION_EVIDENCE") {
        annotated.save(path).unwrap();
    }
    let admitted = *generation;
    *generation += 1;
    let lease = viewport(presenter, request, opened, &observed, admitted).await;
    let mut codec = FrameCodec::new(presenter.surface_key(), digest, lease).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        presenter.renew(lease, 1500).await.unwrap();
        if let Some(bytes) = presenter.next_frame(lease).await.unwrap() {
            let frame = codec.decode(bytes).unwrap();
            let index = (500 * 1280 + 200) * 4;
            if frame.pixels[index..index + 4] == [45, 35, 200, 255] {
                break;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "annotated native frame absent"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    call(
        executor,
        context,
        "browser.annotation.clear",
        observed_target(opened, &observed),
    )
    .await
    .unwrap();
    assert!(
        call(
            executor,
            context,
            "browser.screenshot.annotated",
            observed_target(opened, &observed)
        )
        .await
        .is_err()
    );
    mark["ttl_ms"] = json!(80);
    call(executor, context, "browser.annotate", mark.clone())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        call(
            executor,
            context,
            "browser.screenshot.annotated",
            observed_target(opened, &observed)
        )
        .await
        .is_err()
    );
    mark["ttl_ms"] = json!(2000);
    call(executor, context, "browser.annotate", mark)
        .await
        .unwrap();
    let admitted = *generation;
    *generation += 1;
    presenter
        .configure(Configure {
            session: request.session_id.clone(),
            target: BrowserTarget {
                tab_id: observed.tab.tab_id.clone(),
                document_id: observed.tab.document_id.clone(),
            },
            control_generation: opened["control_generation"].as_u64().unwrap(),
            viewport_generation: admitted,
            width: 600,
            height: 450,
            scale_milli: 2000,
            lease_ms: 1500,
        })
        .await
        .unwrap();
    assert!(
        call(
            executor,
            context,
            "browser.screenshot.annotated",
            observed_target(opened, &observed)
        )
        .await
        .is_err()
    );
    eprintln!(
        "PASS real Windows Core hover, double/right click, DPI coordinate click, canvas/HTML drag, clean/annotated PNG, authenticated native annotation frame, clear/expiry/resize fences and child-document rejection"
    );
    observed
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn inspect_adversarial(
    executor: &BrowserToolExecutor,
    context: &ExecutionContext,
    opened: &Value,
    mut observed: BrowserObservation,
    origin: &str,
    presenter: &colossus_browser_presentation::PresentationClient,
    request: &BrowserDriverOpenRequest,
    digest: [u8; 32],
    generation: &mut u64,
) -> BrowserObservation {
    for (path, title, expression) in [
        (
            "inspection-timer",
            "Inspection timer",
            "({type:raceType,value:raceValue})",
        ),
        (
            "inspection-shadow",
            "Inspection shadow",
            "window.closedCredential.value",
        ),
        (
            "inspection-dormant",
            "Inspection dormant",
            "(() => { const until = Date.now() + 750; while (Date.now() < until) {} return {title:document.title,answer:42}; })()",
        ),
    ] {
        let mut navigate = observed_target(opened, &observed);
        navigate["url"] = json!(format!("{origin}/{path}"));
        observed = serde_json::from_value(
            call(executor, context, "browser.navigate", navigate)
                .await
                .unwrap(),
        )
        .unwrap();
        observed = observe(executor, context, observed_target(opened, &observed), title).await;
        current_fixture_view(presenter, request, opened, &observed, digest, generation).await;
        let mut inspect = observed_target(opened, &observed);
        inspect["snapshot_id"] = json!(observed.snapshot.as_ref().unwrap().snapshot_id);
        inspect["expression"] = json!(expression);
        if path == "inspection-timer" {
            for (button, protected) in [("Freeze ordinary", false), ("Freeze protected", true)] {
                current_fixture_view(presenter, request, opened, &observed, digest, generation)
                    .await;
                call(
                    executor,
                    context,
                    "browser.click",
                    element_args(opened, &observed, button),
                )
                .await
                .unwrap();
                observed = observe(
                    executor,
                    context,
                    observed_target(opened, &observed),
                    if protected {
                        "Frozen protected"
                    } else {
                        "Frozen ordinary"
                    },
                )
                .await;
                inspect = observed_target(opened, &observed);
                inspect["snapshot_id"] = json!(observed.snapshot.as_ref().unwrap().snapshot_id);
                inspect["expression"] = json!(expression);
                let result = call(executor, context, "browser.evaluate", inspect.clone()).await;
                if protected {
                    let error = result.unwrap_err();
                    assert!(!error.to_string().contains("owned-race-secret"));
                } else {
                    let result = result.unwrap();
                    assert_eq!(result["inspection"]["value"]["type"], "text");
                    assert_eq!(result["inspection"]["value"]["value"], "public");
                }
            }
            current_fixture_view(presenter, request, opened, &observed, digest, generation).await;
            call(
                executor,
                context,
                "browser.click",
                element_args(opened, &observed, "Resume race"),
            )
            .await
            .unwrap();
            observed = observe(
                executor,
                context,
                observed_target(opened, &observed),
                "Race running",
            )
            .await;
            let tick = |observation: &BrowserObservation| {
                observation
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .find_map(|node| {
                        node.name
                            .strip_prefix("Race tick ")
                            .and_then(|tick| tick.parse::<u64>().ok())
                    })
                    .expect("fixture exposes the running race timer")
            };
            let before = tick(&observed);
            inspect = observed_target(opened, &observed);
            inspect["snapshot_id"] = json!(observed.snapshot.as_ref().unwrap().snapshot_id);
            inspect["expression"] = json!(expression);
            for _ in 0..64 {
                match call(executor, context, "browser.evaluate", inspect.clone()).await {
                    Ok(result) => {
                        assert_eq!(result["inspection"]["value"]["type"], "text");
                        assert_eq!(result["inspection"]["value"]["value"], "public");
                    }
                    Err(error) => {
                        assert!(!error.to_string().contains("owned-race-secret"));
                    }
                }
                tokio::time::sleep(Duration::from_millis(7)).await;
            }
            observed = observe(executor, context, observed_target(opened, &observed), title).await;
            assert!(
                tick(&observed) > before,
                "inspection left page JavaScript paused"
            );
        } else if path == "inspection-shadow" {
            let error = call(executor, context, "browser.evaluate", inspect.clone())
                .await
                .unwrap_err();
            assert!(!error.to_string().contains("owned-shadow-secret"));
            inspect.as_object_mut().unwrap().remove("expression");
            inspect["method"] = json!("DOM.getDocument");
            inspect["params"] = json!({});
            assert!(
                call(executor, context, "browser.cdp", inspect)
                    .await
                    .is_err()
            );
        } else {
            let result = call(executor, context, "browser.evaluate", inspect)
                .await
                .unwrap();
            assert_eq!(result["inspection"]["value"]["title"], title);
            assert_eq!(result["inspection"]["value"]["answer"], 42);
        }
    }
    eprintln!(
        "PASS real Windows paused inspection guard, adversarial DOM timer, debugger restoration, closed shadow credentials and dormant page evaluation"
    );
    observed
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn inspect_navigation_restoration(
    executor: &BrowserToolExecutor,
    context: &ExecutionContext,
    opened: &Value,
    observed: BrowserObservation,
    origin: &str,
) {
    let mut navigate = observed_target(opened, &observed);
    navigate["url"] = json!(format!("{origin}/inspection-navigation"));
    let observed: BrowserObservation = serde_json::from_value(
        call(executor, context, "browser.navigate", navigate)
            .await
            .unwrap(),
    )
    .unwrap();
    let observed = observe(
        executor,
        context,
        observed_target(opened, &observed),
        "Inspection navigation ready",
    )
    .await;
    let expression = "(() => { const until = Date.now() + 750; while (Date.now() < until) {} return {title:document.title,answer:42}; })()";
    let mut inspect = observed_target(opened, &observed);
    inspect["snapshot_id"] = json!(observed.snapshot.as_ref().unwrap().snapshot_id);
    inspect["expression"] = json!(expression);
    if let Ok(result) = call(executor, context, "browser.evaluate", inspect).await {
        // The navigation may not yet be browser-observed when completion
        // commits. A released value must nevertheless keep its old identity;
        // an observed navigation must fail rather than relabel that value.
        assert_eq!(
            result["tab"]["document_id"],
            json!(observed.tab.document_id)
        );
        assert_eq!(
            result["inspection"]["value"]["title"],
            "Inspection navigation"
        );
        assert_eq!(result["inspection"]["value"]["answer"], 42);
    }
    eprintln!(
        "PASS real Windows inspection retains its guarded document across debugger restoration and timed navigation"
    );
}
