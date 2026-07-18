use super::*;

pub(super) fn validate_projection_record(
    event_id: &str,
    memory_id: &str,
    text: &str,
    metadata: &Value,
    embedding: &[f32],
) -> Result<(), StoreError> {
    let metadata_bytes = serde_json::to_vec(metadata).map_err(adapter)?;
    if event_id.is_empty()
        || memory_id.is_empty()
        || text.trim().is_empty()
        || text.len() > MAX_TEXT_BYTES
        || metadata_bytes.len() > MAX_METADATA_BYTES
        || !metadata.is_object()
    {
        return Err(adapter("Chroma projection record is invalid or oversized"));
    }
    validate_vector(embedding, None)
}

pub(super) fn system_index_actor() -> Actor {
    Actor {
        actor_type: ActorType::System,
        id: "memory-indexer".into(),
    }
}

pub(super) fn credential_references(reference: Option<&str>) -> Vec<CredentialReference> {
    reference
        .map(|reference| {
            vec![CredentialReference {
                reference: reference.into(),
                value_hash: None,
            }]
        })
        .unwrap_or_default()
}

pub(super) fn validate_credential_disclosure(
    request: &EffectRequest,
    expected: Option<&str>,
) -> Result<(), StoreError> {
    let actual = request
        .credential_references
        .iter()
        .map(|reference| reference.reference.as_str())
        .collect::<Vec<_>>();
    match expected {
        Some(expected) if actual == [expected] => Ok(()),
        None if actual.is_empty() => Ok(()),
        _ => Err(adapter(
            "semantic request credential references do not match configuration",
        )),
    }
}

pub(super) fn validate_destination(
    permit: &ExecutionPermit,
    expected_origin: &str,
) -> Result<(), ExecutionError> {
    if permit
        .obligations()
        .network_destinations
        .iter()
        .any(|destination| destination == expected_origin)
    {
        Ok(())
    } else {
        Err(execution(
            "semantic endpoint origin is absent from permit obligations",
        ))
    }
}

pub(super) async fn send_http(
    method: Method,
    endpoint: &str,
    payload: Option<&Value>,
    credential_reference: Option<&str>,
    credential_header: &str,
    permit: &ExecutionPermit,
    configured_timeout_ms: u64,
) -> Result<Vec<u8>, StoreError> {
    let url = Url::parse(endpoint).map_err(adapter)?;
    let host = url
        .host_str()
        .ok_or_else(|| adapter("semantic endpoint has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| adapter("semantic endpoint has no port"))?;
    let addresses = resolve_addresses(host, port).await?;
    let timeout_ms = configured_timeout_ms.min(permit.obligations().timeout_ms);
    let client = Client::builder()
        .no_proxy()
        .redirect(RedirectPolicy::none())
        .resolve_to_addrs(host, &addresses)
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .map_err(adapter)?;
    let mut builder = client.request(method, url);
    if let Some(reference) = credential_reference {
        let secret = resolve_credential(reference)?;
        builder = if credential_header == "authorization" {
            builder.bearer_auth(secret)
        } else {
            builder.header(credential_header, secret)
        };
    }
    if let Some(payload) = payload {
        let bytes = serde_json::to_vec(payload).map_err(adapter)?;
        if bytes.len() > 1024 * 1024 {
            return Err(adapter("semantic request exceeds 1 MiB"));
        }
        builder = builder
            .header("content-type", "application/json")
            .body(bytes);
    }
    let response = builder.send().await.map_err(adapter)?;
    if !response.status().is_success() {
        return Err(adapter(format!(
            "semantic endpoint returned HTTP {}",
            response.status().as_u16()
        )));
    }
    let permitted = usize::try_from(permit.obligations().max_output_bytes).map_err(adapter)?;
    let limit = permitted.min(MAX_RESPONSE_BYTES);
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(adapter)?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(adapter(
                "semantic response exceeds the permitted output bound",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(super) async fn resolve_addresses(
    host: &str,
    port: u16,
) -> Result<Vec<SocketAddr>, StoreError> {
    let addresses = lookup_host((host, port)).await.map_err(adapter)?;
    let mut unique = BTreeSet::new();
    for address in addresses {
        unique.insert(address);
        if unique.len() > MAX_RESOLVED_ADDRESSES {
            return Err(adapter("semantic endpoint resolved to too many addresses"));
        }
    }
    if unique.is_empty() {
        return Err(adapter("semantic endpoint did not resolve"));
    }
    Ok(unique.into_iter().collect())
}

pub(super) fn bounded_result(
    bytes: Vec<u8>,
    permit: &ExecutionPermit,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    let limit = usize::try_from(permit.obligations().max_output_bytes).map_err(execution)?;
    if bytes.len() > limit {
        return Err(execution("semantic output exceeds the permitted bound"));
    }
    Ok(QuarantinedEffectResult {
        media_type: "application/json".into(),
        bytes,
        effect_succeeded: true,
    })
}
