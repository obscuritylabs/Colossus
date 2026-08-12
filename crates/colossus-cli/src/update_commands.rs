use super::*;

/// Run one bounded, credential-free stable release discovery check.
pub(super) async fn run_update_check() -> Result<(), Box<dyn Error>> {
    let report = UpdateService::for_current_installation().check().await;
    let terminal = io::stdout().is_terminal();
    if !human_output(terminal) {
        return print_json(&report);
    }
    print_terminal_document(
        &update_check_document(&report),
        &terminal_preferences(),
        terminal,
    );
    Ok(())
}

/// Apply one install-aware stable update without constructing the agent runtime.
pub(super) async fn run_update(requested_version: Option<&str>) -> Result<(), Box<dyn Error>> {
    let report = UpdateService::for_current_installation()
        .update(requested_version)
        .await;
    let terminal = io::stdout().is_terminal();
    if human_output(terminal) {
        print_terminal_document(
            &update_apply_document(&report),
            &terminal_preferences(),
            terminal,
        );
    } else {
        print_json(&report)?;
    }
    if matches!(
        report.status,
        UpdateApplyStatus::Updated | UpdateApplyStatus::Scheduled | UpdateApplyStatus::UpToDate
    ) {
        Ok(())
    } else {
        Err(io::Error::other("Colossus update did not complete").into())
    }
}

fn update_apply_document(report: &UpdateApplyReport) -> PresentationDocument {
    let details = vec![
        ("Current".into(), report.current_version.clone()),
        (
            "Selected".into(),
            report
                .selected_version
                .as_deref()
                .unwrap_or("unavailable")
                .into(),
        ),
        (
            "Installer".into(),
            installer_name(report.installer_kind).into(),
        ),
    ];
    match report.status {
        UpdateApplyStatus::Updated => PresentationDocument::from_block(PresentationBlock::Card {
            title: "Colossus updated".into(),
            tone: PresentationTone::Success,
            body: vec![
                PresentationBlock::KeyValue(details),
                PresentationBlock::Text(
                    "Restart running workers and terminals to use the new executable.".into(),
                ),
            ],
        }),
        UpdateApplyStatus::Scheduled => PresentationDocument::from_block(PresentationBlock::Card {
            title: "Colossus update scheduled".into(),
            tone: PresentationTone::Success,
            body: vec![
                PresentationBlock::KeyValue(details),
                PresentationBlock::Text(
                    "Windows will replace the executable after this command exits.".into(),
                ),
            ],
        }),
        UpdateApplyStatus::UpToDate => PresentationDocument::from_block(PresentationBlock::Card {
            title: "Colossus is up to date".into(),
            tone: PresentationTone::Success,
            body: vec![PresentationBlock::KeyValue(details)],
        }),
        UpdateApplyStatus::Refused => {
            let guidance = match report.refusal_reason {
                Some(UpdateRefusalReason::NotDirectInstallation) => match report.installer_kind {
                    InstallerKind::Homebrew => {
                        "Homebrew owns this executable. Run `brew upgrade obscuritylabs/tap/colossus`."
                    }
                    InstallerKind::Nix => {
                        "Nix owns this executable. Upgrade the Colossus profile or flake input through Nix."
                    }
                    InstallerKind::Source => {
                        "A source workflow owns this executable. Rebuild it from the selected source revision."
                    }
                    InstallerKind::Direct | InstallerKind::Unknown => {
                        "This executable has no matching direct-install receipt. Use the installation channel that owns it; to adopt a direct prefix, run the reviewed installer explicitly."
                    }
                },
                Some(UpdateRefusalReason::InvalidVersion) => {
                    "Use an exact stable version in the form `--version vX.Y.Z`."
                }
                Some(UpdateRefusalReason::Downgrade) => {
                    "Automatic replacement never downgrades the installed executable."
                }
                Some(UpdateRefusalReason::PreviewInstallation) => {
                    "Preview installations remain on the explicit preview installer path."
                }
                None => "The installation ownership policy refused replacement.",
            };
            PresentationDocument::from_block(PresentationBlock::Card {
                title: "Colossus update refused".into(),
                tone: PresentationTone::Warning,
                body: vec![
                    PresentationBlock::KeyValue(details),
                    PresentationBlock::Text(guidance.into()),
                ],
            })
        }
        UpdateApplyStatus::Unavailable => {
            let reason = match report.failure_reason {
                Some(UpdateApplyFailure::DiscoveryUnavailable) => {
                    "stable release discovery is unavailable"
                }
                Some(UpdateApplyFailure::LaunchFailed) => "the reviewed updater could not start",
                Some(UpdateApplyFailure::InstallFailed) => {
                    "the requested release was not installed; the prior executable was preserved"
                }
                Some(UpdateApplyFailure::TimedOut) => {
                    "the bounded installer timed out; inspect the receipt before retrying"
                }
                Some(UpdateApplyFailure::UnsupportedHost) => {
                    "this host has no supported native release target"
                }
                None => "the update is unavailable",
            };
            PresentationDocument::from_block(PresentationBlock::Card {
                title: "Colossus update unavailable".into(),
                tone: PresentationTone::Warning,
                body: vec![
                    PresentationBlock::KeyValue(details),
                    PresentationBlock::Text(reason.into()),
                ],
            })
        }
    }
}

