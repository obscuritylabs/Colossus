use super::*;
use std::path::PathBuf;

pub(super) const COMMAND: &str = "/support-report";
pub(super) const UNAVAILABLE: &str = "Support reports are unavailable in this terminal host.";

pub(super) fn arguments(input: &str) -> Option<&str> {
    let rest = input.strip_prefix(COMMAND)?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then_some(rest)
}

fn parse(arguments: &str) -> Option<(PathBuf, bool)> {
    if arguments.contains(['\r', '\n']) {
        return None;
    }
    let arguments = arguments.trim_start();
    // Quotes delimit paths containing spaces. Backslashes always belong to the path.
    let (path, rest) = if let Some(quote @ ('\'' | '"')) = arguments.chars().next() {
        let (path, rest) = arguments[1..].split_once(quote)?;
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return None;
        }
        (path, rest)
    } else {
        let (path, rest) = arguments.split_at(
            arguments
                .find(char::is_whitespace)
                .unwrap_or(arguments.len()),
        );
        if path.starts_with('-') || path.contains(['\'', '"']) {
            return None;
        }
        (path, rest)
    };
    if path.is_empty() || path.chars().any(char::is_control) {
        return None;
    }
    let replace = match rest.trim() {
        "" => false,
        "--replace" => true,
        _ => return None,
    };
    Some((PathBuf::from(path), replace))
}

struct PendingSave {
    control: RunControl,
    result: oneshot::Receiver<Result<(), String>>,
}

#[derive(Default)]
pub(super) struct SupportReportState {
    pending: Option<PendingSave>,
    notice: Option<&'static str>,
}

impl SupportReportState {
    pub(super) fn start(&mut self, arguments: &str, host: Arc<dyn InteractiveHost>) {
        self.poll();
        if self.is_pending() {
            return;
        }
        let Some((output, replace)) = parse(arguments) else {
            self.notice = Some("Use /support-report PATH [--replace]");
            return;
        };
        let control = RunControl::default();
        let (sender, result) = oneshot::channel();
        self.pending = Some(PendingSave {
            control: control.clone(),
            result,
        });
        self.notice = Some("Saving support report · Esc cancels");
        tokio::spawn(async move {
            let _ = sender.send(host.save_support_report(output, replace, control).await);
        });
    }

    pub(super) fn poll(&mut self) {
        use tokio::sync::oneshot::error::TryRecvError;

        let Some(pending) = &mut self.pending else {
            return;
        };
        self.notice = Some(match pending.result.try_recv() {
            Ok(Ok(())) => "Support report saved.",
            Ok(Err(error)) => match error.as_str() {
                "The support report already exists. Choose a new file or use --replace." => {
                    "File exists · add --replace"
                }
                "Support report save cancelled." => "Support report save cancelled.",
                "The support report destination is unavailable." => {
                    "Report destination unavailable."
                }
                UNAVAILABLE => "Support reports unavailable here.",
                _ => "Support report could not be saved.",
            },
            Err(TryRecvError::Closed) => "Support report could not be saved.",
            Err(TryRecvError::Empty) => return,
        });
        self.pending = None;
    }

    pub(super) fn cancel(&mut self) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        if pending.control.is_cancelled() {
            return false;
        }
        pending.control.cancel();
        self.notice = Some("Cancelling support report…");
        true
    }

    pub(super) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn notice(&self) -> Option<&'static str> {
        self.notice
    }

    pub(super) fn clear_notice(&mut self) {
        if !self.is_pending() {
            self.notice = None;
        }
    }
}

impl Drop for SupportReportState {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests;
