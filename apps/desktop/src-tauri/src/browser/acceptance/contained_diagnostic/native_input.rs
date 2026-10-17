//! Operator keyboard/Unicode commit checkpoint through the actual native `NSView`.
use super::*;

pub(super) fn enabled() -> anyhow::Result<bool> {
    parse_flag(std::env::var_os("COLOSSUS_BROWSER_ACCEPTANCE_NATIVE_INPUT").as_deref())
}
fn parse_flag(value: Option<&std::ffi::OsStr>) -> anyhow::Result<bool> {
    match value.and_then(std::ffi::OsStr::to_str) {
        None if value.is_none() => Ok(false),
        Some("1") => Ok(true),
        _ => anyhow::bail!("native input checkpoint requires the exact opt-in value 1"),
    }
}

pub(super) async fn checkpoint(
    app: &tauri::AppHandle,
    id: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
    origin: &str,
) -> anyhow::Result<()> {
    println!(
        "HUMAN_INPUT_CHECKPOINT click Chromium input, type Mac input café😀x, then press Backspace; timeout_ms=90000"
    );
    std::io::stdout().flush()?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    let initial = view.acceptance_probe().await?.page.target.tab_id;
    loop {
        let probe = view.acceptance_probe().await?;
        ensure!(
            view.human_control_available()
                && probe.page.target.tab_id == initial
                && probe.page.url == format!("{origin}/first"),
            "native human input checkpoint lost its exact owned human page"
        );
        if let Some(frame) = probe.frame {
            ensure!(
                frame.lease.control_generation == 0,
                "native human input checkpoint acquired an agent writer"
            );
        }
        if probe.page.title == "Human input complete" {
            let accepted = accepted_frame(
                app,
                id,
                view,
                rect,
                ExpectedFrame {
                    title: "Human input complete",
                    url: &format!("{origin}/first"),
                    center_bgra: [255, 241, 231, 255],
                    core_target: None,
                    previous: None,
                    after: None,
                },
            )
            .await?;
            ensure!(
                accepted
                    .frame
                    .is_some_and(|frame| frame.lease.control_generation == 0),
                "native human input lacked a fresh authenticated human-frame acknowledgement"
            );
            println!(
                "PASS native Mac keyboard Backspace and Unicode commit reached Chromium; OS IME preedit remains untested"
            );
            return Ok(());
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "operator native keyboard/Unicode input was not acknowledged within its deadline"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_input_checkpoint_has_only_one_explicit_opt_in() {
        assert!(!parse_flag(None).unwrap());
        assert!(parse_flag(Some(std::ffi::OsStr::new("1"))).unwrap());
        for value in ["", "true", "0", "2", "inject"] {
            assert!(parse_flag(Some(std::ffi::OsStr::new(value))).is_err());
        }
    }
}
