use super::*;
use std::os::unix::fs::symlink;

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn directory() -> Directory {
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).unwrap();
    let path =
        std::env::temp_dir().join(format!("colossus-pki-input-test-{}", fingerprint(&nonce)));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    Directory(path)
}

#[test]
fn native_inputs_reject_indirection_shared_files_and_excessive_permissions() {
    let directory = directory();
    let source = directory.0.join("secret");
    write_private(&source, b"synthetic-password").unwrap();
    assert_eq!(&*private_input(&source, 64).unwrap(), b"synthetic-password");
    let link = directory.0.join("symlink");
    symlink(&source, &link).unwrap();
    assert!(private_input(&link, 64).is_err());
    let hardlink = directory.0.join("hardlink");
    std::fs::hard_link(&source, &hardlink).unwrap();
    assert!(private_input(&source, 64).is_err());
    std::fs::remove_file(hardlink).unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(private_input(&source, 64).is_err());
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(private_input(&source, 4).is_err());
    std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(private_input(&source, 64).is_err());
}

#[test]
fn pinned_utility_password_format_rejects_truncation_and_encoding_ambiguity() {
    assert!(validate_password(b"synthetic password!#$%").is_ok());
    assert!(validate_password(&[b'a'; 128]).is_ok());
    for password in [
        b"".as_slice(),
        b"password\n",
        b"password\r",
        b"password\0",
        "passwordé".as_bytes(),
        &[b'a'; 129],
    ] {
        assert_eq!(validate_password(password), Err(BrowserDriverError::Denied));
    }
}

#[test]
fn source_retirement_requires_missing_inputs_and_rejects_false_or_forged_ack_state() {
    let directory = directory();
    let pfx = directory.0.join("identity.pfx");
    let password = directory.0.join("password");
    write_private(&pfx, b"synthetic-pfx").unwrap();
    write_private(&password, b"synthetic-password").unwrap();
    let mut configuration = Configuration {
        require_source_retirement: true,
        certutil: Tool {
            path: PathBuf::new(),
            sha256: String::new(),
        },
        pk12util: Tool {
            path: PathBuf::new(),
            sha256: String::new(),
        },
        ca_files: Vec::new(),
        identities: vec![Identity {
            pfx_file: pfx.clone(),
            passphrase_file: password.clone(),
        }],
        bindings: Vec::new(),
    };
    assert_eq!(
        configuration.confirm_source_retirement(),
        Err(BrowserDriverError::Denied)
    );
    std::fs::remove_file(pfx).unwrap();
    assert_eq!(
        configuration.confirm_source_retirement(),
        Err(BrowserDriverError::Denied)
    );
    std::fs::remove_file(password).unwrap();
    configuration.confirm_source_retirement().unwrap();
    configuration.require_source_retirement = false;
    assert_eq!(
        configuration.confirm_source_retirement(),
        Err(BrowserDriverError::Denied)
    );
}

#[test]
fn owned_home_finish_and_drop_remove_only_the_created_tree() {
    for explicit in [true, false] {
        let parent = directory();
        let external = parent.0.join("external-synthetic-file");
        write_private(&external, b"preserve external target").unwrap();
        let home = Home::create(&parent.0).unwrap();
        let path = home.path.clone();
        write_private(&path.join("synthetic-key-db"), b"synthetic").unwrap();
        symlink(&external, path.join("external-symlink")).unwrap();
        if explicit {
            home.finish().unwrap();
        } else {
            drop(home);
        }
        assert!(!path.exists());
        assert_eq!(
            std::fs::read(&external).unwrap(),
            b"preserve external target"
        );
        assert_eq!(std::fs::read_dir(&parent.0).unwrap().count(), 1);
    }
}

