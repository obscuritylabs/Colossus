use super::*;

#[test]
fn renderer_paths_are_replaced_and_cancelled_dialogs_release_no_request() {
    let request = PluginManagementRequest::Package {
        directory: "/renderer/input".into(),
        output: "/renderer/output".into(),
    };
    let mut selections = [
        Some("/native/input".to_owned()),
        Some("/native/output".to_owned()),
    ]
    .into_iter();
    let mut dialogs = Vec::new();
    let selected = select_paths::<()>(request.clone(), false, |file, save| {
        dialogs.push((file, save));
        Ok(selections.next().flatten())
    })
    .expect("selection")
    .expect("request");
    assert_eq!(dialogs, [(false, false), (false, true)]);
    assert!(
        matches!(selected, PluginManagementRequest::Package { directory, output } if directory == "/native/input" && output == "/native/output")
    );
    let mut calls = 0;
    assert!(
        select_paths::<()>(request, false, |_, _| {
            calls += 1;
            Ok((calls == 1).then(|| "/native/input".into()))
        })
        .expect("cancelled")
        .is_none()
    );
}

#[test]
fn activation_cannot_request_a_file_dialog_or_change_the_exact_digest() {
    let request = PluginManagementRequest::Enable {
        name: "example".into(),
        digest: format!("sha256:{}", "a".repeat(64)),
        allow_untrusted: true,
    };
    let selected = select_paths::<()>(request.clone(), false, |_, _| {
        panic!("no filesystem operation")
    })
    .expect("selection")
    .expect("request");
    assert_eq!(
        serde_json::to_value(selected).expect("selected"),
        serde_json::to_value(request).expect("original")
    );
}
