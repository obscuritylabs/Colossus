//! Personal history visibility shared by deterministic storage and application checks.
use crate::storage::{EntityKey, EntityKind, EntityRecord, EntityValue};
use crate::{CloudCaller, CloudError, CloudResult, CloudThread};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn require_thread(thread: &CloudThread, caller: &CloudCaller) -> CloudResult<()> {
    if thread
        .owner_user_id
        .as_deref()
        .is_none_or(|owner| owner == caller.subject())
    {
        Ok(())
    } else {
        // Personal identities and their existence are not released to other project members.
        Err(CloudError::NotFound)
    }
}

pub(crate) fn record_visible(
    records: &BTreeMap<EntityKey, EntityRecord>,
    deleted: &BTreeSet<EntityKey>,
    record: &EntityRecord,
    subject: &str,
) -> bool {
    let lookup = |kind, id: &str| {
        let key = EntityKey {
            kind,
            project_id: record.key.project_id.clone(),
            parent_id: None,
            id: id.into(),
        };
        (!deleted.contains(&key))
            .then(|| records.get(&key))
            .flatten()
    };
    let node_visible = |id: &str| {
        lookup(EntityKind::Node, id).is_none_or(|record| {
            matches!(&record.value, EntityValue::Node(node)
                if node.owner_user_id.as_deref().is_none_or(|owner| owner == subject))
        })
    };
    let thread_visible = |id: &str| {
        lookup(EntityKind::Thread, id).is_some_and(|record| {
            matches!(&record.value, EntityValue::Thread(thread)
                if thread.owner_user_id.as_deref().is_none_or(|owner| owner == subject) && node_visible(&thread.node_id))
        })
    };
    let task_visible = |task: &crate::CloudTask| {
        node_visible(&task.node_id) && task.thread_id.as_deref().is_none_or(thread_visible)
    };
    let task_id_visible = |id: &str| {
        lookup(EntityKind::Task, id).is_some_and(
            |record| matches!(&record.value, EntityValue::Task(task) if task_visible(task)),
        )
    };
    match &record.value {
        EntityValue::Thread(thread) => {
            thread
                .owner_user_id
                .as_deref()
                .is_none_or(|owner| owner == subject)
                && node_visible(&thread.node_id)
        }
        EntityValue::Node(node) => node
            .owner_user_id
            .as_deref()
            .is_none_or(|owner| owner == subject),
        EntityValue::Workspace(workspace) => node_visible(&workspace.node_id),
        EntityValue::Host(host) => {
            let mut nodes = records.values().filter(|candidate| {
                candidate.key.project_id == record.key.project_id && !deleted.contains(&candidate.key)
                    && matches!(&candidate.value, EntityValue::Node(node) if node.host_id.as_deref() == Some(host.host_id.as_str()))
            }).peekable();
            nodes.peek().is_none() || nodes.any(|candidate| matches!(&candidate.value,
                EntityValue::Node(node) if node.execution_kind == crate::ExecutionKind::Connected && node.owner_user_id.as_deref().is_none_or(|owner| owner == subject)))
        }
        EntityValue::Task(task) => task_visible(task),
        EntityValue::ThreadMessage(message) => thread_visible(&message.thread_id),
        EntityValue::Command(command) => {
            node_visible(&command.node_id) && task_id_visible(&command.task_id)
        }
        EntityValue::Reference(id) if record.key.kind == EntityKind::SessionMapping => {
            record.key.parent_id.as_deref().is_some_and(node_visible) && thread_visible(id)
        }
        EntityValue::Reference(id)
            if matches!(record.key.kind, EntityKind::Run | EntityKind::NodeTask) =>
        {
            record.key.parent_id.as_deref().is_some_and(node_visible) && task_id_visible(id)
        }
        // Admission carries private task identities and has no human read contract.
        EntityValue::Admission(_) => false,
        _ => true,
    }
}
