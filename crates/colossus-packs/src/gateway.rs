use super::*;

pub(super) fn source_path(operation: &PackOperation) -> Option<&Path> {
    match operation {
        PackOperation::Verify { path }
        | PackOperation::Install { path, .. }
        | PackOperation::BundleVerify { path }
        | PackOperation::BundleInstall { path, .. }
        | PackOperation::CollectionVerify { path }
        | PackOperation::CollectionInstall { path }
        | PackOperation::RegistryPush { path, .. } => Some(Path::new(path)),
        PackOperation::BundleBuild { source, .. }
        | PackOperation::CollectionBuild { source, .. } => Some(Path::new(source)),
        _ => None,
    }
}

pub(super) fn destination_path(operation: &PackOperation) -> Option<&Path> {
    match operation {
        PackOperation::BundleBuild { destination, .. } => Some(Path::new(destination)),
        PackOperation::BundleInstall { prefix, .. } => Some(Path::new(prefix)),
        PackOperation::CollectionBuild { destination, .. } => Some(Path::new(destination)),
        PackOperation::RegistryPull { destination, .. } => Some(Path::new(destination)),
        _ => None,
    }
}

pub(super) fn enforce_registry_credentials(
    operation: &PackOperation,
    request: &EffectRequest,
) -> Result<(), ExecutionError> {
    let expected = match operation {
        PackOperation::RegistryPull {
            credential_reference,
            ..
        }
        | PackOperation::RegistryPush {
            credential_reference,
            ..
        } => credential_reference.as_deref(),
        _ => None,
    };
    let actual = request
        .credential_references
        .iter()
        .map(|credential| credential.reference.as_str())
        .collect::<Vec<_>>();
    let matches = match expected {
        Some(reference) => actual == [reference],
        None => actual.is_empty(),
    };
    if !matches {
        return Err(ExecutionError::Failed(
            "registry credential references do not match the authorized operation".into(),
        ));
    }
    Ok(())
}

pub(super) fn signing_key_info(seed: [u8; 32]) -> BundleSigningKeyInfo {
    let signing_key = SigningKey::from_bytes(&seed);
    let public = signing_key.verifying_key().to_bytes();
    BundleSigningKeyInfo {
        key_id: digest_hex(&public),
        public_key: BASE64.encode(public),
    }
}

pub(super) fn enforce_read_grant(
    path: &Path,
    permit: &ExecutionPermit,
) -> Result<(), ExecutionError> {
    let canonical = fs::canonicalize(path).map_err(execution)?;
    if permit.obligations().resource_authority == ResourceAuthority::Ambient {
        return Ok(());
    }
    let allowed = permit.obligations().filesystem.iter().any(|grant| {
        matches!(grant.mode.as_str(), "read" | "write")
            && fs::canonicalize(&grant.root).is_ok_and(|root| canonical.starts_with(root))
    });
    if !allowed {
        return Err(ExecutionError::Failed(format!(
            "pack source {} is outside policy-authorized filesystem roots",
            canonical.display()
        )));
    }
    Ok(())
}

pub(super) fn enforce_write_grant(
    path: &Path,
    permit: &ExecutionPermit,
) -> Result<(), ExecutionError> {
    validate_absolute_normalized(path, "bundle write destination").map_err(execution)?;
    let mut existing = path;
    loop {
        match fs::symlink_metadata(existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing.parent().ok_or_else(|| {
                    ExecutionError::Failed(format!(
                        "bundle destination {} has no existing ancestor",
                        path.display()
                    ))
                })?;
            }
            Err(error) => return Err(execution(error)),
        }
    }
    let resolved_existing = fs::canonicalize(existing).map_err(execution)?;
    if permit.obligations().resource_authority == ResourceAuthority::Ambient {
        return Ok(());
    }
    let allowed = permit.obligations().filesystem.iter().any(|grant| {
        if grant.mode != "write" {
            return false;
        }
        fs::canonicalize(&grant.root).is_ok_and(|root| resolved_existing.starts_with(root))
    });
    if !allowed {
        return Err(ExecutionError::Failed(format!(
            "bundle destination {} is outside policy-authorized write roots",
            path.display()
        )));
    }
    Ok(())
}

