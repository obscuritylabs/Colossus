//! Per-user launchd keeper description and exact live-process admission.
//!
//! The keeper is deliberately independent of the browser producer's fresh audit
//! session. It may retain authenticated observation and continue cleanup after the
//! producer's parent crashes. launchd only kills members that remain in the job's
//! process group, so this module does not claim whole-browser cleanup or expose an
//! availability receipt.

#[cfg(test)]
mod tests;

use std::{
    fmt::Write as _,
    io,
    path::{Path, PathBuf},
};

use crate::{DarwinAuditPeer, DarwinProcessIdentity};

const MAX_NAME_BYTES: usize = 255;
const MAX_PATH_BYTES: usize = 4096;

/// Fixed per-allocation launchd keeper job supplied by trusted native composition.
///
/// The generated job creates a fresh audit session, registers one fixed Mach
/// service before process start, disables restart, and retains launchd's ordinary
/// same-process-group cleanup. It never embeds URLs, credentials, profile paths or
/// model-controlled data. The caller must atomically create and privately own the
/// plist before bootstrapping it into its own GUI domain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DarwinKeeperJob {
    label: String,
    service: String,
    executable: PathBuf,
    standard_out: PathBuf,
    standard_error: PathBuf,
    allocation_nonce: [u8; 16],
    policy_sha256: [u8; 32],
}

impl DarwinKeeperJob {
    /// Validate one immutable per-user keeper definition.
    pub fn new(
        label: String,
        service: String,
        executable: PathBuf,
        standard_out: PathBuf,
        standard_error: PathBuf,
        allocation_nonce: [u8; 16],
        policy_sha256: [u8; 32],
    ) -> io::Result<Self> {
        require_name("keeper label", &label)?;
        require_name("keeper Mach service", &service)?;
        if label == service {
            return Err(invalid("keeper label and Mach service must be distinct"));
        }
        for (kind, path) in [
            ("keeper executable", &executable),
            ("keeper stdout", &standard_out),
            ("keeper stderr", &standard_error),
        ] {
            require_absolute(kind, path)?;
        }
        if standard_out == standard_error
            || standard_out == executable
            || standard_error == executable
        {
            return Err(invalid(
                "keeper executable and output paths must be distinct",
            ));
        }
        if allocation_nonce == [0; 16] || policy_sha256 == [0; 32] {
            return Err(invalid(
                "keeper allocation nonce and policy digest must be nonzero",
            ));
        }
        Ok(Self {
            label,
            service,
            executable,
            standard_out,
            standard_error,
            allocation_nonce,
            policy_sha256,
        })
    }

    /// Fixed launchd label, used only in the caller's own GUI domain.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Fixed bootstrap service used by the authenticated audit-trailer protocol.
    pub fn service(&self) -> &str {
        &self.service
    }

    /// Trusted policy digest the authenticated keeper must echo before admission.
    pub fn policy_sha256(&self) -> [u8; 32] {
        self.policy_sha256
    }

    /// One-use allocation nonce bound into the authenticated keeper exchange.
    pub fn allocation_nonce(&self) -> [u8; 16] {
        self.allocation_nonce
    }

    /// Render the bounded launchd property list for private atomic installation.
    pub fn property_list(&self) -> io::Result<String> {
        let executable = utf8_path("keeper executable", &self.executable)?;
        let stdout = utf8_path("keeper stdout", &self.standard_out)?;
        let stderr = utf8_path("keeper stderr", &self.standard_error)?;
        let nonce = encode_hex(&self.allocation_nonce);
        let policy = encode_hex(&self.policy_sha256);
        let arguments = [
            executable,
            "--mach-service",
            &self.service,
            "--allocation-nonce",
            &nonce,
            "--policy-sha256",
            &policy,
        ]
        .into_iter()
        .map(|value| format!("<string>{}</string>", xml(value)))
        .collect::<String>();
        Ok(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\"><dict>\
<key>Label</key><string>{}</string>\
<key>ProgramArguments</key><array>{arguments}</array>\
<key>RunAtLoad</key><true/>\
<key>KeepAlive</key><false/>\
<key>SessionCreate</key><true/>\
<key>AbandonProcessGroup</key><false/>\
<key>Umask</key><string>0077</string>\
<key>StandardOutPath</key><string>{}</string>\
<key>StandardErrorPath</key><string>{}</string>\
<key>MachServices</key><dict><key>{}</key><true/></dict>\
</dict></plist>\n",
            xml(&self.label),
            xml(stdout),
            xml(stderr),
            xml(&self.service),
        ))
    }
}

