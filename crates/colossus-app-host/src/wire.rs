use std::io::Write as _;
use tokio::io::{AsyncRead, AsyncReadExt as _};
use zeroize::Zeroizing;

pub(crate) const MAX_FRAME: usize = 2 * 1024 * 1024;
pub(crate) type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(crate) async fn read(input: &mut (impl AsyncRead + Unpin)) -> Result<Zeroizing<Vec<u8>>> {
    let size = input.read_u32().await? as usize;
    if size == 0 || size > MAX_FRAME {
        return Err("invalid private host frame".into());
    }
    let mut payload = Zeroizing::new(vec![0; size]);
    input.read_exact(&mut payload).await?;
    Ok(payload)
}

pub(crate) fn write(value: &impl serde::Serialize) -> Result<()> {
    let payload = Zeroizing::new(serde_json::to_vec(value)?);
    if payload.is_empty() || payload.len() > MAX_FRAME {
        return Err("invalid private host frame".into());
    }
    let mut output = std::io::stdout().lock();
    output.write_all(&(payload.len() as u32).to_be_bytes())?;
    output.write_all(&payload)?;
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn rejects_empty_truncated_and_oversized_frames() {
        for mut input in [&[0, 0, 0, 0][..], &[0, 32, 0, 1][..], &[0, 0, 0, 3, 1][..]] {
            assert!(super::read(&mut input).await.is_err());
        }
    }
}
