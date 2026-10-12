use super::{Failure, cdhash};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    os::unix::fs::{MetadataExt as _, PermissionsExt as _},
    path::PathBuf,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Positive,
    Eof,
    WrongParent,
    ParentCrash,
    CrashChild,
}

pub struct Arguments {
    pub mode: Mode,
    pub broker: PathBuf,
    pub broker_cdhash: [u8; 20],
    pub parent_cdhash: [u8; 20],
    pub evidence: PathBuf,
}

impl Arguments {
    pub fn parse(values: Vec<OsString>) -> Result<Self, Failure> {
        if values.len() != 10 {
            return Err("invalid_native_fixture_arguments");
        }
        let mut fields = BTreeMap::new();
        for pair in values.chunks_exact(2) {
            let name = pair[0].to_str().ok_or("invalid_native_fixture_arguments")?;
            if ![
                "--mode",
                "--broker",
                "--broker-cdhash",
                "--parent-cdhash",
                "--evidence",
            ]
            .contains(&name)
                || fields.insert(name.to_owned(), pair[1].clone()).is_some()
            {
                return Err("invalid_native_fixture_arguments");
            }
        }
        let field = |name| fields.get(name).ok_or("invalid_native_fixture_arguments");
        let mode = match field("--mode")?
            .to_str()
            .ok_or("invalid_native_fixture_arguments")?
        {
            "positive" => Mode::Positive,
            "eof" => Mode::Eof,
            "wrong-parent" => Mode::WrongParent,
            "parent-crash" => Mode::ParentCrash,
            "crash-child" => Mode::CrashChild,
            _ => return Err("invalid_native_fixture_arguments"),
        };
        let broker = PathBuf::from(field("--broker")?);
        let evidence = PathBuf::from(field("--evidence")?);
        let broker_cdhash = cdhash::parse(
            field("--broker-cdhash")?
                .to_str()
                .ok_or("invalid_native_fixture_arguments")?,
        )
        .map_err(|_| "invalid_native_fixture_arguments")?;
        let parent_cdhash = cdhash::parse(
            field("--parent-cdhash")?
                .to_str()
                .ok_or("invalid_native_fixture_arguments")?,
        )
        .map_err(|_| "invalid_native_fixture_arguments")?;
        let identity = colossus_darwin_process::DarwinProcessIdentity::bind(std::process::id())
            .map_err(|_| "fixture_owner_unavailable")?;
        if identity.real_uid() == 0 || identity.real_uid() != identity.effective_uid() {
            return Err("fixture_owner_rejected");
        }
        validate_path(&broker, false, identity.real_uid())?;
        validate_path(&evidence, true, identity.real_uid())?;
        Ok(Self {
            mode,
            broker,
            broker_cdhash,
            parent_cdhash,
            evidence,
        })
    }
}

fn validate_path(path: &PathBuf, directory: bool, owner: u32) -> Result<(), Failure> {
    if !path.is_absolute()
        || path
            .canonicalize()
            .map_err(|_| "invalid_native_fixture_path")?
            != *path
    {
        return Err("invalid_native_fixture_path");
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| "invalid_native_fixture_path")?;
    // The preparer supplies owned private paths; a direct operator must provide
    // the same shape. Actual process code is independently checked after spawn.
    if (directory && (!metadata.is_dir() || metadata.permissions().mode() & 0o7777 != 0o700))
        || (!directory
            && (!metadata.is_file()
                || metadata.nlink() != 1
                || metadata.permissions().mode() & 0o7777 != 0o500))
        || metadata.uid() != owner
    {
        return Err("invalid_native_fixture_path");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_or_duplicate_operator_fields_are_rejected_before_any_spawn() {
        let unknown = [
            "--mode",
            "positive",
            "--broker",
            "/none",
            "--broker-cdhash",
            "0101010101010101010101010101010101010101",
            "--parent-cdhash",
            "0202020202020202020202020202020202020202",
            "--unknown",
            "/none",
        ]
        .map(OsString::from)
        .to_vec();
        assert!(Arguments::parse(unknown).is_err());
        let repeated = [
            "--mode",
            "positive",
            "--broker",
            "/none",
            "--broker-cdhash",
            "0101010101010101010101010101010101010101",
            "--parent-cdhash",
            "0202020202020202020202020202020202020202",
            "--mode",
            "eof",
        ]
        .map(OsString::from)
        .to_vec();
        assert!(Arguments::parse(repeated).is_err());
    }
}
