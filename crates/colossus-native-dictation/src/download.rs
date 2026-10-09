use crate::{DictationError, DictationSettings, InstalledModel, ModelId};
use colossus_home::ConfinedRoot;
use std::{
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

static NEXT_DOWNLOAD: AtomicU64 = AtomicU64::new(1);

/// Cancellation for an explicit, model-only network transfer. No audio is uploaded.
#[derive(Clone, Default)]
pub struct DownloadCancellation(Arc<CancellationState>);
#[derive(Default)]
struct CancellationState {
    cancelled: AtomicBool,
    changed: tokio::sync::Notify,
}
impl DownloadCancellation {
    /// Stop the transfer before its verified file is published.
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        self.0.changed.notify_one();
    }
    fn check(&self) -> Result<(), DictationError> {
        if self.0.cancelled.load(Ordering::Acquire) {
            Err(DictationError::Download)
        } else {
            Ok(())
        }
    }
    async fn wait(&self) {
        let changed = self.0.changed.notified();
        if self.0.cancelled.load(Ordering::Acquire) {
            return;
        }
        changed.await;
    }
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Download exactly one reviewed model into the private shared cache.
/// Call on a blocking worker. Redirects, bytes, duration, and provenance are bounded.
/// # Errors
/// Returns a categorical failure on cancellation, connection, integrity, or storage errors.
pub fn download_model(
    settings: &DictationSettings,
    model: ModelId,
    cancellation: &DownloadCancellation,
) -> Result<(), DictationError> {
    cancellation.check()?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| DictationError::Download)?
        .block_on(transfer(settings, model, cancellation))
}

async fn transfer(
    settings: &DictationSettings,
    model: ModelId,
    cancellation: &DownloadCancellation,
) -> Result<(), DictationError> {
    cancellation.check()?;
    let destination = settings.model_path(model)?;
    let directory = destination.parent().ok_or(DictationError::Settings)?;
    let root = ConfinedRoot::bind(directory).map_err(|_| DictationError::Settings)?;
    let name = format!(
        "download-{}-{}.part",
        std::process::id(),
        NEXT_DOWNLOAD.fetch_add(1, Ordering::Relaxed)
    );
    let output = root
        .open_file(Path::new(&name))
        .map_err(|_| DictationError::Settings)?;
    let _temporary = Temporary(output.path().to_owned());
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(15))
        .timeout(Duration::from_mins(10))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 5 && reviewed_url(attempt.url()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|_| DictationError::Download)?;
    let mut response = tokio::select! {
        () = cancellation.wait() => return Err(DictationError::Download),
        response = client.get(model.url()).send() => response.and_then(reqwest::Response::error_for_status).map_err(|_| DictationError::Download)?,
    };
    if !reviewed_url(response.url())
        || response
            .content_length()
            .is_some_and(|length| length != model.bytes())
    {
        return Err(DictationError::ModelIntegrity);
    }
    let mut writer = output.file();
    let mut bytes = 0_u64;
    loop {
        let chunk = tokio::select! {
            () = cancellation.wait() => return Err(DictationError::Download),
            chunk = response.chunk() => chunk.map_err(|_| DictationError::Download)?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        bytes += u64::try_from(chunk.len()).map_err(|_| DictationError::ModelIntegrity)?;
        if bytes > model.bytes() {
            return Err(DictationError::ModelIntegrity);
        }
        writer
            .write_all(&chunk)
            .map_err(|_| DictationError::Settings)?;
    }
    writer.sync_all().map_err(|_| DictationError::Settings)?;
    if bytes != model.bytes() || InstalledModel::open(output.path().to_owned())?.id() != model {
        return Err(DictationError::ModelIntegrity);
    }
    cancellation.check()?;
    output
        .revalidate(&root)
        .map_err(|_| DictationError::Settings)?;
    root.prepare_file(Path::new(model.filename()))
        .map_err(|_| DictationError::Settings)?;
    std::fs::rename(output.path(), destination).map_err(|_| DictationError::Settings)?;
    root.sync_directory().map_err(|_| DictationError::Settings)
}
fn reviewed_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(
                "huggingface.co"
                    | "cdn-lfs.huggingface.co"
                    | "cdn-lfs-us-1.hf.co"
                    | "cas-bridge.xethub.hf.co"
                    | "us.aws.cdn.hf.co"
            )
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancellation_interrupts_a_waiting_transport_and_remains_latched() {
        let cancellation = DownloadCancellation::default();
        let waiting = cancellation.clone();
        let transfer = tokio::spawn(async move {
            tokio::select! {
                () = waiting.wait() => true,
                () = std::future::pending::<()>() => false,
            }
        });
        tokio::task::yield_now().await;
        cancellation.cancel();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), transfer)
                .await
                .unwrap()
                .unwrap()
        );
        tokio::time::timeout(Duration::from_secs(1), cancellation.wait())
            .await
            .unwrap();
    }
    #[test]
    fn redirect_authority_is_fixed_and_https_only() {
        for url in [
            "http://huggingface.co/a",
            "https://huggingface.co.evil.test/a",
            "https://user@huggingface.co/a",
            "https://huggingface.co:8443/a",
            "https://us.aws.cdn.hf.co.evil.test/a",
        ] {
            assert!(!reviewed_url(&reqwest::Url::parse(url).unwrap()));
        }
        assert!(reviewed_url(
            &reqwest::Url::parse(&ModelId::BaseEnglish.url()).unwrap()
        ));
        assert!(reviewed_url(
            &reqwest::Url::parse("https://us.aws.cdn.hf.co/xet-bridge-us/model").unwrap()
        ));
        let cancellation = DownloadCancellation::default();
        cancellation.cancel();
        assert!(cancellation.check().is_err());
    }
}
