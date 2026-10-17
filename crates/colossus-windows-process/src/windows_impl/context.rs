//! Core-owned fresh browser context. No helper receives Core mutation or outer Job authority.
mod helper;
mod identity;
mod ipc;
mod launch;
#[cfg(test)]
mod startup_tests;
mod token_security;

use super::{
    LocalSid, NetworkGuard, OwnedHandle, PipeSet, SpawnRequest, WindowsProcessError,
    channels::ChannelPipes, last_error, new_job, resource_limit_violation, terminate, wait_timeout,
    wait_tree_timeout,
};
use crate::{
    ContextShutdown, PendingBrowserCleanup, ResourceLimitViolation, SandboxedChild,
    SupervisedChild, api::SandboxedChildParts,
};
use colossus_windows_native::BoundPath;
use identity::{native, require};
use ipc::{Frame, Pipe};
use launch::{Arguments, Created, Role};
use std::{
    ffi::OsStr,
    fs::File,
    os::windows::io::BorrowedHandle,
    ptr::null_mut,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE_FLAG_INHERIT, SetHandleInformation,
    },
    Security::{
        Authorization::ConvertStringSidToSidW,
        Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
    },
    System::{
        JobObjects::IsProcessInJob,
        LibraryLoader::GetModuleHandleW,
        SystemServices::{JOB_OBJECT_ASSIGN_PROCESS, JOB_OBJECT_QUERY},
        Threading::{
            GetCurrentProcess, GetCurrentProcessId, GetProcessIdOfThread, GetThreadId,
            PROCESS_DUP_HANDLE, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
            THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME, TerminateProcess,
        },
    },
};

const SYNC: u32 = 0x0010_0000;
const SETUP_BOUND: Duration = Duration::from_secs(8);

pub(super) fn payload(values: &[u64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}
fn fields(frame: Frame, op: u16, count: usize) -> Result<Vec<u64>, WindowsProcessError> {
    require(
        frame.op == op
            && frame.payload.len().is_multiple_of(8)
            && (count == usize::MAX || frame.payload.len() == count * 8),
        "unexpected browser context frame",
    )?;
    frame
        .payload
        .chunks_exact(8)
        .map(|bytes| {
            bytes
                .try_into()
                .map(u64::from_le_bytes)
                .map_err(|_| WindowsProcessError::Invalid("context integer frame".into()))
        })
        .collect()
}
fn held<T>(value: &Option<T>) -> Result<&T, WindowsProcessError> {
    value.as_ref().ok_or_else(|| {
        WindowsProcessError::Invalid("browser context ownership state incomplete".into())
    })
}
fn held_mut<T>(value: &mut Option<T>) -> Result<&mut T, WindowsProcessError> {
    value.as_mut().ok_or_else(|| {
        WindowsProcessError::Invalid("browser context ownership state incomplete".into())
    })
}
fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}
fn member(process: &OwnedHandle, job: &OwnedHandle) -> Result<(), WindowsProcessError> {
    let mut yes = 0;
    // SAFETY: exact retained process/Job objects and writable membership output.
    if unsafe { IsProcessInJob(process.raw(), job.raw(), &mut yes) } == 0 {
        return Err(last_error("IsProcessInJob(browser context)"));
    }
    require(yes != 0, "atomic browser context Job membership absent")
}

/// Dispatch only the two fixed internal helper modes, before the driver enters CEF.
/// Ordinary arguments return None; malformed/mixed helper arguments fail closed.
pub fn dispatch_browser_context_helper() -> Option<i32> {
    let parsed = launch::parse()?;
    Some(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let arguments = parsed?;
            let cef = super::wide(OsStr::new("libcef.dll"));
            // SAFETY: observation only; this never loads CEF to inspect availability.
            require(
                unsafe { GetModuleHandleW(cef.as_ptr()) }.is_null(),
                "browser helper entered after CEF was loaded",
            )?;
            helper::run(arguments)
        }))
        .map_or(1, |result| if result.is_ok() { 0 } else { 1 }),
    )
}

