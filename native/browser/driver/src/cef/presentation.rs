//! Main-thread pixel leases and separately admitted human navigation/input.
use std::{
    ffi::CString,
    time::{Duration, Instant},
};

use colossus_browser_presentation::{
    Configure, Frame, HumanCommand, HumanFenceReceipt, Input, Lease, LeaseGuard,
    PresentationCommand, PresentationError, PresentationReply,
};
use colossus_contracts::BrowserUrl;

use super::Host;
use crate::ffi;

mod observe;

#[derive(Default)]
pub(super) struct State {
    pub(super) enabled: bool,
    pub(super) input_admitted: bool,
    pub(super) human: bool,
    pub(super) control_generation: u64,
    pub(super) handoff: crate::handoff::Cache,
    active: Option<(Configure, LeaseGuard)>,
}
fn status(status: i32) -> Result<(), PresentationError> {
    match status {
        0 => Ok(()),
        1 => Err(PresentationError::Invalid),
        3 => Err(PresentationError::Stale),
        5 => Err(PresentationError::Hidden),
        _ => Err(PresentationError::OutcomeUnknown),
    }
}
impl Host {
    /// Install only supervisor-owned presentation configuration before session admission.
    pub fn enable_presentation(&mut self, human_input: bool) {
        self.presentation.enabled = true;
        self.presentation.input_admitted = human_input;
    }
    pub fn revoke_human_presentation(&mut self) {
        self.presentation.human = false;
        if let Some((_, guard)) = self.presentation.active.as_mut() {
            let lease = guard.lease();
            guard.hide();
            // SAFETY: exact retained native tab and generations, called on CEF UI thread.
            let _ = unsafe {
                ffi::colossus_cef_presentation_visible(
                    lease.tab,
                    lease.session_generation,
                    lease.viewport_generation,
                    lease.document_generation,
                    0,
                    0,
                )
            };
        }
    }
    fn owned_lease(&self, lease: Lease) -> Result<(), PresentationError> {
        let (configuration, guard) = self
            .presentation
            .active
            .as_ref()
            .ok_or(PresentationError::Stale)?;
        if !self.presentation.enabled
            || !self.initialized
            || self.cancelled.load(super::Ordering::Acquire)
            || guard.lease() != lease
            || lease.control_generation != self.presentation.control_generation
            || self.session.as_ref() != Some(&configuration.session)
            || self
                .tabs
                .get(&configuration.target.tab_id)
                .is_none_or(|tab| tab.native != lease.tab)
        {
            return Err(PresentationError::Stale);
        }
        Ok(())
    }
    fn current_document(&self, lease: Lease) -> Result<(), PresentationError> {
        self.owned_lease(lease)?;
        let mut document = 0;
        // SAFETY: valid stack output and exact private native tab on its owning thread.
        status(unsafe {
            ffi::colossus_cef_presentation_document(
                lease.tab,
                lease.session_generation,
                &mut document,
            )
        })?;
        if document != lease.document_generation {
            return Err(PresentationError::Stale);
        }
        Ok(())
    }
    fn configure_view(&mut self, configuration: Configure) -> Result<Lease, PresentationError> {
        configuration.validate()?;
        if !self.presentation.enabled
            || !self.initialized
            || self.cancelled.load(super::Ordering::Acquire)
            || self.session.as_ref() != Some(&configuration.session)
            || configuration.control_generation != self.presentation.control_generation
        {
            return Err(PresentationError::Stale);
        }
        let tab = self
            .tabs
            .get(&configuration.target.tab_id)
            .ok_or(PresentationError::Stale)?;
        if tab.summary.document_id != configuration.target.document_id
            || self
                .metadata(tab.native)
                .map_err(|_| PresentationError::OutcomeUnknown)?
                .2
                != tab.revision
        {
            return Err(PresentationError::Stale);
        }
        let mut document = 0;
        // SAFETY: exact owned native tab, bounded stack output on the CEF UI thread.
        status(unsafe { ffi::colossus_cef_presentation_document(tab.native, 1, &mut document) })?;
        if document != tab.native_document {
            return Err(PresentationError::Stale);
        }
        let lease = Lease {
            tab: tab.native,
            session_generation: 1,
            control_generation: configuration.control_generation,
            viewport_generation: configuration.viewport_generation,
            document_generation: document,
            pixel_width: (u64::from(configuration.width) * u64::from(configuration.scale_milli))
                .div_ceil(1000) as u32,
            pixel_height: (u64::from(configuration.height) * u64::from(configuration.scale_milli))
                .div_ceil(1000) as u32,
        };
        lease.validate()?;
        // SAFETY: exact private mapping and validated finite dimensions/scale/lifetime.
        status(unsafe {
            ffi::colossus_cef_presentation_configure(
                lease.tab,
                1,
                lease.viewport_generation,
                document,
                configuration.width,
                configuration.height,
                f64::from(configuration.scale_milli) / 1000.0,
                u32::from(configuration.lease_ms),
            )
        })?;
        let guard = LeaseGuard::new(
            lease,
            Instant::now(),
            Duration::from_millis(u64::from(configuration.lease_ms)),
        )?;
        self.presentation.active = Some((configuration, guard));
        Ok(lease)
    }
    pub fn presentation_command(
        &mut self,
        command: PresentationCommand,
    ) -> Result<PresentationReply, PresentationError> {
        command.validate()?;
        match command {
            PresentationCommand::Configure(value) => {
                Ok(PresentationReply::Configured(self.configure_view(value)?))
            }
            PresentationCommand::Observe { lease } => {
                Ok(PresentationReply::State(self.observe_view(lease, false)?))
            }
            PresentationCommand::FenceHuman { lease } => {
                Ok(PresentationReply::Fenced(self.fence_human(lease)?))
            }
            PresentationCommand::Ready | PresentationCommand::Poll { .. } => {
                Err(PresentationError::Invalid)
            }
            PresentationCommand::Human { lease, command } => {
                self.human_navigation(lease, command)?;
                Ok(PresentationReply::Ack)
            }
            PresentationCommand::Input {
                lease,
                input,
                modifiers,
            } => {
                self.human_input(lease, input, modifiers)?;
                Ok(PresentationReply::Ack)
            }
            PresentationCommand::Renew { lease, lease_ms } => {
                self.current_document(lease)?;
                let (_, guard) = self
                    .presentation
                    .active
                    .as_mut()
                    .ok_or(PresentationError::Stale)?;
                guard.renew(
                    lease,
                    Instant::now(),
                    Duration::from_millis(u64::from(lease_ms)),
                )?;
                // SAFETY: exact native lease and checked bounded heartbeat, CEF UI thread.
                status(unsafe {
                    ffi::colossus_cef_presentation_visible(
                        lease.tab,
                        1,
                        lease.viewport_generation,
                        lease.document_generation,
                        1,
                        u32::from(lease_ms),
                    )
                })?;
                Ok(PresentationReply::Ack)
            }
            PresentationCommand::Focus { lease, focused } => {
                self.current_document(lease)?;
                if focused && !self.presentation.human {
                    return Err(PresentationError::Hidden);
                }
                self.presentation
                    .active
                    .as_mut()
                    .ok_or(PresentationError::Stale)?
                    .1
                    .focus(focused, Instant::now())?;
                // SAFETY: exact native lease; no OS parent handle crosses this boundary.
                status(unsafe {
                    ffi::colossus_cef_presentation_focus(
                        lease.tab,
                        1,
                        lease.viewport_generation,
                        lease.document_generation,
                        i32::from(focused),
                    )
                })?;
                Ok(PresentationReply::Ack)
            }
            PresentationCommand::Hide { lease } => {
                self.owned_lease(lease)?;
                self.presentation
                    .active
                    .as_mut()
                    .ok_or(PresentationError::Stale)?
                    .1
                    .hide();
                // SAFETY: hide exact old lease; a navigation may already have revoked it.
                let _ = unsafe {
                    ffi::colossus_cef_presentation_visible(
                        lease.tab,
                        1,
                        lease.viewport_generation,
                        lease.document_generation,
                        0,
                        0,
                    )
                };
                Ok(PresentationReply::Ack)
            }
        }
    }
    fn human_navigation(
        &mut self,
        lease: Lease,
        command: HumanCommand,
    ) -> Result<(), PresentationError> {
        self.current_document(lease)?;
        if !self.presentation.human
            || lease.control_generation != 0
            || !self
                .presentation
                .active
                .as_ref()
                .ok_or(PresentationError::Stale)?
                .1
                .visible(Instant::now())
        {
            return Err(PresentationError::Hidden);
        }
        match command {
            HumanCommand::Navigate { url } => {
                let url = BrowserUrl::parse(&url).map_err(|_| PresentationError::Invalid)?;
                if !self
                    .callbacks
                    .origins
                    .lock()
                    .map_err(|_| PresentationError::OutcomeUnknown)?
                    .contains(&url.origin())
                {
                    return Err(PresentationError::Hidden);
                }
                let url = CString::new(url.as_str()).map_err(|_| PresentationError::Invalid)?;
                // SAFETY: bounded admitted URL, current private tab mapping, owning UI thread.
                status(unsafe { ffi::colossus_cef_navigate(lease.tab, 1, url.as_ptr()) })
            }
            command => {
                let action = match command {
                    HumanCommand::Back => 1,
                    HumanCommand::Forward => 2,
                    HumanCommand::Reload => 3,
                    HumanCommand::Stop => 4,
                    HumanCommand::Navigate { .. } => return Err(PresentationError::Invalid),
                };
                // SAFETY: closed private human navigation operation on its current owned tab.
                status(unsafe { ffi::colossus_cef_control(lease.tab, 1, action) })
            }
        }
    }
    fn fence_human(&mut self, lease: Lease) -> Result<HumanFenceReceipt, PresentationError> {
        self.owned_lease(lease)?;
        if !self.presentation.human || lease.control_generation != 0 {
            return Err(PresentationError::Hidden);
        }
        // Run on CEF's owning thread before observing state. Every subsequent
        // human event stays fenced even if metadata or receipt delivery fails.
        self.revoke_human_presentation();
        let state = self
            .observe_view(lease, true)
            .map_err(|_| PresentationError::OutcomeUnknown)?;
        let receipt = HumanFenceReceipt {
            prior_lease: lease,
            state,
            native_document_generation: self
                .native_document(lease.tab)
                .map_err(|_| PresentationError::OutcomeUnknown)?,
        };
        receipt
            .validate()
            .map_err(|_| PresentationError::OutcomeUnknown)?;
        self.presentation
            .handoff
            .record(
                receipt.state.target.clone(),
                receipt.native_document_generation,
            )
            .map_err(|_| PresentationError::OutcomeUnknown)?;
        Ok(receipt)
    }
    pub fn confirm_native_handoff(
        &mut self,
        request: colossus_ports::BrowserNativeHandoffRequest,
        control: &colossus_ports::BrowserDriverControl,
    ) -> Result<colossus_contracts::BrowserTabSummary, colossus_ports::BrowserDriverError> {
        use colossus_ports::BrowserDriverError;
        if self.interrupted(control) {
            return Err(BrowserDriverError::Cancelled);
        }
        if !self.initialized
            || !self.presentation.enabled
            || self.presentation.human
            || self.presentation.control_generation != 0
            || self.session.as_ref() != Some(&request.session_id)
        {
            return Err(BrowserDriverError::Stale);
        }
        let tab = self
            .tabs
            .get(&request.confirmed_target.tab_id)
            .ok_or(BrowserDriverError::Stale)?;
        let native_document = self.native_document(tab.native)?;
        if native_document != tab.native_document || self.metadata(tab.native)?.2 != tab.revision {
            return Err(BrowserDriverError::Stale);
        }
        let summary = tab.summary.clone();
        self.presentation.handoff.consume(
            &request,
            &colossus_contracts::BrowserTarget {
                tab_id: summary.tab_id.clone(),
                document_id: summary.document_id.clone(),
            },
            native_document,
        )?;
        Ok(summary)
    }
    fn human_input(
        &mut self,
        lease: Lease,
        input: Input,
        modifiers: u32,
    ) -> Result<(), PresentationError> {
        self.current_document(lease)?;
        if !self.presentation.human || lease.control_generation != 0 {
            return Err(PresentationError::Hidden);
        }
        self.presentation
            .active
            .as_ref()
            .ok_or(PresentationError::Stale)?
            .1
            .authorize_input(lease, &input, Instant::now())?;
        let (native, _text) = super::presentation::input::map(&input, modifiers)?;
        // SAFETY: exact owned current lease; borrowed UTF16 storage outlives this copying call.
        status(unsafe {
            ffi::colossus_cef_presentation_input(
                lease.tab,
                1,
                lease.viewport_generation,
                lease.document_generation,
                &native,
            )
        })
    }
    pub fn presentation_frame(&self, lease: Lease) -> Result<Option<Frame>, PresentationError> {
        self.current_document(lease)?;
        if !self
            .presentation
            .active
            .as_ref()
            .ok_or(PresentationError::Stale)?
            .1
            .visible(Instant::now())
        {
            return Err(PresentationError::Hidden);
        }
        let mut pixels = vec![0; lease.pixel_width as usize * lease.pixel_height as usize * 4];
        let mut native = ffi::PresentationFrame {
            version: 1,
            width: 0,
            height: 0,
            stride: 0,
            tab: 0,
            generation: 0,
            viewport_generation: 0,
            document_generation: 0,
            sequence: 0,
            bytes: 0,
        };
        // SAFETY: checked bounded writable pixel storage and exact native current lease.
        let result = unsafe {
            ffi::colossus_cef_presentation_frame(
                lease.tab,
                1,
                lease.viewport_generation,
                lease.document_generation,
                &mut native,
                pixels.as_mut_ptr(),
                pixels.len(),
            )
        };
        if result == 6 {
            return Ok(None);
        }
        status(result)?;
        if native.version != 1
            || native.tab != lease.tab
            || native.generation != 1
            || native.viewport_generation != lease.viewport_generation
            || native.document_generation != lease.document_generation
            || native.width != lease.pixel_width
            || native.height != lease.pixel_height
            || native.bytes != pixels.len()
        {
            return Err(PresentationError::Stale);
        }
        let frame = Frame {
            lease,
            sequence: native.sequence,
            stride: native.stride,
            pixels,
        };
        frame.validate()?;
        Ok(Some(frame))
    }
}

mod input;
