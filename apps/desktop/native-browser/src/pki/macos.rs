//! Security.framework provisioning in the user's default keychain/trust domain.

use std::ffi::c_void;

use super::{PkiError, fingerprint};

type CF = *const c_void;
const UTF8: u32 = 0x0800_0100;
const DUPLICATE_ITEM: i32 = -25299;

#[repr(C)]
struct DictionaryKeyCallbacks {
    version: isize,
    retain: Option<unsafe extern "C" fn(CF, CF) -> CF>,
    release: Option<unsafe extern "C" fn(CF, CF)>,
    copy_description: Option<unsafe extern "C" fn(CF) -> CF>,
    equal: Option<unsafe extern "C" fn(CF, CF) -> u8>,
    hash: Option<unsafe extern "C" fn(CF) -> usize>,
}

#[cfg(test)]
mod tests;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeDictionaryKeyCallBacks: DictionaryKeyCallbacks;
    fn CFRelease(value: CF);
    fn CFDataCreate(allocator: CF, bytes: *const u8, length: isize) -> CF;
    fn CFDataGetLength(data: CF) -> isize;
    fn CFDataGetBytePtr(data: CF) -> *const u8;
    fn CFStringCreateWithBytes(
        allocator: CF,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: u8,
    ) -> CF;
    fn CFDictionaryCreate(
        allocator: CF,
        keys: *const CF,
        values: *const CF,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CF;
    fn CFDictionaryGetValue(dictionary: CF, key: CF) -> CF;
    fn CFArrayGetCount(array: CF) -> isize;
    fn CFArrayGetValueAtIndex(array: CF, index: isize) -> CF;
    fn CFArrayCreate(
        allocator: CF,
        values: *const CF,
        count: isize,
        callbacks: *const c_void,
    ) -> CF;
    fn CFNumberCreate(allocator: CF, kind: isize, value: *const c_void) -> CF;
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecImportExportPassphrase: CF;
    static kSecImportExportKeychain: CF;
    static kSecImportItemIdentity: CF;
    fn SecKeychainCopyDefault(keychain: *mut CF) -> i32;
    fn SecCertificateCreateWithData(allocator: CF, data: CF) -> CF;
    fn SecCertificateAddToKeychain(certificate: CF, keychain: CF) -> i32;
    fn SecTrustSettingsSetTrustSettings(certificate: CF, domain: u32, settings: CF) -> i32;
    fn SecPKCS12Import(data: CF, options: CF, items: *mut CF) -> i32;
    fn SecIdentityCopyCertificate(identity: CF, certificate: *mut CF) -> i32;
    fn SecCertificateCopyData(certificate: CF) -> CF;
}

struct Owned(CF);
impl Owned {
    fn new(value: CF) -> Result<Self, PkiError> {
        if value.is_null() {
            Err(PkiError::Unavailable)
        } else {
            Ok(Self(value))
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: Every owner represents one non-null Create/Copy reference.
        unsafe {
            CFRelease(self.0);
        }
    }
}

fn data(bytes: &[u8]) -> Result<Owned, PkiError> {
    // SAFETY: CF copies the bounded borrowed bytes into its native buffer.
    Owned::new(unsafe {
        CFDataCreate(
            std::ptr::null(),
            bytes.as_ptr(),
            isize::try_from(bytes.len()).map_err(|_| PkiError::InvalidIdentity)?,
        )
    })
}

fn keychain() -> Result<Owned, PkiError> {
    let mut keychain = std::ptr::null();
    // SAFETY: Copy API writes one retained keychain reference on success.
    if unsafe { SecKeychainCopyDefault(&raw mut keychain) } != 0 {
        return Err(PkiError::Unavailable);
    }
    Owned::new(keychain)
}

fn trust_dictionary(result: &Owned) -> Result<Owned, PkiError> {
    // SecTrustSettings.h defines this key with CFSTR, not an exported symbol.
    // Create the same string value and retain/compare it by CF content, since
    // Security's constant key has a different address from this owned string.
    let bytes = b"kSecTrustSettingsResult";
    // SAFETY: CF copies the fixed valid UTF-8 key into a retained string.
    let key = Owned::new(unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            bytes.as_ptr(),
            isize::try_from(bytes.len()).map_err(|_| PkiError::Unavailable)?,
            UTF8,
            0,
        )
    })?;
    let keys = [key.0];
    let values = [result.0];
    // SAFETY: CoreFoundation's exported key callbacks retain CF references and
    // compare/hash CFString values. The caller keeps the no-retain value alive
    // through the synchronous trust operation.
    Owned::new(unsafe {
        CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            (&raw const kCFTypeDictionaryKeyCallBacks).cast(),
            std::ptr::null(),
        )
    })
}

