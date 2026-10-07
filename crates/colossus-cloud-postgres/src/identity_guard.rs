//! Low-rate structural administration invariants, never a normal event write-head lock.
use crate::store::TransactionError;
use colossus_cloud::storage::{EntityKind, EntityMutation};
use colossus_ports::StoreError;
use diesel::{sql_query, sql_types::Bool};
use diesel_async::{AsyncPgConnection, RunQueryDsl};

#[derive(diesel::QueryableByName)]
struct Flag {
    #[diesel(sql_type=Bool)]
    present: bool,
}
pub(super) async fn begin(
    conn: &mut AsyncPgConnection,
    mutations: &[EntityMutation],
) -> Result<Option<bool>, TransactionError> {
    if !mutations
        .iter()
        .any(|m| matches!(m.key.kind, EntityKind::User | EntityKind::Project))
    {
        return Ok(None);
    }
    sql_query(
        "SELECT pg_advisory_xact_lock(hashtext(current_schema()||':product-administration'))",
    )
    .execute(conn)
    .await?;
    let admins=sql_query("SELECT EXISTS(SELECT 1 FROM cloud_users WHERE active AND is_admin AND NOT deleted) AS present").get_result::<Flag>(conn).await?;
    if admins.present
        && mutations.iter().any(|m| {
            m.key.kind == EntityKind::User
                && m.actor == "operator-bootstrap"
                && m.value
                    .pointer("/user/is_admin")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
        })
    {
        return Err(StoreError::Conflict {
            stream_id: "cloud.administrator-bootstrap".into(),
            expected: 0,
            actual: 1,
        }
        .into());
    }
    Ok(Some(admins.present))
}
pub(super) async fn finish(
    conn: &mut AsyncPgConnection,
    prior: Option<bool>,
) -> Result<(), TransactionError> {
    let Some(had_admin) = prior else {
        return Ok(());
    };
    if had_admin&&!sql_query("SELECT EXISTS(SELECT 1 FROM cloud_users WHERE active AND is_admin AND NOT deleted) AS present").get_result::<Flag>(conn).await?.present {
        return Err(StoreError::Conflict{stream_id:"cloud.last-administrator".into(),expected:1,actual:0}.into());
    }
    let cycle=sql_query("WITH RECURSIVE lineage AS(SELECT project_id AS origin,parent_project_id AS next,ARRAY[project_id] AS path,FALSE AS cycle,1 AS depth FROM projects WHERE NOT deleted UNION ALL SELECT l.origin,p.parent_project_id,l.path||p.project_id,p.project_id=ANY(l.path),l.depth+1 FROM lineage l JOIN projects p ON p.project_id=l.next WHERE NOT l.cycle AND l.depth<65 AND NOT p.deleted) SELECT EXISTS(SELECT 1 FROM lineage WHERE cycle OR depth>=65) AS present").get_result::<Flag>(conn).await?;
    if cycle.present {
        return Err(StoreError::Conflict {
            stream_id: "cloud.project-hierarchy".into(),
            expected: 0,
            actual: 1,
        }
        .into());
    }
    Ok(())
}
