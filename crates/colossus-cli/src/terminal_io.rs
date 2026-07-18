use super::*;

#[cfg(windows)]
pub(super) struct WindowsMainError(pub(super) String);

#[cfg(windows)]
impl fmt::Debug for WindowsMainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(windows)]
impl fmt::Display for WindowsMainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(windows)]
impl Error for WindowsMainError {}

impl ApprovalMode {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Ask => "ask",
            Self::RiskAuto => "risk_auto",
            Self::FullAccess => "full_access",
        }
    }
}

pub(super) struct TerminalApproval {
    pub(super) risk_auto: bool,
    pub(super) lock: Mutex<()>,
}

pub(super) struct TerminalUserPrompt {
    pub(super) lock: Mutex<()>,
}

#[async_trait]
impl UserPromptProvider for TerminalUserPrompt {
    async fn prompt(&self, request: UserPromptRequest) -> Result<UserPromptResponse, ToolError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| ToolError::Failed("user prompt terminal lock is poisoned".into()))?;
        let mut choices = PresentationTable::new(["#", "Choice"], "Enter a free-form answer.");
        for (index, choice) in request.choices.iter().enumerate() {
            choices.push_row([(index + 1).to_string(), choice.clone()]);
        }
        write_stderr_document(&PresentationDocument::from_block(PresentationBlock::Card {
            title: "Input needed".into(),
            tone: colossus_presentation::PresentationTone::Warning,
            body: vec![
                PresentationBlock::Markdown(request.question.clone()),
                PresentationBlock::Table(choices),
                PresentationBlock::Markdown(
                    "_The current agent turn is paused. Type an answer and press Enter; leave it blank to cancel this question._"
                        .into(),
                ),
            ],
        }))
        .map_err(|error| ToolError::Failed(error.to_string()))?;
        for _ in 0..3 {
            if request.choices.is_empty() {
                eprint!("Answer (blank cancels): ");
            } else if request.allow_free_form {
                eprint!("Choose a number or enter an answer (blank cancels): ");
            } else {
                eprint!("Choose a number (blank cancels): ");
            }
            io::stderr()
                .flush()
                .map_err(|error| ToolError::Failed(error.to_string()))?;
            let mut answer = String::new();
            io::stdin()
                .read_line(&mut answer)
                .map_err(|error| ToolError::Failed(error.to_string()))?;
            let answer = answer.trim();
            if answer.is_empty() {
                return Err(ToolError::Failed("user cancelled the question".into()));
            }
            if let Ok(index) = answer.parse::<usize>()
                && let Some(choice) = index
                    .checked_sub(1)
                    .and_then(|index| request.choices.get(index))
            {
                return Ok(UserPromptResponse {
                    answer: choice.clone(),
                    selected_index: Some(index - 1),
                });
            }
            if request.allow_free_form {
                return Ok(UserPromptResponse {
                    answer: answer.into(),
                    selected_index: request.choices.iter().position(|choice| choice == answer),
                });
            }
            eprintln!("Enter one of the numbered choices.");
        }
        Err(ToolError::Failed(
            "user did not provide a valid choice after three attempts".into(),
        ))
    }
}

#[async_trait]
impl ApprovalProvider for TerminalApproval {
    fn risk_auto_enabled(&self) -> bool {
        self.risk_auto
    }

