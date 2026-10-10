use super::{NativeBrowserReply, NativeBrowserRequest};
use crate::{CONTROL_HEADER_BYTES, ControlCodec, PresentationError, Role};
use hmac::{Hmac, Mac as _};
use serde::{Serialize, de::DeserializeOwned};
use sha2::Sha256;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"CLSNADM1";
const DEADLINE: Duration = Duration::from_secs(5);
const EFFECT_DEADLINE: Duration = Duration::from_secs(40);
type Auth = Hmac<Sha256>;

fn mac(
    key: &[u8; 32],
    purpose: &[u8],
    digest: &[u8; 32],
    client: &[u8; 32],
    server: &[u8; 32],
) -> Auth {
    let mut mac =
        <Auth as hmac::Mac>::new_from_slice(key).unwrap_or_else(|_| unreachable!("fixed key"));
    mac.update(b"colossus-desktop-browser-admission-v1\0");
    mac.update(purpose);
    mac.update(digest);
    mac.update(client);
    mac.update(server);
    mac
}
fn nonce() -> Result<[u8; 32], PresentationError> {
    let mut value = [0; 32];
    getrandom::fill(&mut value).map_err(|_| PresentationError::Unauthenticated)?;
    if value == [0; 32] {
        return Err(PresentationError::Unauthenticated);
    }
    Ok(value)
}

/// One fresh nonce-bound native admission exchange. It owns its complete stream.
pub struct AdmissionChannel<S> {
    stream: S,
    outgoing: ControlCodec,
    incoming: ControlCodec,
    relay_key: Zeroizing<[u8; 32]>,
}
impl<S: AsyncRead + AsyncWrite + Unpin> AdmissionChannel<S> {
    fn new(
        stream: S,
        key: Zeroizing<[u8; 32]>,
        digest: [u8; 32],
        client: bool,
        relay_key: Zeroizing<[u8; 32]>,
    ) -> Self {
        let (outgoing, incoming) = if client {
            (Role::NativeToHost, Role::HostToNative)
        } else {
            (Role::HostToNative, Role::NativeToHost)
        };
        Self {
            stream,
            outgoing: ControlCodec::new(Zeroizing::new(*key), digest, outgoing),
            incoming: ControlCodec::new(key, digest, incoming),
            relay_key,
        }
    }
    async fn read<T: DeserializeOwned>(
        &mut self,
        deadline: Duration,
    ) -> Result<T, PresentationError> {
        tokio::time::timeout(deadline, async {
            let mut header = [0; CONTROL_HEADER_BYTES];
            self.stream
                .read_exact(&mut header)
                .await
                .map_err(|_| PresentationError::OutcomeUnknown)?;
            let length = self.incoming.payload_length(&header)?;
            let mut bytes = vec![0; CONTROL_HEADER_BYTES + length];
            bytes[..CONTROL_HEADER_BYTES].copy_from_slice(&header);
            self.stream
                .read_exact(&mut bytes[CONTROL_HEADER_BYTES..])
                .await
                .map_err(|_| PresentationError::OutcomeUnknown)?;
            self.incoming.decode(&bytes)
        })
        .await
        .map_err(|_| PresentationError::OutcomeUnknown)?
    }
    async fn write<T: Serialize>(&mut self, value: &T) -> Result<(), PresentationError> {
        let bytes = self.outgoing.encode(value)?;
        tokio::time::timeout(DEADLINE, self.stream.write_all(&bytes))
            .await
            .map_err(|_| PresentationError::OutcomeUnknown)?
            .map_err(|_| PresentationError::OutcomeUnknown)
    }
    /// Send bounded native intent and receive one authenticated categorical result.
    pub async fn request(
        &mut self,
        request: &NativeBrowserRequest,
    ) -> Result<NativeBrowserReply, PresentationError> {
        request.validate()?;
        self.write(request).await?;
        self.read(EFFECT_DEADLINE).await
    }
    /// Receive native intent only after the fresh authenticated endpoint handshake.
    pub async fn receive_request(&mut self) -> Result<NativeBrowserRequest, PresentationError> {
        let request: NativeBrowserRequest = self.read(DEADLINE).await?;
        request.validate()?;
        Ok(request)
    }
    /// Acknowledge native admission only after Runtime confirms ownership.
    pub async fn reply(&mut self, reply: &NativeBrowserReply) -> Result<(), PresentationError> {
        self.write(reply).await
    }
    /// Transfer the admitted stream into its independently keyed native presentation role.
    pub fn into_presentation(self) -> (S, Zeroizing<[u8; 32]>) {
        (self.stream, self.relay_key)
    }
}

