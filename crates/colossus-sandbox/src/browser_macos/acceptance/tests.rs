use super::*;
use std::os::unix::fs::{PermissionsExt as _, symlink};

#[test]
fn tls_bootstrap_requires_exact_signed_numeric_loopback_destination_and_public_roots() {
    let valid = MacosBrowserTlsConfig {
        broker_cdhash: [1; 20],
        origin: colossus_contracts::BrowserOrigin::parse("https://127.0.0.1:60924").unwrap(),
        destinations: vec!["127.0.0.1:60924".parse().unwrap()],
        public_roots: vec![vec![1]],
        pki_registry_sha256: None,
    };
    validate_tls(&valid).unwrap();
    for invalid in [
        MacosBrowserTlsConfig {
            broker_cdhash: [0; 20],
            ..valid.clone()
        },
        MacosBrowserTlsConfig {
            origin: colossus_contracts::BrowserOrigin::parse("http://127.0.0.1:60924").unwrap(),
            ..valid.clone()
        },
        MacosBrowserTlsConfig {
            origin: colossus_contracts::BrowserOrigin::parse("https://localhost:60924").unwrap(),
            ..valid.clone()
        },
        MacosBrowserTlsConfig {
            destinations: vec!["127.0.0.1:60925".parse().unwrap()],
            ..valid.clone()
        },
        MacosBrowserTlsConfig {
            destinations: vec!["192.0.2.1:60924".parse().unwrap()],
            ..valid.clone()
        },
        MacosBrowserTlsConfig {
            public_roots: Vec::new(),
            ..valid.clone()
        },
    ] {
        assert_eq!(validate_tls(&invalid), Err(BrowserDriverError::Denied));
    }
}

fn policy(allocation: &str, app: &str, profile: &str) -> String {
    format!(
        "COLOSSUS_MAC_NETWORK_ENVELOPE_V1\n123\n60924\n{allocation}\n{app}\ncom.obscuritylabs.ColossusBrowser\n{profile}\n/private/tmp/broker\n/Users/test\n"
    )
}

#[test]
fn policy_requires_exact_allocation_app_and_profile() {
    let allocation = std::path::Path::new("/private/tmp/allocation");
    let app = allocation.join("Colossus Browser Host.app");
    let valid = policy(
        allocation.to_str().unwrap(),
        app.to_str().unwrap(),
        allocation.join("profile").to_str().unwrap(),
    );
    assert_eq!(
        parse_policy(&valid, allocation, &app),
        Ok(PolicyBinding {
            audit_session: 123,
            proxy_port: 60924,
            custody_root: "/private/tmp/broker".into()
        })
    );
    for invalid in [
        valid.replacen("/private/tmp/allocation\n", "/private/tmp/other\n", 1),
        valid.replacen("Colossus Browser Host.app", "Other.app", 1),
        valid.replacen("/profile\n", "/other-profile\n", 1),
        format!("{valid}extra\n"),
        valid.replace("/private/tmp/broker\n", "/private/tmp/allocation/profile\n"),
        valid.replace("/private/tmp/broker\n", "/private/tmp/../broker\n"),
        valid.replace("/private/tmp/broker\n", "/private/tmp\n"),
    ] {
        assert_eq!(
            parse_policy(&invalid, allocation, &app),
            Err(BrowserDriverError::Denied)
        );
    }
}

#[test]
fn policy_rejects_placeholder_asid_and_port() {
    let allocation = std::path::Path::new("/private/tmp/allocation");
    let app = allocation.join("Colossus Browser Host.app");
    let valid = policy(
        allocation.to_str().unwrap(),
        app.to_str().unwrap(),
        allocation.join("profile").to_str().unwrap(),
    );
    assert!(
        parse_policy(
            &valid.replacen("123\n60924", "0\n60924", 1),
            allocation,
            &app
        )
        .is_err()
    );
    assert!(parse_policy(&valid.replacen("123\n60924", "123\n0", 1), allocation, &app).is_err());
}

fn fixture_entries(root: &std::path::Path) -> Vec<Entry> {
    std::fs::create_dir(root.join("bin")).unwrap();
    std::fs::set_permissions(root.join("bin"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(root.join("bin/host"), b"trusted-host").unwrap();
    std::fs::set_permissions(
        root.join("bin/host"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    symlink("bin/host", root.join("host-link")).unwrap();
    vec![
        Entry {
            path: "bin".into(),
            kind: "directory".into(),
            mode: Some(0o755),
            size: None,
            sha256: None,
            target: None,
        },
        Entry {
            path: "bin/host".into(),
            kind: "file".into(),
            mode: Some(0o755),
            size: Some(12),
            sha256: Some(hex::encode(Sha256::digest(b"trusted-host"))),
            target: None,
        },
        Entry {
            path: "host-link".into(),
            kind: "symlink".into(),
            mode: None,
            size: None,
            sha256: Some(hex::encode(Sha256::digest(b"bin/host"))),
            target: Some("bin/host".into()),
        },
    ]
}

#[test]
fn inventory_rejects_tampering_duplicates_and_unlisted_files() {
    let directory = tempfile::tempdir().unwrap();
    let entries = fixture_entries(directory.path());
    assert_eq!(verify_entries(directory.path(), &entries), Ok(()));

    std::fs::write(directory.path().join("bin/host"), b"changed-host").unwrap();
    assert_eq!(
        verify_entries(directory.path(), &entries),
        Err(BrowserDriverError::Denied)
    );
    std::fs::write(directory.path().join("bin/host"), b"trusted-host").unwrap();

    let mut duplicate = entries.clone();
    duplicate.push(entries[0].clone());
    assert_eq!(
        verify_entries(directory.path(), &duplicate),
        Err(BrowserDriverError::Denied)
    );

    std::fs::write(directory.path().join("unlisted"), b"extra").unwrap();
    assert_eq!(
        verify_entries(directory.path(), &entries),
        Err(BrowserDriverError::Denied)
    );
}

#[test]
fn inventory_census_does_not_honor_ignore_rules_or_git_directory_exclusions() {
    let directory = tempfile::tempdir().unwrap();
    let mut entries = fixture_entries(directory.path());
    let ignore_text = b"unsigned-payload\n.DS_Store\n";
    std::fs::write(directory.path().join(".ignore"), ignore_text).unwrap();
    std::fs::set_permissions(
        directory.path().join(".ignore"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    entries.push(Entry {
        path: ".ignore".into(),
        kind: "file".into(),
        mode: Some(0o644),
        size: Some(ignore_text.len() as u64),
        sha256: Some(hex::encode(Sha256::digest(ignore_text))),
        target: None,
    });
    assert_eq!(verify_entries(directory.path(), &entries), Ok(()));
    for name in ["unsigned-payload", ".DS_Store"] {
        let extra = directory.path().join(name);
        std::fs::write(&extra, b"unlisted native content").unwrap();
        assert_eq!(
            verify_entries(directory.path(), &entries),
            Err(BrowserDriverError::Denied)
        );
        std::fs::remove_file(extra).unwrap();
    }
    std::fs::create_dir(directory.path().join(".git")).unwrap();
    std::fs::write(directory.path().join(".git/config"), b"unlisted config").unwrap();
    assert_eq!(
        verify_entries(directory.path(), &entries),
        Err(BrowserDriverError::Denied)
    );
}
