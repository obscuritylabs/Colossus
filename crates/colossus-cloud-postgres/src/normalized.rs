//! Mapping of typed cloud values to authoritative relational columns.
//! JSON projections exist only at the adapter boundary and for canonical audit hashing.
use colossus_cloud::{
    CloudError, CloudResult,
    storage::{EntityKind, EntityValue},
};

pub(super) struct Column {
    pub name: &'static str,
    pub path: &'static str,
    pub sql_type: &'static str,
}

pub(super) fn columns(kind: EntityKind) -> &'static [Column] {
    match kind {
        EntityKind::Project => &[
            Column {
                name: "project_name",
                path: "name",
                sql_type: "TEXT",
            },
            Column {
                name: "description",
                path: "description",
                sql_type: "TEXT",
            },
            Column {
                name: "parent_project_id",
                path: "parent_project_id",
                sql_type: "TEXT",
            },
            Column {
                name: "archived",
                path: "archived",
                sql_type: "BOOLEAN",
            },
            Column {
                name: "created_at_text",
                path: "created_at",
                sql_type: "TEXT",
            },
            Column {
                name: "updated_at_text",
                path: "updated_at",
                sql_type: "TEXT",
            },
        ],
        EntityKind::User => &[
            Column {
                name: "display_name",
                path: "user,display_name",
                sql_type: "TEXT",
            },
            Column {
                name: "email",
                path: "user,email",
                sql_type: "TEXT",
            },
            Column {
                name: "active",
                path: "user,active",
                sql_type: "BOOLEAN",
            },
            Column {
                name: "is_admin",
                path: "user,is_admin",
                sql_type: "BOOLEAN",
            },
            Column {
                name: "security_epoch",
                path: "security_epoch",
                sql_type: "BIGINT",
            },
            Column {
                name: "created_at_text",
                path: "user,created_at",
                sql_type: "TEXT",
            },
            Column {
                name: "updated_at_text",
                path: "user,updated_at",
                sql_type: "TEXT",
            },
        ],
        EntityKind::OidcIdentity => &[
            Column {
                name: "user_id",
                path: "user_id",
                sql_type: "TEXT",
            },
            Column {
                name: "issuer",
                path: "issuer",
                sql_type: "TEXT",
            },
            Column {
                name: "subject",
                path: "subject",
                sql_type: "TEXT",
            },
        ],
        EntityKind::LocalCredential => &[
            Column {
                name: "user_id",
                path: "user_id",
                sql_type: "TEXT",
            },
            Column {
                name: "username",
                path: "username",
                sql_type: "TEXT",
            },
            Column {
                name: "password_hash",
                path: "password_hash",
                sql_type: "TEXT",
            },
        ],
        EntityKind::Membership => &[
            Column {
                name: "user_id",
                path: "user_id",
                sql_type: "TEXT",
            },
            Column {
                name: "project_role",
                path: "role",
                sql_type: "TEXT",
            },
            Column {
                name: "permissions",
                path: "permissions",
                sql_type: "TEXT[]",
            },
        ],
        EntityKind::Host => &[
            Column {
                name: "host_name",
                path: "label",
                sql_type: "TEXT",
            },
            Column {
                name: "platform",
                path: "platform",
                sql_type: "TEXT",
            },
            Column {
                name: "deployment_kind",
                path: "deployment_kind",
                sql_type: "TEXT",
            },
            Column {
                name: "last_seen_at",
                path: "last_seen_at",
                sql_type: "BIGINT",
            },
        ],
        EntityKind::Node => &[
            Column {
                name: "instance_id",
                path: "instance_id",
                sql_type: "TEXT",
            },
            Column {
                name: "label",
                path: "label",
                sql_type: "TEXT",
            },
            Column {
                name: "certificate_sha256",
                path: "certificate_sha256",
                sql_type: "TEXT",
            },
            Column {
                name: "roles",
                path: "roles",
                sql_type: "TEXT[]",
            },
            Column {
                name: "revoked",
                path: "revoked",
                sql_type: "BOOLEAN",
            },
            Column {
                name: "host_id",
                path: "host_id",
                sql_type: "TEXT",
            },
            Column {
                name: "workspace_id",
                path: "workspace_id",
                sql_type: "TEXT",
            },
            Column {
                name: "workspace_label",
                path: "workspace_label",
                sql_type: "TEXT",
            },
            Column {
                name: "runtime_ready",
                path: "runtime_ready",
                sql_type: "BOOLEAN",
            },
            Column {
                name: "policy",
                path: "policy",
                sql_type: "JSONB",
            },
            Column {
                name: "policy_observed_at",
                path: "policy_observed_at",
                sql_type: "BIGINT",
            },
        ],
        EntityKind::Workspace => &[
            Column {
                name: "host_id",
                path: "host_id",
                sql_type: "TEXT",
            },
            Column {
                name: "node_id",
                path: "node_id",
                sql_type: "TEXT",
            },
            Column {
                name: "workspace_name",
                path: "label",
                sql_type: "TEXT",
            },
            Column {
                name: "sharing_mode",
                path: "sharing",
                sql_type: "TEXT",
            },
        ],
        EntityKind::Thread => &[
            Column {
                name: "node_id",
                path: "node_id",
                sql_type: "TEXT",
            },
            Column {
                name: "host_id",
                path: "host_id",
                sql_type: "TEXT",
            },
            Column {
                name: "workspace_id",
                path: "workspace_id",
                sql_type: "TEXT",
            },
            Column {
                name: "title",
                path: "title",
                sql_type: "TEXT",
            },
            Column {
                name: "created_at_text",
                path: "created_at",
                sql_type: "TEXT",
            },
            Column {
                name: "updated_at_text",
                path: "updated_at",
                sql_type: "TEXT",
            },
            Column {
                name: "archived",
                path: "archived",
                sql_type: "BOOLEAN",
            },
            Column {
                name: "local_session_id",
                path: "session_id",
                sql_type: "TEXT",
            },
            Column {
                name: "sync_status",
                path: "sync_status",
                sql_type: "TEXT",
            },
            Column {
                name: "source",
                path: "source",
                sql_type: "TEXT",
            },
            Column {
                name: "can_continue",
                path: "can_continue",
                sql_type: "BOOLEAN",
            },
            Column {
                name: "active_task_id",
                path: "active_task_id",
                sql_type: "TEXT",
            },
            Column {
                name: "queued_task_ids",
                path: "queued_task_ids",
                sql_type: "TEXT[]",
            },
        ],
        EntityKind::ThreadMessage => &[
            Column {
                name: "role",
                path: "role",
                sql_type: "TEXT",
            },
            Column {
                name: "message_text",
                path: "text",
                sql_type: "TEXT",
            },
            Column {
                name: "created_at_text",
                path: "created_at",
                sql_type: "TEXT",
            },
            Column {
                name: "task_id",
                path: "task_id",
                sql_type: "TEXT",
            },
        ],
        EntityKind::Task => &[
            Column {
                name: "node_id",
                path: "node_id",
                sql_type: "TEXT",
            },
            Column {
                name: "subject",
                path: "subject",
                sql_type: "TEXT",
            },
            Column {
                name: "created_at_text",
                path: "created_at",
                sql_type: "TEXT",
            },
            Column {
                name: "updated_at_text",
                path: "updated_at",
                sql_type: "TEXT",
            },
            Column {
                name: "request",
                path: "request",
                sql_type: "JSONB",
            },
            Column {
                name: "thread_id",
                path: "thread_id",
                sql_type: "TEXT",
            },
            Column {
                name: "source_read_only",
                path: "source_read_only",
                sql_type: "BOOLEAN",
            },
            Column {
                name: "history_complete",
                path: "history_complete",
                sql_type: "BOOLEAN",
            },
            Column {
                name: "history_bounded",
                path: "history_bounded",
                sql_type: "BOOLEAN",
            },
            Column {
                name: "run_id",
                path: "run_id",
                sql_type: "TEXT",
            },
            Column {
                name: "snapshot",
                path: "snapshot",
                sql_type: "JSONB",
            },
            Column {
                name: "dispatch_error",
                path: "dispatch_error",
                sql_type: "JSONB",
            },
            Column {
                name: "last_sequence",
                path: "last_sequence",
                sql_type: "BIGINT",
            },
            Column {
                name: "released_bytes",
                path: "released_bytes",
                sql_type: "BIGINT",
            },
            Column {
                name: "output_limited",
                path: "output_limited",
                sql_type: "BOOLEAN",
            },
        ],
        EntityKind::Command => &[
            Column {
                name: "task_id",
                path: "task_id",
                sql_type: "TEXT",
            },
            Column {
                name: "operation",
                path: "command",
                sql_type: "JSONB",
            },
            Column {
                name: "reply",
                path: "reply",
                sql_type: "JSONB",
            },
        ],
        EntityKind::Run => &[Column {
            name: "task_id",
            path: "",
            sql_type: "TEXT",
        }],
        EntityKind::NodeTask => &[Column {
            name: "task_id",
            path: "",
            sql_type: "TEXT",
        }],
        EntityKind::SessionMapping => &[Column {
            name: "thread_id",
            path: "",
            sql_type: "TEXT",
        }],
        EntityKind::Admission => &[Column {
            name: "active_tasks",
            path: "active",
            sql_type: "TEXT[]",
        }],
        EntityKind::Invitation => &[
            Column {
                name: "node_id",
                path: "node_id",
                sql_type: "TEXT",
            },
            Column {
                name: "label",
                path: "label",
                sql_type: "TEXT",
            },
            Column {
                name: "roles",
                path: "roles",
                sql_type: "TEXT[]",
            },
            Column {
                name: "expires_at",
                path: "expires_at",
                sql_type: "BIGINT",
            },
            Column {
                name: "redeemed_certificate",
                path: "redeemed_certificate",
                sql_type: "TEXT",
            },
            Column {
                name: "redeemed_csr",
                path: "redeemed_csr",
                sql_type: "TEXT",
            },
            Column {
                name: "certificate_pem",
                path: "certificate_pem",
                sql_type: "TEXT",
            },
        ],
        EntityKind::Renewal => &[
            Column {
                name: "renewal_id",
                path: "id",
                sql_type: "TEXT",
            },
            Column {
                name: "previous_fingerprint",
                path: "previous_fingerprint",
                sql_type: "TEXT",
            },
            Column {
                name: "csr_sha256",
                path: "csr_sha256",
                sql_type: "TEXT",
            },
            Column {
                name: "certificate_pem",
                path: "certificate_pem",
                sql_type: "TEXT",
            },
            Column {
                name: "certificate_sha256",
                path: "certificate_sha256",
                sql_type: "TEXT",
            },
            Column {
                name: "issued_at",
                path: "issued_at",
                sql_type: "BIGINT",
            },
        ],
        EntityKind::Setting => &[
            Column {
                name: "classification_enabled",
                path: "classification,enabled",
                sql_type: "BOOLEAN",
            },
            Column {
                name: "classification_text",
                path: "classification,text",
                sql_type: "TEXT",
            },
            Column {
                name: "classification_tone",
                path: "classification,tone",
                sql_type: "TEXT",
            },
            Column {
                name: "classification_position",
                path: "classification,position",
                sql_type: "TEXT",
            },
            Column {
                name: "required_sandbox_profile",
                path: "required_sandbox_profile",
                sql_type: "TEXT",
            },
            Column {
                name: "allowed_approval_modes",
                path: "allowed_approval_modes",
                sql_type: "TEXT[]",
            },
            Column {
                name: "allowed_tools",
                path: "allowed_tools",
                sql_type: "TEXT[]",
            },
            Column {
                name: "completed",
                path: "completed",
                sql_type: "BOOLEAN",
            },
            Column {
                name: "bootstrap_version",
                path: "version",
                sql_type: "BIGINT",
            },
        ],
        EntityKind::AuthFlow => &[Column {
            name: "record",
            path: "",
            sql_type: "JSONB",
        }],
    }
}