fn keys(
    authentication: &[u8; 32],
    digest: &[u8; 32],
    client: &[u8; 32],
    server: &[u8; 32],
) -> (Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>) {
    (
        Zeroizing::new(
            mac(
                authentication,
                b"admission-stream\0",
                digest,
                client,
                server,
            )
            .finalize()
            .into_bytes()
            .into(),
        ),
        Zeroizing::new(
            mac(
                authentication,
                b"presentation-relay\0",
                digest,
                client,
                server,
            )
            .finalize()
            .into_bytes()
            .into(),
        ),
    )
}

/// Authenticate the exact attested child using fresh nonces and a distinct bootstrap key.
pub async fn client_handshake<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    authentication: &Zeroizing<[u8; 32]>,
    digest: [u8; 32],
) -> Result<AdmissionChannel<S>, PresentationError> {
    if **authentication == [0; 32] || digest == [0; 32] {
        return Err(PresentationError::Unauthenticated);
    }
    let client = nonce()?;
    let mut hello = [0; 40];
    hello[..8].copy_from_slice(MAGIC);
    hello[8..].copy_from_slice(&client);
    let mut response = [0; 64];
    tokio::time::timeout(DEADLINE, async {
        stream.write_all(&hello).await?;
        stream.read_exact(&mut response).await?;
        Ok::<_, std::io::Error>(())
    })
    .await
    .map_err(|_| PresentationError::OutcomeUnknown)?
    .map_err(|_| PresentationError::OutcomeUnknown)?;
    let server: [u8; 32] = response[..32]
        .try_into()
        .map_err(|_| PresentationError::Invalid)?;
    if server == [0; 32] {
        return Err(PresentationError::Unauthenticated);
    }
    mac(authentication, b"server-ready\0", &digest, &client, &server)
        .verify_slice(&response[32..])
        .map_err(|_| PresentationError::Unauthenticated)?;
    let (key, relay) = keys(authentication, &digest, &client, &server);
    Ok(AdmissionChannel::new(stream, key, digest, true, relay))
}

/// Enroll only the native parent that possesses this exact inherited-channel authority.
/// Client proof arrives in the first MAC-protected request; this function grants no effect.
pub async fn server_handshake<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    authentication: &Zeroizing<[u8; 32]>,
    digest: [u8; 32],
) -> Result<AdmissionChannel<S>, PresentationError> {
    if **authentication == [0; 32] || digest == [0; 32] {
        return Err(PresentationError::Unauthenticated);
    }
    let mut hello = [0; 40];
    tokio::time::timeout(DEADLINE, stream.read_exact(&mut hello))
        .await
        .map_err(|_| PresentationError::OutcomeUnknown)?
        .map_err(|_| PresentationError::OutcomeUnknown)?;
    if &hello[..8] != MAGIC {
        return Err(PresentationError::Unauthenticated);
    }
    let client = hello[8..]
        .try_into()
        .map_err(|_| PresentationError::Invalid)?;
    if client == [0; 32] {
        return Err(PresentationError::Unauthenticated);
    }
    let server = nonce()?;
    let mut response = [0; 64];
    response[..32].copy_from_slice(&server);
    response[32..].copy_from_slice(
        &mac(authentication, b"server-ready\0", &digest, &client, &server)
            .finalize()
            .into_bytes(),
    );
    tokio::time::timeout(DEADLINE, stream.write_all(&response))
        .await
        .map_err(|_| PresentationError::OutcomeUnknown)?
        .map_err(|_| PresentationError::OutcomeUnknown)?;
    let (key, relay) = keys(authentication, &digest, &client, &server);
    Ok(AdmissionChannel::new(stream, key, digest, false, relay))
}