pub(crate) struct BrowserContextOwner {
    outer: Option<(OwnedHandle, OwnedHandle)>,
    launcher: Option<Created>,
    launcher_pipe: Option<Pipe>,
    control: Option<Pipe>,
    process: Option<OwnedHandle>,
    thread: Option<OwnedHandle>,
    pid: u32,
    tid: u32,
    bound: bool,
    cpwl_pending: bool,
    returned: Option<Vec<u64>>,
    pulled_ack: bool,
    station_ready: bool,
    browser_create_pending: bool,
    browser_returned: Option<Vec<u64>>,
    close_sent: bool,
    close_received: bool,
    shutdown: Option<ContextShutdown>,
    bindings: Vec<BoundPath>,
}

impl BrowserContextOwner {
    fn empty() -> Self {
        Self {
            outer: None,
            launcher: None,
            launcher_pipe: None,
            control: None,
            process: None,
            thread: None,
            pid: 0,
            tid: 0,
            bound: false,
            cpwl_pending: false,
            returned: None,
            pulled_ack: false,
            station_ready: false,
            browser_create_pending: false,
            browser_returned: None,
            close_sent: false,
            close_received: false,
            shutdown: None,
            bindings: Vec::new(),
        }
    }

    fn receive_cpwl(&mut self, deadline: Instant) -> Result<bool, WindowsProcessError> {
        let frame = held_mut(&mut self.launcher_pipe)?
            .receive(&held(&self.launcher)?.process, Some(deadline))?;
        match frame.op {
            11 => {
                let values = fields(frame, 11, 4)?;
                require(
                    values[2] != 0
                        && values[2] <= u64::from(u32::MAX)
                        && values[3] != 0
                        && values[3] <= u64::from(u32::MAX),
                    "returned context identity bound",
                )?;
                self.returned = Some(values);
                self.cpwl_pending = false;
                Ok(true)
            }
            12 => {
                fields(frame, 12, 1)?;
                self.cpwl_pending = false;
                Ok(false)
            }
            _ => Err(WindowsProcessError::Invalid(
                "unrecognized CPWL terminal frame".into(),
            )),
        }
    }

    fn pull_context(&mut self) -> Result<(), WindowsProcessError> {
        let returned = held(&self.returned)?;
        self.pid = returned[2] as u32;
        self.tid = returned[3] as u32;
        let launcher = &held(&self.launcher)?.process;
        if self.process.is_none() {
            self.process = Some(identity::pull(
                launcher,
                returned[0],
                PROCESS_DUP_HANDLE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | SYNC,
            )?);
        }
        let process = held(&self.process)?;
        identity::verify_process(
            process,
            self.pid,
            identity::birth(process.raw())?,
            self.bindings[0].identity(),
        )?;
        // SAFETY: query this actual Core's token to bind exceptional returned-child
        // cleanup before a later thread/Job check can fail. No thread is resumed.
        identity::ordinary(
            &identity::token(process.raw())?,
            &identity::token(unsafe { GetCurrentProcess() })?,
            true,
        )?;
        self.bound = true;
        if self.thread.is_none() {
            self.thread = Some(identity::pull(
                launcher,
                returned[1],
                THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION | SYNC,
            )?);
        }
        // SAFETY: query the retained returned thread, never a thread-ID enumeration.
        require(
            unsafe { GetThreadId(held(&self.thread)?.raw()) } == self.tid
                && unsafe { GetProcessIdOfThread(held(&self.thread)?.raw()) } == self.pid,
            "returned primary thread is not the exact context thread",
        )
    }

    fn all_exited(&self, deadline: Instant) -> Result<bool, WindowsProcessError> {
        if let Some(launcher) = &self.launcher
            && wait_timeout(&launcher.process, remaining(deadline))?.is_none()
        {
            return Ok(false);
        }
        if let Some(process) = &self.process
            && wait_timeout(process, remaining(deadline))?.is_none()
        {
            return Ok(false);
        }
        if let Some((job, _)) = &self.outer {
            return wait_tree_timeout(job, remaining(deadline));
        }
        Ok(true)
    }

