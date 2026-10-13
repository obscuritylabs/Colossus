use crate::BrowserError;
use colossus_contracts::*;
use std::collections::BTreeSet;
use std::io::{self, Write};

fn bounded_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

pub(crate) fn binding(value: &BrowserSessionBinding) -> Result<(), BrowserError> {
    let scope = match &value.scope {
        BrowserScope::Conversation { id } | BrowserScope::Workflow { id } => id,
    };
    if [
        &value.runtime_id,
        &value.workspace_id,
        &value.application_id,
        scope,
    ]
    .into_iter()
    .all(|id| bounded_identity(id))
    {
        Ok(())
    } else {
        Err(BrowserError::InvalidArguments)
    }
}

pub(crate) fn actor(value: &BrowserActor) -> Result<(), BrowserError> {
    binding(&value.binding)?;
    if bounded_identity(&value.run_id) {
        Ok(())
    } else {
        Err(BrowserError::InvalidArguments)
    }
}

pub(crate) fn options(value: &BrowserOpenOptions) -> Result<(), BrowserError> {
    if value.allowed_origins.is_empty()
        || value.allowed_origins.len() > 32
        || value.allowed_origins.iter().collect::<BTreeSet<_>>().len()
            != value.allowed_origins.len()
    {
        return Err(BrowserError::InvalidArguments);
    }
    if let Some(url) = &value.initial_url {
        destination(value, url)?;
    }
    Ok(())
}

pub(crate) fn destination(
    options: &BrowserOpenOptions,
    url: &BrowserUrl,
) -> Result<(), BrowserError> {
    if options.allowed_origins.contains(&url.origin()) {
        Ok(())
    } else {
        Err(BrowserError::OriginDenied)
    }
}

pub(crate) fn limits(requested: BrowserLimits, driver: &BrowserLimits) -> BrowserLimits {
    let hard = BrowserLimits::default();
    BrowserLimits {
        max_sessions: requested
            .max_sessions
            .min(driver.max_sessions)
            .min(hard.max_sessions),
        max_tabs: requested.max_tabs.min(driver.max_tabs).min(hard.max_tabs),
        max_concurrent_actions: requested
            .max_concurrent_actions
            .min(driver.max_concurrent_actions)
            .min(hard.max_concurrent_actions),
        max_snapshot_nodes: requested
            .max_snapshot_nodes
            .min(driver.max_snapshot_nodes)
            .min(hard.max_snapshot_nodes),
        max_observation_bytes: requested
            .max_observation_bytes
            .min(driver.max_observation_bytes)
            .min(hard.max_observation_bytes),
        action_timeout_ms: requested
            .action_timeout_ms
            .min(driver.action_timeout_ms)
            .min(hard.action_timeout_ms),
        navigation_timeout_ms: requested
            .navigation_timeout_ms
            .min(driver.navigation_timeout_ms)
            .min(hard.navigation_timeout_ms),
        max_lease_ms: requested
            .max_lease_ms
            .min(driver.max_lease_ms)
            .min(hard.max_lease_ms),
    }
}

pub(crate) fn action(
    action: &BrowserAction,
    options: &BrowserOpenOptions,
    limits: &BrowserLimits,
) -> Result<(), BrowserError> {
    interactions(action)?;
    match action {
        BrowserAction::Navigate { url } | BrowserAction::TabOpen { url: Some(url) } => {
            destination(options, url)
        }
        BrowserAction::Snapshot { max_nodes }
            if *max_nodes == 0 || *max_nodes > limits.max_snapshot_nodes =>
        {
            Err(BrowserError::LimitExceeded)
        }
        BrowserAction::Fill { text, .. } if text.len() > 8192 => Err(BrowserError::LimitExceeded),
        BrowserAction::Upload {
            max_bytes,
            artifact_id,
            ..
        } if *max_bytes == 0
            || *max_bytes > colossus_ports::MAX_BROWSER_TRANSFER_BYTES
            || !artifact_id.strip_prefix("artifact-").is_some_and(|value| {
                value.len() == 64
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            }) =>
        {
            Err(BrowserError::InvalidArguments)
        }
        BrowserAction::Download { max_bytes, .. }
            if *max_bytes == 0 || *max_bytes > colossus_ports::MAX_BROWSER_TRANSFER_BYTES =>
        {
            Err(BrowserError::LimitExceeded)
        }
        BrowserAction::Select { values, .. }
            if values.is_empty()
                || values.len() > 32
                || values.iter().any(|value| value.len() > 1024) =>
        {
            Err(BrowserError::LimitExceeded)
        }
        BrowserAction::Scroll { x, y }
            if x.unsigned_abs() > 10_000 || y.unsigned_abs() > 10_000 =>
        {
            Err(BrowserError::LimitExceeded)
        }
        BrowserAction::Wait { timeout_ms, .. }
            if *timeout_ms == 0 || *timeout_ms > limits.action_timeout_ms =>
        {
            Err(BrowserError::LimitExceeded)
        }
        _ => Ok(()),
    }
}

