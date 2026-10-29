use super::*;
use colossus_ports::RunInteractionContext;

#[tokio::test]
async fn child_context_follows_its_exact_parent_instead_of_the_draining_task() {
    let sinks = Arc::new(StdMutex::new(HashMap::new()));
    let mut senders = Vec::new();
    let mut receivers = Vec::new();
    for parent in ["parent-a", "parent-b"] {
        let (sender, receiver) = mpsc::channel(1);
        senders.push(sender.clone());
        receivers.push(receiver);
        let mut observer = BufferedRunObserver {
            sender,
            sinks: Arc::clone(&sinks),
            run_id: None,
        };
        RunInteractionContext::default()
            .with(Arc::new(parent.to_owned()))
            .scope(observer.observe(RunEventEnvelope {
                schema_version: 1,
                run_id: parent.into(),
                session_id: format!("session-{parent}"),
                event: RunEvent::Provider {
                    event: ProviderEvent::ModelDelta {
                        text: "started".into(),
                    },
                },
            }))
            .await
            .expect("parent registered");
    }
    RunInteractionContext::default()
        .with(Arc::new("unrelated-drainer".to_owned()))
        .scope(async {
            let mut children = Vec::new();
            for parent in ["parent-a", "parent-b", "unknown"] {
                let context = subagent_interaction_context(&sinks, parent);
                children.push(tokio::spawn(context.scope(async move {
                    let actual = RunInteractionContext::current::<String>();
                    assert_eq!(
                        actual.as_deref().map(String::as_str),
                        (parent != "unknown").then_some(parent)
                    );
                })));
            }
            for child in children {
                child.await.expect("scoped child");
            }
        })
        .await;
    assert!(RunInteractionContext::current::<String>().is_none());

    drop(RunEventRegistration {
        sinks: Arc::clone(&sinks),
        sender: senders.remove(0),
    });
    assert!(!sinks.lock().expect("sinks").contains_key("parent-a"));
    assert!(sinks.lock().expect("sinks").contains_key("parent-b"));
    subagent_interaction_context(&sinks, "parent-a")
        .scope(async {
            assert!(RunInteractionContext::current::<String>().is_none());
        })
        .await;
}
