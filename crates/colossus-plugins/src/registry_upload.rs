//! Bounded Distribution-v2 chunk transfer and recovery of an interrupted PATCH.

use super::*;

const CHUNK_BYTES: usize = 4 * 1024 * 1024;
const MAX_UPLOAD_RECOVERIES: usize = 3;

impl PluginRegistryClient {
    pub(super) async fn start_upload(
        &self,
        reference: &RegistryReference,
    ) -> Result<Url, StoreError> {
        let uploads = Url::parse(&format!(
            "{}/v2/{}/blobs/uploads/",
            self.profile.origin, reference.repository,
        ))
        .map_err(adapter)?;
        let response = self
            .authenticated_send(
                Method::POST,
                uploads.clone(),
                Some(format!("repository:{}:pull,push", reference.repository)),
                None,
                Vec::new(),
            )
            .await?;
        require_status(&response, StatusCode::ACCEPTED, "blob upload start")?;
        upload_location(&uploads, &response)
    }

    pub(super) async fn upload_content(
        &self,
        reference: &RegistryReference,
        mut location: Url,
        bytes: &[u8],
    ) -> Result<Url, StoreError> {
        let scope = format!("repository:{}:pull,push", reference.repository);
        let mut offset = 0;
        let mut recoveries = 0;
        while offset < bytes.len() {
            let end = offset.saturating_add(CHUNK_BYTES).min(bytes.len());
            let response = self
                .authenticated_send(
                    Method::PATCH,
                    location.clone(),
                    Some(scope.clone()),
                    Some("application/octet-stream"),
                    bytes[offset..end].to_vec(),
                )
                .await;
            match response {
                Ok(response) if response.status() == StatusCode::ACCEPTED => {
                    let received = uploaded_length(&response)?;
                    if received != end {
                        return Err(adapter("registry acknowledged an unexpected upload range"));
                    }
                    location = upload_location(&location, &response)?;
                    offset = end;
                }
                Ok(response)
                    if !response.status().is_server_error()
                        && response.status() != StatusCode::RANGE_NOT_SATISFIABLE =>
                {
                    return Err(registry_status("blob upload chunk", response.status()));
                }
                _ => {
                    recoveries += 1;
                    if recoveries > MAX_UPLOAD_RECOVERIES {
                        return Err(adapter(
                            "OCI upload recovery limit exceeded; retry the explicit push",
                        ));
                    }
                    // An interrupted send may already have committed bytes. Query this
                    // exact upload session before resending any content.
                    let response = self
                        .authenticated_get(location.clone(), Some(scope.clone()), false)
                        .await?;
                    if !matches!(
                        response.status(),
                        StatusCode::NO_CONTENT | StatusCode::ACCEPTED
                    ) {
                        return Err(registry_status("blob upload recovery", response.status()));
                    }
                    let received = uploaded_length(&response)?;
                    if received < offset || received > end {
                        return Err(adapter(
                            "registry recovery range exceeds the submitted chunk",
                        ));
                    }
                    location = upload_location(&location, &response)?;
                    if offset == 0 && received == 1 {
                        // Distribution reports 0-0 both for an empty upload and
                        // one committed byte. Never guess and skip the first byte.
                        let cancelled = self
                            .authenticated_send(
                                Method::DELETE,
                                location.clone(),
                                Some(scope.clone()),
                                None,
                                Vec::new(),
                            )
                            .await?;
                        require_status(
                            &cancelled,
                            StatusCode::NO_CONTENT,
                            "ambiguous upload cancellation",
                        )?;
                        location = self.start_upload(reference).await?;
                        continue;
                    }
                    offset = received;
                }
            }
        }
        Ok(location)
    }
}

fn uploaded_length(response: &Response) -> Result<usize, StoreError> {
    let range = response
        .headers()
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| adapter("registry upload response omitted its Range"))?;
    let end = range
        .strip_prefix("0-")
        .and_then(|end| end.parse::<usize>().ok())
        .ok_or_else(|| adapter("registry upload returned an invalid Range"))?;
    end.checked_add(1)
        .ok_or_else(|| adapter("registry upload range overflow"))
}