/// Exact signed live keeper admitted against its immutable launch description.
///
/// Code identity and the genuine audit token are revalidated by [`DarwinAuditPeer`].
/// The policy digest and nonce remain expected challenge values; callers must compare
/// them only inside the authenticated Mach exchange. This object does not prove that
/// the browser's sandbox policy was applied and does not own browser descendants.
#[derive(Clone)]
pub struct DarwinKeeperAdmission {
    peer: DarwinAuditPeer,
    source_audit_session: u32,
    allocation_nonce: [u8; 16],
    policy_sha256: [u8; 32],
}

impl DarwinKeeperAdmission {
    /// Bind one signed launchd keeper that is outside the producer's audit session.
    pub fn enroll(
        pid: u32,
        source: &DarwinProcessIdentity,
        expected_cdhash: [u8; 20],
        job: &DarwinKeeperJob,
    ) -> io::Result<Self> {
        let identity = DarwinProcessIdentity::bind(pid)?;
        if identity.real_uid() != source.real_uid()
            || identity.effective_uid() != source.effective_uid()
        {
            return Err(invalid("keeper and source user identities differ"));
        }
        if identity.audit_session_id() == source.audit_session_id() {
            return Err(invalid(
                "keeper must remain outside the source audit session",
            ));
        }
        let peer = DarwinAuditPeer::enroll(&identity, expected_cdhash)?;
        Ok(Self {
            peer,
            source_audit_session: source.audit_session_id(),
            allocation_nonce: job.allocation_nonce,
            policy_sha256: job.policy_sha256,
        })
    }

    /// Revalidate the exact live process incarnation and signed code.
    pub fn verify(&self) -> io::Result<()> {
        self.peer.verify()?;
        if self.peer.identity().audit_session_id() == self.source_audit_session {
            return Err(invalid("keeper entered the source audit session"));
        }
        Ok(())
    }

    /// Exact peer for the existing authenticated Mach audit-trailer handoff.
    pub fn peer(&self) -> &DarwinAuditPeer {
        &self.peer
    }

    /// Compare values received through that authenticated exchange.
    pub fn accepts_policy(&self, nonce: [u8; 16], policy_sha256: [u8; 32]) -> bool {
        matches_policy(
            self.allocation_nonce,
            self.policy_sha256,
            nonce,
            policy_sha256,
        )
    }
}

fn matches_policy(
    expected_nonce: [u8; 16],
    expected_policy: [u8; 32],
    received_nonce: [u8; 16],
    received_policy: [u8; 32],
) -> bool {
    received_nonce == expected_nonce && received_policy == expected_policy
}

fn require_name(kind: &str, value: &str) -> io::Result<()> {
    if value.is_empty()
        || value.len() > MAX_NAME_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(invalid(&format!("{kind} is not a bounded launchd name")));
    }
    Ok(())
}

fn require_absolute(kind: &str, path: &Path) -> io::Result<()> {
    let value = utf8_path(kind, path)?;
    if !value.starts_with('/')
        || value.len() > MAX_PATH_BYTES
        || value == "/"
        || value.split('/').skip(1).any(|part| {
            part.is_empty() || part == "." || part == ".." || part.chars().any(char::is_control)
        })
    {
        return Err(invalid(&format!(
            "{kind} must be a bounded absolute normalized path"
        )));
    }
    Ok(())
}

fn utf8_path<'a>(kind: &str, path: &'a Path) -> io::Result<&'a str> {
    path.to_str()
        .ok_or_else(|| invalid(&format!("{kind} must be valid UTF-8")))
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
