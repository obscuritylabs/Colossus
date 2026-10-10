use super::*;
use colossus_contracts::BrowserUrl;
use colossus_ports::BrowserDownloadDescriptor;
impl Host {
    pub fn download(
        &mut self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserDownloadDescriptor, BrowserDriverError> {
        let BrowserAction::Download { element, max_bytes } = &command.action else {
            return Err(BrowserDriverError::Denied);
        };
        if *max_bytes == 0 || *max_bytes > colossus_ports::MAX_BROWSER_TRANSFER_BYTES {
            return Err(BrowserDriverError::Denied);
        }
        let (element, max_bytes) = (element.clone(), *max_bytes);
        let owner = self.transfer_owner(command, control)?;
        let (_, name, attributes) = self.transfer_node(&owner, &element, control)?;
        if !matches!(name.as_str(), "A" | "AREA") {
            return Err(BrowserDriverError::Denied);
        }
        let href = attribute(&attributes, "href")
            .filter(|href| !href.is_empty() && href.len() <= 4096)
            .ok_or(BrowserDriverError::Denied)?;
        // Use the native document's effective base URL, including its ordinary
        // <base href>. Neither a caller nor a raw protocol argument nominates it.
        let document = self.method(owner.native, "DOM.getDocument", json!({"depth":0}), control)?;
        let address = document
            .get("root")
            .and_then(|node| node.get("baseURL"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 4096)
            .ok_or(BrowserDriverError::Stale)?;
        let mut url = url::Url::parse(address)
            .and_then(|base| base.join(href))
            .map_err(|_| BrowserDriverError::Denied)?;
        url.set_fragment(None);
        let url = BrowserUrl::parse(url.as_str()).map_err(|_| BrowserDriverError::Denied)?;
        if !self
            .callbacks
            .origins
            .lock()
            .is_ok_and(|origins| origins.contains(&url.origin()))
        {
            return Err(BrowserDriverError::Denied);
        }
        let reservation = self
            .transfer
            .stage
            .as_mut()
            .ok_or(BrowserDriverError::Unavailable)?
            .reserve_download()?;
        let path = CString::new(
            reservation
                .path()
                .to_str()
                .ok_or(BrowserDriverError::Denied)?,
        )
        .map_err(|_| BrowserDriverError::Denied)?;
        let url = CString::new(url.as_str()).map_err(|_| BrowserDriverError::Denied)?;
        let nonce = CString::new(owner.token.as_str()).map_err(|_| BrowserDriverError::Failed)?;
        self.check_document_fence()?;
        // SAFETY: exact owned UI-thread tab/doc; path comes only from retained private stage.
        status_result(unsafe {
            ffi::colossus_cef_download_arm(
                owner.native,
                1,
                owner.document,
                nonce.as_ptr(),
                path.as_ptr(),
                url.as_ptr(),
                u64::from(max_bytes),
                30_000,
            )
        })?;
        // SAFETY: same exact one-shot cached arm, with no new URL or path authority.
        status_result(unsafe {
            ffi::colossus_cef_download_start(owner.native, 1, owner.document, nonce.as_ptr())
        })?;
        let finished = loop {
            self.pump()?;
            if !self.transfer_current(&owner) {
                // SAFETY: cancellation addresses the exact owned nonce; shutdown retains writer custody.
                unsafe {
                    ffi::colossus_cef_download_cancel(
                        owner.native,
                        1,
                        owner.document,
                        nonce.as_ptr(),
                    );
                }
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            let mut state = ffi::DownloadState {
                status: 0,
                received_bytes: 0,
                total_bytes: 0,
                final_url: [0; 4097],
                final_url_len: 0,
            };
            // SAFETY: correctly sized repr(C) output and live exact owned tab/doc/nonce.
            status_result(unsafe {
                ffi::colossus_cef_download_poll(
                    owner.native,
                    1,
                    owner.document,
                    nonce.as_ptr(),
                    &mut state,
                )
            })?;
            if state.received_bytes > u64::from(max_bytes)
                || state.total_bytes > u64::from(max_bytes)
                || matches!(state.status, 3 | 4)
            {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            if state.status == 2 {
                break state;
            }
            if state.status > 4 {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        if finished.final_url_len == 0 || finished.final_url_len > 4096 {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let final_url = std::str::from_utf8(&finished.final_url[..finished.final_url_len as usize])
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let final_url =
            BrowserUrl::parse(final_url).map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if !self
            .callbacks
            .origins
            .lock()
            .is_ok_and(|origins| origins.contains(&final_url.origin()))
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let bytes = self
            .transfer
            .stage
            .as_mut()
            .ok_or(BrowserDriverError::Unavailable)?
            .completed(reservation, finished.received_bytes as u32)?;
        if bytes.len() > max_bytes as usize || !self.transfer_current(&owner) {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let descriptor = BrowserDownloadDescriptor {
            session_id: owner.command.session_id.clone(),
            target: owner.command.target.clone(),
            control_generation: owner.command.control_generation,
            transfer_id: owner.token.clone(),
            size_bytes: bytes.len() as u32,
            sha256: digest(&bytes),
            origin: final_url.origin(),
        };
        self.transfer.download = Some(Download {
            owner,
            bytes,
            offset: 0,
        });
        Ok(descriptor)
    }
}
