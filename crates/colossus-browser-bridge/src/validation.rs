use std::collections::BTreeSet;

pub(crate) mod transfer;

use colossus_contracts::{
    BrowserAction, BrowserCapabilities, BrowserLimits, BrowserObservation, BrowserScope,
    BrowserSessionBinding,
};
use colossus_ports::{BrowserDriverCommand, BrowserDriverError, BrowserDriverOpenRequest};

fn identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

pub(crate) fn binding(value: &BrowserSessionBinding) -> Result<(), BrowserDriverError> {
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
    .all(|value| identity(value))
    {
        Ok(())
    } else {
        Err(BrowserDriverError::Denied)
    }
}

pub(crate) fn capabilities(value: &BrowserCapabilities) -> Result<(), BrowserDriverError> {
    let hard = BrowserLimits::default();
    let limits = &value.limits;
    if !value.available
        || !value.restrictive_egress
        || value.modes.is_empty()
        || value
            .engine_version
            .as_ref()
            .is_none_or(|value| !identity(value))
        || limits.max_sessions == 0
        || limits.max_sessions > hard.max_sessions
        || limits.max_tabs == 0
        || limits.max_tabs > hard.max_tabs
        || limits.max_concurrent_actions == 0
        || limits.max_concurrent_actions > hard.max_concurrent_actions
        || limits.max_snapshot_nodes == 0
        || limits.max_snapshot_nodes > hard.max_snapshot_nodes
        || limits.max_observation_bytes == 0
        || limits.max_observation_bytes > hard.max_observation_bytes
        || limits.action_timeout_ms == 0
        || limits.action_timeout_ms > hard.action_timeout_ms
        || limits.navigation_timeout_ms == 0
        || limits.navigation_timeout_ms > hard.navigation_timeout_ms
        || limits.max_lease_ms == 0
        || limits.max_lease_ms > hard.max_lease_ms
    {
        return Err(BrowserDriverError::Unavailable);
    }
    Ok(())
}

pub(crate) fn open(
    value: &BrowserDriverOpenRequest,
    enrollment: &crate::BrowserBridgeEnrollment,
) -> Result<(), BrowserDriverError> {
    allocation(value, &enrollment.capabilities)?;
    if value.binding != enrollment.binding {
        return Err(BrowserDriverError::Denied);
    }
    let options = &value.options;
    if options.profile != enrollment.profile
        || options.mode != enrollment.mode
        || options.allowed_origins.iter().collect::<BTreeSet<_>>()
            != enrollment.allowed_origins.iter().collect::<BTreeSet<_>>()
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}

pub(crate) fn allocation(
    value: &BrowserDriverOpenRequest,
    capabilities: &BrowserCapabilities,
) -> Result<(), BrowserDriverError> {
    binding(&value.binding)?;
    let options = &value.options;
    if value.run_id.as_ref().is_some_and(|value| !identity(value))
        || !capabilities.modes.contains(&options.mode)
        || options.allowed_origins.is_empty()
        || options.allowed_origins.len() > 32
        || options
            .allowed_origins
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != options.allowed_origins.len()
        || options
            .initial_url
            .as_ref()
            .is_some_and(|url| !options.allowed_origins.contains(&url.origin()))
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}

pub(crate) fn command(
    command: &BrowserDriverCommand,
    open: &BrowserDriverOpenRequest,
    enrollment: &crate::BrowserBridgeEnrollment,
) -> Result<(), BrowserDriverError> {
    if command.binding != enrollment.binding {
        return Err(BrowserDriverError::Denied);
    }
    command_for_open(command, open, &enrollment.capabilities)
}

