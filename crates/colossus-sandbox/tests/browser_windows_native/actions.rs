//! Functional typed automation on the real supervised Windows CEF instance.
use colossus_contracts::{
    BrowserAction, BrowserDocumentId, BrowserKey, BrowserTabId, BrowserTabSummary, BrowserTarget,
    BrowserUrl, BrowserWaitCondition,
};
use colossus_ports::{BrowserDriver, BrowserDriverControl, BrowserDriverOpenRequest};

pub async fn check(
    driver: &dyn BrowserDriver,
    request: &BrowserDriverOpenRequest,
    target: &mut BrowserTarget,
    control: &BrowserDriverControl,
    origin: &str,
) {
    let observed = super::exercise::snapshot(driver, request, target, control, 300).await;
    target.document_id = observed.tab.document_id;
    let field = observed
        .snapshot
        .unwrap()
        .nodes
        .into_iter()
        .find(|node| node.name == "Ordinary native input")
        .unwrap()
        .element;
    let filled = driver
        .execute(
            super::exercise::command(
                request,
                target,
                BrowserAction::Fill {
                    element: field,
                    text: "Windows agent Ω😀".into(),
                },
                301,
            ),
            control,
        )
        .await
        .expect("real agent field fill");
    target.document_id = filled.tab.document_id;
    let observed = super::exercise::snapshot(driver, request, target, control, 302).await;
    target.document_id = observed.tab.document_id;
    assert!(
        observed
            .snapshot
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|node| node.value.as_deref() == Some("Windows agent Ω😀"))
    );
    driver
        .execute(
            super::exercise::command(
                request,
                target,
                BrowserAction::Press {
                    key: BrowserKey::Backspace,
                },
                303,
            ),
            control,
        )
        .await
        .expect("real safe guest key");
    let observed = super::exercise::snapshot(driver, request, target, control, 304).await;
    target.document_id = observed.tab.document_id;
    assert!(
        observed
            .snapshot
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|node| node.value.as_deref() == Some("Windows agent Ω")),
        "Chromium must process the key against the actual focused field"
    );
    let field = observed
        .snapshot
        .unwrap()
        .nodes
        .into_iter()
        .find(|node| node.name == "Ordinary native input")
        .unwrap()
        .element;
    driver
        .execute(
            super::exercise::command(
                request,
                target,
                BrowserAction::Wait {
                    condition: BrowserWaitCondition::ElementVisible { element: field },
                    timeout_ms: 1000,
                },
                305,
            ),
            control,
        )
        .await
        .expect("current native element visibility wait");
    let observed = super::exercise::snapshot(driver, request, target, control, 306).await;
    target.document_id = observed.tab.document_id;
    let choice = observed
        .snapshot
        .unwrap()
        .nodes
        .into_iter()
        .find(|node| node.name == "Native Windows choice")
        .unwrap()
        .element;
    driver
        .execute(
            super::exercise::command(
                request,
                target,
                BrowserAction::Select {
                    element: choice,
                    values: vec!["two".into()],
                },
                307,
            ),
            control,
        )
        .await
        .expect("real native select control");
    let mut selected = false;
    for index in 330..350 {
        let observed = super::exercise::snapshot(driver, request, target, control, index).await;
        target.document_id = observed.tab.document_id;
        if observed.tab.title == "Owned Windows selection two" {
            selected = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        selected,
        "actual native change event must observe the selected option"
    );
    for (index, action) in [
        (
            310,
            BrowserAction::Navigate {
                url: BrowserUrl::parse(format!("{origin}/second")).unwrap(),
            },
        ),
        (311, BrowserAction::Back {}),
        (312, BrowserAction::Forward {}),
        (313, BrowserAction::Reload {}),
        (314, BrowserAction::Stop {}),
        (315, BrowserAction::Scroll { x: 0, y: 100 }),
        (
            316,
            BrowserAction::Wait {
                condition: BrowserWaitCondition::Load {},
                timeout_ms: 1000,
            },
        ),
    ] {
        let observed =
            super::exercise::snapshot(driver, request, target, control, index + 1000).await;
        target.document_id = observed.tab.document_id;
        let result = driver
            .execute(
                super::exercise::command(request, target, action, index),
                control,
            )
            .await
            .expect("real typed navigation/history/scroll/load operation");
        target.document_id = result.tab.document_id;
    }
    let first = target.clone();
    let reserved = BrowserTabSummary {
        tab_id: BrowserTabId::parse("bt_03000000000000000000000000000001").unwrap(),
        document_id: BrowserDocumentId::parse("bd_03000000000000000000000000000001").unwrap(),
        origin: None,
        title: String::new(),
    };
    let mut open = super::exercise::command(
        request,
        target,
        BrowserAction::TabOpen {
            url: Some(BrowserUrl::parse(format!("{origin}/second-tab")).unwrap()),
        },
        320,
    );
    open.new_tab = Some(reserved.clone());
    open.next_document_id = reserved.document_id.clone();
    let opened = driver
        .execute(open, control)
        .await
        .expect("real tab allocation");
    assert_eq!(opened.tab.tab_id, reserved.tab_id);
    assert_eq!(opened.tab.origin.as_ref().unwrap().as_str(), origin);
    let second = BrowserTarget {
        tab_id: opened.tab.tab_id,
        document_id: opened.tab.document_id,
    };
    let selected = driver
        .execute(
            super::exercise::command(
                request,
                &first,
                BrowserAction::TabSelect {
                    tab_id: first.tab_id.clone(),
                },
                321,
            ),
            control,
        )
        .await
        .expect("real tab selection");
    assert_eq!(selected.tab.tab_id, first.tab_id);
    let closed = driver
        .execute(
            super::exercise::command(
                request,
                &second,
                BrowserAction::TabClose {
                    tab_id: second.tab_id.clone(),
                },
                322,
            ),
            control,
        )
        .await
        .expect("real native tab close");
    assert_eq!(closed.tab.tab_id, second.tab_id);
    *target = first;
    let _ = super::exercise::snapshot(driver, request, target, control, 323).await;
    eprintln!("native Windows typed automation and tab allocation/selection/close passed");
}
