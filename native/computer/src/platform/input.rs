//! Balanced ordinary-user input. Only WM_NULL probes foreign activation completion.
use super::{
    Error,
    identity::{self, Window},
    indicator::{self, Signals},
};
use crate::{
    TrustedComputerContext, focus_settle, geometry,
    operator_control::{input_failure, undispatched_failure},
};
use colossus_contracts::{ComputerInput, ComputerMouseButton, ComputerNode, ComputerTarget};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use windows_sys::Win32::{
    Foundation::POINT,
    UI::{
        Input::KeyboardAndMouse::*,
        WindowsAndMessaging::{
            GA_ROOT, GetAncestor, SMTO_ABORTIFHUNG, SMTO_BLOCK, SMTO_ERRORONEXIT,
            SendMessageTimeoutW, SetForegroundWindow, WM_NULL, WindowFromPoint,
        },
    },
};

struct Gate<'a> {
    lifetime: crate::lifetime::Fence,
    root: &'a Window,
    window: &'a Window,
    context: &'a TrustedComputerContext,
    signals: &'a Arc<Signals>,
    human: u64,
    cancel: &'a AtomicBool,
    started: bool,
    deadline: std::time::Instant,
    native_epoch: u64,
}
impl Gate<'_> {
    fn check(&self) -> Result<(), Error> {
        let pending = self.signals.action.load(Ordering::Acquire);
        if self.cancel.load(Ordering::Acquire)
            || !self.lifetime.valid()
            || std::time::Instant::now() >= self.deadline
            || self.signals.native_epoch.load(Ordering::Acquire) != self.native_epoch
            || pending != 0
            || self.signals.human.load(Ordering::Acquire) != self.human
        {
            return Err(input_failure(
                undispatched_failure(Error::Denied, pending),
                self.started,
            ));
        }
        identity::revalidate_admission(self.root, self.window, self.context)
            .map_err(|error| input_failure(error, self.started))?;
        if identity::foreground() != self.window.hwnd() {
            return Err(if self.started {
                Error::OutcomeUnknown
            } else {
                Error::Stale
            });
        }
        Ok(())
    }
    fn point(&self, x: i32, y: i32) -> Result<(i32, i32), Error> {
        let (sx, sy) = geometry::point(self.window.rect, x, y).ok_or(Error::InvalidInput)?;
        // SAFETY: read-only native hit test ensures an occluding foreign app cannot receive input.
        let hit = unsafe { GetAncestor(WindowFromPoint(POINT { x: sx, y: sy }), GA_ROOT) };
        if hit != self.window.hwnd() {
            return Err(if self.started {
                Error::OutcomeUnknown
            } else {
                Error::Stale
            });
        }
        geometry::normalize(self.window.desktop, sx, sy).ok_or(Error::Stale)
    }
    fn send(&mut self, events: &[INPUT]) -> Result<(), Error> {
        self.check()?;
        if events.is_empty() || events.len() > 256 {
            return Err(Error::InvalidInput);
        }
        self.started = true;
        #[cfg(feature = "native-test-driver")]
        let before = (
            self.signals.own_mouse.load(Ordering::Relaxed),
            self.signals.foreign_mouse.load(Ordering::Relaxed),
            self.signals.foreign_injected_mouse.load(Ordering::Relaxed),
            self.signals
                .foreign_matching_low_tag
                .load(Ordering::Relaxed),
        );
        #[cfg(feature = "native-test-driver")]
        let send_started = std::time::Instant::now();
        #[cfg(feature = "native-test-driver")]
        eprintln!(
            "COMPUTER_NATIVE_INPUT phase=send_begin requested={}",
            events.len()
        );
        // SAFETY: fixed typed balanced native events backed by this slice, all carry our marker.
        let accepted = unsafe {
            SendInput(
                events.len() as u32,
                events.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        #[cfg(feature = "native-test-driver")]
        eprintln!(
            "COMPUTER_NATIVE_INPUT phase=send_receipt requested={} accepted={accepted} elapsed_ms={}",
            events.len(),
            send_started.elapsed().as_millis()
        );
        if accepted != events.len() as u32 {
            balance_prefix(&events[..accepted.min(events.len() as u32) as usize]);
            return Err(Error::OutcomeUnknown);
        }
        indicator::pump();
        #[cfg(feature = "native-test-driver")]
        eprintln!(
            "COMPUTER_NATIVE_INPUT phase=post_send cancelled={} control_action={} human_changed={} own_mouse={} foreign_mouse={} foreign_injected_mouse={} foreign_matching_low_tag={}",
            self.cancel.load(Ordering::Acquire),
            self.signals.action.load(Ordering::Acquire) != 0,
            self.signals.human.load(Ordering::Acquire) != self.human,
            self.signals
                .own_mouse
                .load(Ordering::Relaxed)
                .saturating_sub(before.0),
            self.signals
                .foreign_mouse
                .load(Ordering::Relaxed)
                .saturating_sub(before.1),
            self.signals
                .foreign_injected_mouse
                .load(Ordering::Relaxed)
                .saturating_sub(before.2),
            self.signals
                .foreign_matching_low_tag
                .load(Ordering::Relaxed)
                .saturating_sub(before.3)
        );
        // Full balanced insertion is the terminal effect receipt. An app may
        // legitimately open its owned modal or resize in response. A subsequent
        // action must observe that state afresh; do not relabel accepted input
        // as a known no-effect Stale just because its response changed focus.
        if self.cancel.load(Ordering::Acquire)
            || self.signals.action.load(Ordering::Acquire) != 0
            || self.signals.human.load(Ordering::Acquire) != self.human
        {
            Err(Error::OutcomeUnknown)
        } else {
            Ok(())
        }
    }
}
fn mouse(dx: i32, dy: i32, flags: u32, data: u32, tag: usize) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: tag,
            },
        },
    }
}
fn key(vk: u16, scan: u16, flags: u32, tag: usize) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: tag,
            },
        },
    }
}
fn motion(x: i32, y: i32, tag: usize) -> INPUT {
    mouse(
        x,
        y,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        0,
        tag,
    )
}
fn target(target: &ComputerTarget, nodes: &[ComputerNode]) -> Result<(i32, i32), Error> {
    match target {
        ComputerTarget::Point { x, y } => Ok((*x, *y)),
        ComputerTarget::Element { element_id } => {
            let node = nodes
                .iter()
                .find(|node| &node.element_id == element_id)
                .ok_or(Error::Stale)?;
            if !node.enabled {
                return Err(Error::InvalidInput);
            }
            if node.is_password {
                return Err(Error::ProtectedInput);
            }
            Ok((
                node.bounds.x + (node.bounds.width / 2) as i32,
                node.bounds.y + (node.bounds.height / 2) as i32,
            ))
        }
    }
}

