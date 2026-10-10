//! Real package-profile writes: staged upload and native downloader bytes over HTTP.
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{BrowserAction, BrowserElementRef, BrowserTarget};
use colossus_ports::{
    BROWSER_SCREENSHOT_CHUNK_BYTES, BrowserDownloadReadRequest, BrowserDriver,
    BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest, BrowserUploadDescriptor,
    BrowserUploadPrepareRequest, BrowserUploadWriteRequest, MAX_BROWSER_TRANSFER_BYTES,
};
use sha2::{Digest as _, Sha256};
use std::sync::atomic::{AtomicUsize, Ordering};
use zeroize::Zeroizing;

pub const UPLOAD: &[u8] = b"Colossus owned Windows upload\n";
pub fn upload_bytes() -> Zeroizing<Vec<u8>> {
    Zeroizing::new(UPLOAD.repeat(3073))
}
pub fn download_bytes() -> Zeroizing<Vec<u8>> {
    Zeroizing::new((0_u8..=255).cycle().take(256 * 513).collect())
}

pub async fn check(
    driver: &dyn BrowserDriver,
    opened: &BrowserDriverOpenRequest,
    target: &mut BrowserTarget,
    control: &BrowserDriverControl,
    origin: &str,
    uploaded: &AtomicUsize,
) {
    let upload_input = element(
        driver,
        opened,
        target,
        control,
        100,
        "Owned native file input",
    )
    .await;
    let bytes = upload_bytes();
    let artifact_id = "owned-native-windows-fixture-artifact".to_owned();
    let prepared = driver
        .prepare_upload(
            BrowserUploadPrepareRequest {
                command: super::exercise::command(
                    opened,
                    target,
                    BrowserAction::Upload {
                        element: upload_input,
                        artifact_id: artifact_id.clone(),
                        max_bytes: MAX_BROWSER_TRANSFER_BYTES,
                    },
                    101,
                ),
                descriptor: BrowserUploadDescriptor {
                    artifact_id,
                    size_bytes: bytes.len().try_into().unwrap(),
                    sha256: hex::encode(Sha256::digest(&*bytes)),
                    file_name: "upload.txt".into(),
                },
            },
            control,
        )
        .await
        .expect("native upload requires writable exact-package private stage");
    assert_eq!(prepared.next_offset, 0);
    let mut transfer = BrowserDownloadReadRequest {
        binding: opened.binding.clone(),
        run_id: "native-windows-acceptance-run".into(),
        session_id: opened.session_id.clone(),
        target: target.clone(),
        control_generation: 1,
        transfer_id: prepared.transfer_id,
        offset: 0,
    };
    for chunk in bytes.chunks(BROWSER_SCREENSHOT_CHUNK_BYTES) {
        let receipt = driver
            .write_upload_chunk(
                BrowserUploadWriteRequest {
                    transfer: transfer.clone(),
                    data_base64: BASE64.encode(chunk),
                },
                control,
            )
            .await
            .expect("actual ordered private upload bytes");
        transfer.offset += u32::try_from(chunk.len()).unwrap();
        assert_eq!(receipt.next_offset, transfer.offset);
        assert_eq!(receipt.transfer_id, transfer.transfer_id);
    }
    let observed = driver
        .commit_upload(transfer, control)
        .await
        .expect("native package may create and freeze the upload file");
    target.document_id = observed.tab.document_id;
    let submit = element(driver, opened, target, control, 102, "Submit owned upload").await;
    let submitted = driver
        .execute(
            super::exercise::command(
                opened,
                target,
                BrowserAction::Click { element: submit },
                103,
            ),
            control,
        )
        .await
        .expect("ordinary form submits the staged native file");
    target.document_id = submitted.tab.document_id;
    for index in 104..=128 {
        let observed = super::exercise::snapshot(driver, opened, target, control, index).await;
        target.document_id = observed.tab.document_id;
        if uploaded.load(Ordering::SeqCst) == 1
            && observed.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot
                    .nodes
                    .iter()
                    .any(|node| node.name.contains("Owned upload received"))
            })
        {
            break;
        }
        assert!(
            index < 128,
            "HTTP must receive the exact actual uploaded file bytes"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    for (index, name, expected) in [
        (130, "Owned binary download", download_bytes()),
        (131, "Owned empty download", Zeroizing::new(Vec::new())),
    ] {
        let link = element(driver, opened, target, control, index, name).await;
        let descriptor = driver
            .download(
                super::exercise::command(
                    opened,
                    target,
                    BrowserAction::Download {
                        element: link,
                        max_bytes: MAX_BROWSER_TRANSFER_BYTES,
                    },
                    index + 100,
                ),
                control,
            )
            .await
            .expect("native downloader must write the exact-package private stage");
        assert_eq!(descriptor.target, *target);
        assert_eq!(descriptor.origin.as_str(), origin);
        assert_eq!(descriptor.size_bytes as usize, expected.len());
        assert_eq!(descriptor.sha256, hex::encode(Sha256::digest(&*expected)));
        let mut received = Zeroizing::new(Vec::new());
        let mut read = BrowserDownloadReadRequest {
            binding: opened.binding.clone(),
            run_id: "native-windows-acceptance-run".into(),
            session_id: opened.session_id.clone(),
            target: target.clone(),
            control_generation: 1,
            transfer_id: descriptor.transfer_id,
            offset: 0,
        };
        loop {
            read.offset = u32::try_from(received.len()).unwrap();
            let chunk = driver
                .read_download_chunk(read.clone(), control)
                .await
                .expect("completed exact native file can be read privately");
            assert_eq!(chunk.offset, read.offset);
            assert!(chunk.data_base64.len() <= BROWSER_SCREENSHOT_CHUNK_BYTES.div_ceil(3) * 4);
            let bytes = Zeroizing::new(BASE64.decode(&chunk.data_base64).unwrap());
            assert!(bytes.len() <= BROWSER_SCREENSHOT_CHUNK_BYTES);
            assert!(received.len() + bytes.len() <= expected.len());
            received.extend_from_slice(&bytes);
            if received.len() == expected.len() {
                break;
            }
            assert!(
                !bytes.is_empty(),
                "complete native file cannot be truncated"
            );
        }
        assert!(
            *received == *expected,
            "actual HTTP download bytes must match"
        );
        read.offset = 0;
        assert_eq!(
            driver.read_download_chunk(read, control).await,
            Err(BrowserDriverError::Stale)
        );
    }
    let denied = element(
        driver,
        opened,
        target,
        control,
        132,
        "Forbidden binary download",
    )
    .await;
    assert_eq!(
        driver
            .download(
                super::exercise::command(
                    opened,
                    target,
                    BrowserAction::Download {
                        element: denied,
                        max_bytes: MAX_BROWSER_TRANSFER_BYTES
                    },
                    232,
                ),
                control
            )
            .await,
        Err(BrowserDriverError::Denied),
        "private file custody does not expand immutable origin authority",
    );
}

async fn element(
    driver: &dyn BrowserDriver,
    opened: &BrowserDriverOpenRequest,
    target: &mut BrowserTarget,
    control: &BrowserDriverControl,
    index: u32,
    name: &str,
) -> BrowserElementRef {
    let observed = super::exercise::snapshot(driver, opened, target, control, index).await;
    target.document_id = observed.tab.document_id;
    observed
        .snapshot
        .expect("actual native semantic snapshot")
        .nodes
        .into_iter()
        .find(|node| node.name == name)
        .expect("fresh native transfer element")
        .element
}
