use super::*;

/// One run's counters survive transcript trimming and are never a session total.
#[derive(Default)]
pub(super) struct RunUsageState {
    session_id: String,
    current: Option<RunTokenUsage>,
    accepting: bool,
    fresh_run_scope: bool,
    root_run_id: Option<String>,
    root_terminal: bool,
    observed_run: bool,
    last_presented: Option<RunTokenUsage>,
}

impl RunUsageState {
    pub(super) fn restored(session_id: &str, report: Option<RunTokenUsage>) -> Self {
        let mut state = Self {
            session_id: session_id.into(),
            ..Self::default()
        };
        state.replace_canonical(session_id, report);
        state
    }

    pub(super) fn current(&self) -> Option<&RunTokenUsage> {
        self.current.as_ref()
    }

    pub(super) fn begin(&mut self, session_id: &str, reset: bool) {
        if self.session_id != session_id {
            *self = Self::restored(session_id, None);
        }
        if reset {
            self.current = None;
        }
        self.accepting = true;
        self.fresh_run_scope = reset;
        self.root_run_id = None;
        self.root_terminal = false;
        self.observed_run = false;
    }

    pub(super) fn observe(&mut self, envelope: &RunEventEnvelope) -> bool {
        if !self.accepting || envelope.session_id != self.session_id {
            return false;
        }
        let next_direct_run = self.root_terminal
            && matches!(
                &envelope.event,
                RunEvent::Phase {
                    phase: colossus_contracts::RunPhase::Preparing,
                    ..
                }
            );
        if self.root_run_id.is_none()
            || (self.root_run_id.as_deref() != Some(envelope.run_id.as_str()) && next_direct_run)
        {
            // The direct run's first phase precedes forwarded child activity.
            // Usage alone cannot claim an operation's identity.
            if !matches!(&envelope.event, RunEvent::Phase { .. }) {
                return false;
            }
            self.root_run_id = Some(envelope.run_id.clone());
            self.root_terminal = false;
            self.observed_run = true;
            self.current = Some(RunTokenUsage {
                run_id: envelope.run_id.clone(),
                session_id: envelope.session_id.clone(),
                through_version: 0,
                reports: 0,
                usage: None,
            });
        }
        if self.root_run_id.as_deref() != Some(envelope.run_id.as_str()) {
            return false;
        }
        if matches!(
            &envelope.event,
            RunEvent::Phase {
                phase: colossus_contracts::RunPhase::Completed
                    | colossus_contracts::RunPhase::Cancelled,
                ..
            }
        ) {
            self.root_terminal = true;
            return true;
        }
        if self.root_terminal {
            return false;
        }
        let RunEvent::Provider {
            event: ProviderEvent::Usage { usage },
        } = &envelope.event
        else {
            return true;
        };
        let current = self
            .current
            .as_mut()
            .expect("direct run phase established usage state");
        current.usage = if current.reports == 0 {
            Some(usage.clone())
        } else {
            current
                .usage
                .as_ref()
                .and_then(|total| total.checked_add(usage))
        };
        match current.reports.checked_add(1) {
            Some(reports) => current.reports = reports,
            None => {
                current.reports = u64::MAX;
                current.usage = None;
            }
        }
        true
    }

    pub(super) fn replace_canonical(&mut self, session_id: &str, report: Option<RunTokenUsage>) {
        if self.session_id != session_id {
            *self = Self::restored(session_id, None);
        }
        if let Some(report) =
            report.filter(|report| report.session_id == session_id && !report.run_id.is_empty())
        {
            if self.observed_run && self.root_run_id.as_deref() != Some(report.run_id.as_str()) {
                return;
            }
            // Replace the canonical baseline. Adding it would count the live
            // callbacks a second time. An unavailable read retains received usage.
            self.current = Some(report);
        }
    }

    /// Only command completion may establish a unary run without live events.
    /// Ordinary footer restoration remains passive and never creates a summary.
    fn complete_command(&mut self, session_id: &str, report: Option<&RunTokenUsage>) {
        if !self.accepting || !self.fresh_run_scope || self.observed_run {
            return;
        }
        let Some(report) = report.filter(|report| {
            self.session_id == session_id
                && report.session_id == session_id
                && !report.run_id.is_empty()
        }) else {
            return;
        };
        self.root_run_id = Some(report.run_id.clone());
        self.root_terminal = true;
        self.observed_run = true;
        self.current = Some(report.clone());
    }

    pub(super) fn finish(&mut self) -> Option<RunTokenUsage> {
        self.accepting = false;
        self.fresh_run_scope = false;
        if !self.observed_run {
            return None;
        }
        self.observed_run = false;
        let report = self.current.clone()?;
        if self.last_presented.as_ref() == Some(&report) {
            return None;
        }
        self.last_presented = Some(report.clone());
        Some(report)
    }
}

impl TuiState {
    pub(super) fn begin_run_usage(&mut self, reset: bool) {
        self.run_usage.begin(&self.session_id, reset);
        self.footer.usage = self.run_usage.current().cloned();
    }

    pub(super) fn replace_footer(&mut self, mut footer: FooterState) {
        self.run_usage
            .replace_canonical(&self.session_id, footer.usage.take());
        footer.usage = self.run_usage.current().cloned();
        self.footer = footer;
    }

    pub(super) fn replace_command_footer(&mut self, footer: FooterState) {
        self.run_usage
            .complete_command(&self.session_id, footer.usage.as_ref());
        self.replace_footer(footer);
    }

    pub(super) fn finish_run_usage(&mut self) {
        if let Some(report) = self.run_usage.finish() {
            self.append_entry(TranscriptEntry {
                sequence: None,
                kind: TranscriptKind::Command,
                document: run_token_usage_document(&report),
                temporary: false,
            });
        }
    }
}

pub(super) fn command_starts_run_usage(command: &RuntimeCommand) -> bool {
    matches!(command, RuntimeCommand::Known { name, arguments }
        if !arguments.trim().is_empty()
            && (name == "goal" || (name == "research" && arguments.trim() != "list")))
}

pub(super) fn compact_token_count(value: u64) -> String {
    let mut divisor = 1_u64;
    let mut suffix = "";
    for unit in ["k", "M", "B", "T", "P", "E"] {
        let Some(next) = divisor.checked_mul(1_000) else {
            break;
        };
        if value < next {
            break;
        }
        divisor = next;
        suffix = unit;
    }
    if divisor == 1 {
        return value.to_string();
    }
    let whole = value / divisor;
    let tenths = (u128::from(value % divisor) * 10 / u128::from(divisor)) as u64;
    if tenths == 0 {
        format!("{whole}{suffix}")
    } else {
        format!("{whole}.{tenths}{suffix}")
    }
}

pub(super) fn usage_footer_text(report: &RunTokenUsage, width: usize) -> String {
    let Some(usage) = report.usage.as_ref() else {
        return if width >= 4 {
            "tok—".into()
        } else {
            String::new()
        };
    };
    let total = compact_token_count(usage.total_tokens);
    let candidates = [
        format!(
            "{total} tok · {} in / {} out",
            compact_token_count(usage.input_tokens),
            compact_token_count(usage.output_tokens)
        ),
        format!("{total} tok"),
        format!("{total}t"),
    ];
    candidates
        .into_iter()
        .find(|text| UnicodeWidthStr::width(text.as_str()) <= width)
        .unwrap_or_default()
}