fn installer_name(installer: InstallerKind) -> &'static str {
    match installer {
        InstallerKind::Direct => "direct",
        InstallerKind::Homebrew => "homebrew",
        InstallerKind::Nix => "nix",
        InstallerKind::Source => "source",
        InstallerKind::Unknown => "unknown",
    }
}

/// Construct the one-shot background notice used by both embedded and worker-backed TUIs.
pub(super) fn default_update_notice_provider() -> Arc<dyn BackgroundNoticeProvider> {
    Arc::new(UpdateNoticeProvider::new(Arc::new(
        UpdateService::for_current_installation(),
    )))
}

struct UpdateNoticeProvider {
    checker: Arc<dyn UpdateChecker>,
}

impl UpdateNoticeProvider {
    fn new(checker: Arc<dyn UpdateChecker>) -> Self {
        Self { checker }
    }
}

#[async_trait]
impl BackgroundNoticeProvider for UpdateNoticeProvider {
    async fn notice(&self) -> Option<PresentationDocument> {
        let report = self.checker.check().await;
        (report.status == UpdateCheckStatus::UpdateAvailable)
            .then(|| update_available_document(&report))
    }
}

fn update_check_document(report: &UpdateCheckReport) -> PresentationDocument {
    match report.status {
        UpdateCheckStatus::UpdateAvailable => update_available_document(report),
        UpdateCheckStatus::UpToDate => PresentationDocument::from_block(PresentationBlock::Card {
            title: "Colossus is up to date".into(),
            tone: PresentationTone::Success,
            body: vec![PresentationBlock::KeyValue(version_details(report))],
        }),
        UpdateCheckStatus::Ahead => PresentationDocument::from_block(PresentationBlock::Card {
            title: "This Colossus build is newer than stable".into(),
            tone: PresentationTone::Neutral,
            body: vec![
                PresentationBlock::KeyValue(version_details(report)),
                PresentationBlock::Text("No downgrade will be offered.".into()),
            ],
        }),
        UpdateCheckStatus::Unavailable => {
            let reason = unavailable_reason(report.unavailable_reason);
            let mut body = vec![PresentationBlock::KeyValue(vec![
                ("Current".into(), report.current_version.clone()),
                ("Channel".into(), report.channel.clone()),
                ("Reason".into(), reason.into()),
            ])];
            if let Some(latest) = report.latest_version.as_ref() {
                body.push(PresentationBlock::Text(format!(
                    "Last known stable version: {latest}."
                )));
            }
            body.push(PresentationBlock::Text(
                "Colossus can continue normally; try the check again when connectivity returns."
                    .into(),
            ));
            PresentationDocument::from_block(PresentationBlock::Card {
                title: "Update status unavailable".into(),
                tone: PresentationTone::Neutral,
                body,
            })
        }
    }
}

fn update_available_document(report: &UpdateCheckReport) -> PresentationDocument {
    let guidance = match report.installer_kind {
        InstallerKind::Direct => {
            "A newer stable release is available. Run `colossus update` when ready."
        }
        InstallerKind::Homebrew => {
            "A newer stable release is available. Run `brew upgrade obscuritylabs/tap/colossus`."
        }
        InstallerKind::Nix => {
            "A newer stable release is available. Upgrade the Colossus profile or flake input through Nix."
        }
        InstallerKind::Source => {
            "A newer stable release is available. Update through your source-build workflow."
        }
        InstallerKind::Unknown => {
            "A newer stable release is available. Use the installation channel that owns this binary."
        }
    };
    PresentationDocument::from_block(PresentationBlock::Card {
        title: "Colossus update available".into(),
        tone: PresentationTone::Warning,
        body: vec![
            PresentationBlock::KeyValue(version_details(report)),
            PresentationBlock::Text(guidance.into()),
        ],
    })
}

