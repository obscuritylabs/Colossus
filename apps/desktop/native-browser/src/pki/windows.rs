//! Current User stores only; keys are never marked exportable or overwritten.

use windows_sys::Win32::Security::Cryptography::{
    CERT_CONTEXT, CERT_KEY_PROV_INFO_PROP_ID, CERT_STORE_ADD_NEW, CRYPT_INTEGER_BLOB,
    CRYPT_USER_KEYSET, CertAddCertificateContextToStore, CertCloseStore,
    CertCreateCertificateContext, CertEnumCertificatesInStore, CertFreeCertificateContext,
    CertGetCertificateContextProperty, CertOpenSystemStoreW, HCERTSTORE, PFXImportCertStore,
    PFXVerifyPassword, PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
};
use zeroize::Zeroizing;

use super::{PkiError, fingerprint};

struct Store(HCERTSTORE);
impl Drop for Store {
    fn drop(&mut self) {
        // SAFETY: This owner holds exactly one returned store handle.
        unsafe {
            CertCloseStore(self.0, 0);
        }
    }
}
struct Certificate(*const CERT_CONTEXT);
impl Drop for Certificate {
    fn drop(&mut self) {
        // SAFETY: This owner holds exactly one certificate context reference.
        unsafe {
            CertFreeCertificateContext(self.0);
        }
    }
}

fn user_store(name: &str) -> Result<Store, PkiError> {
    let name = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    // SAFETY: Null provider and fixed ROOT/MY names select Current User stores.
    let store = unsafe { CertOpenSystemStoreW(0, name.as_ptr()) };
    if store.is_null() {
        Err(PkiError::Unavailable)
    } else {
        Ok(Store(store))
    }
}

pub(super) fn import_ca(der: &[u8]) -> Result<(), PkiError> {
    let store = user_store("ROOT")?;
    // SAFETY: Validated bounded DER is borrowed for this call; API returns a ref.
    let context = unsafe {
        CertCreateCertificateContext(
            X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
            der.as_ptr(),
            u32::try_from(der.len()).map_err(|_| PkiError::InvalidCertificate)?,
        )
    };
    if context.is_null() {
        return Err(PkiError::InvalidCertificate);
    }
    let context = Certificate(context);
    // ADD_NEW prevents silently replacing an existing trust record/properties.
    // SAFETY: Both handles live through this synchronous native call.
    if unsafe {
        CertAddCertificateContextToStore(
            store.0,
            context.0,
            CERT_STORE_ADD_NEW,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(PkiError::Unavailable);
    }
    Ok(())
}

pub(super) fn import_pfx(bytes: &[u8], password: &str) -> Result<Vec<String>, PkiError> {
    let mut password = Zeroizing::new(password.encode_utf16().chain(Some(0)).collect::<Vec<_>>());
    let blob = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).map_err(|_| PkiError::InvalidIdentity)?,
        pbData: bytes.as_ptr().cast_mut(),
    };
    // SAFETY: Borrowed PFX and zeroizing password remain alive. First prove
    // password/structure before the persistent key import has side effects.
    if unsafe { PFXVerifyPassword(&raw const blob, password.as_ptr(), 0) } == 0 {
        return Err(PkiError::InvalidIdentity);
    }
    let target = user_store("MY")?;
    // CRYPT_USER_KEYSET creates user keys; neither CRYPT_EXPORTABLE nor
    // PKCS12_ALLOW_OVERWRITE_KEY is requested. Never use NO_PERSIST_KEY for the
    // effective import: Chromium helpers must access the native persisted key.
    // SAFETY: Native buffers are valid for synchronous import; no key leaves OS.
    let imported =
        unsafe { PFXImportCertStore(&raw const blob, password.as_mut_ptr(), CRYPT_USER_KEYSET) };
    if imported.is_null() {
        return Err(PkiError::StoreChangeUnknown);
    }
    let imported = Store(imported);
    let mut previous = std::ptr::null();
    let mut fingerprints = Vec::new();
    loop {
        // Enumeration frees the previous reference on each call, including end.
        // SAFETY: Store lives throughout enumeration; previous is API-owned ref.
        let certificate = unsafe { CertEnumCertificatesInStore(imported.0, previous) };
        if certificate.is_null() {
            break;
        }
        previous = certificate;
        if fingerprints.len() >= 64 {
            // SAFETY: Enumeration will not consume this remaining reference.
            unsafe {
                CertFreeCertificateContext(certificate);
            }
            return Err(PkiError::StoreChangeUnknown);
        }
        let mut property_size = 0;
        // Only identities with a persisted provider binding enter MY. Root and
        // intermediate certificates in a PFX never become new trust anchors.
        // SAFETY: Query with null data obtains property length only.
        let has_key = unsafe {
            CertGetCertificateContextProperty(
                certificate,
                CERT_KEY_PROV_INFO_PROP_ID,
                std::ptr::null_mut(),
                &raw mut property_size,
            )
        } != 0;
        if !has_key {
            continue;
        }
        // SAFETY: Context's DER buffer lives until next enumeration step.
        let der = unsafe {
            std::slice::from_raw_parts(
                (*certificate).pbCertEncoded,
                (*certificate).cbCertEncoded as usize,
            )
        };
        if unsafe {
            CertAddCertificateContextToStore(
                target.0,
                certificate,
                CERT_STORE_ADD_NEW,
                std::ptr::null_mut(),
            )
        } == 0
        {
            unsafe {
                CertFreeCertificateContext(certificate);
            }
            return Err(PkiError::StoreChangeUnknown);
        }
        fingerprints.push(fingerprint(der));
    }
    if fingerprints.is_empty() {
        return Err(PkiError::StoreChangeUnknown);
    }
    Ok(fingerprints)
}