pub(crate) struct InputScope<'a> {
    pub lifetime: crate::lifetime::Fence,
    pub root: &'a Window,
    pub window: &'a Window,
    pub context: &'a TrustedComputerContext,
    pub nodes: &'a [ComputerNode],
    pub signals: &'a Arc<Signals>,
    pub cancel: &'a AtomicBool,
    pub timeout: std::time::Duration,
    pub native_epoch: u64,
}
pub(crate) fn apply(
    scope: InputScope<'_>,
    input: &ComputerInput,
    mut protected: impl FnMut() -> Result<bool, Error>,
) -> Result<(), Error> {
    let action_started = std::time::Instant::now();
    let InputScope {
        lifetime,
        root,
        window,
        context,
        nodes,
        signals,
        cancel,
        timeout,
        native_epoch,
    } = scope;
    input.validate().map_err(|_| Error::InvalidInput)?;
    indicator::pump();
    let pending = signals.action.load(Ordering::Acquire);
    if pending != 0 {
        return Err(undispatched_failure(Error::Denied, pending));
    }
    if signals.native_epoch.load(Ordering::Acquire) != native_epoch {
        return Err(Error::Stale);
    }
    if !lifetime.valid() {
        return Err(Error::Stale);
    }
    let human = signals.human.load(Ordering::Acquire);
    // A physical held button/modifier is human authority, never ours to release.
    // SAFETY: read-only asynchronous state query; no key events are generated.
    if [
        VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN,
    ]
    .iter()
    .any(|vk| {
        // SAFETY: fixed key-state query; no human key is modified or released.
        (unsafe { GetAsyncKeyState(i32::from(*vk)) }) < 0
    }) {
        return Err(Error::InputHeld);
    }
    if matches!(input, ComputerInput::Focus { .. }) {
        #[cfg(feature = "native-test-driver")]
        eprintln!("COMPUTER_NATIVE_FOCUS phase=admission");
        let accepted = focus_settle::activate(
            || {
                identity::revalidate_admission(root, window, context).inspect_err(|_| {
                    #[cfg(feature = "native-test-driver")]
                    eprintln!("COMPUTER_NATIVE_FOCUS phase=admission_denied");
                })
            },
            || {
                indicator::pump();
                let conditions = focus_settle::Conditions {
                    foreground: false,
                    human_unchanged: signals.human.load(Ordering::Acquire) == human,
                    cancelled: cancel.load(Ordering::Acquire),
                    control_pending: signals.action.load(Ordering::Acquire) != 0,
                    epoch_unchanged: signals.native_epoch.load(Ordering::Acquire) == native_epoch,
                    admission_valid: true,
                    lifetime_valid: lifetime.valid(),
                };
                #[cfg(feature = "native-test-driver")]
                trace_focus_conditions("preactivation", conditions);
                (conditions, action_started.elapsed() < timeout)
            },
            || {
                // SAFETY: the original lifetime/authority fences were just drained
                // and checked; focus this exact admitted HWND only, once.
                unsafe { SetForegroundWindow(window.hwnd()) != 0 }
            },
            Error::Denied,
        )
        // Every activate error precedes its sole effect closure. A Stop queued
        // by the admission probe therefore has a trustworthy no-effect receipt.
        .map_err(|error| undispatched_failure(error, signals.action.load(Ordering::Acquire)))?;
        #[cfg(feature = "native-test-driver")]
        eprintln!("COMPUTER_NATIVE_FOCUS phase=activate accepted={accepted}");
        if !accepted {
            return Err(Error::FocusDenied);
        }
        // Cross-process activation is asynchronous. Acknowledging WM_NULL lets
        // the foreign queue process its activation nudge without attaching queues
        // or requesting activation again. Authority is rechecked between slices.
        let settling_started = std::time::Instant::now();
        #[cfg(feature = "native-test-driver")]
        let mut first_probe = true;
        let settled = focus_settle::settle(
            timeout.saturating_sub(action_started.elapsed()),
            || settling_started.elapsed(),
            || {
                indicator::pump();
                let admission_valid = identity::revalidate_admission(root, window, context).is_ok();
                // Native identity queries can race queued destroy/takeover
                // callbacks. Drain again before taking the final fence snapshot.
                indicator::pump();
                let conditions = focus_settle::Conditions {
                    foreground: identity::foreground() == window.hwnd(),
                    human_unchanged: signals.human.load(Ordering::Acquire) == human,
                    cancelled: cancel.load(Ordering::Acquire),
                    control_pending: signals.action.load(Ordering::Acquire) != 0,
                    epoch_unchanged: signals.native_epoch.load(Ordering::Acquire) == native_epoch,
                    admission_valid,
                    lifetime_valid: lifetime.valid(),
                };
                #[cfg(feature = "native-test-driver")]
                if first_probe {
                    trace_focus_conditions("initial_postcondition", conditions);
                    first_probe = false;
                }
                conditions
            },
            |slice| {
                let mut result = 0;
                // SAFETY: the exact admitted foreign HWND receives only WM_NULL
                // with no pointers/data. SMTO_BLOCK prevents sender reentrancy;
                // no input queues are attached and a hung/exited target is bounded.
                unsafe {
                    SendMessageTimeoutW(
                        window.hwnd(),
                        WM_NULL,
                        0,
                        0,
                        SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
                        slice.as_millis() as u32,
                        &mut result,
                    ) != 0
                }
            },
        );
        if let Err(failure) = settled {
            #[cfg(feature = "native-test-driver")]
            {
                eprintln!(
                    "COMPUTER_NATIVE_FOCUS phase=postcondition_failed reason={:?}",
                    failure.kind
                );
                trace_focus_conditions("final_postcondition", failure.conditions);
            }
            #[cfg(not(feature = "native-test-driver"))]
            let _ = failure;
            return Err(Error::OutcomeUnknown);
        }
        #[cfg(feature = "native-test-driver")]
        eprintln!("COMPUTER_NATIVE_FOCUS phase=complete");
        return Ok(());
    }
    let mut gate = Gate {
        lifetime,
        root,
        window,
        context,
        signals,
        human,
        cancel,
        started: false,
        deadline: std::time::Instant::now() + timeout,
        native_epoch,
    };
    gate.check()?;
    let tag = signals.tag.value();
    match input {
        ComputerInput::Move { x, y } => {
            let (x, y) = gate.point(*x, *y)?;
            gate.send(&[motion(x, y, tag)])
        }
        ComputerInput::Click {
            target: location,
            button,
            count,
        } => {
            let (x, y) = target(location, nodes)?;
            let (x, y) = gate.point(x, y)?;
            let (down, up) = match button {
                ComputerMouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
                ComputerMouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
                ComputerMouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
            };
            let mut events = vec![motion(x, y, tag)];
            for _ in 0..*count {
                events.push(mouse(0, 0, down, 0, tag));
                events.push(mouse(0, 0, up, 0, tag));
            }
            gate.send(&events)
        }
        ComputerInput::Scroll {
            x,
            y,
            delta_x,
            delta_y,
        } => {
            let (x, y) = gate.point(*x, *y)?;
            let mut events = vec![motion(x, y, tag)];
            if *delta_y != 0 {
                events.push(mouse(0, 0, MOUSEEVENTF_WHEEL, *delta_y as u32, tag));
            }
            if *delta_x != 0 {
                events.push(mouse(0, 0, MOUSEEVENTF_HWHEEL, *delta_x as u32, tag));
            }
            gate.send(&events)
        }
        ComputerInput::Type { text } => {
            for chunk in text_chunks(text) {
                if protected().map_err(|error| input_failure(error, gate.started))? {
                    return Err(if gate.started {
                        Error::OutcomeUnknown
                    } else {
                        Error::ProtectedInput
                    });
                }
                let mut events = Vec::with_capacity(chunk.len() * 2);
                for unit in chunk {
                    events.push(key(0, unit, KEYEVENTF_UNICODE, tag));
                    events.push(key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP, tag));
                }
                gate.send(&events)?;
            }
            Ok(())
        }
        ComputerInput::Press { keys } => {
            if protected()? {
                return Err(Error::ProtectedInput);
            }
            let mut events = Vec::new();
            let mut keys = keys
                .iter()
                .map(|name| virtual_key(name))
                .collect::<Result<Vec<_>, _>>()?;
            keys.sort_by_key(|(vk, _)| u8::from(![VK_CONTROL, VK_MENU, VK_SHIFT].contains(vk)));
            for (vk, extended) in &keys {
                events.push(key(*vk, 0, *extended, tag));
            }
            for (vk, extended) in keys.iter().rev() {
                events.push(key(*vk, 0, *extended | KEYEVENTF_KEYUP, tag));
            }
            gate.send(&events)
        }
        ComputerInput::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            steps,
        } => {
            gate.point(*to_x, *to_y)?;
            let (x, y) = gate.point(*from_x, *from_y)?;
            let mut points = Vec::with_capacity(usize::from(*steps));
            for step in 1..=*steps {
                let x = i64::from(*from_x)
                    + (i64::from(*to_x) - i64::from(*from_x)) * i64::from(step) / i64::from(*steps);
                let y = i64::from(*from_y)
                    + (i64::from(*to_y) - i64::from(*from_y)) * i64::from(step) / i64::from(*steps);
                let (x, y) = gate.point(x as i32, y as i32)?;
                points.push((x, y));
            }
            // The complete balanced gesture enters Windows atomically. There is
            // no synthetic hold spanning calls that helper/job death can strand.
            gate.send(&drag_events((x, y), &points, tag))
        }
        ComputerInput::Focus { .. } => Err(Error::InvalidInput),
    }
}

