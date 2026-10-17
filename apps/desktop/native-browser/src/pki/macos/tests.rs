//! Pure `CoreFoundation` payload checks; no keychain or trust domain is modified.

use super::{CF, CFDictionaryGetValue, CFNumberCreate, Owned, trust_dictionary};

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCharacters(allocator: CF, characters: *const u16, length: isize) -> CF;
}

fn key(value: &str) -> Owned {
    let characters: Vec<_> = value.encode_utf16().collect();
    // SAFETY: CF copies the valid UTF-16 buffer into a retained test-only key.
    Owned::new(unsafe {
        CFStringCreateWithCharacters(
            std::ptr::null(),
            characters.as_ptr(),
            isize::try_from(characters.len()).unwrap(),
        )
    })
    .unwrap()
}

#[test]
fn trust_result_key_is_retained_and_looked_up_by_cfstring_value() {
    let value: i32 = 2;
    // SAFETY: CF copies one SInt32 value into a retained native number.
    let value =
        Owned::new(unsafe { CFNumberCreate(std::ptr::null(), 3, (&raw const value).cast()) })
            .unwrap();
    // The dictionary retains its dynamic UTF-8 key after the builder returns.
    let dictionary = trust_dictionary(&value).unwrap();
    let equivalent = key("kSecTrustSettingsResult");
    let other = key("kSecTrustSettingsPolicy");
    // SAFETY: Each dictionary/key/value is live for these pure native lookups.
    unsafe {
        assert_eq!(CFDictionaryGetValue(dictionary.0, equivalent.0), value.0);
        assert!(CFDictionaryGetValue(dictionary.0, other.0).is_null());
    }
}
