//! Length-delimited private framing; image bytes never become JSON arrays.
use serde::{Serialize, de::DeserializeOwned};
#[cfg(windows)]
use std::io::Write;
use std::io::{self, Read};

pub(crate) const MAX_JSON: usize = 1024 * 1024;
pub(crate) const MAX_PNG: usize = 16 * 1024 * 1024;

pub(crate) fn encode<T: Serialize>(value: &T, binary: &[u8]) -> io::Result<Vec<u8>> {
    let json = serde_json::to_vec(value).map_err(|_| invalid())?;
    if json.is_empty() || json.len() > MAX_JSON || binary.len() > MAX_PNG {
        return Err(invalid());
    }
    let mut bytes = Vec::with_capacity(8 + json.len() + binary.len());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&json);
    bytes.extend_from_slice(binary);
    Ok(bytes)
}

pub(crate) fn read<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<(T, Vec<u8>)> {
    let mut header = [0; 8];
    reader.read_exact(&mut header)?;
    let json = u32::from_le_bytes(header[..4].try_into().map_err(|_| invalid())?) as usize;
    let image = u32::from_le_bytes(header[4..].try_into().map_err(|_| invalid())?) as usize;
    if json == 0 || json > MAX_JSON || image > MAX_PNG {
        return Err(invalid());
    }
    let mut body = vec![0; json];
    reader.read_exact(&mut body)?;
    let value = serde_json::from_slice(&body).map_err(|_| invalid())?;
    let mut binary = vec![0; image];
    reader.read_exact(&mut binary)?;
    Ok((value, binary))
}

#[cfg(windows)]
pub(crate) fn write<T: Serialize>(
    writer: &mut impl Write,
    value: &T,
    binary: &[u8],
) -> io::Result<()> {
    writer.write_all(&encode(value, binary)?)?;
    writer.flush()
}

pub(crate) fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid computer helper frame")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binary_is_separate_and_declared_sizes_are_bounded_before_allocation() {
        let json = serde_json::json!({"sequence":1});
        let bytes = encode(&json, &[137, 80, 78, 71]).unwrap();
        let (decoded, image) = read::<serde_json::Value>(&mut bytes.as_slice()).unwrap();
        assert_eq!(decoded, json);
        assert_eq!(image, [137, 80, 78, 71]);
        for header in [[0_u8; 8], [255; 8]] {
            assert!(read::<serde_json::Value>(&mut header.as_slice()).is_err());
        }
        assert!(read::<serde_json::Value>(&mut bytes[..bytes.len() - 1].as_ref()).is_err());
    }
}
