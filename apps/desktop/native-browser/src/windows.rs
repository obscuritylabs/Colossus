//! `WebView2` operations run only inside Tauri's owning-thread callback.

use tauri::webview::PlatformWebview;
use webview2_com::{
    ClearBrowsingDataCompletedHandler, ClientCertificateRequestedEventHandler, CoTaskMemPWSTR,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_PROFILE, COREWEBVIEW2_PERMISSION_STATE_DENY,
        COREWEBVIEW2_WEB_ERROR_STATUS, COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL, ICoreWebView2, ICoreWebView2_5, ICoreWebView2_13,
        ICoreWebView2Controller4, ICoreWebView2Environment,
        ICoreWebView2NavigationCompletedEventArgs2, ICoreWebView2Profile2, ICoreWebView2Settings2,
        ICoreWebView2Settings3, ICoreWebView2Settings4,
    },
    NavigationCompletedEventHandler, NavigationStartingEventHandler,
    PermissionRequestedEventHandler, ProcessFailedEventHandler, WebResourceRequestedEventHandler,
};
use windows::{
    Win32::{
        Foundation::HWND,
        System::Com::IStream,
        UI::{
            Shell::ShellExecuteW,
            WindowsAndMessaging::{GA_ROOT, GetAncestor, GetForegroundWindow, SW_SHOWNORMAL},
        },
    },
    core::{BOOL, HSTRING, Interface, PWSTR, w},
};

use crate::{BrowserError, BrowserEvent, EventSink, NavigationAction, NavigationPolicy, PageState};

/// A fixed engine policy, not a renderer-accessible `DevTools` endpoint. Install
/// before the first navigation and fail closed if the runtime cannot apply it.
pub(crate) async fn restrict_file_picker(view: &tauri::Webview) -> Result<(), BrowserError> {
    fixed_protocol_command(
        view,
        "Page.setInterceptFileChooserDialog",
        r#"{"enabled":true,"cancel":true}"#,
    )
    .await
    .map(|_| ())
}

async fn fixed_protocol_command(
    view: &tauri::Webview,
    method: &'static str,
    parameters: &'static str,
) -> Result<String, BrowserError> {
    let (send, receive) = tokio::sync::oneshot::channel();
    view.with_webview(move |native| {
        let send = std::sync::Arc::new(std::sync::Mutex::new(Some(send)));
        let completed = send.clone();
        // SAFETY: The COM view and completion handler remain on their owning
        // apartment. The command and its parameters are fixed by native code.
        let result = unsafe {
            native.controller().CoreWebView2().and_then(|view| {
                view.CallDevToolsProtocolMethod(
                    &HSTRING::from(method),
                    &HSTRING::from(parameters),
                    &webview2_com::CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                        move |result, value| {
                            if let Ok(mut sender) = completed.lock()
                                && let Some(sender) = sender.take()
                            {
                                let _ = sender.send(
                                    result
                                        .map(|()| value)
                                        .map_err(|_| BrowserError::Unavailable),
                                );
                            }
                            Ok(())
                        },
                    )),
                )
            })
        };
        if result.is_err()
            && let Ok(mut sender) = send.lock()
            && let Some(sender) = sender.take()
        {
            let _ = sender.send(Err(BrowserError::Unavailable));
        }
    })
    .map_err(|_| BrowserError::Closed)?;
    tokio::time::timeout(std::time::Duration::from_secs(5), receive)
        .await
        .map_err(|_| BrowserError::TimedOut)?
        .map_err(|_| BrowserError::Closed)?
}

pub(crate) fn harden(
    native: &PlatformWebview,
    policy: NavigationPolicy,
    sink: EventSink,
) -> Result<(), BrowserError> {
    // SAFETY: Tauri dispatches this closure to the controller's apartment thread.
    // COM interfaces and callback arguments remain owned for each call; callbacks
    // retain only policy, event sinks, and reference-counted native handles.
    unsafe { configure(native, policy, sink, false, "") }.map_err(|_| BrowserError::Unavailable)
}

