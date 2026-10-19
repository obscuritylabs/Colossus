//! Opt-in compatibility against an isolated, unlocked native Secret Service.

use crate::{PlatformKeyStore as _, SystemCredentialEntry, SystemKeyStore};
use colossus_contracts::CredentialError;
use dbus_secret_service::{EncryptionType, Item, SecretService};
use std::collections::HashMap;

struct SeededItem<'a>(Item<'a>);

impl Drop for SeededItem<'_> {
    fn drop(&mut self) {
        // Only the exact newly created synthetic item path is owned by this test.
        let _ = self.0.delete();
    }
}

#[test]
#[ignore = "requires an isolated unlocked Secret Service; included by the native credential CI filter"]
fn platform_master_key_survives_vault_reopen_with_legacy_secret_service_attributes() {
    assert!(std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some());
    let service = SecretService::connect(EncryptionType::Dh).unwrap();
    let collection = service.get_default_collection().unwrap();
    assert!(!collection.is_locked().unwrap());
    let mut random = [0; 16];
    getrandom::fill(&mut random).unwrap();
    let suffix = hex::encode(random);
    let generic_service = "com.obscuritylabs.colossus.legacy-keyring-test";
    let generic_account = format!("legacy.{suffix}");
    let generic_secret = b"synthetic-legacy-bearer";
    let generic = SeededItem(
        collection
            .create_item(
                "Colossus synthetic legacy generic entry",
                HashMap::from([
                    ("service", generic_service),
                    ("username", generic_account.as_str()),
                ]),
                generic_secret,
                false,
                "application/octet-stream",
            )
            .unwrap(),
    );
    let vault_service = "com.obscuritylabs.colossus.credentials.v1";
    let vault_account = format!("v1.{suffix}.{suffix}");
    let vault_envelope = b"synthetic-existing-ascii-key-envelope";
    let vault = SeededItem(
        collection
            .create_item(
                "Colossus synthetic legacy vault entry",
                HashMap::from([
                    ("service", vault_service),
                    ("username", vault_account.as_str()),
                    ("target", "default"),
                ]),
                vault_envelope,
                false,
                "application/octet-stream",
            )
            .unwrap(),
    );
    // Seed the old wire attributes directly, without linking the retired zbus
    // backend. Reading must find the existing entries and leave them unchanged.
    let entry = SystemCredentialEntry::new(generic_service, &generic_account).unwrap();
    assert_eq!(entry.read().unwrap().unwrap().as_slice(), generic_secret);
    assert_eq!(
        SystemKeyStore
            .read(&vault_account)
            .unwrap()
            .unwrap()
            .as_slice(),
        vault_envelope
    );
    assert_eq!(generic.0.get_secret().unwrap().as_slice(), generic_secret);
    assert_eq!(vault.0.get_secret().unwrap().as_slice(), vault_envelope);

    // Updating a legacy entry must keep its exact item and support binary values.
    let binary_secret = [0, 255, 1, 128];
    entry.write(&binary_secret).unwrap();
    assert_eq!(entry.read().unwrap().unwrap().as_slice(), binary_secret);
    assert_eq!(generic.0.get_secret().unwrap(), binary_secret);
    assert_eq!(vault.0.get_secret().unwrap().as_slice(), vault_envelope);

    // A proven missing generic entry is created in the persistent default
    // collection with the same legacy attributes, without an added target.
    let new_account = format!("new.{suffix}");
    let new_entry = SystemCredentialEntry::new(generic_service, &new_account).unwrap();
    assert!(new_entry.read().unwrap().is_none());
    new_entry.write(&binary_secret).unwrap();
    let attributes = HashMap::from([
        ("service", generic_service),
        ("username", new_account.as_str()),
    ]);
    let mut matches = collection.search_items(attributes).unwrap();
    assert_eq!(matches.len(), 1);
    let new_item = SeededItem(matches.pop().unwrap());
    assert!(!new_item.0.get_attributes().unwrap().contains_key("target"));
    assert_eq!(new_item.0.get_secret().unwrap(), binary_secret);
    new_entry.delete().unwrap();
    assert!(new_entry.read().unwrap().is_none());

    // Duplicate legacy identities cannot be arbitrarily read, updated or deleted.
    let ambiguous_account = format!("ambiguous.{suffix}");
    let duplicate_attributes = HashMap::from([
        ("service", generic_service),
        ("username", ambiguous_account.as_str()),
    ]);
    let first = SeededItem(
        collection
            .create_item(
                "Colossus synthetic duplicate one",
                duplicate_attributes.clone(),
                b"first",
                false,
                "application/octet-stream",
            )
            .unwrap(),
    );
    let second = SeededItem(
        collection
            .create_item(
                "Colossus synthetic duplicate two",
                duplicate_attributes,
                b"second",
                false,
                "application/octet-stream",
            )
            .unwrap(),
    );
    let ambiguous = SystemCredentialEntry::new(generic_service, &ambiguous_account).unwrap();
    assert_eq!(ambiguous.read(), Err(CredentialError::Corrupt));
    assert_eq!(
        ambiguous.write(b"replacement"),
        Err(CredentialError::Corrupt)
    );
    assert_eq!(ambiguous.delete(), Err(CredentialError::Corrupt));
    assert_eq!(first.0.get_secret().unwrap(), b"first");
    assert_eq!(second.0.get_secret().unwrap(), b"second");
    first.0.delete().unwrap();
    second.0.delete().unwrap();
    assert!(ambiguous.read().unwrap().is_none());

    entry.delete().unwrap();
    SystemKeyStore.delete(&vault_account).unwrap();
    assert!(entry.read().unwrap().is_none());
    assert!(SystemKeyStore.read(&vault_account).unwrap().is_none());
}