#[cfg(feature = "native-test-driver")]
fn trace_focus_conditions(phase: &'static str, conditions: focus_settle::Conditions) {
    eprintln!(
        "COMPUTER_NATIVE_FOCUS phase={phase} foreground={} human_unchanged={} cancelled={} control_pending={} epoch_unchanged={} admission_valid={} lifetime_valid={}",
        conditions.foreground,
        conditions.human_unchanged,
        conditions.cancelled,
        conditions.control_pending,
        conditions.epoch_unchanged,
        conditions.admission_valid,
        conditions.lifetime_valid,
    );
}

fn drag_events(start: (i32, i32), points: &[(i32, i32)], tag: usize) -> Vec<INPUT> {
    let mut events = Vec::with_capacity(points.len() + 3);
    events.push(motion(start.0, start.1, tag));
    events.push(mouse(0, 0, MOUSEEVENTF_LEFTDOWN, 0, tag));
    for &(x, y) in points {
        let mut event = motion(x, y, tag);
        // SAFETY: motion's constructor initializes the mouse arm.
        unsafe { event.Anonymous.mi.dwFlags |= MOUSEEVENTF_MOVE_NOCOALESCE };
        events.push(event);
    }
    events.push(mouse(0, 0, MOUSEEVENTF_LEFTUP, 0, tag));
    events
}