pub(crate) fn command_for_open(
    command: &BrowserDriverCommand,
    open: &BrowserDriverOpenRequest,
    capabilities: &BrowserCapabilities,
) -> Result<(), BrowserDriverError> {
    if command.binding != open.binding
        || !identity(&command.run_id)
        || command.control_generation == 0
    {
        return Err(BrowserDriverError::Denied);
    }
    if !capabilities.actions.contains(&command.action.kind()) {
        return Err(BrowserDriverError::Unsupported);
    }
    let limits = &capabilities.limits;
    match &command.action {
        BrowserAction::Navigate { url } | BrowserAction::TabOpen { url: Some(url) }
            if !open.options.allowed_origins.contains(&url.origin()) =>
        {
            return Err(BrowserDriverError::Denied);
        }
        BrowserAction::Snapshot { max_nodes }
            if *max_nodes == 0 || *max_nodes > limits.max_snapshot_nodes =>
        {
            return Err(BrowserDriverError::LimitExceeded);
        }
        BrowserAction::Screenshot { max_bytes }
            if *max_bytes < 33 || *max_bytes > colossus_ports::MAX_BROWSER_SCREENSHOT_BYTES =>
        {
            return Err(BrowserDriverError::LimitExceeded);
        }
        BrowserAction::Upload { max_bytes, .. } | BrowserAction::Download { max_bytes, .. }
            if *max_bytes == 0 || *max_bytes > colossus_ports::MAX_BROWSER_TRANSFER_BYTES =>
        {
            return Err(BrowserDriverError::LimitExceeded);
        }
        BrowserAction::Upload { element, .. } | BrowserAction::Download { element, .. }
            if element.document_id != command.target.document_id =>
        {
            return Err(BrowserDriverError::Stale);
        }
        BrowserAction::Fill { text, .. } if text.len() > 8192 => {
            return Err(BrowserDriverError::LimitExceeded);
        }
        BrowserAction::Select { values, .. }
            if values.is_empty()
                || values.len() > 32
                || values.iter().any(|value| value.len() > 1024) =>
        {
            return Err(BrowserDriverError::LimitExceeded);
        }
        BrowserAction::Scroll { x, y } if x.unsigned_abs() > 10000 || y.unsigned_abs() > 10000 => {
            return Err(BrowserDriverError::LimitExceeded);
        }
        BrowserAction::Wait { timeout_ms, .. }
            if *timeout_ms == 0 || *timeout_ms > limits.action_timeout_ms =>
        {
            return Err(BrowserDriverError::LimitExceeded);
        }
        BrowserAction::TabSelect { tab_id } | BrowserAction::TabClose { tab_id }
            if tab_id != &command.target.tab_id =>
        {
            return Err(BrowserDriverError::Stale);
        }
        _ => {}
    }
    if matches!(command.action, BrowserAction::Snapshot { .. }) != command.snapshot_id.is_some()
        || matches!(command.action, BrowserAction::TabOpen { .. }) != command.new_tab.is_some()
        || command
            .new_tab
            .as_ref()
            .is_some_and(|tab| !tab.title.is_empty() || tab.origin.is_some())
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}

