//! Relational operational aggregates. These read projections are never authority inputs.
use crate::CloudPostgresStore;
use colossus_cloud::{CloudError, CloudResult, observability::OperationalStatistics};
use diesel::{
    sql_query,
    sql_types::{BigInt, Jsonb, Nullable, Text},
};
use diesel_async::RunQueryDsl;

#[derive(diesel::QueryableByName)]
struct StatisticsRow {
    #[diesel(sql_type=Jsonb)]
    value: serde_json::Value,
}

impl CloudPostgresStore {
    pub(super) async fn operational_statistics(
        &self,
        project: &str,
        node: Option<&str>,
        now: u64,
        days: u16,
    ) -> CloudResult<OperationalStatistics> {
        if !(1..=90).contains(&days) {
            return Err(CloudError::InvalidArgument);
        }
        let now = i64::try_from(now).map_err(|_| CloudError::InvalidArgument)?;
        let first = (now / 86_400 - i64::from(days) + 1) * 86_400;
        let mut connection = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        let row = sql_query(SQL)
            .bind::<Text, _>(project)
            .bind::<Nullable<Text>, _>(node)
            .bind::<BigInt, _>(first)
            .bind::<BigInt, _>(now)
            .get_result::<StatisticsRow>(&mut connection)
            .await
            .map_err(|_| CloudError::Storage)?;
        serde_json::from_value(row.value).map_err(|_| CloudError::Storage)
    }
}

const SQL: &str = r#"
WITH agent_set AS (
    SELECT id,host_id,record FROM runtime_agents WHERE project_id=$1 AND NOT deleted AND NOT revoked AND ($2::TEXT IS NULL OR id=$2)
), thread_set AS (
    SELECT id FROM conversation_threads WHERE project_id=$1 AND NOT deleted AND ($2::TEXT IS NULL OR node_id=$2)
), task_set AS (
    SELECT id,status,domain_created_at,record FROM tasks WHERE project_id=$1 AND NOT deleted AND ($2::TEXT IS NULL OR node_id=$2)
), recent AS (
    SELECT * FROM task_set WHERE domain_created_at>=to_timestamp($3) AND domain_created_at<=to_timestamp($4)
), usage_events AS (
    SELECT t.id,t.domain_created_at,e.record#>'{update,usage}' AS usage FROM recent t JOIN released_events e ON e.project_id=$1 AND e.scope_id=t.id
    WHERE e.record#>'{update,usage}' IS NOT NULL AND e.record#>'{update,usage}'<>'null'::JSONB
), usage_totals AS (
    SELECT SUM((usage->>'input_tokens')::NUMERIC) AS input_tokens,SUM((usage->>'output_tokens')::NUMERIC) AS output_tokens FROM usage_events
), dates AS (
    SELECT generate_series((to_timestamp($3) AT TIME ZONE 'UTC')::DATE,(to_timestamp($4) AT TIME ZONE 'UTC')::DATE,'1 day')::DATE AS day
), daily_runs AS (
    SELECT (domain_created_at AT TIME ZONE 'UTC')::DATE AS day,COUNT(*) AS runs,COUNT(*) FILTER(WHERE status='completed') AS completed,COUNT(*) FILTER(WHERE status='failed') AS failed FROM recent GROUP BY 1
), daily_usage AS (
    SELECT (domain_created_at AT TIME ZONE 'UTC')::DATE AS day,SUM((usage->>'input_tokens')::NUMERIC) AS input_tokens,SUM((usage->>'output_tokens')::NUMERIC) AS output_tokens FROM usage_events GROUP BY 1
)
SELECT jsonb_build_object(
    'complete',TRUE,
    'counts',jsonb_build_object(
        'projects',0,
        'hosts',CASE WHEN $2::TEXT IS NULL THEN (SELECT COUNT(*) FROM hosts WHERE project_id=$1 AND NOT deleted) ELSE (SELECT COUNT(DISTINCT host_id) FROM agent_set WHERE host_id IS NOT NULL) END,
        'agents',(SELECT COUNT(*) FROM agent_set),
        'online_agents',(SELECT COUNT(*) FROM agent_set a JOIN connection_leases l ON l.project_id=$1 AND l.node_id=a.id AND l.expires_at>$4 WHERE COALESCE((a.record->>'runtime_ready')::BOOLEAN,FALSE)),
        'threads',(SELECT COUNT(*) FROM thread_set),
        'runs',(SELECT COUNT(*) FROM recent),
        'completed',(SELECT COUNT(*) FROM recent WHERE status='completed'),
        'failed',(SELECT COUNT(*) FROM recent WHERE status='failed'),
        'cancelled',(SELECT COUNT(*) FROM recent WHERE status='cancelled'),
        'interrupted',(SELECT COUNT(*) FROM recent WHERE status='interrupted'),
        'outcome_unknown',(SELECT COUNT(*) FROM recent WHERE status='outcome_unknown'),
        'active',(SELECT COUNT(*) FROM task_set WHERE status IN('running','waiting','cancelling')),
        'queued',(SELECT COUNT(*) FROM task_set WHERE status='queued')
    ),
    'activity',(SELECT COALESCE(jsonb_agg(jsonb_build_object('date',to_char(d.day,'YYYY-MM-DD'),'runs',COALESCE(r.runs,0),'completed',COALESCE(r.completed,0),'failed',COALESCE(r.failed,0),'input_tokens',u.input_tokens,'output_tokens',u.output_tokens) ORDER BY d.day),'[]'::JSONB) FROM dates d LEFT JOIN daily_runs r ON r.day=d.day LEFT JOIN daily_usage u ON u.day=d.day),
    'usage',(SELECT jsonb_build_object('input_tokens',input_tokens,'output_tokens',output_tokens,'estimated_cost',NULL,'coverage','Only retained provider usage reports are counted; imported or bounded history may omit usage. Pricing is not configured.') FROM usage_totals)
) AS value
"#;