    async fn request_approval(
        &self,
        request: &EffectRequest,
        request_hash: &str,
        decision: &PolicyDecision,
    ) -> Result<Option<ApprovalProof>, PolicyError> {
        let guard = self
            .lock
            .lock()
            .map_err(|_| PolicyError::Unavailable("approval terminal lock is poisoned".into()))?;
        let content = serde_json::to_string_pretty(&request.content)
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        let mut details = vec![
            ("Action".into(), request.action.clone()),
            ("Resource".into(), request.resource.clone()),
            ("Reason".into(), decision.reason.clone()),
        ];
        if let Some(reason) = request.risk.reason.as_deref() {
            let level = request.risk.level.as_deref().unwrap_or("unavailable");
            details.push(("Risk".into(), format!("{level}: {reason}")));
        }
        write_stderr_document(&PresentationDocument::from_block(PresentationBlock::Card {
            title: "Approval required".into(),
            tone: colossus_presentation::PresentationTone::Warning,
            body: vec![
                PresentationBlock::KeyValue(details),
                PresentationBlock::Code {
                    language: Some("proposed content".into()),
                    content: bounded_preview(&content, 1200).into(),
                },
            ],
        }))
        .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        eprint!("Approve this effect? [y/N] ");
        io::stderr()
            .flush()
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        let mut answer = String::new();
        io::stdin()
            .read_line(&mut answer)
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        let approved = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
        drop(guard);
        if !approved {
            return Ok(None);
        }
        ApprovalProvider::request_approval(
            &AllowApproval {
                approved_by: "terminal-user".into(),
            },
            request,
            request_hash,
            decision,
        )
        .await
    }
}

#[derive(Clone, Copy)]
pub(super) enum StreamTarget {
    Stdout,
    Stderr,
}

pub(super) struct TerminalStreamObserver {
    pub(super) target: StreamTarget,
    pub(super) wrote_text: bool,
    pub(super) buffered_text: String,
    pub(super) final_rendered: bool,
    pub(super) tool_calls: BTreeMap<String, ToolCall>,
    pub(super) preferences: TerminalPreferences,
    pub(super) activity: Option<tokio::task::JoinHandle<()>>,
    pub(super) output_lock: Arc<Mutex<()>>,
}

impl TerminalStreamObserver {
    pub(super) fn new(target: StreamTarget) -> Self {
        Self {
            target,
            wrote_text: false,
            buffered_text: String::new(),
            final_rendered: false,
            tool_calls: BTreeMap::new(),
            preferences: TerminalPreferences::default(),
            activity: None,
            output_lock: Arc::new(Mutex::new(())),
        }
    }

    pub(super) fn with_preferences(target: StreamTarget, preferences: TerminalPreferences) -> Self {
        Self {
            target,
            wrote_text: false,
            buffered_text: String::new(),
            final_rendered: false,
            tool_calls: BTreeMap::new(),
            preferences,
            activity: None,
            output_lock: Arc::new(Mutex::new(())),
        }
    }

    fn write_line(&mut self, line: &str) -> io::Result<()> {
        self.stop_activity()?;
        self.finish_line()?;
        let _guard = self
            .output_lock
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        match self.target {
            StreamTarget::Stdout => {
                println!("{line}");
                io::stdout().flush()
            }
            StreamTarget::Stderr => {
                eprintln!("{line}");
                io::stderr().flush()
            }
        }
    }

    pub(super) fn finish_line(&mut self) -> io::Result<()> {
        self.stop_activity()?;
        if self.wrote_text {
            let _guard = self
                .output_lock
                .lock()
                .map_err(|error| io::Error::other(error.to_string()))?;
            match self.target {
                StreamTarget::Stdout => {
                    println!();
                    io::stdout().flush()?;
                }
                StreamTarget::Stderr => {
                    eprintln!();
                    io::stderr().flush()?;
                }
            }
            self.wrote_text = false;
        }
        Ok(())
    }

    pub(super) fn finish_response(&mut self, fallback: &str) -> io::Result<()> {
        self.finish_line()?;
        if matches!(self.target, StreamTarget::Stdout)
            && self.preferences.stream_mode != StreamDisplayMode::Raw
            && !self.final_rendered
        {
            let output = if fallback.is_empty() {
                self.buffered_text.clone()
            } else {
                fallback.into()
            };
            self.write_markdown(&output)?;
            self.final_rendered = true;
        }
        Ok(())
    }

    fn write_markdown(&mut self, markdown: &str) -> io::Result<()> {
        let markdown = PresentationBlock::Markdown(markdown.into());
        let document = PresentationDocument::from_block(
            if self.preferences.transcript_density == TranscriptDensity::Comfortable {
                PresentationBlock::Card {
                    title: "Colossus".into(),
                    tone: colossus_presentation::PresentationTone::Neutral,
                    body: vec![markdown],
                }
            } else {
                markdown
            },
        );
        let rendered = TerminalDocumentRenderer::new(self.preferences.clone(), terminal_width())
            .with_color(self.is_terminal())
            .render(&document);
        self.write_line(&rendered)
    }