pub(super) fn input(column: &Column) -> String {
    let path = column.path;
    match column.sql_type {
        "JSONB" => format!("NULLIF($5::JSONB#>'{{{path}}}','null'::JSONB)"),
        "TEXT[]" => format!(
            "CASE WHEN jsonb_typeof($5::JSONB#>'{{{path}}}')='array' THEN ARRAY(SELECT value FROM jsonb_array_elements_text($5::JSONB#>'{{{path}}}') WITH ORDINALITY AS elements(value,ordinal) ORDER BY ordinal) ELSE NULL END"
        ),
        sql_type => format!("($5::JSONB#>>'{{{path}}}')::{sql_type}"),
    }
}

pub(super) fn projection(kind: EntityKind, alias: &str) -> String {
    let expression = match kind {
        EntityKind::Project => {
            r#"jsonb_build_object('id',@.id,'revision',@.revision,'name',@.project_name,'description',@.description,'parent_project_id',@.parent_project_id,'archived',@.archived,'created_at',@.created_at_text,'updated_at',@.updated_at_text)"#
        }
        EntityKind::User => {
            r#"jsonb_build_object('user',jsonb_build_object('id',@.id,'revision',@.revision,'display_name',@.display_name,'email',@.email,'active',@.active,'is_admin',@.is_admin,'created_at',@.created_at_text,'updated_at',@.updated_at_text,'identities',COALESCE((SELECT jsonb_agg(jsonb_build_object('kind',m.kind,'label',m.label,'username',m.username,'issuer',m.issuer,'subject',m.subject) ORDER BY m.ordinal) FROM user_login_metadata m WHERE m.user_id=@.id),'[]'::JSONB)),'security_epoch',@.security_epoch)"#
        }
        EntityKind::OidcIdentity => {
            r#"jsonb_build_object('user_id',@.user_id,'issuer',@.issuer,'subject',@.subject)"#
        }
        EntityKind::LocalCredential => {
            r#"jsonb_build_object('user_id',@.user_id,'username',@.username,'password_hash',@.password_hash)"#
        }
        EntityKind::Membership => {
            r#"jsonb_build_object('project_id',@.project_id,'revision',@.revision,'user_id',@.user_id,'subject',@.user_id,'role',@.project_role,'permissions',@.permissions)"#
        }
        EntityKind::Host => {
            r#"jsonb_build_object('host_id',@.id,'project_id',@.project_id,'revision',@.revision,'label',@.host_name,'platform',@.platform,'deployment_kind',@.deployment_kind,'last_seen_at',@.last_seen_at)"#
        }
        EntityKind::Node => {
            r#"jsonb_build_object('node_id',@.id,'project_id',@.project_id,'revision',@.revision,'instance_id',@.instance_id,'label',@.label,'certificate_sha256',@.certificate_sha256,'roles',@.roles,'revoked',@.revoked,'host_id',@.host_id,'workspace_id',@.workspace_id,'workspace_label',@.workspace_label,'runtime_ready',@.runtime_ready,'policy',@.policy,'policy_observed_at',@.policy_observed_at)"#
        }
        EntityKind::Workspace => {
            r#"jsonb_build_object('workspace_id',@.id,'project_id',@.project_id,'revision',@.revision,'host_id',@.host_id,'node_id',@.node_id,'label',@.workspace_name,'sharing',@.sharing_mode)"#
        }
        EntityKind::Thread => {
            r#"jsonb_build_object('thread_id',@.id,'project_id',@.project_id,'revision',@.revision,'node_id',@.node_id,'host_id',@.host_id,'workspace_id',@.workspace_id,'title',@.title,'created_at',@.created_at_text,'updated_at',@.updated_at_text,'archived',@.archived,'session_id',@.local_session_id,'sync_status',@.sync_status,'source',@.source,'can_continue',@.can_continue,'active_task_id',@.active_task_id,'queued_task_ids',@.queued_task_ids)"#
        }
        EntityKind::ThreadMessage => {
            r#"jsonb_build_object('message_id',@.id,'thread_id',@.parent_id,'project_id',@.project_id,'revision',@.revision,'role',@.role,'text',@.message_text,'created_at',@.created_at_text,'task_id',@.task_id)"#
        }
        EntityKind::Task => {
            r#"jsonb_build_object('task_id',@.id,'project_id',@.project_id,'revision',@.revision,'node_id',@.node_id,'subject',@.subject,'created_at',@.created_at_text,'updated_at',@.updated_at_text,'request',@.request,'thread_id',@.thread_id,'source_read_only',@.source_read_only,'history_complete',@.history_complete,'history_bounded',@.history_bounded,'run_id',@.run_id,'snapshot',@.snapshot,'dispatch_error',@.dispatch_error,'last_sequence',@.last_sequence,'released_bytes',@.released_bytes,'output_limited',@.output_limited)"#
        }
        EntityKind::Command => {
            r#"jsonb_build_object('command_id',@.id,'node_id',@.parent_id,'revision',@.revision,'task_id',@.task_id,'command',@.operation,'reply',@.reply)"#
        }
        EntityKind::Run => r#"to_jsonb(@.task_id)"#,
        EntityKind::NodeTask => r#"to_jsonb(@.task_id)"#,
        EntityKind::SessionMapping => r#"to_jsonb(@.thread_id)"#,
        EntityKind::Admission => r#"jsonb_build_object('active',@.active_tasks)"#,
        EntityKind::Invitation => {
            r#"jsonb_build_object('token_hash',@.id,'project_id',@.project_id,'node_id',@.node_id,'label',@.label,'roles',@.roles,'expires_at',@.expires_at,'redeemed_certificate',@.redeemed_certificate,'redeemed_csr',@.redeemed_csr,'certificate_pem',@.certificate_pem)"#
        }
        EntityKind::Renewal => {
            r#"jsonb_build_object('id',@.renewal_id,'previous_fingerprint',@.previous_fingerprint,'csr_sha256',@.csr_sha256,'certificate_pem',@.certificate_pem,'certificate_sha256',@.certificate_sha256,'issued_at',@.issued_at)"#
        }
        EntityKind::Setting => {
            r#"CASE @.setting_type WHEN 'display' THEN jsonb_build_object('revision',@.revision,'classification',jsonb_build_object('enabled',@.classification_enabled,'text',@.classification_text,'tone',@.classification_tone,'position',@.classification_position)) WHEN 'policy' THEN jsonb_build_object('revision',@.revision,'required_sandbox_profile',@.required_sandbox_profile,'allowed_approval_modes',@.allowed_approval_modes,'allowed_tools',@.allowed_tools) ELSE jsonb_build_object('completed',@.completed,'version',@.bootstrap_version) END"#
        }
        EntityKind::AuthFlow => r#"@.record"#,
    };
    expression.replace('@', alias)
}

