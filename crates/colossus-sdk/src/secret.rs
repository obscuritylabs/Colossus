use crate::SdkResult;
use async_trait::async_trait;
use std::fmt;
use zeroize::Zeroizing;

const MAX_CREDENTIAL_BYTES: usize = 761;

/// Owned secret bytes that redact debug output and clear their allocation on drop.
///
/// The value is deliberately non-cloneable. Transport code should borrow it only while
/// preparing authenticated metadata and must not place it in diagnostics, argv, or
/// environment variables.
///
/// ```compile_fail
/// use colossus_sdk::Secret;
///
/// let credential = Secret::new(b"cls_v1.example.secret".to_vec()).unwrap();
/// let duplicated = credential.clone();
/// ```
pub struct Secret {
    bytes: Zeroizing<Vec<u8>>,
}

impl Secret {
    /// Wrap non-empty credential bytes.
    pub fn new(bytes: impl Into<Vec<u8>>) -> SdkResult<Self> {
        let bytes = bytes.into();
        if bytes.is_empty()
            || bytes.len() > MAX_CREDENTIAL_BYTES
            || !bytes.iter().all(|byte| (0x21..=0x7e).contains(byte))
        {
            return Err(crate::SdkError::InvalidConfiguration(
                "credential must be bounded visible ASCII",
            ));
        }
        Ok(Self {
            bytes: Zeroizing::new(bytes),
        })
    }

    /// Borrow the secret for an authenticated transport operation.
    pub fn expose(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret([REDACTED])")
    }
}

/// Application credential source, normally backed by a platform credential store.
#[async_trait]
pub trait CredentialProvider: Send + Sync {
    /// Load one credential for an authenticated connection.
    ///
    /// Implementations must return a fresh owned value and must not log its contents.
    async fn load(&self) -> SdkResult<Secret>;
}