    fn normal_exited(&self, deadline: Instant) -> Result<bool, WindowsProcessError> {
        if let Some(launcher) = &self.launcher
            && wait_timeout(&launcher.process, remaining(deadline))? != Some(0)
        {
            return Ok(false);
        }
        if let Some(process) = &self.process
            && wait_timeout(process, remaining(deadline))? != Some(0)
        {
            return Ok(false);
        }
        self.all_exited(deadline)
    }

    fn finish_io(&mut self) -> bool {
        let launcher = self.launcher_pipe.as_mut().is_none_or(Pipe::finish);
        let context = self.control.as_mut().is_none_or(Pipe::finish);
        launcher && context
    }

    pub(crate) fn retired(&mut self) -> Result<bool, WindowsProcessError> {
        Ok(self.shutdown.is_some()
            && !self.cpwl_pending
            && !self.browser_create_pending
            && self.all_exited(Instant::now())?)
    }

    pub(crate) fn resource_limit_violation(
        &self,
    ) -> Result<Option<ResourceLimitViolation>, WindowsProcessError> {
        match &self.outer {
            Some((job, port)) => resource_limit_violation(job, port),
            None => Ok(None),
        }
    }

    pub(crate) fn close(
        &mut self,
        timeout: Duration,
    ) -> Result<ContextShutdown, WindowsProcessError> {
        if let Some(result) = self.shutdown {
            require(self.retired()?, "retired context became live")?;
            return Ok(result);
        }
        let deadline = Instant::now() + timeout;
        let graceful = Instant::now() + timeout / 2;
        if self.station_ready {
            let normal = (|| {
                let peer = held(&self.process)?;
                let pipe = held_mut(&mut self.control)?;
                if !self.close_sent {
                    self.close_sent = true;
                    pipe.send(24, &[], peer, graceful)?;
                }
                if !self.close_received {
                    fields(pipe.receive(peer, Some(graceful))?, 25, 0)?;
                    self.close_received = true;
                }
                Ok::<_, WindowsProcessError>(self.normal_exited(graceful)? && self.finish_io())
            })();
            if matches!(normal, Ok(true)) {
                self.shutdown = Some(ContextShutdown::Graceful);
                return Ok(ContextShutdown::Graceful);
            }
        }
        if self.force(deadline)? {
            self.shutdown = Some(ContextShutdown::Forced);
            return Ok(ContextShutdown::Forced);
        }
        Err(WindowsProcessError::Invalid(
            "browser context shutdown remains unknown".into(),
        ))
    }

    fn force(&mut self, deadline: Instant) -> Result<bool, WindowsProcessError> {
        // An unreturned CPWL request cannot be discharged by killing its launcher
        // or observing a momentarily empty Job. Preserve its exact return channel.
        if self.cpwl_pending {
            match self.receive_cpwl(deadline) {
                Ok(_) => {}
                Err(_) => return Ok(false),
            }
        }
        if self.returned.is_some()
            && !self.pulled_ack
            && self.pull_context().is_err()
            && !self.bound
        {
            return Ok(false);
        }
        if let Some((job, _)) = &self.outer {
            terminate(job, 1)?;
        }
        let mut termination_error = None;
        if self.bound
            && let Some(process) = &self.process
            && wait_timeout(process, Duration::ZERO)?.is_none()
        {
            // SAFETY: actual Core-attested exact returned context; no PID mutation.
            if unsafe { TerminateProcess(process.raw(), 1) } == 0 {
                termination_error = Some(last_error("TerminateProcess(exact browser context)"));
            }
        }
        // TerminateJobObject can win this race before the direct termination.
        // Only positive retained-object exits may supersede that redundant error.
        let exited = self.all_exited(deadline)?;
        if !exited && let Some(error) = termination_error {
            return Err(error);
        }
        if exited {
            // Unlike external CPWL, this local synchronous CreateProcess cannot
            // continue after its exact retained creator positively exited.
            self.browser_create_pending = false;
        }
        Ok(exited && self.finish_io())
    }
}

pub(crate) struct BrowserSetupOwner {
    #[cfg(test)]
    test_control: Option<startup_tests::StartupControl>,
    context: Option<BrowserContextOwner>,
    inner: Option<(OwnedHandle, OwnedHandle)>,
    network: Option<NetworkGuard>,
    browser: Option<Created>,
    partial_browser: Option<OwnedHandle>,
    pipes: Option<PipeSet>,
    channels: Vec<ChannelPipes>,
    stdio: Option<(File, File, File)>,
    parents: Vec<crate::PrivateProcessChannel>,
}

