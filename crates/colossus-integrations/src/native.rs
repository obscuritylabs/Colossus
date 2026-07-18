use super::*;

/// Compile one first-party native connector into strict dynamic tool contracts.
#[allow(clippy::too_many_arguments)]
pub fn compile_native(
    name: &str,
    base_url: Option<&str>,
    auth: IntegrationAuth,
    credential_reference: Option<String>,
    credential_references: BTreeMap<String, String>,
    scopes: Vec<String>,
    connected_at: String,
    updated_at: String,
) -> Result<IntegrationConnection, StoreError> {
    let (title, description, default_url, operations) = match name {
        "github" => (
            "GitHub",
            "Native GitHub connector for repositories, issues, pull requests, checks, and releases.",
            "https://api.github.com",
            github_operations()?,
        ),
        "searxng" => (
            "SearXNG",
            "Native local/private metasearch connector for normalized SearXNG JSON results.",
            "http://127.0.0.1:8888",
            searxng_operations()?,
        ),
        "opensearch" => (
            "OpenSearch",
            "Native OpenSearch connector for document search, retrieval, indexing, updates, deletes, mappings, and cluster health.",
            "http://127.0.0.1:9200",
            opensearch_operations()?,
        ),
        _ => {
            return Err(StoreError::Adapter(
                "native integration must be github, searxng, or opensearch".into(),
            ));
        }
    };
    validate_native_auth(
        name,
        &auth,
        credential_reference.as_deref(),
        &credential_references,
    )?;
    let base_url = base_url.unwrap_or(default_url).to_owned();
    validate_base_url(&base_url)?;
    for reference in credential_references.values() {
        validate_credential_reference(Some(reference))?;
    }
    let required_refs = usize::from(credential_reference.is_some()) + credential_references.len();
    let status = if auth_requires_credential(&auth) && required_refs == 0 {
        IntegrationStatus::PendingAuth
    } else {
        IntegrationStatus::Connected
    };
    let manifest = serde_json::to_vec(&json!({
        "name": name,
        "base_url": base_url,
        "auth": auth,
        "operations": operations,
    }))
    .map_err(adapter)?;
    let scopes = if scopes.is_empty() && name == "github" {
        vec!["repo".into(), "workflow".into()]
    } else {
        scopes
    };
    let connection = IntegrationConnection {
        name: name.into(),
        kind: IntegrationKind::Native,
        status,
        title: title.into(),
        description: description.into(),
        base_url,
        auth,
        credential_reference,
        credential_references,
        scopes,
        operations,
        manifest_sha256: format!("{:x}", Sha256::digest(manifest)),
        connected_at,
        updated_at,
    };
    validate_connection(&connection)?;
    Ok(connection)
}

pub(super) fn native_operation(
    name: &str,
    description: &str,
    schema: Value,
    method: &str,
    path: &str,
    max_output_bytes: u64,
) -> Result<IntegrationOperation, StoreError> {
    jsonschema::validator_for(&schema).map_err(adapter)?;
    Ok(IntegrationOperation {
        tool: ToolSpec {
            name: name.into(),
            description: description.into(),
            input_schema: schema,
            effect_action: Some(name.into()),
            capability: Some("integration.invoke".into()),
            max_output_bytes,
        },
        operation_id: name
            .split_once('.')
            .map_or(name, |(_, operation)| operation)
            .into(),
        method: method.into(),
        path: path.into(),
        path_parameters: Vec::new(),
        query_parameters: Vec::new(),
        accepts_body: !matches!(method, "GET" | "DELETE"),
    })
}

pub(super) fn github_operations() -> Result<Vec<IntegrationOperation>, StoreError> {
    let bounded = || json!({"type":"integer","minimum":1,"maximum":100});
    Ok(vec![
        native_operation(
            "github.repos",
            "List repositories visible to the connected GitHub token.",
            json!({"type":"object","additionalProperties":false,"properties":{
                "visibility":{"type":"string","enum":["all","public","private"],"default":"all"},
                "max_results":bounded()
            }}),
            "GET",
            "/user/repos",
            64_000,
        )?,
        native_operation(
            "github.issues",
            "List issues for a GitHub repository.",
            github_repo_schema(
                json!({
                    "state":{"type":"string","enum":["open","closed","all"],"default":"open"},
                    "max_results":bounded()
                }),
                &[],
            ),
            "GET",
            "/repos/{owner}/{repo}/issues",
            64_000,
        )?,
        native_operation(
            "github.pull_requests",
            "List pull requests for a GitHub repository.",
            github_repo_schema(
                json!({
                    "state":{"type":"string","enum":["open","closed","all"],"default":"open"},
                    "max_results":bounded()
                }),
                &[],
            ),
            "GET",
            "/repos/{owner}/{repo}/pulls",
            64_000,
        )?,
        native_operation(
            "github.checks",
            "List check runs for a GitHub commit ref.",
            github_repo_schema(
                json!({
                    "ref":{"type":"string","minLength":1,"maxLength":512},
                    "max_results":bounded()
                }),
                &["ref"],
            ),
            "GET",
            "/repos/{owner}/{repo}/commits/{ref}/check-runs",
            64_000,
        )?,
        native_operation(
            "github.releases",
            "List releases for a GitHub repository.",
            github_repo_schema(json!({"max_results":bounded()}), &[]),
            "GET",
            "/repos/{owner}/{repo}/releases",
            64_000,
        )?,
    ])
}