fn version_details(report: &UpdateCheckReport) -> Vec<(String, String)> {
    vec![
        ("Current".into(), report.current_version.clone()),
        (
            "Latest stable".into(),
            report
                .latest_version
                .as_deref()
                .unwrap_or("unavailable")
                .into(),
        ),
        ("Channel".into(), report.channel.clone()),
    ]
}

fn unavailable_reason(reason: Option<UpdateUnavailableReason>) -> &'static str {
    match reason {
        Some(UpdateUnavailableReason::Offline) => "offline or timed out",
        Some(UpdateUnavailableReason::RateLimited) => "release service rate limited the check",
        Some(UpdateUnavailableReason::ServiceUnavailable) => {
            "release service temporarily unavailable"
        }
        Some(UpdateUnavailableReason::InvalidMetadata) => "release metadata was not trusted",
        Some(UpdateUnavailableReason::UnsupportedHost) => "this host has no release target",
        None => "unavailable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedChecker(UpdateCheckReport);

    #[async_trait]
    impl UpdateChecker for FixedChecker {
        async fn check(&self) -> UpdateCheckReport {
            self.0.clone()
        }
    }

    fn report(status: UpdateCheckStatus) -> UpdateCheckReport {
        UpdateCheckReport {
            schema_version: 1,
            status,
            current_version: "0.10.4".into(),
            latest_version: (status == UpdateCheckStatus::UpdateAvailable).then(|| "0.10.5".into()),
            channel: "stable".into(),
            target: Some("aarch64-apple-darwin".into()),
            source: colossus_update::UpdateCheckSource::Live,
            checked_at_unix_seconds: Some(2_000_000),
            next_check_after_unix_seconds: Some(2_086_400),
            installer_kind: InstallerKind::Direct,
            release_url: Some(
                "https://github.com/obscuritylabs/Colossus/releases/tag/v0.10.5".into(),
            ),
            unavailable_reason: None,
            retry_after_seconds: None,
            cache_warning: false,
        }
    }

    fn apply_report(status: UpdateApplyStatus) -> UpdateApplyReport {
        UpdateApplyReport {
            schema_version: 1,
            status,
            current_version: "0.10.4".into(),
            selected_version: Some("0.10.5".into()),
            target: Some("aarch64-apple-darwin".into()),
            installer_kind: InstallerKind::Direct,
            refusal_reason: None,
            failure_reason: None,
        }
    }

    #[tokio::test]
    async fn background_notice_is_silent_when_update_discovery_is_offline() {
        let mut unavailable = report(UpdateCheckStatus::Unavailable);
        unavailable.unavailable_reason = Some(UpdateUnavailableReason::Offline);
        let provider = UpdateNoticeProvider::new(Arc::new(FixedChecker(unavailable)));
        assert!(provider.notice().await.is_none());
    }

    #[tokio::test]
    async fn background_notice_contains_versions_without_workspace_state() {
        let provider = UpdateNoticeProvider::new(Arc::new(FixedChecker(report(
            UpdateCheckStatus::UpdateAvailable,
        ))));
        let notice = provider.notice().await.expect("update notice");
        let PresentationBlock::Card { title, body, .. } = &notice.blocks[0] else {
            panic!("expected update notice card");
        };
        assert_eq!(title, "Colossus update available");
        let rendered = format!("{body:?}");
        assert!(rendered.contains("0.10.4"));
        assert!(rendered.contains("0.10.5"));
        assert!(!rendered.contains("workspace"));
        assert!(!rendered.contains("session"));
    }

    #[test]
    fn apply_documents_distinguish_success_refusal_and_failure() {
        let updated = update_apply_document(&apply_report(UpdateApplyStatus::Updated));
        assert!(format!("{updated:?}").contains("Colossus updated"));

        let mut refused = apply_report(UpdateApplyStatus::Refused);
        refused.installer_kind = InstallerKind::Homebrew;
        refused.refusal_reason = Some(UpdateRefusalReason::NotDirectInstallation);
        let rendered = format!("{:?}", update_apply_document(&refused));
        assert!(rendered.contains("brew upgrade obscuritylabs/tap/colossus"));

        let mut unavailable = apply_report(UpdateApplyStatus::Unavailable);
        unavailable.failure_reason = Some(UpdateApplyFailure::InstallFailed);
        let rendered = format!("{:?}", update_apply_document(&unavailable));
        assert!(rendered.contains("prior executable was preserved"));
    }
}
