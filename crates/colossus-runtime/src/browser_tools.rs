//! Runtime-only browser dispatch, ownership, and permit-bearing effect adapter.

mod arguments;
mod download;
mod effect;
mod execution;
mod host;
mod lifecycle;
mod native;
mod public;
mod screenshot;
mod service;
mod transfer;

pub(super) use execution::BrowserToolExecutor;
pub use host::RuntimeBrowserHost;
pub(super) use lifecycle::RuntimeRunLifecycle;
pub use native::{
    NativeBrowserOpenRequest, RuntimeBrowserPresenter, RuntimeNativeBrowserAuthority,
    RuntimeNativeBrowserError, RuntimeNativeBrowserGranted, RuntimeNativeBrowserSession,
};
pub(super) use service::RuntimeBrowserTools;

pub(super) fn supports_tool(
    capabilities: Option<&colossus_contracts::BrowserCapabilities>,
    name: &str,
) -> bool {
    use colossus_contracts::BrowserActionKind as Kind;
    if !name.starts_with("browser.") {
        return true;
    }
    let Some(capabilities) = capabilities else {
        return false;
    };
    if !capabilities.available || capabilities.modes.is_empty() {
        return false;
    }
    let kind = match name {
        "browser.open" | "browser.close" | "browser.status" | "browser.tabs" => return true,
        "browser.navigate" => Kind::Navigate,
        "browser.snapshot" => Kind::Snapshot,
        "browser.screenshot" => Kind::Screenshot,
        "browser.screenshot.annotated" => Kind::AnnotatedScreenshot,
        "browser.upload" => Kind::Upload,
        "browser.download" => Kind::Download,
        "browser.click" => Kind::Click,
        "browser.evaluate" => Kind::Evaluate,
        "browser.cdp" => Kind::Cdp,
        "browser.hover" => Kind::Hover,
        "browser.double_click" => Kind::DoubleClick,
        "browser.right_click" => Kind::RightClick,
        "browser.drag" => Kind::Drag,
        "browser.click_at" => Kind::ClickAt,
        "browser.drag_at" => Kind::DragAt,
        "browser.annotate" => Kind::Annotate,
        "browser.annotation.clear" => Kind::AnnotationClear,
        "browser.fill" => Kind::Fill,
        "browser.select" => Kind::Select,
        "browser.press" => Kind::Press,
        "browser.scroll" => Kind::Scroll,
        "browser.wait" => Kind::Wait,
        "browser.back" => Kind::Back,
        "browser.forward" => Kind::Forward,
        "browser.reload" => Kind::Reload,
        "browser.stop" => Kind::Stop,
        "browser.tab.open" => Kind::TabOpen,
        "browser.tab.select" => Kind::TabSelect,
        "browser.tab.close" => Kind::TabClose,
        _ => return false,
    };
    capabilities.actions.contains(&kind)
}

#[cfg(test)]
mod tests;
