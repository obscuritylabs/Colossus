//! Actual native PNG receipt on Windows's authenticated bounded private bridge.
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{BrowserAction, BrowserTarget};
use colossus_ports::{
    BROWSER_SCREENSHOT_CHUNK_BYTES, BrowserDriver, BrowserDriverCommand, BrowserDriverControl,
    BrowserDriverError, BrowserDriverOpenRequest, BrowserScreenshotReadRequest,
    MAX_BROWSER_SCREENSHOT_BYTES,
};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

pub async fn check(
    driver: &dyn BrowserDriver,
    opened: &BrowserDriverOpenRequest,
    target: &BrowserTarget,
    control: &BrowserDriverControl,
) {
    let descriptor = driver
        .capture(
            BrowserDriverCommand {
                binding: opened.binding.clone(),
                run_id: "native-windows-acceptance-run".into(),
                session_id: opened.session_id.clone(),
                target: target.clone(),
                control_generation: 1,
                action: BrowserAction::Screenshot {
                    max_bytes: MAX_BROWSER_SCREENSHOT_BYTES,
                },
                next_document_id: target.document_id.clone(),
                snapshot_id: None,
                new_tab: None,
            },
            control,
        )
        .await
        .expect("actual native PNG capture required");
    assert_eq!(descriptor.session_id, opened.session_id);
    assert_eq!(descriptor.target, *target);
    assert_eq!((descriptor.width, descriptor.height), (1000, 750));
    assert!((33..=MAX_BROWSER_SCREENSHOT_BYTES).contains(&descriptor.size_bytes));
    let mut bytes = Zeroizing::new(Vec::new());
    let mut request = BrowserScreenshotReadRequest {
        binding: opened.binding.clone(),
        run_id: "native-windows-acceptance-run".into(),
        session_id: opened.session_id.clone(),
        target: target.clone(),
        control_generation: 1,
        transfer_id: descriptor.transfer_id,
        offset: 0,
    };
    while bytes.len() < descriptor.size_bytes as usize {
        request.offset = bytes.len() as u32;
        let chunk = driver
            .read_screenshot_chunk(request.clone(), control)
            .await
            .expect("bounded authenticated PNG chunk required");
        assert_eq!(chunk.offset, request.offset);
        assert!(chunk.data_base64.len() <= BROWSER_SCREENSHOT_CHUNK_BYTES.div_ceil(3) * 4);
        let decoded = Zeroizing::new(
            BASE64
                .decode(&chunk.data_base64)
                .expect("canonical private PNG chunk"),
        );
        assert!(!decoded.is_empty() && decoded.len() <= BROWSER_SCREENSHOT_CHUNK_BYTES);
        assert!(bytes.len() + decoded.len() <= descriptor.size_bytes as usize);
        bytes.extend_from_slice(&decoded);
    }
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(hex::encode(Sha256::digest(&*bytes)), descriptor.sha256);
    request.offset = 0;
    assert_eq!(
        driver.read_screenshot_chunk(request, control).await,
        Err(BrowserDriverError::Stale),
        "consumed capture cannot be replayed"
    );
}
