//! Explicit secret serialization confined to the private inherited bootstrap channel.

use colossus_contracts::HostSecret;
use serde::{Deserialize, Deserializer, Serializer};

pub(crate) fn serialize<S: Serializer>(
    secret: &HostSecret,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose())
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<HostSecret, D::Error> {
    HostSecret::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}
