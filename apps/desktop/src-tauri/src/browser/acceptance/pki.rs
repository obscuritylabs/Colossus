//! Opt-in real Chromium TLS/key-use evidence using operator-provisioned unique fixtures.
//! This module never installs/deletes OS certificates or reads a private key/password.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use colossus_native_browser::{BrowserView, chromium::Surface};
use serde::Deserialize;
use tauri::Manager as _;

use crate::{browser::dto::BrowserAction, state::AppState};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema_version: u16,
    fixture_id: String,
    fingerprints_sha256: BTreeMap<String, String>,
    ca_file: String,
    identity_file: String,
    alternate_identity_file: String,
    passphrase_file: String,
    changes_os_trust: bool,
    production_acceptance: bool,
    urls: BTreeMap<String, String>,
}

impl Fixture {
    fn read(path: PathBuf) -> anyhow::Result<Self> {
        anyhow::ensure!(path.is_absolute(), "PKI fixture manifest must be absolute");
        let bytes = crate::desktop_settings::read_ca_bundle_source(&path)
            .map_err(|_| anyhow::anyhow!("PKI fixture manifest could not be opened"))?;
        anyhow::ensure!(
            bytes.len() <= 16 * 1024,
            "PKI fixture manifest exceeded bounds"
        );
        let fixture: Self = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            fixture.schema_version == 1
                && uuid::Uuid::parse_str(&fixture.fixture_id).is_ok()
                && fixture.ca_file == "ca.der"
                && fixture.identity_file == "client.pfx"
                && fixture.alternate_identity_file == "alternate_client.pfx"
                && fixture.passphrase_file == "passphrase.txt"
                && !fixture.changes_os_trust
                && !fixture.production_acceptance
                && fixture.urls.len() == 6
                && fixture.fingerprints_sha256.len() == 10,
            "PKI fixture metadata was invalid"
        );
        for fingerprint in fixture.fingerprints_sha256.values() {
            anyhow::ensure!(
                fingerprint.len() == 64
                    && fingerprint
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "PKI fixture fingerprint was invalid"
            );
        }
        for name in [
            "public",
            "mtls",
            "redirect_mtls",
            "untrusted",
            "wrong_hostname",
            "expired",
        ] {
            let origin = fixture.origin(name)?;
            let url = tauri::Url::parse(origin)?;
            anyhow::ensure!(
                url.scheme() == "https"
                    && url.host_str() == Some("127.0.0.1")
                    && url.port().is_some()
                    && url.origin().ascii_serialization() == origin,
                "PKI fixture origin was invalid"
            );
        }
        Ok(fixture)
    }

    fn origin(&self, name: &str) -> anyhow::Result<&str> {
        self.urls
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| anyhow::anyhow!("PKI fixture origin missing"))
    }

    fn fingerprint(&self, name: &str) -> anyhow::Result<&str> {
        self.fingerprints_sha256
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| anyhow::anyhow!("PKI fixture fingerprint missing"))
    }
}