impl BrowserSetupOwner {
    fn empty() -> Self {
        Self {
            #[cfg(test)]
            test_control: None,
            context: Some(BrowserContextOwner::empty()),
            inner: None,
            network: None,
            browser: None,
            partial_browser: None,
            pipes: None,
            channels: Vec::new(),
            stdio: None,
            parents: Vec::new(),
        }
    }

    pub(crate) fn cleanup(&mut self, timeout: Duration) -> Result<bool, WindowsProcessError> {
        let deadline = Instant::now() + timeout;
        let context = held_mut(&mut self.context)?;
        if context.cpwl_pending {
            match context.receive_cpwl(deadline) {
                Ok(_) => {}
                Err(_) => return Ok(false),
            }
        }
        // Stop and positively wait the exact creator before the FINAL inner zero:
        // a zero observed while CreateProcess is still possible proves nothing.
        if !context.force(deadline)? {
            return Ok(false);
        }
        if let Some((job, _)) = &self.inner {
            terminate(job, 1)?;
        }
        let browser = self
            .browser
            .as_ref()
            .map(|browser| &browser.process)
            .or(self.partial_browser.as_ref());
        if let Some(browser) = browser
            && wait_timeout(browser, remaining(deadline))?.is_none()
        {
            return Ok(false);
        }
        if let Some((job, _)) = &self.inner
            && !wait_tree_timeout(job, remaining(deadline))?
        {
            return Ok(false);
        }
        context.shutdown = Some(ContextShutdown::Forced);
        if let Some(network) = &mut self.network {
            network.retire()?;
        }
        self.network.take();
        self.browser.take();
        self.partial_browser.take();
        self.pipes.take();
        self.channels.clear();
        self.stdio.take();
        self.parents.clear();
        Ok(true)
    }

