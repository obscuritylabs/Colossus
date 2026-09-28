//! `WebView2` operations run only inside Tauri's owning-thread callback.

use tauri::webview::PlatformWebview;
use webview2_com::{
    CoTaskMemPWSTR,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PERMISSION_STATE_DENY, COREWEBVIEW2_WEB_ERROR_STATUS,
        COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        ICoreWebView2, ICoreWebView2_13, ICoreWebView2Controller4, ICoreWebView2Environment,
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

pub(crate) fn harden(
    native: &PlatformWebview,
    policy: NavigationPolicy,
    sink: EventSink,
) -> Result<(), BrowserError> {
    // SAFETY: Tauri dispatches this closure to the controller's apartment thread.
    // COM interfaces and callback arguments remain owned for each call; callbacks
    // retain only policy, event sinks, and reference-counted native handles.
    unsafe { configure(native, policy, sink) }.map_err(|_| BrowserError::Unavailable)
}

unsafe fn configure(
    native: &PlatformWebview,
    policy: NavigationPolicy,
    sink: EventSink,
) -> windows::core::Result<()> {
    let view = unsafe { configure_settings(native)? };
    let mut token = 0;
    let navigation_policy = policy.clone();
    let navigation_sink = sink.clone();
    unsafe {
        view.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let url = read_string(|value| args.Uri(value))?;
                    if navigation_policy.allows(&url) {
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
    let completed_sink = sink.clone();
    unsafe {
        view.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                completed_sink(BrowserEvent::Loading(false));
                if let Some(args) = args {
                    let mut success = BOOL::default();
                    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                    args.IsSuccess(&raw mut success)?;
                    args.WebErrorStatus(&raw mut status)?;
                    if !success.as_bool()
                        && status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED
                    {
                        completed_sink(BrowserEvent::Failed);
                    }
                }
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
    unsafe { restrict_resources(&view, native.environment(), policy) }
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

unsafe fn configure_settings(native: &PlatformWebview) -> windows::core::Result<ICoreWebView2> {
    let controller = native.controller();
    let view = unsafe { controller.CoreWebView2()? };
    let mut private = BOOL::default();
    unsafe {
        view.cast::<ICoreWebView2_13>()?
            .Profile()?
            .IsInPrivateModeEnabled(&raw mut private)?;
    }
    if !private.as_bool() {
        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x8000_4005_u32.cast_signed(),
        )));
    }
    let settings = unsafe { view.Settings()? };
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
