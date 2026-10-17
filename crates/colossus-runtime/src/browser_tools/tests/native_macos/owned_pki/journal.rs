//! Supplemental journal evidence requires the independently completed TLS owner.
use colossus_sandbox::MacosBrowserCustodyRetirementReceipt;
use serde_json::Value;
use std::{
    fs::OpenOptions,
    io::Read as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::Path,
};

pub(super) fn completed(custody: &Path, receipt: MacosBrowserCustodyRetirementReceipt) -> bool {
    let observed = (|| {
        let path = custody.join("colossus-custody-generation.jsonl");
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(0x0100 | 0x01000000)
            .open(&path)
            .ok()?;
        let before = file.metadata().ok()?;
        if !before.is_file()
            || before.mode() & 0o7777 != 0o600
            || before.nlink() != 1
            || before.len() > 32 * 1024
            || before.uid() != std::fs::metadata(custody).ok()?.uid()
        {
            return None;
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(32 * 1024 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        let after = file.metadata().ok()?;
        let entry = std::fs::symlink_metadata(path).ok()?;
        let id = |value: &std::fs::Metadata| {
            (
                value.dev(),
                value.ino(),
                value.len(),
                value.mode(),
                value.uid(),
                value.nlink(),
                value.mtime(),
                value.mtime_nsec(),
                value.ctime(),
                value.ctime_nsec(),
            )
        };
        if entry.file_type().is_symlink()
            || id(&before) != id(&after)
            || id(&before) != id(&entry)
            || bytes.len() as u64 != before.len()
        {
            return None;
        }
        validate(&bytes, &Expected::from(receipt)).then_some(())
    })();
    observed.is_some()
}

struct Expected {
    generation: [u8; 32],
    code: [u8; 20],
    pid: u32,
    broker: [u8; 32],
    parent: [u8; 32],
}
impl From<MacosBrowserCustodyRetirementReceipt> for Expected {
    fn from(receipt: MacosBrowserCustodyRetirementReceipt) -> Self {
        Self {
            generation: receipt.generation_sha256(),
            code: receipt.broker_cdhash(),
            pid: receipt.broker_pid(),
            broker: receipt.broker_generation_binding_sha256(),
            parent: receipt.parent_generation_binding_sha256(),
        }
    }
}
fn validate(bytes: &[u8], receipt: &Expected) -> bool {
    if bytes.is_empty() || bytes.len() > 32 * 1024 || bytes.last() != Some(&b'\n') {
        return false;
    }
    let rows = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(serde_json::from_slice::<Value>)
        .collect::<Result<Vec<_>, _>>();
    let Ok(rows) = rows else {
        return false;
    };
    let (Some(header), Some(last)) = (rows.first(), rows.last()) else {
        return false;
    };
    rows.len() >= 2
        && header["schema_version"] == 1
        && header["purpose"] == "colossus-macos-owned-custody-generation"
        && header["generation_sha256"] == hex::encode(receipt.generation)
        && header["broker_cdhash"] == hex::encode(receipt.code)
        && header["broker_pid"].as_u64() == Some(u64::from(receipt.pid))
        && header["broker_generation_binding_sha256"] == hex::encode(receipt.broker)
        && header["parent_generation_binding_sha256"] == hex::encode(receipt.parent)
        && last.as_object().is_some_and(|fields| fields.len() == 4)
        && last["event"] == "retirement"
        && last["step"] == 3
        && last["sequence"]
            .as_u64()
            .is_some_and(|sequence| sequence > 0)
        && last["suffix"].as_str().is_some_and(|suffix| {
            suffix.len() == 32
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn journal_cannot_substitute_the_owned_actor_or_publish_partial_retirement() {
        // This effect-free projection cannot manufacture the public completed
        // native receipt, whose fields and construction remain private.
        let expected = Expected {
            generation: [1; 32],
            code: [2; 20],
            pid: 3,
            broker: [4; 32],
            parent: [5; 32],
        };
        let header = serde_json::json!({"schema_version":1,"purpose":"colossus-macos-owned-custody-generation",
            "generation_sha256":hex::encode(expected.generation),"broker_cdhash":hex::encode(expected.code),
            "broker_pid":expected.pid,"broker_generation_binding_sha256":hex::encode(expected.broker),
            "parent_generation_binding_sha256":hex::encode(expected.parent)});
        let last =
            serde_json::json!({"event":"retirement","step":3,"sequence":4,"suffix":"a".repeat(32)});
        let encode = |h: &Value, l: &Value| format!("{h}\n{l}\n").into_bytes();
        assert!(validate(&encode(&header, &last), &expected));
        for field in [
            "generation_sha256",
            "broker_cdhash",
            "broker_generation_binding_sha256",
            "parent_generation_binding_sha256",
        ] {
            let mut altered = header.clone();
            altered[field] = serde_json::json!("foreign");
            assert!(!validate(&encode(&altered, &last), &expected));
        }
        let mut altered = header.clone();
        altered["broker_pid"] = serde_json::json!(999);
        assert!(!validate(&encode(&altered, &last), &expected));
        for step in 0..3 {
            let mut partial = last.clone();
            partial["step"] = serde_json::json!(step);
            assert!(!validate(&encode(&header, &partial), &expected));
        }
        let bytes = encode(&header, &last);
        assert!(!validate(&bytes[..bytes.len() - 1], &expected));
        assert!(!validate(format!("{header}\n").as_bytes(), &expected));
    }
}