pub(super) fn github_repo_schema(extra: Value, extra_required: &[&str]) -> Value {
    let mut properties = Map::from_iter([
        (
            "owner".into(),
            json!({"type":"string","minLength":1,"maxLength":256}),
        ),
        (
            "repo".into(),
            json!({"type":"string","minLength":1,"maxLength":256}),
        ),
    ]);
    if let Some(extra) = extra.as_object() {
        properties.extend(extra.clone());
    }
    let required = ["owner", "repo"]
        .into_iter()
        .chain(extra_required.iter().copied())
        .collect::<Vec<_>>();
    json!({
        "type":"object",
        "additionalProperties":false,
        "properties":properties,
        "required":required
    })
}

pub(super) fn searxng_operations() -> Result<Vec<IntegrationOperation>, StoreError> {
    Ok(vec![
        native_operation(
            "searxng.search",
            "Search a configured SearXNG instance and return normalized results.",
            json!({"type":"object","additionalProperties":false,"properties":{
                "query":{"type":"string","minLength":1,"maxLength":4096},
                "max_results":{"type":"integer","minimum":1,"maximum":20,"default":10}
            },"required":["query"]}),
            "GET",
            "/search",
            128_000,
        )?,
        native_operation(
            "searxng.health",
            "Check that a configured SearXNG instance returns JSON results.",
            json!({"type":"object","additionalProperties":false,"properties":{}}),
            "GET",
            "/search",
            16_000,
        )?,
    ])
}

pub(super) fn opensearch_operations() -> Result<Vec<IntegrationOperation>, StoreError> {
    let empty = || json!({"type":"object","additionalProperties":false,"properties":{}});
    let index = || json!({"type":"string","minLength":1,"maxLength":1024});
    let id = || json!({"type":"string","minLength":1,"maxLength":1024});
    let refresh = || json!({"type":"string","enum":["false","true","wait_for"]});
    Ok(vec![
        native_operation(
            "opensearch.info",
            "Fetch basic OpenSearch endpoint information.",
            empty(),
            "GET",
            "/",
            16_000,
        )?,
        native_operation(
            "opensearch.health",
            "Fetch OpenSearch cluster health.",
            empty(),
            "GET",
            "/_cluster/health",
            16_000,
        )?,
        native_operation(
            "opensearch.list_indices",
            "List OpenSearch indices through the JSON cat API.",
            empty(),
            "GET",
            "/_cat/indices",
            64_000,
        )?,
        native_operation(
            "opensearch.get_mapping",
            "Fetch an OpenSearch index mapping.",
            json!({"type":"object","additionalProperties":false,"properties":{"index":index()},"required":["index"]}),
            "GET",
            "/{index}/_mapping",
            64_000,
        )?,
        native_operation(
            "opensearch.search",
            "Run a bounded OpenSearch query.",
            json!({"type":"object","additionalProperties":false,"properties":{
            "index":index(),"query":{"type":"object"},
            "size":{"type":"integer","minimum":1,"maximum":100,"default":10},
            "from":{"type":"integer","minimum":0,"maximum":10000,"default":0},
            "source_includes":{"type":"array","maxItems":256,"items":{"type":"string","maxLength":1024}},
            "sort":{"type":"array","maxItems":64,"items":{"type":"object"}}
        },"required":["index","query"]}),
            "POST",
            "/{index}/_search",
            128_000,
        )?,
        native_operation(
            "opensearch.get_document",
            "Fetch one OpenSearch document.",
            json!({"type":"object","additionalProperties":false,"properties":{"index":index(),"id":id()},"required":["index","id"]}),
            "GET",
            "/{index}/_doc/{id}",
            64_000,
        )?,
        native_operation(
            "opensearch.index_document",
            "Create or replace one OpenSearch document.",
            json!({"type":"object","additionalProperties":false,"properties":{"index":index(),"id":id(),"document":{"type":"object"},"refresh":refresh()},"required":["index","document"]}),
            "POST",
            "/{index}/_doc",
            32_000,
        )?,
        native_operation(
            "opensearch.update_document",
            "Partially update one OpenSearch document.",
            json!({"type":"object","additionalProperties":false,"properties":{"index":index(),"id":id(),"doc":{"type":"object"},"doc_as_upsert":{"type":"boolean"},"refresh":refresh()},"required":["index","id","doc"]}),
            "POST",
            "/{index}/_update/{id}",
            32_000,
        )?,
        native_operation(
            "opensearch.delete_document",
            "Delete one OpenSearch document.",
            json!({"type":"object","additionalProperties":false,"properties":{"index":index(),"id":id(),"refresh":refresh()},"required":["index","id"]}),
            "DELETE",
            "/{index}/_doc/{id}",
            32_000,
        )?,
    ])
}