pub(super) fn decode(
    kind: EntityKind,
    id: &str,
    value: serde_json::Value,
) -> CloudResult<EntityValue> {
    macro_rules! decode {
        ($variant:ident) => {
            serde_json::from_value(value)
                .map(EntityValue::$variant)
                .map_err(|_| CloudError::Storage)
        };
    }
    match kind {
        EntityKind::Project => decode!(Project),
        EntityKind::User => decode!(User),
        EntityKind::OidcIdentity => decode!(OidcIdentity),
        EntityKind::LocalCredential => decode!(LocalCredential),
        EntityKind::Membership => decode!(Membership),
        EntityKind::Host => decode!(Host),
        EntityKind::Node => decode!(Node),
        EntityKind::Workspace => decode!(Workspace),
        EntityKind::Thread => decode!(Thread),
        EntityKind::ThreadMessage => decode!(ThreadMessage),
        EntityKind::Task => decode!(Task),
        EntityKind::Command => decode!(Command),
        EntityKind::Run => decode!(Reference),
        EntityKind::NodeTask => decode!(Reference),
        EntityKind::SessionMapping => decode!(Reference),
        EntityKind::Admission => decode!(Admission),
        EntityKind::Invitation => decode!(Invitation),
        EntityKind::Renewal => decode!(Renewal),
        EntityKind::Setting => match id {
            "display" => decode!(DisplaySettings),
            "policy-expectation" => decode!(PolicyExpectation),
            _ => decode!(BootstrapMarker),
        },
        EntityKind::AuthFlow => Ok(EntityValue::AuthFlow(value)),
    }
}