    fn start(&mut self, request: &SpawnRequest, count: usize) -> Result<(), WindowsProcessError> {
        let deadline = Instant::now() + SETUP_BOUND;
        launch::fixed_executable(&request.executable)?;
        let directory = request.executable.parent().ok_or_else(|| {
            WindowsProcessError::Invalid("browser executable directory absent".into())
        })?;
        require(
            directory == request.cwd,
            "browser context cwd differs from its loaded component directory",
        )?;
        let context = held_mut(&mut self.context)?;
        context
            .bindings
            .push(native(BoundPath::open_immutable_file(&request.executable))?);
        context.bindings.push(native(BoundPath::open_immutable_file(
            &directory.join("colossus-native-browser-host.dll"),
        ))?);
        // SAFETY: current process pseudo handle is only used for observation/duplication.
        let current = unsafe { GetCurrentProcess() };
        let core_token = identity::token(current)?;
        context.bindings.push(identity::image(current)?);
        let mut nonce = [0_u8; 16];
        // SAFETY: exact writable nonce buffer; no caller-selected entropy source.
        require(
            unsafe {
                BCryptGenRandom(
                    null_mut(),
                    nonce.as_mut_ptr(),
                    16,
                    BCRYPT_USE_SYSTEM_PREFERRED_RNG,
                )
            } >= 0,
            "context nonce RNG failed",
        )?;
        let arguments = Arguments {
            role: Role::Launcher,
            nonce,
            // SAFETY: current process identity query has no pointer inputs.
            core_pid: unsafe { GetCurrentProcessId() },
            core_birth: identity::birth(current)?,
            core_image: context.bindings[2].identity(),
        };
        let mut raw_sid = null_mut();
        let sid_text = super::wide(OsStr::new(&request.appcontainer_sid));
        // SAFETY: validated terminated SID string and writable LocalAlloc output.
        if unsafe { ConvertStringSidToSidW(sid_text.as_ptr(), &mut raw_sid) } == 0 {
            return Err(last_error("parse browser package SID"));
        }
        let sid = LocalSid(raw_sid);
        if let (Some(port), Some(id)) = (request.proxy_port, request.network_filter_id) {
            self.network = Some(NetworkGuard::open(id)?);
            held(&self.network)?.configure(sid.0, port, id)?;
        }
        self.inner = Some(new_job(request, count)?);
        context.outer = Some(new_job(request, count)?);
        let owner = identity::sid_text(&core_token.user)?;
        context.launcher_pipe = Some(ipc::server(
            &arguments.pipe_name(Role::Launcher),
            nonce,
            &owner,
        )?);
        context.control = Some(ipc::server(
            &arguments.pipe_name(Role::Context),
            nonce,
            &owner,
        )?);
        context.launcher = Some(launch::helper(
            request,
            &arguments,
            &held(&context.outer)?.0,
        )?);
        let launcher = held(&context.launcher)?;
        identity::verify_process(
            &launcher.process,
            launcher.pid,
            identity::birth(launcher.process.raw())?,
            context.bindings[0].identity(),
        )?;
        identity::ordinary(
            &identity::token(launcher.process.raw())?,
            &core_token,
            false,
        )?;
        member(&launcher.process, &held(&context.outer)?.0)?;
        identity::resume(&launcher.thread)?;
        let launcher_pipe = held_mut(&mut context.launcher_pipe)?;
        launcher_pipe.connect(&launcher.process, deadline)?;
        // SAFETY: the Pipe owns this connected native handle during validation.
        let borrowed = unsafe { BorrowedHandle::borrow_raw(launcher_pipe.raw().cast()) };
        native(colossus_windows_native::validate_named_pipe_client(
            &borrowed,
            launcher.pid,
        ))?;
        fields(
            launcher_pipe.receive(&launcher.process, Some(deadline))?,
            1,
            0,
        )?;
        context.cpwl_pending = true; // Set BEFORE any possibly delivered start request.
        launcher_pipe.send(10, &[], &launcher.process, deadline)?;
        #[cfg(test)]
        startup_tests::reach(
            &mut self.test_control,
            startup_tests::Point::RequestQueued,
            &[],
        )?;
        require(
            context.receive_cpwl(deadline)?,
            "basic CPWL returned failure",
        )?;
        context.pull_context()?;
        let process = held(&context.process)?;
        let birth = identity::birth(process.raw())?;
        identity::verify_process(process, context.pid, birth, context.bindings[0].identity())?;
        let context_token = identity::token(process.raw())?;
        identity::ordinary(&context_token, &core_token, true)?;
        context.bound = true;
        member(process, &held(&context.outer)?.0)?;
        #[cfg(test)]
        startup_tests::reach(
            &mut self.test_control,
            startup_tests::Point::ReturnedContextBound,
            &[(1, &held(&context.launcher)?.process), (2, process)],
        )?;
        held_mut(&mut context.launcher_pipe)?.send(
            13,
            &[],
            &held(&context.launcher)?.process,
            deadline,
        )?;
        context.pulled_ack = true;
        require(
            wait_timeout(&held(&context.launcher)?.process, remaining(deadline))? == Some(0),
            "launcher did not exit normally",
        )?;
        identity::resume(held(&context.thread)?)?;
        let control = held_mut(&mut context.control)?;
        control.connect(process, deadline)?;
        // SAFETY: the connected control pipe remains retained through validation.
        let borrowed = unsafe { BorrowedHandle::borrow_raw(control.raw().cast()) };
        native(colossus_windows_native::validate_named_pipe_client(
            &borrowed,
            context.pid,
        ))?;
        fields(control.receive(process, Some(deadline))?, 1, 0)?;
        self.pipes = Some(PipeSet::new()?);
        for _ in 0..count {
            self.channels.push(ChannelPipes::new()?);
        }
        let pipes = held(&self.pipes)?;
        let mut endpoints = vec![
            pipes.child_stdin.raw(),
            pipes.child_stdout.raw(),
            pipes.child_stderr.raw(),
        ];
        for channel in &self.channels {
            endpoints.extend(channel.child_handles());
        }
        let mut remote = Vec::with_capacity(endpoints.len());
        for endpoint in endpoints {
            // SAFETY: clear inheritance on every Core child end; only exact remote
            // copies in the retained fresh creator may be inherited by browser.
            if unsafe { SetHandleInformation(endpoint, HANDLE_FLAG_INHERIT, 0) } == 0 {
                return Err(last_error("clear Core endpoint inheritance"));
            }
            let mut target = null_mut();
            // SAFETY: pipe remains Core-owned; duplicate only into exact retained ctx.
            if unsafe {
                DuplicateHandle(
                    current,
                    endpoint,
                    process.raw(),
                    &mut target,
                    0,
                    1,
                    DUPLICATE_SAME_ACCESS,
                )
            } == 0
            {
                return Err(last_error("duplicate context pipe endpoint"));
            }
            remote.push(target as usize as u64);
        }
        control.send(20, &payload(&remote), process, deadline)?;
        fields(control.receive(process, Some(deadline))?, 21, 0)?;
        #[cfg(test)]
        startup_tests::reach(
            &mut self.test_control,
            startup_tests::Point::EndpointsRegistered,
            &[],
        )?;
        let envelope = launch::BrowserValues::encode(request)?;
        let job = identity::duplicate(
            held(&self.inner)?.0.raw(),
            process.raw(),
            JOB_OBJECT_ASSIGN_PROCESS | JOB_OBJECT_QUERY,
            false,
        )?;
        control.send(
            26,
            &payload(&[job, envelope.len() as u64]),
            process,
            deadline,
        )?;
        for chunk in envelope.chunks(224) {
            control.send(27, chunk, process, deadline)?;
        }
        // Set BEFORE possibly delivering the only command allowed to create.
        context.browser_create_pending = true;
        control.send(28, &[], process, deadline)?;
        #[cfg(test)]
        startup_tests::reach(
            &mut self.test_control,
            startup_tests::Point::BrowserCommitQueued,
            &[],
        )?;
        let returned = fields(control.receive(process, Some(deadline))?, 29, 5)?;
        context.browser_returned = Some(returned);
        let returned = held(&context.browser_returned)?;
        let pid = u32::try_from(returned[2])
            .map_err(|_| WindowsProcessError::Invalid("returned browser PID bound".into()))?;
        let tid = u32::try_from(returned[3])
            .map_err(|_| WindowsProcessError::Invalid("returned browser TID bound".into()))?;
        require(
            pid != 0 && tid != 0 && returned[4] != 0 && returned[0] != returned[1],
            "returned browser identity absent or aliased",
        )?;
        self.partial_browser = Some(identity::pull(
            process,
            returned[0],
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_DUP_HANDLE | PROCESS_TERMINATE | SYNC,
        )?);
        let thread = identity::pull(
            process,
            returned[1],
            THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION | SYNC,
        )?;
        self.browser = Some(Created {
            process: self.partial_browser.take().ok_or_else(|| {
                WindowsProcessError::Invalid("partial browser process missing".into())
            })?,
            thread,
            pid,
            tid,
        });
        let browser = held(&self.browser)?;
        identity::verify_process(
            &browser.process,
            browser.pid,
            returned[4],
            context.bindings[0].identity(),
        )?;
        // SAFETY: query the exact pulled primary thread, never a TID lookup.
        require(
            unsafe { GetThreadId(browser.thread.raw()) } == browser.tid
                && unsafe { GetProcessIdOfThread(browser.thread.raw()) } == browser.pid,
            "returned primary thread is not the exact browser thread",
        )?;
        identity::browser(
            &identity::token(browser.process.raw())?,
            &core_token,
            &context_token,
            sid.0,
        )?;
        member(&browser.process, &held(&self.inner)?.0)?;
        member(&browser.process, &held(&context.outer)?.0)?;
        #[cfg(test)]
        startup_tests::reach(
            &mut self.test_control,
            startup_tests::Point::BrowserPulled,
            &[],
        )?;
        control.send(30, &[], process, deadline)?;
        control.send(22, &[], process, deadline)?;
        fields(control.receive(process, Some(deadline))?, 23, 0)?;
        context.station_ready = true;
        context.browser_create_pending = false;
        #[cfg(test)]
        startup_tests::reach(
            &mut self.test_control,
            startup_tests::Point::StationReady,
            &[(2, process), (3, &browser.process)],
        )?;
        // Core alone resumes only after exact token/dual-Job and station/EOF barrier.
        identity::browser(
            &identity::token(browser.process.raw())?,
            &core_token,
            &context_token,
            sid.0,
        )?;
        member(&browser.process, &held(&self.inner)?.0)?;
        member(&browser.process, &held(&context.outer)?.0)?;
        // Retire every Core child-end copy BEFORE resume; retain only parent ends
        // in this same rollback owner until the infallible successful transfer.
        let PipeSet {
            child_stdin,
            parent_stdin,
            parent_stdout,
            child_stdout,
            parent_stderr,
            child_stderr,
        } = self.pipes.take().ok_or_else(|| {
            WindowsProcessError::Invalid("stdio ownership missing before resume".into())
        })?;
        drop((child_stdin, child_stdout, child_stderr));
        self.stdio = Some((
            parent_stdin.into_file(),
            parent_stdout.into_file(),
            parent_stderr.into_file(),
        ));
        self.parents = self
            .channels
            .drain(..)
            .map(ChannelPipes::into_parent)
            .collect();
        identity::resume(&browser.thread)
    }

