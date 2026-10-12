//! Native package composition for the independently deployed creator and keeper.

use super::{Entry, MacosBrowserOwnedProcessConfig, decode_hash};
use colossus_mac_browser_process_broker::{
    CoreProcessOwner, DeploymentImage, DeploymentLaunch, DeploymentSetup,
};
use colossus_ports::BrowserDriverError;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub(super) struct Installation {
    broker: PathBuf,
    cdhash: [u8; 20],
    main_body: [u8; 32],
    helpers: Vec<DeploymentImage>,
}

impl Installation {
    pub(super) fn bind(
        app: &Path,
        config: MacosBrowserOwnedProcessConfig,
        entries: &[Entry],
    ) -> Result<Self, BrowserDriverError> {
        if config.broker_cdhash == [0; 20] || config.helper_cdhashes.contains(&[0; 20]) {
            return Err(BrowserDriverError::Denied);
        }
        let broker = "Contents/MacOS/colossus-mac-browser-process-broker";
        file_digest(entries, broker)?;
        let main_body = file_digest(
            entries,
            "Contents/MacOS/colossus-native-browser-host Body.dylib",
        )?;
        let mut helpers = Vec::new();
        // Network and general utility use the same reviewed generic front.
        // The stopped child's separately sealed role bootstrap selects its
        // immutable native policy before either role loads CEF.
        for (index, suffix) in [" (Renderer)", " (GPU)", "", ""].into_iter().enumerate() {
            let name = format!("Colossus Browser Helper{suffix}");
            let front = format!("Contents/Frameworks/{name}.app/Contents/MacOS/{name}");
            file_digest(entries, &front)?;
            helpers.push(DeploymentImage {
                role: (index + 1) as u8,
                executable: app.join(&front),
                cdhash: config.helper_cdhashes[index],
                body_sha256: file_digest(entries, &format!("{front} Body.dylib"))?,
            });
        }
        Ok(Self {
            broker: app.join(broker),
            cdhash: config.broker_cdhash,
            main_body,
            helpers,
        })
    }

    pub(super) fn owner(
        &self,
        main: &Path,
        front_cdhash: [u8; 20],
        nonce: [u8; 16],
        policy_sha256: [u8; 32],
        environment: &[OsString],
    ) -> Result<CoreProcessOwner, BrowserDriverError> {
        let mut images = self.helpers.clone();
        images.push(DeploymentImage {
            role: 0,
            executable: main.to_path_buf(),
            cdhash: front_cdhash,
            body_sha256: self.main_body,
        });
        let environment = environment
            .iter()
            .map(|entry| {
                entry
                    .to_str()
                    .map(str::to_owned)
                    .ok_or(BrowserDriverError::Denied)
            })
            .collect::<Result<Vec<_>, _>>()?;
        CoreProcessOwner::new(DeploymentLaunch {
            executable: self.broker.clone(),
            cdhash: self.cdhash,
            setup: DeploymentSetup {
                nonce,
                policy_sha256,
                images,
                environment,
            },
        })
        .map_err(|_| BrowserDriverError::Denied)
    }
}

fn file_digest(entries: &[Entry], path: &str) -> Result<[u8; 32], BrowserDriverError> {
    let entry = entries
        .iter()
        .find(|entry| entry.path == path)
        .ok_or(BrowserDriverError::Denied)?;
    if entry.kind != "file" {
        return Err(BrowserDriverError::Denied);
    }
    decode_hash(entry.sha256.as_deref())
}