pub(crate) fn harden_human_profile(
    native: &PlatformWebview,
    policy: NavigationPolicy,
    sink: EventSink,
    persistent: bool,
    user_agent: &str,
) -> Result<(), BrowserError> {
    // SAFETY: Called only from Tauri's owning-apartment dispatch. The caller
    // derives the profile and mode from trusted native workspace preferences.
    unsafe { configure(native, policy, sink, persistent, user_agent) }
        .map_err(|_| BrowserError::Unavailable)
}

unsafe fn configure(
    native: &PlatformWebview,
    policy: NavigationPolicy,
    sink: EventSink,
    persistent: bool,
    user_agent: &str,
) -> windows::core::Result<()> {
    let view = unsafe { configure_settings(native, persistent, user_agent)? };
    let latest = unsafe { register_navigation_policy(&view, &policy, &sink)? };
    unsafe {
        register_navigation_completion(&view, &sink, latest)?;
        register_security_observers(&view, sink)?;
        restrict_resources(&view, native.environment(), policy)
    }
}

unsafe fn register_navigation_policy(
    view: &ICoreWebView2,
    policy: &NavigationPolicy,
    sink: &EventSink,
) -> windows::core::Result<std::sync::Arc<std::sync::atomic::AtomicU64>> {
    let mut token = 0;
    let latest_navigation = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let navigation_epoch = latest_navigation.clone();
    let navigation_policy = policy.clone();
    let navigation_sink = sink.clone();
    unsafe {
        view.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let mut navigation_id = 0;
                    args.NavigationId(&raw mut navigation_id)?;
                    navigation_epoch.store(navigation_id, std::sync::atomic::Ordering::Release);
                    let url = read_string(|value| args.Uri(value))?;
                    if navigation_policy.allows(&url) {
                        let mut redirected = BOOL::default();
                        args.IsRedirected(&raw mut redirected)?;
                        if redirected.as_bool() {
                            navigation_sink(BrowserEvent::Redirected);
                        }
                        navigation_sink(BrowserEvent::Loading(true));
                    } else {
                        args.SetCancel(true)?;
                        navigation_sink(BrowserEvent::Blocked);
                    }
                }
                Ok(())
            })),
            &raw mut token,
        )?;
    }
    let frame_policy = policy.clone();
    unsafe {
        view.add_FrameNavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args
                    && !frame_policy.allows(&read_string(|value| args.Uri(value))?)
                {
                    args.SetCancel(true)?;
                }
                Ok(())
            })),
            &raw mut token,
        )?;
    }
    Ok(latest_navigation)
}

unsafe fn register_navigation_completion(
    view: &ICoreWebView2,
    sink: &EventSink,
    latest_navigation: std::sync::Arc<std::sync::atomic::AtomicU64>,
) -> windows::core::Result<()> {
    let mut token = 0;
    let completed_sink = sink.clone();
    unsafe {
        view.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let mut navigation_id = 0;
                    args.NavigationId(&raw mut navigation_id)?;
                    if latest_navigation.load(std::sync::atomic::Ordering::Acquire) != navigation_id
                    {
                        return Ok(());
                    }
                    completed_sink(BrowserEvent::Loading(false));
                    let mut success = BOOL::default();
                    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                    args.IsSuccess(&raw mut success)?;
                    args.WebErrorStatus(&raw mut status)?;
                    let mut http_status = 0;
                    args.cast::<ICoreWebView2NavigationCompletedEventArgs2>()?
                        .HttpStatusCode(&raw mut http_status)?;
                    if let Ok(http_status) = u16::try_from(http_status)
                        && (100..=599).contains(&http_status)
                    {
                        completed_sink(BrowserEvent::HttpStatus(http_status));
                    }
                    if !success.as_bool()
                        && status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED
                    {
                        completed_sink(BrowserEvent::NavigationFailure(status.0));
                        completed_sink(BrowserEvent::Failed);
                        // WebView2's documented certificate error categories.
                        if (1..=5).contains(&status.0) {
                            completed_sink(BrowserEvent::TlsFailed);
                        }
                    }
                }
                Ok(())
            })),
            &raw mut token,
        )?;
    }
    Ok(())
}

