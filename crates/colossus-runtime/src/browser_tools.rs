//! Runtime-only browser dispatch, ownership, and permit-bearing effect adapter.

mod arguments;
mod execution;
mod host;
mod lifecycle;
mod public;
mod service;

pub(super) use execution::BrowserToolExecutor;
pub use host::RuntimeBrowserHost;
pub(super) use lifecycle::RuntimeRunLifecycle;
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
    if !capabilities.available || !capabilities.restrictive_egress || capabilities.modes.is_empty()
    {
        return false;
    }
    let kind = match name {
        "browser.open" | "browser.close" | "browser.status" | "browser.tabs" => return true,
        "browser.navigate" => Kind::Navigate,
        "browser.snapshot" => Kind::Snapshot,
        "browser.click" => Kind::Click,
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
