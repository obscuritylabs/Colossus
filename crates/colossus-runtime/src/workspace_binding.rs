use super::*;

const WORKSPACE_IDENTITY_FAILURE: &str =
    "workspace identity changed; stop the runtime and explicitly select the workspace again";

/// Permit-bound adapter wrapper that revalidates the opened workspace identity after
/// policy and approval, immediately before the underlying effect can touch a path or
/// launch a workspace process. This is a stable-drift and renderer/agent authority
/// boundary, not an atomic POSIX namespace transaction against a hostile same-UID
/// native process; that deployment boundary is documented in security-architecture.md.
pub(super) struct WorkspaceBoundEffectExecutor {
    identity: workspace_lease::WorkspaceIdentity,
    inner: Arc<dyn EffectExecutor>,
}

impl WorkspaceBoundEffectExecutor {
    pub(super) fn new<T>(identity: workspace_lease::WorkspaceIdentity, inner: Arc<T>) -> Self
    where
        T: EffectExecutor + 'static,
    {
        Self { identity, inner }
    }
}

#[async_trait]
impl EffectExecutor for WorkspaceBoundEffectExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.identity
            .revalidate()
            .map_err(|_| ExecutionError::Failed(WORKSPACE_IDENTITY_FAILURE.into()))?;
        self.inner.execute(request, permit).await
    }
}

/// Top-level tool guard. This rejects drift before any tool can resolve a relative
/// pathname; permit-bound wrappers repeat the check after policy/approval.
pub(super) struct WorkspaceBoundToolExecutor {
    pub(super) identity: workspace_lease::WorkspaceIdentity,
    pub(super) inner: Arc<dyn ToolExecutor>,
}

#[async_trait]
impl ToolExecutor for WorkspaceBoundToolExecutor {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        self.identity
            .revalidate()
            .map_err(|_| ToolError::Failed(WORKSPACE_IDENTITY_FAILURE.into()))?;
        self.inner.execute(call, context).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use colossus_testkit::InMemoryEventJournal;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct RecordingEffect(AtomicBool);

    #[async_trait]
    impl EffectExecutor for RecordingEffect {
        async fn execute(
            &self,
            _request: &EffectRequest,
            _permit: ExecutionPermit,
        ) -> Result<QuarantinedEffectResult, ExecutionError> {
            self.0.store(true, Ordering::SeqCst);
            Err(ExecutionError::Failed("unexpected invocation".into()))
        }
    }

    #[cfg(unix)]
    struct RecordingTool(AtomicBool);

    #[cfg(unix)]
    #[async_trait]
    impl ToolExecutor for RecordingTool {
        async fn execute(
            &self,
            call: ToolCall,
            _context: ExecutionContext,
        ) -> Result<ToolResult, ToolError> {
            self.0.store(true, Ordering::SeqCst);
            Ok(ToolResult {
                call_id: call.call_id,
                name: call.name,
                output: "unexpected invocation".into(),
                exit_code: 0,
            })
        }
    }

    // ExecutionPermit cannot be constructed outside the policy kernel, so the full
    // wrapper path is covered by runtime effect tests. Identity drift itself is tested
    // in workspace_lease; retain this type assertion to prevent the wrapper losing its
    // effect boundary as composition evolves.
    #[test]
    fn workspace_bound_executor_remains_an_effect_executor() {
        fn assert_effect<T: EffectExecutor>() {}
        assert_effect::<WorkspaceBoundEffectExecutor>();
        let recording = RecordingEffect(AtomicBool::new(false));
        assert!(!recording.0.load(Ordering::SeqCst));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn renamed_workspace_replacement_never_reaches_permit_bound_effect_adapter() {
        let parent = tempfile::tempdir().expect("workspace parent");
        let lease_root = tempfile::tempdir().expect("lease root");
        let workspace = parent.path().join("workspace");
        let moved = parent.path().join("workspace-moved");
        fs::create_dir(&workspace).expect("workspace");
        let lease = workspace_lease::WorkspaceOwnershipLease::acquire_at(
            &workspace,
            &lease_root.path().join("leases"),
        )
        .expect("lease");
        let recording = Arc::new(RecordingEffect(AtomicBool::new(false)));
        let executor = WorkspaceBoundEffectExecutor::new(lease.identity(), Arc::clone(&recording));
        fs::rename(&workspace, moved).expect("rename original");
        fs::create_dir(&workspace).expect("replacement");

        let action = "workspace.identity.test";
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let gateway = EffectGateway::new(
            journal,
            Arc::new(BuiltInPolicy::offline_default().with_action(action, DecisionOutcome::Allow)),
            Arc::new(DenyApproval),
            SafetyKernel::new([action.into()]),
            [0x5a; 32],
        );
        let mut request = effect_request(
            system_actor("workspace-test"),
            action,
            "workspace",
            json!({}),
        );
        request.capabilities = vec![action.into()];

        gateway
            .execute(request, &executor)
            .await
            .expect_err("identity drift must stop the permit-bound adapter");
        assert!(!recording.0.load(Ordering::SeqCst));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn renamed_workspace_replacement_never_reaches_tool_adapter() {
        let parent = tempfile::tempdir().expect("workspace parent");
        let lease_root = tempfile::tempdir().expect("lease root");
        let lease_directory = lease_root.path().join("leases");
        let workspace = parent.path().join("workspace");
        let moved = parent.path().join("workspace-moved");
        fs::create_dir(&workspace).expect("workspace");
        let lease =
            workspace_lease::WorkspaceOwnershipLease::acquire_at(&workspace, &lease_directory)
                .expect("lease");
        let recording = Arc::new(RecordingTool(AtomicBool::new(false)));
        let executor = WorkspaceBoundToolExecutor {
            identity: lease.identity(),
            inner: Arc::clone(&recording) as Arc<dyn ToolExecutor>,
        };
        fs::rename(&workspace, moved).expect("rename original");
        fs::create_dir(&workspace).expect("replacement");

        let error = executor
            .execute(
                ToolCall {
                    call_id: "call-1".into(),
                    name: "filesystem.read".into(),
                    arguments: json!({"path": "replacement-secret"}),
                },
                ExecutionContext::default(),
            )
            .await
            .expect_err("identity drift must fail");

        assert!(matches!(error, ToolError::Failed(_)));
        assert!(!recording.0.load(Ordering::SeqCst));
    }
}