    fn commit(mut self) -> SupervisedChild {
        // Success established every required owner; no fallible operation or await
        // follows this transfer. No fabricated child exists on an error path.
        let Created {
            process,
            thread,
            pid,
            ..
        } = self.browser.take().expect("successful setup owns browser");
        drop(thread);
        let (job, completion_port) = self.inner.take().expect("successful setup owns inner Job");
        let (stdin, stdout, stderr) = self
            .stdio
            .take()
            .expect("successful setup owns parent stdio");
        let child = SandboxedChild::from_parts(SandboxedChildParts {
            pid,
            stdin,
            stdout,
            stderr,
            process,
            job,
            completion_port,
            network: self.network.take(),
            context: self.context.take(),
        });
        SupervisedChild {
            child,
            channels: std::mem::take(&mut self.parents),
        }
    }
}

pub(super) fn spawn(
    request: &SpawnRequest,
    count: usize,
) -> Result<SupervisedChild, WindowsProcessError> {
    run_setup(Box::new(BrowserSetupOwner::empty()), request, count)
}

fn run_setup(
    mut setup: Box<BrowserSetupOwner>,
    request: &SpawnRequest,
    count: usize,
) -> Result<SupervisedChild, WindowsProcessError> {
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| setup.start(request, count)));
    match result {
        Ok(Ok(())) => Ok(setup.commit()),
        result => {
            let source = match result {
                Ok(Err(source)) => source,
                _ => WindowsProcessError::Invalid(
                    "browser startup panicked with native custody retained".into(),
                ),
            };
            Err(WindowsProcessError::BrowserSetup {
                source: Box::new(source),
                cleanup: PendingBrowserCleanup { owner: Some(setup) },
            })
        }
    }
}

fn preserve<T>(value: &mut Option<T>) {
    if let Some(value) = value.take() {
        std::mem::forget(value);
    }
}

impl Drop for BrowserSetupOwner {
    fn drop(&mut self) {
        if self.context.is_none() {
            return;
        } // Infallible success moved all custody.
        if !matches!(self.cleanup(Duration::from_secs(1)), Ok(true)) {
            preserve(&mut self.context);
            preserve(&mut self.inner);
            preserve(&mut self.network);
            preserve(&mut self.browser);
            preserve(&mut self.partial_browser);
            preserve(&mut self.pipes);
            preserve(&mut self.stdio);
            std::mem::forget(std::mem::take(&mut self.channels));
            std::mem::forget(std::mem::take(&mut self.parents));
        }
    }
}

impl Drop for BrowserContextOwner {
    fn drop(&mut self) {
        if !matches!(
            self.force(Instant::now() + Duration::from_secs(1)),
            Ok(true)
        ) {
            preserve(&mut self.outer);
            preserve(&mut self.launcher);
            preserve(&mut self.process);
            preserve(&mut self.thread);
            preserve(&mut self.launcher_pipe);
            preserve(&mut self.control);
            std::mem::forget(std::mem::take(&mut self.bindings));
        }
    }
}
