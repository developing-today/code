/// Serde adapters. A key or a hash is written as 64 hex characters, so a log line reads as text.
/// Each module is named for the type it adapts, and is used as `#[serde(with = "...")]`.
use p2panda_core::Hash;
use serde::de;

use super::Key;

fn key_text(key: &Key) -> String {
    Hash::from_bytes(*key).to_hex()
}

fn hash_text(hash: &Hash) -> String {
    hash.to_hex()
}

fn text_to_hash<E: de::Error>(text: &str) -> Result<Hash, E> {
    text.parse()
        .map_err(|error| E::custom(format!("`{text}` is not 64 hex characters: {error}")))
}

fn text_to_key<E: de::Error>(text: &str) -> Result<Key, E> {
    text_to_hash::<E>(text).map(|hash| *hash.as_bytes())
}

/// A key, as hex.
pub mod key {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::{Key, key_text, text_to_key};

    pub fn serialize<S: Serializer>(key: &Key, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&key_text(key))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Key, D::Error> {
        text_to_key(&String::deserialize(deserializer)?)
    }
}

/// An optional key, as hex.
pub mod opt_key {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{Key, key_text, text_to_key};

    #[allow(clippy::ref_option)] // serde passes the field by reference.
    pub fn serialize<S: Serializer>(key: &Option<Key>, serializer: S) -> Result<S::Ok, S::Error> {
        key.as_ref().map(key_text).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Key>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .as_deref()
            .map(text_to_key::<D::Error>)
            .transpose()
    }
}

/// A hash, as hex.
pub mod hash {
    use p2panda_core::Hash;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::{hash_text, text_to_hash};

    pub fn serialize<S: Serializer>(hash: &Hash, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hash_text(hash))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Hash, D::Error> {
        text_to_hash(&String::deserialize(deserializer)?)
    }
}

/// An optional hash, as hex.
pub mod opt_hash {
    use p2panda_core::Hash;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{hash_text, text_to_hash};

    #[allow(clippy::ref_option)] // serde passes the field by reference.
    pub fn serialize<S: Serializer>(hash: &Option<Hash>, serializer: S) -> Result<S::Ok, S::Error> {
        hash.as_ref().map(hash_text).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Hash>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .as_deref()
            .map(text_to_hash::<D::Error>)
            .transpose()
    }
}

/// A list of hashes, as hex.
pub mod hashes {
    use p2panda_core::Hash;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{hash_text, text_to_hash};

    pub fn serialize<S: Serializer>(hashes: &[Hash], serializer: S) -> Result<S::Ok, S::Error> {
        hashes
            .iter()
            .map(hash_text)
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Hash>, D::Error> {
        Vec::<String>::deserialize(deserializer)?
            .iter()
            .map(|text| text_to_hash::<D::Error>(text))
            .collect()
    }
}

/// A create's members, each with its role, as hex.
pub mod members {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::groups::Role;

    use super::{Key, key_text, text_to_key};

    pub fn serialize<S: Serializer>(
        members: &[(Key, Role)],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        members
            .iter()
            .map(|(key, role)| (key_text(key), *role))
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<(Key, Role)>, D::Error> {
        Vec::<(String, Role)>::deserialize(deserializer)?
            .iter()
            .map(|(text, role)| Ok((text_to_key::<D::Error>(text)?, *role)))
            .collect()
    }
}