unsafe fn register_security_observers(
    view: &ICoreWebView2,
    sink: EventSink,
) -> windows::core::Result<()> {
    let mut token = 0;
    let identity_sink = sink.clone();
    unsafe {
        view.cast::<ICoreWebView2_5>()?
            .add_ClientCertificateRequested(
                &ClientCertificateRequestedEventHandler::create(Box::new(move |_, _| {
                    // Observe only. Leave Handled and Cancel at their native defaults
                    // so WebView2 shows the operating-system certificate chooser.
                    identity_sink(BrowserEvent::AuthenticationRequired);
                    Ok(())
                })),
                &raw mut token,
            )?;
    }
    let permission_sink = sink.clone();
    unsafe {
        view.add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                }
                permission_sink(BrowserEvent::Blocked);
                Ok(())
            })),
            &raw mut token,
        )?;
    }
    unsafe {
        view.add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new(move |_, _| {
                sink(BrowserEvent::Crashed);
                Ok(())
            })),
            &raw mut token,
        )?;
    }
    Ok(())
}

unsafe fn restrict_resources(
    view: &ICoreWebView2,
    environment: ICoreWebView2Environment,
    policy: NavigationPolicy,
) -> windows::core::Result<()> {
    // Restrict fetches/frames as well as navigation. This does not claim to be a
    // process-wide private-network firewall (for example, DNS rebinding).
    let mut token = 0;
    unsafe {
        view.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
        view.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let address = read_string(|value| args.Request()?.Uri(value))?;
                    if !policy.allows(&address)
                        && !address.starts_with("data:")
                        && !address.starts_with("blob:")
                    {
                        let response = environment.CreateWebResourceResponse(
                            None::<&IStream>,
                            403,
                            w!("Blocked"),
                            w!("Content-Type: text/plain"),
                        )?;
                        args.SetResponse(&response)?;
                    }
                }
                Ok(())
            })),
            &raw mut token,
        )?;
    }
    Ok(())
}

unsafe fn configure_settings(
    native: &PlatformWebview,
    persistent: bool,
    user_agent: &str,
) -> windows::core::Result<ICoreWebView2> {
    let controller = native.controller();
    let view = unsafe { controller.CoreWebView2()? };
    let mut private = BOOL::default();
    unsafe {
        view.cast::<ICoreWebView2_13>()?
            .Profile()?
            .IsInPrivateModeEnabled(&raw mut private)?;
    }
    if private.as_bool() == persistent {
        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x8000_4005_u32.cast_signed(),
        )));
    }
    let settings = unsafe { view.Settings()? };
    if !user_agent.is_empty() {
        // Verify the real native value before permitting external navigation.
        let actual = unsafe {
            read_string(|out| settings.cast::<ICoreWebView2Settings2>()?.UserAgent(out))?
        };
        if actual != user_agent {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x8000_4005_u32.cast_signed(),
            )));
        }
    }
    unsafe {
        settings.SetAreHostObjectsAllowed(false)?;
        settings.SetIsWebMessageEnabled(false)?;
        settings.SetAreDefaultScriptDialogsEnabled(false)?;
        settings.SetAreDefaultContextMenusEnabled(false)?;
        settings.SetAreDevToolsEnabled(false)?;
        settings
            .cast::<ICoreWebView2Settings3>()?
            .SetAreBrowserAcceleratorKeysEnabled(false)?;
        settings
            .cast::<ICoreWebView2Settings4>()?
            .SetIsPasswordAutosaveEnabled(false)?;
        settings
            .cast::<ICoreWebView2Settings4>()?
            .SetIsGeneralAutofillEnabled(false)?;
        controller
            .cast::<ICoreWebView2Controller4>()?
            .SetAllowExternalDrop(false)?;
    }
    Ok(view)
}

