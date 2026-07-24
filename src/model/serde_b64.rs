//! Base64 helpers for opaque byte blobs in `project.json`.

use base64::Engine as _;
use serde::{Deserialize, Deserializer, Serializer};

pub fn serialize<S>(value: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(bytes) => {
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            serializer.serialize_some(&encoded)
        }
        None => serializer.serialize_none(),
    }
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(deserializer)?;
    match opt {
        None => Ok(None),
        Some(text) if text.is_empty() => Ok(None),
        Some(text) => base64::engine::general_purpose::STANDARD
            .decode(text.as_bytes())
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}
