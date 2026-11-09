// src/api/assistant_cost.rs
//
// Where a workspace's assistant tokens go, for its owner.
//
// A message costs (model calls) × (size of each call), and the number of calls
// is what varies: one that guessed a value wrong three times took eight calls
// where two would have done. So this reports the most expensive messages, how
// many calls each took, and which endpoints failed along the way.
//
// Sits inside GET /api/tenant/stats as `assistant_cost`. Only this tenant's own
// assistant turns (`method = 'LLM'`). Nothing a person wrote is stored or shown:
// the numbers and the endpoints' names are all there is.

use serde_json::{json, Value};
use tokio_postgres::Client;

const MOST_EXPENSIVE: i64 = 10;
const MOST_FAILED: i64 = 5;

const SCOPE: &str = "l.tenant_id = $1 AND l.method = 'LLM'
     AND ($2::INT = 0 OR l.timestamp >= NOW() - make_interval(hours => $2::INT))";

const FAILED_TOOLS: &str = "COALESCE(l.metadata->'failed_tools', '[]'::jsonb)";

/// `hours` = 0 means all time, as everywhere in the stats.
pub async fn assistant_cost(client: &Client, tenant_id: &str, hours: i32) -> Result<Value, tokio_postgres::Error> {
    let params: [&(dyn tokio_postgres::types::ToSql + Sync); 2] = [&tenant_id, &hours];

    let totals = client
        .query_one(
            &format!(
                "SELECT count(*)::BIGINT,
                        COALESCE(sum(l.total_tokens), 0)::BIGINT,
                        COALESCE(sum(COALESCE((l.metadata->>'llm_calls')::INT, 1)), 0)::BIGINT,
                        (count(*) FILTER (WHERE jsonb_array_length({FAILED_TOOLS}) > 0))::BIGINT,
                        COALESCE(sum(l.total_tokens) FILTER (WHERE jsonb_array_length({FAILED_TOOLS}) > 0), 0)::BIGINT
                   FROM api_usage_logs l
                  WHERE {SCOPE}"
            ),
            &params,
        )
        .await?;

    let messages = client
        .query(
            &format!(
                "SELECT l.id, l.timestamp, l.email, l.endpoint_path,
                        COALESCE(l.total_tokens, 0)::BIGINT,
                        COALESCE((l.metadata->>'llm_calls')::INT, 1),
                        {FAILED_TOOLS}
                   FROM api_usage_logs l
                  WHERE {SCOPE}
                  ORDER BY l.total_tokens DESC NULLS LAST, l.timestamp DESC
                  LIMIT {MOST_EXPENSIVE}"
            ),
            &params,
        )
        .await?;

    let failing = client
        .query(
            &format!(
                "SELECT f.tool, count(*)::BIGINT
                   FROM api_usage_logs l,
                        jsonb_array_elements_text({FAILED_TOOLS}) AS f(tool)
                  WHERE {SCOPE}
                  GROUP BY f.tool
                  ORDER BY 2 DESC, 1
                  LIMIT {MOST_FAILED}"
            ),
            &params,
        )
        .await?;

    let messages: Vec<Value> = messages
        .iter()
        .map(|row| {
            let at: chrono::DateTime<chrono::Utc> = row.get(1);
            let channel: String = row.get(3);
            json!({
                "id": row.get::<_, String>(0),
                "timestamp": at.to_rfc3339(),
                "email": row.get::<_, String>(2),
                "channel": channel.strip_prefix("assistant/").unwrap_or(&channel),
                "tokens": row.get::<_, i64>(4),
                "model_calls": row.get::<_, i32>(5),
                "failed_tools": row.get::<_, Value>(6),
            })
        })
        .collect();

    let failing: Vec<Value> = failing
        .iter()
        .map(|row| json!({ "tool": row.get::<_, String>(0), "failures": row.get::<_, i64>(1) }))
        .collect();

    Ok(json!({
        "messages": totals.get::<_, i64>(0),
        "tokens": totals.get::<_, i64>(1),
        "model_calls": totals.get::<_, i64>(2),
        "messages_with_failures": totals.get::<_, i64>(3),
        "tokens_in_messages_with_failures": totals.get::<_, i64>(4),
        "most_expensive": messages,
        "failing_endpoints": failing,
    }))
}

#[cfg(test)]
mod tests {
    use super::assistant_cost;
    use crate::endpoint_store::tenant_management::get_default_tenant;
    use crate::endpoint_store::EndpointStore;
    use serde_json::json;

    /// Needs a real database, so it is opt-in:
    ///   TEST_DATABASE_URL=postgres://app:app@localhost:55432/app \
    ///     cargo test --bin store -- --ignored only_this_tenants
    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL"]
    async fn only_this_tenants_turns_are_counted_and_the_failures_are_named() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let store = EndpointStore::new(&url).await.expect("store");
        let email = format!("cost-{}@example.com", uuid::Uuid::new_v4().simple());
        let other_email = format!("cost-other-{}@example.com", uuid::Uuid::new_v4().simple());
        let tenant = get_default_tenant(&store, &email).await.unwrap().id;
        let other = get_default_tenant(&store, &other_email).await.unwrap().id;

        let c = store.get_admin_conn().await.unwrap();
        let key = format!("key-{}", uuid::Uuid::new_v4().simple());
        c.execute(
            "INSERT INTO user_preferences (email, hidden_defaults) VALUES ($1, '') ON CONFLICT DO NOTHING",
            &[&email],
        )
        .await
        .unwrap();
        c.execute(
            "INSERT INTO api_keys (id, email, key_hash, key_prefix, key_name, generated_at, tenant_id)
             VALUES ($1, $2, $1, 'p', 'n', NOW(), $3)",
            &[&key, &email, &tenant],
        )
        .await
        .unwrap();

        let rows = [
            (&tenant, "LLM", 95_000_i64, json!({"llm_calls": 8, "failed_tools": ["update", "update", "update"]})),
            (&tenant, "LLM", 20_000, json!({"llm_calls": 2, "failed_tools": []})),
            // A turn logged before failures were recorded.
            (&tenant, "LLM", 5_000, json!({"llm_calls": 1})),
            // Not an assistant turn.
            (&tenant, "POST", 900_000, json!({})),
            // Someone else's.
            (&other, "LLM", 700_000, json!({"llm_calls": 9, "failed_tools": ["secret-tool"]})),
        ];
        for (t, method, tokens, metadata) in rows {
            c.execute(
                "INSERT INTO api_usage_logs (id, key_id, email, endpoint_path, method, timestamp, total_tokens, metadata, tenant_id)
                 VALUES ($1, $2, $3, 'assistant/teams', $4, NOW(), $5, $6, $7)",
                &[&uuid::Uuid::new_v4().to_string(), &key, &email, &method, &tokens, &metadata, t],
            )
            .await
            .unwrap();
        }

        let cost = assistant_cost(&c, &tenant, 24).await.unwrap();
        assert_eq!(cost["messages"], 3);
        assert_eq!(cost["tokens"], 120_000);
        assert_eq!(cost["model_calls"], 11);
        assert_eq!(cost["messages_with_failures"], 1);
        assert_eq!(cost["tokens_in_messages_with_failures"], 95_000);
        assert_eq!(cost["most_expensive"][0]["tokens"], 95_000);
        assert_eq!(cost["most_expensive"][0]["channel"], "teams");
        assert_eq!(cost["most_expensive"][2]["failed_tools"], json!([]));
        assert_eq!(cost["failing_endpoints"], json!([{ "tool": "update", "failures": 3 }]));
    }
}
