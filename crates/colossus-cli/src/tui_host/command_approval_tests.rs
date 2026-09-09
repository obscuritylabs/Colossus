use super::*;

fn context() -> colossus_contracts::CommandApprovalContext {
    colossus_contracts::CommandApprovalContext {
        justification: "Check the workspace build.".into(),
        executable: "/bin/sh".into(),
        arguments: vec![
            "-c".into(),
            format!("echo 'two  spaces'; # {} COMMAND_TAIL", "x".repeat(70000)),
        ],
        working_directory: "/work/project".into(),
        redacted: true,
    }
}

fn inspect(document: &PresentationDocument) {
    for width in [32, 80] {
        let text = colossus_presentation::StyledDocumentRenderer::new(
            colossus_presentation::TerminalPreferences::default(),
            width,
        )
        .render(document)
        .iter()
        .map(colossus_presentation::StyledLine::plain_text)
        .collect::<Vec<_>>()
        .join("\n");
        let unwrapped = text
            .chars()
            .filter(|character| !character.is_whitespace() && *character != '│')
            .collect::<String>();
        assert!(
            unwrapped.contains("COMMAND_TAIL"),
            "tail missing at {width}"
        );
        assert!(text.contains("two  spaces"));
        assert!(text.contains("REDACTED") || text.contains("redacted"));
    }
}

#[tokio::test]
async fn both_hosts_preserve_full_command_and_explicit_decisions() {
    for answer in ["Allow once", "Deny"] {
        let router = Arc::new(TuiPromptRouter::default());
        let (sender, mut events) = mpsc::channel(1);
        router.install(Some(sender));
        let provider = TuiApprovalProvider::new(router, ApprovalMode::Ask);
        let task = tokio::spawn(async move {
            let request = colossus_policy::effect_request(
                colossus_policy::system_actor("test"),
                "shell.run",
                "/bin/sh",
                json!({"args": ["-c", "echo fixture"], "cwd": "/work/project"}),
            );
            let decision = PolicyDecision {
                decision_id: "decision".into(),
                policy_revision: "test-v1".into(),
                outcome: colossus_contracts::DecisionOutcome::RequireApproval,
                reason: "Explicit approval required".into(),
                obligations: colossus_contracts::PolicyObligations::default(),
            };
            provider
                .request_approval(&request, "binding", &decision, Some(&context()))
                .await
        });
        let HostEvent::Prompt(prompt) = events.recv().await.unwrap() else {
            panic!("prompt")
        };
        inspect(&prompt.document);
        assert!(!task.is_finished());
        prompt
            .response
            .send(PromptResponse::Answer(answer.into()))
            .unwrap();
        assert_eq!(
            task.await.unwrap().unwrap().is_some(),
            answer == "Allow once"
        );

        let (sender, mut events) = mpsc::channel(1);
        let handler = worker::TuiWorkerPromptHandler { sender };
        let task =
            tokio::spawn(async move {
                handler.prompt(WorkerPrompt {
                prompt_id: "worker-command".into(), kind: WorkerPromptKind::Approval,
                title: "Approval required".into(), question: "Explicit approval required".into(),
                choices: vec!["Allow once".into(), "Deny".into()], allow_free_form: false,
                details: json!({"action": "shell.run", "reason": "Explicit approval required"}),
                command_context: Some(context()),
            }).await
            });
        let HostEvent::Prompt(prompt) = events.recv().await.unwrap() else {
            panic!("prompt")
        };
        inspect(&prompt.document);
        assert!(!task.is_finished());
        prompt
            .response
            .send(PromptResponse::Answer(answer.into()))
            .unwrap();
        assert_eq!(task.await.unwrap().unwrap().as_deref(), Some(answer));
    }
}