    fn is_terminal(&self) -> bool {
        match self.target {
            StreamTarget::Stdout => io::stdout().is_terminal(),
            StreamTarget::Stderr => io::stderr().is_terminal(),
        }
    }

    fn start_activity(&mut self, line: &str, elapsed_seconds: f64) -> io::Result<()> {
        if !self.is_terminal() {
            return self.write_line(line);
        }
        self.stop_activity()?;
        let target = self.target;
        let output_lock = Arc::clone(&self.output_lock);
        let template = line.to_owned();
        let palette = TerminalPalette::for_preferences(&self.preferences);
        write_transient_line(target, &output_lock, &template, elapsed_seconds, palette)?;
        let started = std::time::Instant::now();
        self.activity = Some(tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let elapsed = elapsed_seconds + started.elapsed().as_secs_f64();
                if write_transient_line(target, &output_lock, &template, elapsed, palette).is_err()
                {
                    break;
                }
            }
        }));
        Ok(())
    }

    fn stop_activity(&mut self) -> io::Result<()> {
        let Some(activity) = self.activity.take() else {
            return Ok(());
        };
        activity.abort();
        let _guard = self
            .output_lock
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        match self.target {
            StreamTarget::Stdout => {
                print!("\r\x1b[2K");
                io::stdout().flush()
            }
            StreamTarget::Stderr => {
                eprint!("\r\x1b[2K");
                io::stderr().flush()
            }
        }
    }
}

impl Drop for TerminalStreamObserver {
    fn drop(&mut self) {
        let _ = self.stop_activity();
    }
}

pub(super) fn activity_elapsed(event: &RunEvent) -> Option<f64> {
    match event {
        RunEvent::Phase {
            phase:
                colossus_contracts::RunPhase::Preparing
                | colossus_contracts::RunPhase::WaitingForModel
                | colossus_contracts::RunPhase::Responding,
            elapsed_seconds,
            ..
        } => Some(*elapsed_seconds),
        RunEvent::ToolStarted {
            call,
            elapsed_seconds,
            ..
        } if call.name != "user.ask" => Some(*elapsed_seconds),
        _ => None,
    }
}

pub(super) fn activity_line_at(template: &str, elapsed_seconds: f64) -> String {
    let Some(start) = template.rfind("elapsed=") else {
        return format!("{template} elapsed={elapsed_seconds:.2}s");
    };
    let value_start = start + "elapsed=".len();
    let Some(value_end) = template[value_start..].find('s') else {
        return format!("{template} elapsed={elapsed_seconds:.2}s");
    };
    let suffix_start = value_start + value_end + 1;
    format!(
        "{}elapsed={elapsed_seconds:.2}s{}",
        &template[..start],
        &template[suffix_start..]
    )
}

pub(super) fn write_transient_line(
    target: StreamTarget,
    output_lock: &Mutex<()>,
    template: &str,
    elapsed_seconds: f64,
    palette: TerminalPalette,
) -> io::Result<()> {
    let line = activity_line_at(template, elapsed_seconds);
    let spinner = palette.activity_frame(elapsed_seconds, true);
    let rendered = format!("{spinner} {line}");
    let _guard = output_lock
        .lock()
        .map_err(|error| io::Error::other(error.to_string()))?;
    match target {
        StreamTarget::Stdout => {
            print!("\r\x1b[2K{rendered}");
            io::stdout().flush()
        }
        StreamTarget::Stderr => {
            eprint!("\r\x1b[2K{rendered}");
            io::stderr().flush()
        }
    }
}