pub(super) fn resolve_signing_seed(reference: &str) -> Result<[u8; 32], PackError> {
    let variable = reference.strip_prefix("env:").ok_or_else(|| {
        PackError::Invalid("bundle signing keys must use env:VARIABLE references".into())
    })?;
    if variable.is_empty()
        || !variable.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        })
    {
        return Err(PackError::Invalid(
            "bundle signing keys must use env:VARIABLE references".into(),
        ));
    }
    let encoded = std::env::var(variable).map_err(|_| {
        PackError::Invalid(format!("bundle signing credential {variable} is unset"))
    })?;
    let decoded = hex::decode(&encoded)
        .or_else(|_| BASE64.decode(&encoded))
        .map_err(|_| PackError::Invalid("bundle signing seed must be hex or base64".into()))?;
    decoded.try_into().map_err(|_| {
        PackError::Invalid("bundle signing seed must decode to exactly 32 bytes".into())
    })
}

pub(super) fn execution(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}

pub(super) fn pack_execution(error: PackError) -> ExecutionError {
    match error {
        PackError::OutcomeUnknown(message) => ExecutionError::OutcomeUnknown(message),
        error => ExecutionError::Failed(error.to_string()),
    }
}

pub(super) async fn registry_client(
    endpoint: &str,
    permit: &ExecutionPermit,
    tls_roots: &AdditionalRootCertificates,
) -> Result<(Url, Client), PackError> {
    let url = Url::parse(endpoint)
        .map_err(|error| PackError::Invalid(format!("invalid registry URL: {error}")))?;
    let host = url
        .host_str()
        .ok_or_else(|| PackError::Invalid("registry URL must include a host".into()))?;
    let host_ip = host.parse::<IpAddr>().ok();
    let loopback_http = url.scheme() == "http" && host_ip.is_some_and(|ip| ip.is_loopback());
    let ambient_http = url.scheme() == "http"
        && permit.obligations().resource_authority == ResourceAuthority::Ambient;
    if !(url.scheme() == "https" || loopback_http || ambient_http)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(PackError::Invalid(
            "registry URLs require HTTPS, explicit loopback HTTP, or acknowledged ambient HTTP and no credentials, query, or fragment"
                .into(),
        ));
    }
    let origin = url.origin().ascii_serialization();
    let matched = network_authority_match(permit.obligations(), &origin)
        .map_err(|error| PackError::Invalid(error.to_string()))?
        .ok_or_else(|| {
            PackError::Invalid(format!(
                "registry origin {origin} is absent from permit obligations"
            ))
        })?;
    if matched == NetworkDestinationMatch::PublicWildcard {
        return Err(PackError::Invalid(format!(
            "registry origin {origin} requires an exact or ambient grant"
        )));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| PackError::Invalid("registry URL must resolve to a known port".into()))?;
    let mut addresses = lookup_host((host, port))
        .await
        .map_err(|error| PackError::Invalid(format!("registry DNS resolution failed: {error}")))?
        .filter(|address| match (matched, host_ip) {
            (NetworkDestinationMatch::Ambient, _) => true,
            (_, Some(ip)) => ip.is_loopback() || !non_public_ip(address.ip()),
            (_, None) => !non_public_ip(address.ip()),
        })
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(PackError::Invalid(
            "registry host resolved to no permitted address".into(),
        ));
    }
    addresses.sort();
    addresses.dedup();
    addresses.truncate(16);
    let client = tls_roots
        .configure_reqwest(Client::builder())
        .no_proxy()
        .redirect(RedirectPolicy::none())
        .resolve_to_addrs(host, &addresses)
        .timeout(Duration::from_millis(permit.obligations().timeout_ms))
        .build()
        .map_err(|error| PackError::Invalid(format!("registry client failed: {error}")))?;
    Ok((url, client))
}

pub(super) fn registry_auth(
    request: reqwest::RequestBuilder,
    credential_reference: Option<&str>,
    permit: &ExecutionPermit,
) -> Result<reqwest::RequestBuilder, PackError> {
    let Some(reference) = credential_reference else {
        return Ok(request);
    };
    let variable = reference.strip_prefix("env:").ok_or_else(|| {
        PackError::Invalid("registry credentials must use env:VARIABLE references".into())
    })?;
    if variable.is_empty()
        || !variable.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        })
        || (permit.obligations().resource_authority != ResourceAuthority::Ambient
            && !permit
                .obligations()
                .allowed_environment
                .iter()
                .any(|allowed| allowed == variable))
    {
        return Err(PackError::Invalid(
            "registry credential is absent from permit environment obligations".into(),
        ));
    }
    let secret = std::env::var(variable)
        .map_err(|_| PackError::Invalid(format!("registry credential {variable} is unset")))?;
    if secret.is_empty() {
        return Err(PackError::Invalid(format!(
            "registry credential {variable} is empty"
        )));
    }
    Ok(request.bearer_auth(secret))
}

pub(super) fn non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}