pub(crate) fn screenshot_descriptor(
    value: &colossus_ports::BrowserScreenshotDescriptor,
    command: &BrowserDriverCommand,
) -> Result<(), BrowserDriverError> {
    let BrowserAction::Screenshot { max_bytes } = command.action else {
        return Err(BrowserDriverError::Unsupported);
    };
    if value.session_id != command.session_id
        || value.target != command.target
        || value.control_generation != command.control_generation
        || value.size_bytes < 33
        || value.size_bytes > max_bytes
        || value.size_bytes > colossus_ports::MAX_BROWSER_SCREENSHOT_BYTES
        || !hex(&value.transfer_id, 32)
        || !hex(&value.sha256, 64)
        || value.width == 0
        || value.height == 0
        || u64::from(value.width) * u64::from(value.height) > 16_777_216
    {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    Ok(())
}

pub(crate) fn screenshot_read(
    value: &colossus_ports::BrowserScreenshotReadRequest,
    open: &BrowserDriverOpenRequest,
) -> Result<(), BrowserDriverError> {
    if value.binding != open.binding
        || value.session_id != open.session_id
        || !identity(&value.run_id)
        || value.control_generation == 0
        || !hex(&value.transfer_id, 32)
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}

pub(crate) fn screenshot_chunk(
    value: &colossus_ports::BrowserScreenshotChunk,
    offset: u32,
    remaining: u32,
) -> Result<u32, BrowserDriverError> {
    if value.offset != offset {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    chunk_length(&value.data_base64, remaining)
}

fn chunk_length(data: &str, remaining: u32) -> Result<u32, BrowserDriverError> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let maximum = colossus_ports::BROWSER_SCREENSHOT_CHUNK_BYTES;
    if data.len() > maximum.div_ceil(3) * 4 {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    let bytes = zeroize::Zeroizing::new(
        STANDARD
            .decode(data)
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?,
    );
    let canonical = zeroize::Zeroizing::new(STANDARD.encode(&*bytes));
    if bytes.is_empty()
        || bytes.len() > maximum
        || bytes.len() > remaining as usize
        || canonical.as_str() != data
    {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    u32::try_from(bytes.len()).map_err(|_| BrowserDriverError::OutcomeUnknown)
}

pub(crate) fn native_handoff(
    value: &colossus_ports::BrowserNativeHandoffRequest,
    open: &BrowserDriverOpenRequest,
) -> Result<(), BrowserDriverError> {
    if value.binding != open.binding
        || value.session_id != open.session_id
        || open.run_id.is_some()
        || value.expected_target.tab_id != open.tab_id
        || value.expected_target.document_id != open.document_id
        || value.confirmed_target.tab_id != open.tab_id
        || value.native_document_generation == 0
    {
        return Err(BrowserDriverError::Denied);
    }
    Ok(())
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn observation(
    value: &BrowserObservation,
    command: &BrowserDriverCommand,
    open: &BrowserDriverOpenRequest,
    enrollment: &crate::BrowserBridgeEnrollment,
) -> Result<(), BrowserDriverError> {
    observed(
        value,
        command,
        open,
        enrollment.capabilities.limits.max_observation_bytes,
    )
}

pub(crate) fn observed(
    value: &BrowserObservation,
    command: &BrowserDriverCommand,
    open: &BrowserDriverOpenRequest,
    max_observation_bytes: u32,
) -> Result<(), BrowserDriverError> {
    let expected_tab = command
        .new_tab
        .as_ref()
        .map_or(&command.target.tab_id, |tab| &tab.tab_id);
    if value.session_id != command.session_id
        || &value.tab.tab_id != expected_tab
        || (value.tab.document_id != command.next_document_id
            && value.tab.document_id != command.target.document_id)
        || command
            .new_tab
            .as_ref()
            .is_some_and(|tab| tab.document_id != value.tab.document_id)
        || value.tab.title.len() > 1024
        || value
            .tab
            .origin
            .as_ref()
            .is_some_and(|origin| !open.options.allowed_origins.contains(origin))
        || value
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| Some(&snapshot.snapshot_id) != command.snapshot_id.as_ref())
        || (command.snapshot_id.is_some() && value.snapshot.is_none())
    {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    if let Some(snapshot) = &value.snapshot {
        let ceiling = match command.action {
            BrowserAction::Snapshot { max_nodes } => max_nodes,
            _ => return Err(BrowserDriverError::OutcomeUnknown),
        };
        if snapshot.document_id != value.tab.document_id
            || snapshot.nodes.len() > usize::from(ceiling)
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let mut seen = BTreeSet::new();
        for node in &snapshot.nodes {
            if node.element.document_id != snapshot.document_id
                || node.element.snapshot_id != snapshot.snapshot_id
                || node.role.len() > 128
                || node.name.len() > 4096
                || node.value.as_ref().is_some_and(|value| value.len() > 4096)
                || !seen.insert(&node.element.element_id)
            {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
        }
    }
    bounded_observation(value, max_observation_bytes as usize)
        .map_err(|_| BrowserDriverError::OutcomeUnknown)
}

pub(crate) fn bounded_observation(
    value: &BrowserObservation,
    remaining: usize,
) -> Result<(), BrowserDriverError> {
    struct Budget {
        remaining: usize,
    }
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.remaining {
                return Err(std::io::ErrorKind::FileTooLarge.into());
            }
            self.remaining -= bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Budget { remaining }, value)
        .map_err(|_| BrowserDriverError::LimitExceeded)
}
