//! Positive final journal observation, bound to the actual admitted native actor.
use colossus_darwin_process::{DarwinAuditPeer, DarwinProcessIdentity};
use sha2::{Digest as _, Sha256};
use std::{
    fs::OpenOptions,
    io::Read as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    path::Path,
};
pub fn completed(
    root: &Path,
    broker: &DarwinAuditPeer,
    generation: [u8; 32],
    code: [u8; 20],
) -> bool {
    let observed = (|| {
        let parent = DarwinProcessIdentity::bind(std::process::id()).ok()?;
        let generation: [u8; 32] = Sha256::digest(generation).into();
        let expected = Expected {
            generation: hex::encode(generation),
            broker_code: hex::encode(code),
            broker_pid: broker.identity().pid(),
            broker_binding: hex::encode(broker.identity().custody_generation_binding(&generation)),
            parent_binding: hex::encode(parent.custody_generation_binding(&generation)),
        };
        let path = root.join("custody/colossus-custody-generation.jsonl");
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(0x0100 | 0x01000000)
            .open(&path)
            .ok()?;
        let before = file.metadata().ok()?;
        if !before.is_file()
            || before.uid() != broker.identity().effective_uid()
            || before.mode() & 0o7777 != 0o600
            || before.nlink() != 1
            || before.len() > 32 * 1024
        {
            return None;
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take(32 * 1024 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        let after = std::fs::symlink_metadata(path).ok()?;
        let identity = |m: &std::fs::Metadata| {
            (
                m.dev(),
                m.ino(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec(),
            )
        };
        if after.file_type().is_symlink()
            || identity(&before) != identity(&after)
            || bytes.len() as u64 != before.len()
        {
            return None;
        }
        validate(&bytes, &expected).then_some(())
    })();
    observed.is_some()
}
struct Expected {
    generation: String,
    broker_code: String,
    broker_pid: u32,
    broker_binding: String,
    parent_binding: String,
}
fn validate(bytes: &[u8], expected: &Expected) -> bool {
    if bytes.is_empty() || bytes.len() > 32 * 1024 || bytes.last() != Some(&b'\n') {
        return false;
    }
    let parsed = bytes
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(serde_json::from_slice::<serde_json::Value>)
        .collect::<Result<Vec<_>, _>>();
    let Ok(rows) = parsed else {
        return false;
    };
    let (Some(header), Some(last)) = (rows.first(), rows.last()) else {
        return false;
    };
    if rows.len() < 2
        || header["schema_version"] != 1
        || header["purpose"] != "colossus-macos-owned-custody-generation"
        || header["generation_sha256"] != expected.generation
        || header["broker_cdhash"] != expected.broker_code
        || header["broker_pid"].as_u64() != Some(u64::from(expected.broker_pid))
        || header["broker_generation_binding_sha256"] != expected.broker_binding
        || header["parent_generation_binding_sha256"] != expected.parent_binding
    {
        return false;
    }
    // This is supplemental evidence to the exact signed native child's typed
    // successful retirement ACK. The protected writer already validates/replays
    // the complete finite journal before persisting this final positive row.
    last.as_object().is_some_and(|object| object.len() == 4)
        && last["event"] == "retirement"
        && last["step"] == 3
        && last["sequence"]
            .as_u64()
            .is_some_and(|sequence| sequence > 0)
        && last["suffix"].as_str().is_some_and(|suffix| {
            suffix.len() == 32
                && suffix
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_ack_requires_current_generation_actor_and_positive_final_row() {
        let expected = Expected {
            generation: "g".into(),
            broker_code: "c".into(),
            broker_pid: 10,
            broker_binding: "b".into(),
            parent_binding: "p".into(),
        };
        let header = serde_json::json!({"schema_version":1,"purpose":"colossus-macos-owned-custody-generation","generation_sha256":"g","broker_cdhash":"c","broker_pid":10,"broker_generation_binding_sha256":"b","parent_generation_binding_sha256":"p"});
        let last =
            serde_json::json!({"event":"retirement","step":3,"sequence":4,"suffix":"a".repeat(32)});
        let encode =
            |h: &serde_json::Value, l: &serde_json::Value| format!("{h}\n{l}\n").into_bytes();
        assert!(validate(&encode(&header, &last), &expected));
        let mut changed = header.clone();
        changed["generation_sha256"] = serde_json::json!("foreign");
        assert!(!validate(&encode(&changed, &last), &expected));
        let mut incomplete = last.clone();
        incomplete["step"] = serde_json::json!(2);
        assert!(!validate(&encode(&header, &incomplete), &expected));
        assert!(!validate(format!("{header}\n").as_bytes(), &expected));
        let bytes = encode(&header, &last);
        assert!(!validate(&bytes[..bytes.len() - 1], &expected));
    }
}