#[async_trait]
impl RunEventObserver for TerminalStreamObserver {
    async fn observe(&mut self, envelope: RunEventEnvelope) -> Result<(), ModelProviderError> {
        if let RunEvent::ToolStarted { call, .. } = &envelope.event {
            self.tool_calls.insert(call.call_id.clone(), call.clone());
        }
        if let RunEvent::Provider {
            event: ProviderEvent::ModelDelta { text },
        } = &envelope.event
        {
            self.stop_activity()
                .map_err(|error| ModelProviderError::Failed(error.to_string()))?;
            if self.preferences.stream_mode == StreamDisplayMode::Off {
                return Ok(());
            }
            if self.preferences.stream_mode == StreamDisplayMode::On
                && matches!(self.target, StreamTarget::Stdout)
            {
                self.buffered_text.push_str(text);
                return Ok(());
            }
            let _guard = self
                .output_lock
                .lock()
                .map_err(|error| ModelProviderError::Failed(error.to_string()))?;
            let text = SemanticRenderer::new(self.preferences.clone())
                .with_color(self.is_terminal())
                .assistant_text(text);
            let result = match self.target {
                StreamTarget::Stdout => {
                    print!("{text}");
                    io::stdout().flush()
                }
                StreamTarget::Stderr => {
                    eprint!("{text}");
                    io::stderr().flush()
                }
            };
            result.map_err(|error| ModelProviderError::Failed(error.to_string()))?;
            self.wrote_text = true;
            return Ok(());
        }
        if let RunEvent::ToolCompleted {
            turn,
            result,
            duration_seconds,
            elapsed_seconds,
        } = &envelope.event
        {
            let call = self.tool_calls.remove(&result.call_id);
            if let Some(line) = SemanticRenderer::new(self.preferences.clone())
                .with_color(self.is_terminal())
                .tool_completed_with_call(
                    *turn,
                    result,
                    *duration_seconds,
                    *elapsed_seconds,
                    call.as_ref(),
                )
                .map_err(|error| ModelProviderError::Failed(error.to_string()))?
            {
                self.write_line(&line)
                    .map_err(|error| ModelProviderError::Failed(error.to_string()))?;
            }
            return Ok(());
        }
        if let RunEvent::Provider {
            event: ProviderEvent::FinalOutput { text },
        } = &envelope.event
            && matches!(self.target, StreamTarget::Stdout)
            && self.preferences.stream_mode != StreamDisplayMode::Raw
        {
            self.write_markdown(text)
                .map_err(|error| ModelProviderError::Failed(error.to_string()))?;
            self.final_rendered = true;
            return Ok(());
        }
        if let Some(line) = SemanticRenderer::new(self.preferences.clone())
            .with_color(self.is_terminal())
            .run_event_envelope(&envelope)
            .map_err(|error| ModelProviderError::Failed(error.to_string()))?
        {
            if let Some(elapsed_seconds) = activity_elapsed(&envelope.event) {
                self.start_activity(&line, elapsed_seconds)
            } else {
                self.write_line(&line)
            }
            .map_err(|error| ModelProviderError::Failed(error.to_string()))?;
        }
        Ok(())
    }
}

pub(super) struct SilentStreamObserver;

#[async_trait]
impl RunEventObserver for SilentStreamObserver {
    async fn observe(&mut self, _event: RunEventEnvelope) -> Result<(), ModelProviderError> {
        Ok(())
    }
}

pub(super) fn bounded_preview(value: &str, max_chars: usize) -> &str {
    value
        .char_indices()
        .nth(max_chars)
        .map_or(value, |(end, _)| &value[..end])
}

pub(super) fn approval_provider(
    command: &Command,
    configured: Option<ApprovalMode>,
) -> Arc<dyn ApprovalProvider> {
    let mode = configured.unwrap_or(if matches!(command, Command::Tui { .. }) {
        ApprovalMode::Ask
    } else {
        ApprovalMode::Deny
    });
    match mode {
        ApprovalMode::Deny => Arc::new(DenyApproval),
        ApprovalMode::Ask | ApprovalMode::RiskAuto => Arc::new(TerminalApproval {
            risk_auto: mode == ApprovalMode::RiskAuto,
            lock: Mutex::new(()),
        }),
        ApprovalMode::FullAccess => Arc::new(AllowApproval {
            approved_by: "terminal-user:full-access".into(),
        }),
    }
}