pub(super) async fn exercise_if_configured(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let Some(path) = std::env::var_os("COLOSSUS_BROWSER_PKI_FIXTURE") else {
        return Ok(());
    };
    let fixture = Fixture::read(PathBuf::from(path))?;
    let state = app.state::<AppState>();
    state
        .select_target(Some("chromium-pki-acceptance".into()))
        .await;
    let (_, trusted) = open(app, &format!("{}/public", fixture.origin("public")?)).await?;
    wait_title(&trusted, "Colossus private CA verified").await?;
    println!("PASS native Chromium validates operator-provisioned fixture CA");
    for name in ["untrusted", "wrong_hostname", "expired"] {
        let (id, view) = open(app, &format!("{}/public", fixture.origin(name)?)).await?;
        wait_denied(app, &id, &view).await?;
    }
    println!("PASS native Chromium rejects unknown CA wrong hostname and expired server");

    let origin = fixture.origin("mtls")?;
    let url = format!("{origin}/client-check");
    let (id, missing) = open(app, &url).await?;
    let review = wait_review(&missing, origin).await?;
    // Unknown selections must fail before consuming the exact native handshake.
    anyhow::ensure!(
        missing
            .select_client_identity(&review, Some(&"0".repeat(64)))
            .await
            .is_err(),
        "unknown certificate fingerprint was accepted"
    );
    missing.select_client_identity(&review, None).await?;
    wait_denied(app, &id, &missing).await?;
    println!("PASS native Chromium rejects unselected and unknown client identity");

    let (_, alternate) = open(app, &url).await?;
    let review = wait_review(&alternate, origin).await?;
    require_candidates(&review, &fixture)?;
    alternate
        .select_client_identity(&review, Some(fixture.fingerprint("alternate_client")?))
        .await?;
    wait_title(&alternate, "Colossus identity rejected").await?;
    println!(
        "PASS native Chromium can select exact alternate fixture identity for server rejection"
    );

    let (_, accepted) = open(app, &url).await?;
    let review = wait_review(&accepted, origin).await?;
    require_candidates(&review, &fixture)?;
    accepted
        .select_client_identity(&review, Some(fixture.fingerprint("client")?))
        .await?;
    wait_title(&accepted, "Colossus mTLS identity verified").await?;
    anyhow::ensure!(
        accepted
            .select_client_identity(&review, Some(fixture.fingerprint("client")?))
            .await
            .is_err(),
        "consumed native identity request was reused"
    );
    println!("PASS native Chromium uses exact reviewed client key and rejects selection replay");

    let (id, redirected) = open(
        app,
        &format!("{}/redirect-denied", fixture.origin("public")?),
    )
    .await?;
    let review = wait_review(&redirected, fixture.origin("redirect_mtls")?).await?;
    anyhow::ensure!(
        review.origin != origin,
        "redirect retained old identity origin"
    );
    // A separately reviewed origin is required. The accepted origin's identity
    // must not silently follow this cross-origin redirect.
    redirected.select_client_identity(&review, None).await?;
    wait_denied(app, &id, &redirected).await?;
    println!("PASS native Chromium redirect requires independent exact-origin identity review");
    super::action(app, BrowserAction::Clear).await?;
    state
        .select_target(Some("chromium-acceptance-b".into()))
        .await;
    println!(
        "PKI_NATIVE_TLS_CONFORMANCE_PASSED fixture={}",
        fixture.fixture_id
    );
    println!(
        "PKI_OS_STORE_CUSTODY_PENDING operator must remove only generated fixture certificates and private keys"
    );
    Ok(())
}

fn require_candidates(
    review: &colossus_native_browser::pki::IdentityRequest,
    fixture: &Fixture,
) -> anyhow::Result<()> {
    for name in ["client", "alternate_client"] {
        let fingerprint = fixture.fingerprint(name)?;
        anyhow::ensure!(
            review
                .candidates
                .iter()
                .any(|candidate| candidate.fingerprint_sha256 == fingerprint),
            "both unique fixture identities must be provisioned in the native store"
        );
    }
    Ok(())
}

async fn open(app: &tauri::AppHandle, url: &str) -> anyhow::Result<(String, Surface)> {
    super::action(app, BrowserAction::Clear).await?;
    let snapshot = super::action(
        app,
        BrowserAction::New {
            conversation_id: None,
            url: url.into(),
        },
    )
    .await?;
    let id = snapshot
        .selected_tab_id
        .ok_or_else(|| anyhow::anyhow!("PKI fixture tab missing"))?;
    let (view, _) = app
        .state::<AppState>()
        .browser
        .tab(&id, "chromium-pki-acceptance")
        .map_err(|error| anyhow::anyhow!(error.message))?;
    match view {
        BrowserView::Chromium(view) => Ok((id, view)),
        _ => anyhow::bail!("PKI child-view acceptance requires the in-process native fixture"),
    }
}

async fn wait_title(view: &Surface, expected: &str) -> anyhow::Result<()> {
    for _ in 0..160 {
        let page = view.inspect().await?;
        if page.title == expected && !page.loading {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!(
        "native PKI fixture page did not complete; verify unique native CA/key provisioning"
    )
}

async fn wait_review(
    view: &Surface,
    origin: &str,
) -> anyhow::Result<colossus_native_browser::pki::IdentityRequest> {
    for _ in 0..160 {
        if let Some(review) = view.pending_client_identity()? {
            anyhow::ensure!(
                review.origin == origin && !review.is_expired(),
                "native identity request origin changed"
            );
            return Ok(review);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!(
        "native client identities unavailable; provision only the generated fixture PFX files"
    )
}

async fn wait_denied(app: &tauri::AppHandle, id: &str, view: &Surface) -> anyhow::Result<()> {
    for _ in 0..160 {
        let page = view.inspect().await?;
        anyhow::ensure!(
            page.title != "Colossus private CA verified"
                && page.title != "Colossus mTLS identity verified",
            "negative native TLS fixture unexpectedly succeeded"
        );
        let snapshot = app
            .state::<AppState>()
            .browser
            .snapshot()
            .await
            .map_err(|error| anyhow::anyhow!(error.message))?;
        if snapshot
            .tabs
            .iter()
            .any(|tab| tab.id == id && (tab.error.is_some() || tab.notice.is_some()))
            && view.pending_client_identity()?.is_none()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("native browser did not acknowledge negative TLS fixture denial")
}
