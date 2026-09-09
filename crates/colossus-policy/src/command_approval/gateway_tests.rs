use super::*;
use colossus_contracts::{
    ApprovalProof, CommandApprovalContext, CommandIntent, EffectRequest, PolicyDecision,
};
use serde_json::json;

struct AuditedApproval {
    journal: Arc<dyn EventJournal>,
    executor: Arc<CountingExecutor>,
    changed_field: &'static str,
    calls: AtomicUsize,
}

struct DigestOnlyPolicy(BuiltInPolicy);

#[async_trait]
impl colossus_ports::PolicyDecisionPoint for DigestOnlyPolicy {
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError> {
        if let Some(intent) = &request.command_intent {
            assert_eq!(
                intent.justification,
                format!(
                    "sha256:{}",
                    crate::kernel::sha256_hex(b"Check the requested build.")
                )
            );
        }
        self.0.decide(request).await
    }

    async fn doctor(&self) -> Result<serde_json::Value, PolicyError> {
        self.0.doctor().await
    }
}

#[async_trait]
impl ApprovalProvider for AuditedApproval {
    async fn request_approval(
        &self,
        request: &EffectRequest,
        request_hash: &str,
        _decision: &PolicyDecision,
        context: Option<&CommandApprovalContext>,
    ) -> Result<Option<ApprovalProof>, PolicyError> {
        assert_eq!(self.executor.calls.load(Ordering::Acquire), 0);
        let events = self.journal.read_global(1, 100).unwrap();
        let recorded = events
            .iter()
            .find(|event| event.event_type == "approval.requested.v1")
            .expect("sanitized approval evidence precedes the decision");
        assert_eq!(
            self.journal.decrypt_payload(recorded).unwrap(),
            json!({"command_context": context.unwrap()})
        );
        assert_eq!(context.unwrap().justification, "Check the requested build.");
        assert_eq!(
            context.unwrap().arguments,
            ["argument with  spaces", "--flag"]
        );
        self.calls.fetch_add(1, Ordering::AcqRel);
        let mut changed = request.clone();
        match self.changed_field {
            "reason" => {
                changed.command_intent.as_mut().unwrap().justification = "Different purpose.".into()
            }
            "executable" => changed.resource.push_str("-changed"),
            "arguments" => changed.content["args"] = json!(["changed"]),
            "cwd" => changed.content["cwd"] = json!("other-directory"),
            _ => {
                return Ok(Some(crate::kernel::approval_proof(
                    request_hash,
                    "fixture-operator",
                )?));
            }
        }
        let other_hash =
            crate::kernel::sha256_hex(&crate::kernel::canonical_bytes(&changed).unwrap());
        Ok(Some(crate::kernel::approval_proof(
            &other_hash,
            "fixture-operator",
        )?))
    }
}

#[tokio::test]
async fn reason_is_audited_before_approval_and_every_changed_field_rejects_its_proof() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
    for field in ["unchanged", "reason", "executable", "arguments", "cwd"] {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let executor = Arc::new(CountingExecutor {
            calls: AtomicUsize::new(0),
        });
        let approvals = Arc::new(AuditedApproval {
            journal: journal.clone(),
            executor: executor.clone(),
            changed_field: field,
            calls: AtomicUsize::new(0),
        });
        let policy = BuiltInPolicy::offline_default()
            .with_action("shell.run", DecisionOutcome::RequireApproval)
            .with_sandbox("native", "approval-fixture", false)
            .with_filesystem_root(executable.display().to_string(), "execute")
            .with_filesystem_read_root(cwd.display().to_string());
        let gateway = EffectGateway::new(
            journal,
            Arc::new(DigestOnlyPolicy(policy)),
            approvals.clone(),
            SafetyKernel::new(["process.spawn".into()]),
            [9; 32],
        );
        let mut request = effect_request(
            system_actor("approval-fixture"),
            "shell.run",
            executable.display().to_string(),
            json!({"cwd": cwd, "args": ["argument with  spaces", "--flag"], "environment": {}, "stdin_base64": null}),
        );
        request.capabilities = vec!["process.spawn".into()];
        request.command_intent = Some(CommandIntent {
            justification: "Check the requested build.".into(),
        });
        let result = gateway.execute(request, executor.as_ref()).await;
        assert_eq!(approvals.calls.load(Ordering::Acquire), 1);
        if field == "unchanged" {
            assert!(result.is_ok(), "{result:?}");
            assert_eq!(executor.calls.load(Ordering::Acquire), 1);
        } else {
            assert!(
                matches!(result, Err(GatewayError::Approval(_))),
                "{result:?}"
            );
            assert_eq!(executor.calls.load(Ordering::Acquire), 0);
        }
    }
}