fn text_chunks(text: &str) -> Vec<Vec<u16>> {
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    for character in text.chars() {
        let mut buffer = [0; 2];
        let units = character.encode_utf16(&mut buffer);
        if current.len() + units.len() > 32 {
            chunks.push(std::mem::take(&mut current));
        }
        current.extend_from_slice(units);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Release only downs known accepted from a partial, otherwise balanced batch.
fn balance_prefix(events: &[INPUT]) {
    let releases = prefix_releases(events);
    if !releases.is_empty() {
        // SAFETY: releases correspond only to our accepted unmatched downs.
        unsafe {
            SendInput(
                releases.len() as u32,
                releases.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
    }
}
fn prefix_releases(events: &[INPUT]) -> Vec<INPUT> {
    let mut keys: Vec<KEYBDINPUT> = Vec::new();
    let mut buttons: Vec<(u32, usize)> = Vec::new();
    for event in events {
        if event.r#type == INPUT_KEYBOARD {
            // SAFETY: type tag and constructor prove the active union arm.
            let key = unsafe { event.Anonymous.ki };
            if key.dwFlags & KEYEVENTF_KEYUP == 0 {
                keys.push(key);
            } else if let Some(index) = keys
                .iter()
                .position(|k| k.wVk == key.wVk && k.wScan == key.wScan)
            {
                keys.remove(index);
            }
        } else if event.r#type == INPUT_MOUSE {
            // SAFETY: type tag and constructor prove the active union arm.
            let mouse = unsafe { event.Anonymous.mi };
            for (down, up) in [
                (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
                (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
                (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
            ] {
                if mouse.dwFlags & down != 0 {
                    buttons.push((up, mouse.dwExtraInfo));
                }
                if mouse.dwFlags & up != 0 {
                    buttons.retain(|(flag, _)| *flag != up);
                }
            }
        }
    }
    let mut releases = keys
        .into_iter()
        .rev()
        .map(|mut k| {
            k.dwFlags |= KEYEVENTF_KEYUP;
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 { ki: k },
            }
        })
        .collect::<Vec<_>>();
    releases.extend(buttons.into_iter().map(|(up, tag)| mouse(0, 0, up, 0, tag)));
    releases
}

fn virtual_key(name: &str) -> Result<(u16, u32), Error> {
    let vk = match name {
        "Control" => VK_CONTROL,
        "Alt" => VK_MENU,
        "Shift" => VK_SHIFT,
        "Enter" => VK_RETURN,
        "Tab" => VK_TAB,
        "Escape" => VK_ESCAPE,
        "Space" => VK_SPACE,
        "Backspace" => VK_BACK,
        "Delete" => VK_DELETE,
        "Insert" => VK_INSERT,
        "Home" => VK_HOME,
        "End" => VK_END,
        "PageUp" => VK_PRIOR,
        "PageDown" => VK_NEXT,
        "ArrowLeft" => VK_LEFT,
        "ArrowRight" => VK_RIGHT,
        "ArrowUp" => VK_UP,
        "ArrowDown" => VK_DOWN,
        value if value.len() == 1 && value.as_bytes()[0].is_ascii_alphanumeric() => {
            u16::from(value.as_bytes()[0])
        }
        value if value.starts_with('F') => {
            VK_F1
                + value[1..]
                    .parse::<u16>()
                    .ok()
                    .filter(|n| (1..=12).contains(n))
                    .ok_or(Error::InvalidInput)?
                - 1
        }
        _ => return Err(Error::InvalidInput),
    };
    let extended = if [
        VK_DELETE, VK_INSERT, VK_HOME, VK_END, VK_PRIOR, VK_NEXT, VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN,
    ]
    .contains(&vk)
    {
        KEYEVENTF_EXTENDEDKEY
    } else {
        0
    };
    Ok((vk, extended))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_batches_never_split_surrogate_pairs_and_are_bounded() {
        let text = format!("{}😀{}", "a".repeat(31), "中😀".repeat(80));
        let chunks = text_chunks(&text);
        assert!(
            chunks
                .iter()
                .all(|c| c.len() <= 32 && !(0xD800..=0xDBFF).contains(c.last().unwrap()))
        );
        assert_eq!(String::from_utf16(&chunks.concat()).unwrap(), text);
    }
    #[test]
    fn every_drag_prefix_releases_only_our_accepted_down_and_full_batch_balances() {
        let points = (0..100).map(|n| (n, n)).collect::<Vec<_>>();
        let events = drag_events((0, 0), &points, 713);
        assert!(events.len() <= 256);
        for prefix in 0..=events.len() {
            let releases = prefix_releases(&events[..prefix]);
            if prefix >= 2 && prefix < events.len() {
                assert_eq!(releases.len(), 1);
                // SAFETY: prefix_releases constructs a tagged mouse-up for this gesture.
                let release = unsafe { releases[0].Anonymous.mi };
                assert_eq!(release.dwFlags, MOUSEEVENTF_LEFTUP);
                assert_eq!(release.dwExtraInfo, 713);
            } else {
                assert!(releases.is_empty());
            }
        }
    }
    #[test]
    fn partially_accepted_chord_releases_exact_unmatched_keys_in_reverse_order() {
        let events = [
            key(VK_CONTROL, 0, 0, 42),
            key(65, 0, 0, 42),
            key(65, 0, KEYEVENTF_KEYUP, 42),
            key(VK_CONTROL, 0, KEYEVENTF_KEYUP, 42),
        ];
        let expected = [
            vec![],
            vec![VK_CONTROL],
            vec![65, VK_CONTROL],
            vec![VK_CONTROL],
            vec![],
        ];
        for prefix in 0..=events.len() {
            let releases = prefix_releases(&events[..prefix]);
            let actual = releases
                .iter()
                .map(|e| {
                    // SAFETY: these releases were constructed solely from tagged keyboard events.
                    let k = unsafe { e.Anonymous.ki };
                    assert_eq!(k.dwFlags & KEYEVENTF_KEYUP, KEYEVENTF_KEYUP);
                    assert_eq!(k.dwExtraInfo, 42);
                    k.wVk
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected[prefix]);
        }
    }
}
