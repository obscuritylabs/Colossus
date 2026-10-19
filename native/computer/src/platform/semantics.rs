//! Bounded UIA queries run only in an externally supervised disposable MTA process.
use super::{Error, failed, identity::Window};
use crate::{TrustedComputerContext, geometry::bounded, protocol::opaque};
use colossus_contracts::{ComputerElementId, ComputerNode, ComputerRect};
use windows::Win32::{
    Foundation::HWND,
    System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx},
    UI::Accessibility::{CUIAutomation, IUIAutomation, IUIAutomationElement},
};

fn automation() -> Result<IUIAutomation, Error> {
    // SAFETY: this read worker owns no windows and uses only its own MTA COM objects.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(failed)?;
    // SAFETY: fixed system UIA class; no caller-nominated COM server.
    unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.map_err(failed)
}

fn sensitive(name: &str) -> bool {
    let value = name.to_ascii_lowercase();
    [
        "password",
        "passphrase",
        "secret",
        "api key",
        "access token",
        "credential",
        "private key",
        "security code",
        "verification code",
    ]
    .iter()
    .any(|word| value.contains(word))
}

/// Query current native focus before text/chords. An unavailable provider fails closed.
pub(crate) fn protected_focus(
    window: &Window,
    context: &TrustedComputerContext,
) -> Result<bool, Error> {
    window.revalidate(context)?;
    let automation = automation()?;
    // SAFETY: UIA focus is query-only and externally bounded by process supervision.
    let element = unsafe { automation.GetFocusedElement() }.map_err(failed)?;
    // SAFETY: retained UIA element queried on the same disposable MTA worker.
    let password = unsafe { element.CurrentIsPassword() }
        .map_err(failed)?
        .as_bool();
    // SAFETY: query-only UIA property, never retained or returned as a secret value.
    let name = unsafe { element.CurrentName() }
        .map_err(failed)?
        .to_string();
    let protected = password || sensitive(&name);
    window.revalidate(context)?;
    Ok(protected)
}

pub(crate) fn snapshot(
    window: &Window,
    context: &TrustedComputerContext,
    max_nodes: u16,
) -> Result<Vec<ComputerNode>, Error> {
    window.revalidate(context)?;
    let automation = automation()?;
    // SAFETY: root is the exact admitted native window. Only its descendants are walked.
    let root = unsafe { automation.ElementFromHandle(HWND(window.hwnd())) }.map_err(failed)?;
    // SAFETY: fixed system UIA view walker in this worker's MTA.
    let walker = unsafe { automation.ControlViewWalker() }.map_err(failed)?;
    let mut queue = std::collections::VecDeque::from([(root, 0u8)]);
    let mut result = Vec::new();
    while let Some((element, depth)) = queue.pop_front() {
        if result.len() >= usize::from(max_nodes) {
            break;
        }
        if let Some(node) = node(&element, window)? {
            result.push(node);
        }
        if depth >= 16 {
            continue;
        }
        // SAFETY: only descendants of the retained selected root are traversed.
        let mut child = unsafe { walker.GetFirstChildElement(&element) }.ok();
        while let Some(current) = child {
            if queue.len() + result.len() >= usize::from(max_nodes) {
                break;
            }
            // SAFETY: sibling belongs to the already admitted root traversal.
            child = unsafe { walker.GetNextSiblingElement(&current) }.ok();
            queue.push_back((current, depth + 1));
        }
    }
    window.revalidate(context)?;
    Ok(result)
}

fn node(element: &IUIAutomationElement, window: &Window) -> Result<Option<ComputerNode>, Error> {
    // SAFETY: query-only rectangle copied out of this worker's retained UIA element.
    let rect = unsafe { element.CurrentBoundingRectangle() }.map_err(failed)?;
    let left = (i64::from(rect.left) - i64::from(window.rect.x)).max(0);
    let top = (i64::from(rect.top) - i64::from(window.rect.y)).max(0);
    let right =
        (i64::from(rect.right) - i64::from(window.rect.x)).min(i64::from(window.rect.width));
    let bottom =
        (i64::from(rect.bottom) - i64::from(window.rect.y)).min(i64::from(window.rect.height));
    if right <= left || bottom <= top {
        return Ok(None);
    }
    // SAFETY: native password marker is queried before any name/value observation.
    let password = unsafe { element.CurrentIsPassword() }
        .map_err(failed)?
        .as_bool();
    let name = if password {
        String::new()
    } else {
        bounded(
            // SAFETY: name is read only after password suppression; values are never queried.
            &unsafe { element.CurrentName() }
                .map_err(failed)?
                .to_string(),
            512,
        )
    };
    let role = bounded(
        // SAFETY: fixed read-only role metadata from the retained element.
        &unsafe { element.CurrentLocalizedControlType() }
            .map_err(failed)?
            .to_string(),
        64,
    );
    Ok(Some(ComputerNode {
        element_id: ComputerElementId::parse(opaque("cu_e_")?).map_err(failed)?,
        role,
        name,
        value: None,
        bounds: ComputerRect {
            x: left as i32,
            y: top as i32,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        },
        // SAFETY: query-only boolean on this worker's retained native element.
        enabled: unsafe { element.CurrentIsEnabled() }
            .map_err(failed)?
            .as_bool(),
        // SAFETY: query-only boolean; no focus mutation or UIA pattern is invoked.
        focused: unsafe { element.CurrentHasKeyboardFocus() }
            .map_err(failed)?
            .as_bool(),
        is_password: password,
    }))
}