pub(crate) async fn clear_site_data(view: &tauri::Webview) -> Result<(), BrowserError> {
    let (send, receive) = tokio::sync::oneshot::channel();
    view.with_webview(move |native| {
        let sender = std::sync::Arc::new(std::sync::Mutex::new(Some(send)));
        let completion = sender.clone();
        // SAFETY: Native profile and callback are used on their owning apartment.
        // ALL_PROFILE is a fixed mask; no renderer-provided cookie data crosses IPC.
        let result = unsafe {
            native.controller().CoreWebView2().and_then(|view| {
                view.cast::<ICoreWebView2_13>()?
                    .Profile()?
                    .cast::<ICoreWebView2Profile2>()?
                    .ClearBrowsingData(
                        COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_PROFILE,
                        &ClearBrowsingDataCompletedHandler::create(Box::new(move |result| {
                            if let Ok(mut sender) = completion.lock()
                                && let Some(sender) = sender.take()
                            {
                                let _ = sender.send(result.map_err(|_| BrowserError::Unavailable));
                            }
                            Ok(())
                        })),
                    )
            })
        };
        if result.is_err()
            && let Ok(mut sender) = sender.lock()
            && let Some(sender) = sender.take()
        {
            let _ = sender.send(Err(BrowserError::Unavailable));
        }
    })
    .map_err(|_| BrowserError::Closed)?;
    tokio::time::timeout(std::time::Duration::from_secs(30), receive)
        .await
        .map_err(|_| BrowserError::TimedOut)?
        .map_err(|_| BrowserError::Closed)?
}

fn read_string(
    read: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>,
) -> windows::core::Result<String> {
    let mut pointer = PWSTR::null();
    read(&raw mut pointer)?;
    Ok(CoTaskMemPWSTR::from(pointer).to_string())
}

pub(crate) fn control(
    native: &PlatformWebview,
    action: NavigationAction,
) -> Result<(), BrowserError> {
    // SAFETY: The controller and its view are used only on their apartment thread.
    unsafe {
        let view = native
            .controller()
            .CoreWebView2()
            .map_err(|_| BrowserError::Closed)?;
        match action {
            NavigationAction::Back => view.GoBack(),
            NavigationAction::Forward => view.GoForward(),
            NavigationAction::Reload => view.Reload(),
            NavigationAction::Stop => view.Stop(),
        }
        .map_err(|_| BrowserError::Closed)
    }
}

pub(crate) fn inspect(native: &PlatformWebview) -> Result<PageState, BrowserError> {
    // SAFETY: WebView2 owns returned strings; read_string releases each allocation.
    unsafe {
        let view = native
            .controller()
            .CoreWebView2()
            .map_err(|_| BrowserError::Closed)?;
        let mut back = BOOL::default();
        let mut forward = BOOL::default();
        view.CanGoBack(&raw mut back)
            .map_err(|_| BrowserError::Closed)?;
        view.CanGoForward(&raw mut forward)
            .map_err(|_| BrowserError::Closed)?;
        Ok(PageState {
            url: read_string(|out| view.Source(out))
                .map_err(|_| BrowserError::Closed)?
                .chars()
                .take(8_192)
                .collect(),
            title: read_string(|out| view.DocumentTitle(out))
                .map_err(|_| BrowserError::Closed)?
                .chars()
                .filter(|c| !c.is_control())
                .take(256)
                .collect(),
            can_go_back: back.as_bool(),
            can_go_forward: forward.as_bool(),
            loading: false,
        })
    }
}

pub(crate) fn open_external(address: &str) -> Result<(), BrowserError> {
    // SAFETY: Validated HTTP(S) URL, no shell command or caller-selected arguments.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(address),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(BrowserError::Unavailable)
    }
}

pub(crate) fn is_active(native: &PlatformWebview) -> Result<bool, BrowserError> {
    // SAFETY: Called on the controller's apartment. GetAncestor compares the
    // top-level owner, so moving keyboard focus into a guest is not focus loss.
    unsafe {
        let mut parent = HWND::default();
        native
            .controller()
            .ParentWindow(&raw mut parent)
            .map_err(|_| BrowserError::Closed)?;
        let root = GetAncestor(parent, GA_ROOT);
        Ok(!root.is_invalid() && root == GetForegroundWindow())
    }
}

/// Synthetic user-activation probe. Never present in ordinary builds or IPC.
///
/// # Errors
/// Fails if the engine cannot execute the test or the picker remains open.
#[cfg(feature = "native-test-driver")]
pub async fn probe_file_picker(view: &tauri::Webview) -> Result<String, BrowserError> {
    fixed_protocol_command(view, "Runtime.evaluate", r#"{"expression":"new Promise(resolve => { const input = document.createElement('input'); input.type = 'file'; input.addEventListener('cancel', () => resolve(true)); document.body.append(input); input.showPicker(); setTimeout(() => resolve(false), 1500); })","awaitPromise":true,"returnByValue":true,"userGesture":true}"#).await
}