#[test]
fn owned_home_finish_and_drop_quarantine_unknown_directory_replacements() {
    for explicit in [true, false] {
        let parent = directory();
        let home = Home::create(&parent.0).unwrap();
        write_private(&home.path.join("owned-synthetic"), b"original").unwrap();
        let held = home.directory.as_ref().unwrap().metadata().unwrap();
        let original = parent.0.join("moved-original");
        std::fs::rename(&home.path, &original).unwrap();
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&home.path)
            .unwrap();
        write_private(&home.path.join("unknown-synthetic"), b"replacement").unwrap();
        let replacement = std::fs::metadata(&home.path).unwrap();
        if explicit {
            assert_eq!(home.finish(), Err(BrowserDriverError::OutcomeUnknown));
        } else {
            drop(home);
        }
        let original_after = std::fs::metadata(&original).unwrap();
        assert_eq!(
            (original_after.dev(), original_after.ino()),
            (held.dev(), held.ino())
        );
        assert_eq!(
            std::fs::read(original.join("owned-synthetic")).unwrap(),
            b"original"
        );
        let unknown = std::fs::read_dir(&parent.0)
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .unwrap()
                    .starts_with(".retired-colossus-nss-")
            })
            .expect("unknown replacement remains quarantined");
        let metadata = unknown.metadata().unwrap();
        assert_eq!(
            (metadata.dev(), metadata.ino()),
            (replacement.dev(), replacement.ino())
        );
        assert_eq!(
            std::fs::read(unknown.path().join("unknown-synthetic")).unwrap(),
            b"replacement"
        );
    }
}

#[test]
fn completed_home_cleanup_cannot_remove_a_reused_basename() {
    let parent = directory();
    let mut home = Home::create(&parent.0).unwrap();
    let path = home.path.clone();
    home.cleanup().unwrap();
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    write_private(&path.join("later-synthetic"), b"later").unwrap();
    home.cleanup().unwrap();
    drop(home);
    assert_eq!(
        std::fs::read(path.join("later-synthetic")).unwrap(),
        b"later"
    );
}

#[test]
fn owned_home_cleanup_uses_the_retained_parent_after_path_replacement() {
    let allocation = directory();
    let parent = allocation.0.join("parent");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&parent)
        .unwrap();
    let home = Home::create(&parent).unwrap();
    let basename = home.path.file_name().unwrap().to_owned();
    let moved = allocation.0.join("moved-parent");
    std::fs::rename(&parent, &moved).unwrap();
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&parent)
        .unwrap();
    let replacement = parent.join(&basename);
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&replacement)
        .unwrap();
    write_private(&replacement.join("unknown-synthetic"), b"replacement").unwrap();
    home.finish().unwrap();
    assert!(!moved.join(&basename).exists());
    assert_eq!(
        std::fs::read(replacement.join("unknown-synthetic")).unwrap(),
        b"replacement"
    );
}

#[test]
fn partial_owned_home_bind_failure_retires_its_empty_created_directory() {
    const CHILD: &str = "COLOSSUS_TEST_HOME_BIND_FD_EXHAUSTION";
    if std::env::var_os(CHILD).is_none() {
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "provision::tests::partial_owned_home_bind_failure_retires_its_empty_created_directory", "--test-threads=1"])
            .env(CHILD, "1").status().unwrap();
        assert!(status.success());
        return;
    }
    let parent = directory();
    let mut home = Home::allocate(&parent.0).unwrap();
    let path = home.path.clone();
    let mut original = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    assert_eq!(
        // SAFETY: this subprocess has no concurrent tests, and the output
        // pointer names a valid initialized rlimit for the entire call.
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut original) },
        0
    );
    let constrained = libc::rlimit {
        rlim_cur: 0,
        rlim_max: original.rlim_max,
    };
    assert_eq!(
        // SAFETY: this isolated subprocess lowers only its soft limit with a
        // valid rlimit pointer, while retaining its existing owner descriptors.
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &constrained) },
        0
    );
    let binding = home.bind();
    drop(home);
    assert_eq!(
        // SAFETY: restore the captured limit through a valid rlimit pointer;
        // this does not raise the subprocess's unchanged hard limit.
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &original) },
        0
    );
    assert_eq!(binding, Err(BrowserDriverError::OutcomeUnknown));
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(&parent.0).unwrap().count(), 0);
}
