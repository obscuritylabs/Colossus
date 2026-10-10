//! Exact profile custody is authenticated with the immutable native enrollment.
use crate::{
    framing::AuthenticatedChannel,
    wire::{ErrorCode, Request, Response},
};
use crate::{tests::*, *};
use async_trait::async_trait;
use colossus_contracts::*;
use colossus_ports::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Default)]
struct Native(AtomicUsize);
#[async_trait]
impl BrowserDriver for Native {
    fn capabilities(&self) -> BrowserCapabilities {
        BrowserCapabilities::unavailable()
    }
    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        _: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(BrowserTabSummary {
            tab_id: request.tab_id,
            document_id: request.document_id,
            title: String::new(),
            origin: None,
        })
    }
    async fn execute(
        &self,
        _: BrowserDriverCommand,
        _: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        Err(BrowserDriverError::Unsupported)
    }
    async fn cancel_session(&self, _: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        Ok(())
    }
    async fn close_session(&self, _: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        Ok(())
    }
}
fn workspace(byte: char) -> BrowserProfileSelection {
    BrowserProfileSelection::Workspace {
        id: BrowserProfileId::parse(format!("bp_{}", byte.to_string().repeat(32))).unwrap(),
    }
}
#[test]
fn private_enrollment_requires_an_explicit_closed_profile() {
    let mut value = serde_json::to_value(enrollment()).unwrap();
    value.as_object_mut().unwrap().remove("profile");
    assert!(serde_json::from_value::<BrowserBridgeEnrollment>(value).is_err());
    let mut value = serde_json::to_value(enrollment()).unwrap();
    value["profile"] = serde_json::json!({"kind":"workspace","id":"bp_01234567890123456789012345678901","path":"/tmp/profile"});
    assert!(serde_json::from_value::<BrowserBridgeEnrollment>(value).is_err());
}
#[tokio::test]
async fn client_and_host_require_identical_profile_selection_before_allocation() {
    for profile in [BrowserProfileSelection::Temporary, workspace('5')] {
        let (data, host_data) = channel_pair();
        let (ctl, host_ctl) = channel_pair();
        let mut admitted = enrollment();
        admitted.profile = profile.clone();
        let native = Arc::new(Native::default());
        let host = tokio::spawn(serve_browser_host(
            host_data,
            host_ctl,
            admitted.clone(),
            BrowserBridgeKey::from_bootstrap(zeroize::Zeroizing::new([7; 32])),
            native.clone(),
        ));
        let bridge = BrowserBridgeDriver::connect(
            data,
            ctl,
            admitted,
            BrowserBridgeKey::from_bootstrap(zeroize::Zeroizing::new([7; 32])),
        )
        .await
        .unwrap();
        let mut request = open_request();
        request.options.profile = workspace('6');
        assert_eq!(
            bridge.open_session(request.clone(), &control()).await,
            Err(BrowserDriverError::Denied)
        );
        assert_eq!(native.0.load(Ordering::SeqCst), 0);
        request.options.profile = profile;
        bridge
            .open_session(request.clone(), &control())
            .await
            .unwrap();
        assert_eq!(native.0.load(Ordering::SeqCst), 1);
        bridge.close_session(&request.session_id).await.unwrap();
        bridge.disconnect_for_shutdown();
        drop(bridge);
        host.await.unwrap().unwrap();
    }
}
#[tokio::test]
async fn authenticated_client_cannot_bypass_host_profile_comparison() {
    let (data, host_data) = channel_pair();
    let (_ctl, host_ctl) = channel_pair();
    let admitted = enrollment();
    let native = Arc::new(Native::default());
    let host = tokio::spawn(serve_browser_host(
        host_data,
        host_ctl,
        admitted.clone(),
        BrowserBridgeKey::from_bootstrap(zeroize::Zeroizing::new([7; 32])),
        native.clone(),
    ));
    let mut data = AuthenticatedChannel::new(
        data,
        Arc::new(BrowserBridgeKey::from_bootstrap(zeroize::Zeroizing::new(
            [7; 32],
        ))),
        &admitted,
        b"data",
    )
    .unwrap();
    let mut request = open_request();
    request.options.profile = workspace('5');
    data.write(
        b"request",
        1,
        &Request::Open {
            request: Box::new(request),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        data.read::<Response>(b"response", 1).await.unwrap(),
        Response::Rejected {
            code: ErrorCode::Denied
        }
    ));
    assert_eq!(native.0.load(Ordering::SeqCst), 0);
    drop(data);
    host.await.unwrap().unwrap();
}