fn interactions(action: &BrowserAction) -> Result<(), BrowserError> {
    use colossus_contracts::{BrowserAnnotationTarget, BrowserPoint, BrowserViewport};
    let valid_viewport = |viewport: &BrowserViewport| {
        viewport.width > 0
            && viewport.height > 0
            && viewport.width <= 4096
            && viewport.height <= 4096
    };
    let valid_point = |point: &BrowserPoint, viewport: &BrowserViewport| {
        point.x < viewport.width && point.y < viewport.height
    };
    let valid = match action {
        BrowserAction::Evaluate { expression, .. } => {
            !expression.is_empty() && expression.len() <= 8192
        }
        BrowserAction::Cdp { params, .. } => {
            params.is_object()
                && serde_json::to_vec(params).is_ok_and(|bytes| bytes.len() <= 16 * 1024)
        }
        BrowserAction::ClickAt {
            viewport,
            point,
            click_count,
            ..
        } => {
            valid_viewport(viewport)
                && valid_point(point, viewport)
                && (1..=2).contains(click_count)
        }
        BrowserAction::DragAt { viewport, path, .. } => {
            valid_viewport(viewport)
                && (2..=64).contains(&path.len())
                && path.iter().all(|point| valid_point(point, viewport))
        }
        BrowserAction::Annotate {
            viewport,
            annotations,
            ttl_ms,
            snapshot_id,
        } => {
            valid_viewport(viewport)
                && (1..=16).contains(&annotations.len())
                && (1..=300000).contains(ttl_ms)
                && annotations.iter().all(|annotation| {
                    annotation.label.len() <= 160
                        && !annotation.label.chars().any(char::is_control)
                        && match &annotation.target {
                            BrowserAnnotationTarget::Element { element } => {
                                &element.snapshot_id == snapshot_id
                            }
                            BrowserAnnotationTarget::Rectangle { rect } => {
                                rect.width > 0
                                    && rect.height > 0
                                    && u32::from(rect.x) + u32::from(rect.width)
                                        <= u32::from(viewport.width)
                                    && u32::from(rect.y) + u32::from(rect.height)
                                        <= u32::from(viewport.height)
                            }
                        }
                })
        }
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(BrowserError::InvalidArguments)
    }
}

pub(crate) fn tab(
    tab: &BrowserTabSummary,
    options: &BrowserOpenOptions,
) -> Result<(), BrowserError> {
    if tab.title.len() > 1024 {
        return Err(BrowserError::LimitExceeded);
    }
    if tab
        .origin
        .as_ref()
        .is_some_and(|origin| !options.allowed_origins.contains(origin))
    {
        return Err(BrowserError::InvalidEvidence);
    }
    Ok(())
}

pub(crate) fn observation(
    value: &BrowserObservation,
    limits: &BrowserLimits,
) -> Result<(), BrowserError> {
    if value.inspection.as_ref().is_some_and(|value| {
        serde_json::to_vec(value).map_or(true, |bytes| bytes.len() > 32 * 1024)
    }) {
        return Err(BrowserError::LimitExceeded);
    }
    if value.tab.title.len() > 1024 {
        return Err(BrowserError::LimitExceeded);
    }
    if let Some(snapshot) = &value.snapshot {
        if snapshot.document_id != value.tab.document_id
            || snapshot.nodes.len() > usize::from(limits.max_snapshot_nodes)
        {
            return Err(BrowserError::InvalidEvidence);
        }
        let mut elements = BTreeSet::new();
        for node in &snapshot.nodes {
            if node.element.document_id != snapshot.document_id
                || node.element.snapshot_id != snapshot.snapshot_id
                || node.role.len() > 128
                || node.name.len() > 4096
                || node.value.as_ref().is_some_and(|v| v.len() > 4096)
                || !elements.insert(&node.element.element_id)
            {
                return Err(BrowserError::InvalidEvidence);
            }
        }
    }
    let mut budget = ByteBudget {
        remaining: limits.max_observation_bytes as usize,
    };
    serde_json::to_writer(&mut budget, value).map_err(|_| BrowserError::LimitExceeded)?;
    Ok(())
}

/// Count actual encoded JSON without allocating an unbounded second copy of page data.
struct ByteBudget {
    remaining: usize,
}

impl Write for ByteBudget {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(io::Error::from(io::ErrorKind::FileTooLarge));
        }
        self.remaining -= bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
