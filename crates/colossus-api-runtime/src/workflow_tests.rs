use super::RuntimeWorkflowApi;
use colossus_api::{
    ApiErrorCode, ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext,
    CreateWorkflowScheduleRequest, RequestId, SetWorkflowScheduleEnabledRequest, WorkflowApi,
    WorkflowScheduleMisfirePolicy, WorkflowStatus, scopes,
};
use colossus_policy::DenyApproval;
use colossus_runtime::{Runtime, RuntimeConfig, RuntimeOpenOptions, StorageAdapter};
use std::{fs, sync::Arc};

const YAML: &str = "apiVersion: colossus.dev/v1alpha1\nkind: Workflow\nmetadata:\n  name: public-health\n  version: 1.0.0\n  description: Public workflow fixture\ninputs: {type: object, additionalProperties: false}\noutputs: {type: object}\ncapabilities: []\nmaxConcurrency: 1\nstepBudget: 2\nsteps:\n  - id: result\n    type: emit\n    value: {ok: true}\n";
fn caller(id: &str, scopes: &[&str]) -> CallerContext {
    CallerContext::authenticated(
        ApplicationPrincipal::authenticated(
            id,
            "workflow-test",
            ApplicationKind::Enrolled,
            scopes.iter().map(|scope| ApiScope::new(*scope).unwrap()),
            ["primary".into()],
            Vec::<String>::new(),
        )
        .unwrap(),
        RequestId::new("workflow-test").unwrap(),
    )
}
const ALL: &[&str] = &[
    scopes::WORKFLOWS_READ,
    scopes::WORKFLOWS_REGISTER,
    scopes::SCHEDULES_READ,
    scopes::SCHEDULES_CREATE,
    scopes::SCHEDULES_CONTROL,
    scopes::WORKFLOW_RUNS_READ,
    scopes::WORKFLOW_RUNS_START,
];

#[tokio::test]
async fn authenticated_resources_bind_scopes_owner_canonical_revisions_and_independent_runs() {
    let root = crate::service_tests::runtime_tempdir();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    config.storage.adapter = StorageAdapter::Ephemeral;
    let runtime = Arc::new(
        Runtime::open_with_options(
            &config,
            Arc::new(DenyApproval),
            None,
            RuntimeOpenOptions::for_workspace(&workspace).unwrap(),
        )
        .unwrap(),
    );
    let api = RuntimeWorkflowApi::new(runtime.clone());
    let owner = caller("app:owner", ALL);
    let other = caller("app:other", ALL);
    let denied = caller("app:owner", &[]);
    assert_eq!(
        api.validate_definition(&denied, "invalid".into())
            .await
            .unwrap_err()
            .code,
        ApiErrorCode::PermissionDenied
    );
    let validated = api.validate_definition(&owner, YAML.into()).await.unwrap();
    let definition = api
        .register_definition(
            &owner,
            YAML.into(),
            validated.workflow_hash.clone(),
            "registration".into(),
        )
        .await
        .unwrap();
    assert_eq!(validated, definition);
    let request = CreateWorkflowScheduleRequest {
        schedule_id: "health".into(),
        workflow_id: definition.workflow_id.clone(),
        expected_hash: definition.workflow_hash.clone(),
        inputs: serde_json::json!({}),
        cadence_seconds: 60,
        calendar: None,
        task: None,
        starts_at: "2026-10-03T12:00:00Z".into(),
        misfire_policy: WorkflowScheduleMisfirePolicy::Skip,
        enabled: true,
        idempotency_key: "create".into(),
    };
    let before = runtime.journal().head().unwrap();
    for grant in [vec![scopes::SCHEDULES_READ], vec![scopes::SCHEDULES_CREATE]] {
        assert_eq!(
            api.create_schedule(&caller("app:owner", &grant), request.clone())
                .await
                .unwrap_err()
                .code,
            ApiErrorCode::PermissionDenied
        );
    }
    assert_eq!(runtime.journal().head().unwrap(), before);
    let created = api.create_schedule(&owner, request.clone()).await.unwrap();
    assert_eq!(created.origin.as_ref().unwrap().owner, owner.actor());
    assert!(created.origin.as_ref().unwrap().run_id.is_none());
    assert!(
        api.list_schedules(&owner, None, 1).await.unwrap().items[0]
            .record
            .inputs
            .is_null()
    );
    assert!(
        api.list_schedules(&other, None, 1)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        api.get_schedule(&other, "health".into())
            .await
            .unwrap_err()
            .code,
        ApiErrorCode::NotFound
    );
    assert!(!api.has_active_work(&owner).await.unwrap());
    runtime
        .workflows()
        .tick_schedules_at("2026-10-03T12:00:00Z")
        .unwrap();
    let current = api.get_schedule(&owner, "health".into()).await.unwrap();
    assert_ne!(created.etag, current.etag);
    assert_eq!(
        api.set_schedule_enabled(
            &owner,
            SetWorkflowScheduleEnabledRequest {
                schedule_id: "health".into(),
                enabled: false,
                etag: created.etag.clone()
            }
        )
        .await
        .unwrap_err()
        .code,
        ApiErrorCode::Conflict
    );
    assert_eq!(api.create_schedule(&owner, request).await.unwrap(), created);
    assert!(api.has_active_work(&owner).await.unwrap());
    let id = current.record.last_run_id.unwrap();
    let run = api.get_run(&owner, id.clone()).await.unwrap();
    assert_eq!(run.status, WorkflowStatus::Queued);
    assert_eq!(run.workflow_id, definition.workflow_id);
    assert_eq!(
        api.get_run(&other, id).await.unwrap_err().code,
        ApiErrorCode::NotFound
    );
    assert_eq!(
        api.list_schedules(&owner, None, 101)
            .await
            .unwrap_err()
            .code,
        ApiErrorCode::InvalidArgument
    );
}
