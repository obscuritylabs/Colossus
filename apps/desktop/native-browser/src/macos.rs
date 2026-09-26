//! `WKWebView` delegates retained only for the lifetime of a guest.

use std::{cell::RefCell, collections::HashMap};

use block2::DynBlock;
use objc2::{MainThreadOnly, define_class, msg_send, rc::Retained, runtime::ProtocolObject};
use objc2_foundation::{MainThreadMarker, NSArray, NSError, NSObject, NSObjectProtocol, NSURL};
use objc2_web_kit::{
    WKFrameInfo, WKMediaCaptureType, WKNavigation, WKNavigationAction, WKNavigationActionPolicy,
    WKNavigationDelegate, WKNavigationResponse, WKNavigationResponsePolicy, WKOpenPanelParameters,
    WKPermissionDecision, WKSecurityOrigin, WKUIDelegate, WKWebView, WKWebViewConfiguration,
    WKWindowFeatures,
};
use tauri::webview::PlatformWebview;

use crate::{BrowserError, BrowserEvent, EventSink, NavigationAction, NavigationPolicy, PageState};

thread_local! {
    static DELEGATES: RefCell<HashMap<usize, Retained<GuestDelegate>>> = RefCell::default();
}

struct DelegateState {
    policy: NavigationPolicy,
    sink: EventSink,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateState]
    struct GuestDelegate;

    unsafe impl NSObjectProtocol for GuestDelegate {}

    unsafe impl WKNavigationDelegate for GuestDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide(
            &self,
            _: &WKWebView,
            action: &WKNavigationAction,
            reply: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            // SAFETY: WebKit supplies valid native objects for the callback duration.
            let (url, download) = unsafe {
                (
                    action
                        .request()
                        .URL()
                        .and_then(|u| u.absoluteString())
                        .map(|u| u.to_string()),
                    action.shouldPerformDownload(),
                )
            };
            let allowed = !download
                && url
                    .as_deref()
                    .is_some_and(|u| self.ivars().policy.allows(u));
            if !allowed {
                (self.ivars().sink)(if download {
                    BrowserEvent::Download
                } else {
                    BrowserEvent::Blocked
                });
            }
            reply.call((if allowed {
                WKNavigationActionPolicy::Allow
            } else {
                WKNavigationActionPolicy::Cancel
            },));
        }

        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn response(
            &self,
            _: &WKWebView,
            response: &WKNavigationResponse,
            reply: &DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            let allowed = unsafe { response.canShowMIMEType() };
            if !allowed {
                (self.ivars().sink)(BrowserEvent::Download);
            }
            reply.call((if allowed {
                WKNavigationResponsePolicy::Allow
            } else {
                WKNavigationResponsePolicy::Cancel
            },));
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn started(&self, _: &WKWebView, _: Option<&WKNavigation>) {
            (self.ivars().sink)(BrowserEvent::Loading(true));
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn finished(&self, _: &WKWebView, _: Option<&WKNavigation>) {
            (self.ivars().sink)(BrowserEvent::Loading(false));
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn provisional_failed(&self, _: &WKWebView, _: Option<&WKNavigation>, error: &NSError) {
            (self.ivars().sink)(BrowserEvent::Loading(false));
            if error.code() != -999 {
                (self.ivars().sink)(BrowserEvent::Failed);
            }
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn failed(&self, _: &WKWebView, _: Option<&WKNavigation>, error: &NSError) {
            (self.ivars().sink)(BrowserEvent::Loading(false));
            if error.code() != -999 {
                (self.ivars().sink)(BrowserEvent::Failed);
            }
        }

        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn terminated(&self, _: &WKWebView) {
            (self.ivars().sink)(BrowserEvent::Crashed);
        }
    }

    unsafe impl WKUIDelegate for GuestDelegate {
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn popup(
            &self,
            _: &WKWebView,
            _: &WKWebViewConfiguration,
            action: &WKNavigationAction,
            _: &WKWindowFeatures,
        ) -> Option<Retained<WKWebView>> {
            if let Some(url) = unsafe { action.request().URL().and_then(|u| u.absoluteString()) } {
                (self.ivars().sink)(BrowserEvent::Popup(
                    url.to_string().chars().take(8_192).collect(),
                ));
            }
            None
        }

        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn media(
            &self,
            _: &WKWebView,
            _: &WKSecurityOrigin,
            _: &WKFrameInfo,
            _: WKMediaCaptureType,
            reply: &DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            reply.call((WKPermissionDecision::Deny,));
            (self.ivars().sink)(BrowserEvent::Blocked);
        }

        #[unsafe(method(webView:requestDeviceOrientationAndMotionPermissionForOrigin:initiatedByFrame:decisionHandler:))]
        fn motion(
            &self,
            _: &WKWebView,
            _: &WKSecurityOrigin,
            _: &WKFrameInfo,
            reply: &DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            reply.call((WKPermissionDecision::Deny,));
        }

        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        fn upload(
            &self,
            _: &WKWebView,
            _: &WKOpenPanelParameters,
            _: &WKFrameInfo,
            reply: &DynBlock<dyn Fn(*mut NSArray<NSURL>)>,
        ) {
            reply.call((std::ptr::null_mut(),));
            (self.ivars().sink)(BrowserEvent::Blocked);
        }
    }
);

pub(crate) fn harden(
    native: &PlatformWebview,
    policy: NavigationPolicy,
    sink: EventSink,
) -> Result<(), BrowserError> {
    let mtm = MainThreadMarker::new().ok_or(BrowserError::Unavailable)?;
    // SAFETY: Tauri supplies a live WKWebView on the main thread. The borrowed
    // object never escapes this callback. Delegates are retained in a main-thread
    // map until release(), since WebKit's delegate properties are weak.
    let view = unsafe { &*native.inner().cast::<WKWebView>() };
    unsafe {
        if view.configuration().websiteDataStore().isPersistent() {
            return Err(BrowserError::Unavailable);
        }
        let scripts = view.configuration().userContentController();
        scripts.removeAllUserScripts();
        scripts.removeAllScriptMessageHandlers();
        let preferences = view.configuration().preferences();
        preferences.setJavaScriptCanOpenWindowsAutomatically(false);
        preferences.setElementFullscreenEnabled(false);
        let allocated = GuestDelegate::alloc(mtm).set_ivars(DelegateState { policy, sink });
        let delegate: Retained<GuestDelegate> = msg_send![super(allocated), init];
        view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        view.setUIDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        DELEGATES.with(|items| {
            items.borrow_mut().insert(native.inner() as usize, delegate);
        });
    }
    Ok(())
}

pub(crate) fn share_session(
    builder: tauri::webview::WebviewBuilder<tauri::Wry>,
    native: &PlatformWebview,
) -> Result<tauri::webview::WebviewBuilder<tauri::Wry>, BrowserError> {
    let mtm = MainThreadMarker::new().ok_or(BrowserError::Unavailable)?;
    // SAFETY: The source belongs to this browser session and is borrowed only on
    // the main thread. Only its temporary data store is shared, never its scripts.
    let view = unsafe { &*native.inner().cast::<WKWebView>() };
    let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
    unsafe {
        configuration.setWebsiteDataStore(&view.configuration().websiteDataStore());
    }
    Ok(builder.with_webview_configuration(configuration))
}

pub(crate) fn release(native: &PlatformWebview) -> Result<(), BrowserError> {
    let _mtm = MainThreadMarker::new().ok_or(BrowserError::Unavailable)?;
    // SAFETY: Tauri supplies the live guest on the verified owning main thread.
    let view = unsafe { &*native.inner().cast::<WKWebView>() };
    unsafe {
        view.setNavigationDelegate(None);
        view.setUIDelegate(None);
    }
    DELEGATES.with(|items| {
        items.borrow_mut().remove(&(native.inner() as usize));
    });
    Ok(())
}

pub(crate) fn control(
    native: &PlatformWebview,
    action: NavigationAction,
) -> Result<(), BrowserError> {
    let _mtm = MainThreadMarker::new().ok_or(BrowserError::Unavailable)?;
    // SAFETY: Tauri keeps the native object alive for this main-thread callback.
    let view = unsafe { &*native.inner().cast::<WKWebView>() };
    unsafe {
        match action {
            NavigationAction::Back => {
                view.goBack();
            }
            NavigationAction::Forward => {
                view.goForward();
            }
            NavigationAction::Reload => {
                view.reload();
            }
            NavigationAction::Stop => view.stopLoading(),
        }
    }
    Ok(())
}

pub(crate) fn inspect(native: &PlatformWebview) -> Result<PageState, BrowserError> {
    let _mtm = MainThreadMarker::new().ok_or(BrowserError::Unavailable)?;
    let view = unsafe { &*native.inner().cast::<WKWebView>() };
    // SAFETY: All properties are read on the owning main thread.
    unsafe {
        Ok(PageState {
            url: view
                .URL()
                .and_then(|url| url.absoluteString())
                .map(|s| s.to_string().chars().take(8_192).collect())
                .unwrap_or_default(),
            title: view
                .title()
                .map(|s| {
                    s.to_string()
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(256)
                        .collect()
                })
                .unwrap_or_default(),
            loading: view.isLoading(),
            can_go_back: view.canGoBack(),
            can_go_forward: view.canGoForward(),
        })
    }
}

pub(crate) fn open_external(address: &str) -> Result<(), BrowserError> {
    let _mtm = MainThreadMarker::new().ok_or(BrowserError::Unavailable)?;
    let url = NSURL::URLWithString(&objc2_foundation::NSString::from_str(address))
        .ok_or(BrowserError::InvalidAddress)?;
    // A validated HTTP(S) URL is passed directly to NSWorkspace on the main thread.
    if objc2_app_kit::NSWorkspace::sharedWorkspace().openURL(&url) {
        Ok(())
    } else {
        Err(BrowserError::Unavailable)
    }
}

pub(crate) fn is_active(native: &PlatformWebview) -> Result<bool, BrowserError> {
    let _mtm = MainThreadMarker::new().ok_or(BrowserError::Unavailable)?;
    // SAFETY: The live view is borrowed only on the verified owning main thread.
    let view = unsafe { &*native.inner().cast::<WKWebView>() };
    Ok(view.window().is_some_and(|window| window.isKeyWindow()))
}