pub(super) fn import_ca(der: &[u8]) -> Result<(), PkiError> {
    let bytes = data(der)?;
    let keychain = keychain()?;
    // SAFETY: Security.framework reads a live CFData and returns retained cert.
    let certificate =
        Owned::new(unsafe { SecCertificateCreateWithData(std::ptr::null(), bytes.0) })?;
    // SAFETY: Explicit user default keychain, never System keychain.
    let code = unsafe { SecCertificateAddToKeychain(certificate.0, keychain.0) };
    if code != 0 && code != DUPLICATE_ITEM {
        return Err(PkiError::Unavailable);
    }
    // Explicit trust root/as-root result supports reviewed root and intermediate
    // CA anchors. This is an OS-user change, not per-browser-profile trust.
    let (_, parsed) =
        x509_parser::parse_x509_certificate(der).map_err(|_| PkiError::InvalidCertificate)?;
    let result: i32 = if parsed.subject() == parsed.issuer() {
        1
    } else {
        2
    };
    // kCFNumberSInt32Type = 3. No-retain values live only within this call.
    let result =
        Owned::new(unsafe { CFNumberCreate(std::ptr::null(), 3, (&raw const result).cast()) })?;
    let dictionary = trust_dictionary(&result)?;
    let settings = Owned::new(unsafe {
        CFArrayCreate(
            std::ptr::null(),
            &raw const dictionary.0,
            1,
            std::ptr::null(),
        )
    })?;
    // The OS may require native authorization before this operation succeeds.
    // SAFETY: Certificate is native-owned and lives through synchronous call.
    if unsafe { SecTrustSettingsSetTrustSettings(certificate.0, 0, settings.0) } != 0 {
        return Err(PkiError::StoreChangeUnknown);
    }
    Ok(())
}

pub(super) fn import_pfx(bytes: &[u8], password: &str) -> Result<Vec<String>, PkiError> {
    let bytes = data(bytes)?;
    let keychain = keychain()?;
    // The passphrase is copied only into a native CFString. Its owned buffer is
    // released after import and is never bridged to JavaScript, DTOs, or logs.
    // OS/native control allocations cannot promise explicit byte zeroization.
    // SAFETY: CF copies a valid bounded UTF-8 buffer.
    let password = Owned::new(unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            password.as_ptr(),
            isize::try_from(password.len()).map_err(|_| PkiError::InvalidIdentity)?,
            UTF8,
            0,
        )
    })?;
    // No-retain callbacks: local owners and Security's static keys outlive call.
    let keys = unsafe { [kSecImportExportPassphrase, kSecImportExportKeychain] };
    let values = [password.0, keychain.0];
    // SAFETY: Both fixed arrays contain live CF references throughout import.
    let options = Owned::new(unsafe {
        CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            2,
            std::ptr::null(),
            std::ptr::null(),
        )
    })?;
    let mut items = std::ptr::null();
    // Explicit keychain option avoids assuming PKCS12's platform default scope.
    // SAFETY: Native objects live during synchronous import; API retains output.
    if unsafe { SecPKCS12Import(bytes.0, options.0, &raw mut items) } != 0 {
        return Err(PkiError::StoreChangeUnknown);
    }
    let items = Owned::new(items)?;
    let count = unsafe { CFArrayGetCount(items.0) };
    if !(1..=64).contains(&count) {
        return Err(PkiError::StoreChangeUnknown);
    }
    let mut fingerprints = Vec::new();
    for index in 0..count {
        // SAFETY: PKCS12 import output is documented as array of dictionaries.
        let dictionary = unsafe { CFArrayGetValueAtIndex(items.0, index) };
        let identity = unsafe { CFDictionaryGetValue(dictionary, kSecImportItemIdentity) };
        if identity.is_null() {
            return Err(PkiError::StoreChangeUnknown);
        }
        let mut certificate = std::ptr::null();
        if unsafe { SecIdentityCopyCertificate(identity, &raw mut certificate) } != 0 {
            return Err(PkiError::StoreChangeUnknown);
        }
        let certificate = Owned::new(certificate)?;
        let der = Owned::new(unsafe { SecCertificateCopyData(certificate.0) })?;
        let length = unsafe { CFDataGetLength(der.0) };
        if !(1..=65_536).contains(&length) {
            return Err(PkiError::StoreChangeUnknown);
        }
        // SAFETY: CFData supplies a live bounded DER buffer until release.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                CFDataGetBytePtr(der.0),
                usize::try_from(length).map_err(|_| PkiError::StoreChangeUnknown)?,
            )
        };
        fingerprints.push(fingerprint(bytes));
    }
    Ok(fingerprints)
}
