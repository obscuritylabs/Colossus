//! Closed synthetic renderer; it cannot load the full Desktop application bundle.
use std::{borrow::Cow, time::Duration};
use tauri::{
    Assets, Manager as _,
    utils::assets::{AssetKey, AssetsIter, CspHash},
};

const HTML: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>Owned browser fixture starting</title><link rel="stylesheet" href="/human-controller.css"><script defer src="/human-controller.js"></script></head><body><h1>Owned Colossus human browser fixture</h1><p>This test controller has no application runtime or workspace initializer.</p><p id="state" role="status">Waiting for the trusted browser bridge.</p></body></html>"#;
const SCRIPT: &str = r"'use strict';
window.__TAURI_INTERNALS__.invoke('browser_context').then(snapshot => {
  if (!Number.isInteger(snapshot.generation) || !Array.isArray(snapshot.tabs)) throw new Error('bridge');
  document.documentElement.dataset.browserBridgeGeneration=String(snapshot.generation);
  document.getElementById('state').textContent='Production browser_context IPC authorized this exact local document.';
  document.title='Owned browser bridge ready';
}).catch(() => {document.title='Owned browser bridge rejected';});
";
const STYLE: &str =
    "body{font-family:system-ui;margin:24px;background:#101214;color:#b9e8c6}h1{font-size:20px}";

pub(super) struct ControllerAssets;
impl Assets<tauri::Wry> for ControllerAssets {
    fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        match key.as_ref() {
            "/" | "/index.html" => Some(Cow::Borrowed(HTML.as_bytes())),
            "/human-controller.js" => Some(Cow::Borrowed(SCRIPT.as_bytes())),
            "/human-controller.css" => Some(Cow::Borrowed(STYLE.as_bytes())),
            _ => None,
        }
    }
    fn iter(&self) -> Box<AssetsIter<'_>> {
        Box::new(
            [
                ("/index.html", HTML.as_bytes()),
                ("/human-controller.js", SCRIPT.as_bytes()),
                ("/human-controller.css", STYLE.as_bytes()),
            ]
            .into_iter()
            .map(|(key, value)| (Cow::Borrowed(key), Cow::Borrowed(value))),
        )
    }
    fn csp_hashes(&self, _: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        // Only same-origin external script/style; no inline CSP relaxation.
        Box::new(std::iter::empty())
    }
}

pub(super) async fn ready(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let main = app
        .get_webview("main")
        .ok_or_else(|| anyhow::anyhow!("fixture controller missing"))?;
    for _ in 0..100 {
        match colossus_native_browser::inspect(&main)
            .await?
            .title
            .as_str()
        {
            "Owned browser bridge ready" => {
                anyhow::ensure!(
                    app.state::<crate::state::AppState>()
                        .browser_selection()
                        .await
                        .is_none(),
                    "unexpected application workspace initialization"
                );
                println!(
                    "PASS synthetic renderer invoked the production trusted browser IPC without application startup"
                );
                return Ok(());
            }
            "Owned browser bridge rejected" => {
                anyhow::bail!("actual trusted controller bridge was rejected")
            }
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    anyhow::bail!("synthetic controller bridge did not initialize")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_assets_cannot_boot_the_desktop_app_or_change_workspace_selection() {
        let assets = ControllerAssets;
        assert!(assets.get(&AssetKey::from("index.html")).is_some());
        for denied in [
            "src/App.tsx",
            "assets/index.js",
            "initialize_desktop",
            "../index.html",
        ] {
            assert!(assets.get(&AssetKey::from(denied)).is_none());
        }
        assert!(SCRIPT.contains("invoke('browser_context')"));
        for denied in ["initialize_desktop", "select_target", "browser_command"] {
            assert!(!SCRIPT.contains(denied));
        }
    }
}
