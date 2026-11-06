//! Git object IDs with explicit SHA-1 and SHA-256 formats.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

const SHA1_BYTES: usize = 20;
const SHA256_BYTES: usize = 32;

/// Hash algorithm used by a Git object database.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ObjectFormat {
    /// Traditional 160-bit Git object IDs.
    Sha1,
    /// 256-bit Git object IDs used by SHA-256 repositories.
    Sha256,
}

impl ObjectFormat {
    /// Returns the digest size in bytes.
    #[must_use]
    pub const fn byte_len(self) -> usize {
        match self {
            Self::Sha1 => SHA1_BYTES,
            Self::Sha256 => SHA256_BYTES,
        }
    }

    /// Returns the full hexadecimal representation length.
    #[must_use]
    pub const fn hex_len(self) -> usize {
        self.byte_len() * 2
    }
}

impl fmt::Display for ObjectFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
        })
    }
}

/// A validated full Git object ID.
///
/// The object format is retained explicitly so SHA-1 and SHA-256 IDs cannot be confused based on
/// a truncated or display-only string.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId {
    format: ObjectFormat,
    bytes: [u8; SHA256_BYTES],
}

impl ObjectId {
    /// Creates an object ID from a digest whose length must match `format`.
    pub fn from_bytes(format: ObjectFormat, digest: &[u8]) -> Result<Self, ObjectIdError> {
        let expected = format.byte_len();
        if digest.len() != expected {
            return Err(ObjectIdError::AlgorithmLengthMismatch {
                format,
                expected,
                actual: digest.len(),
            });
        }

        let mut bytes = [0; SHA256_BYTES];
        bytes[..expected].copy_from_slice(digest);
        Ok(Self { format, bytes })
    }

    /// Returns the object database hash format.
    #[must_use]
    pub const fn format(self) -> ObjectFormat {
        self.format
    }

    /// Returns the full digest bytes without padding.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.format.byte_len()]
    }
}

impl FromStr for ObjectId {
    type Err = ObjectIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let format = match value.len() {
            length if length == ObjectFormat::Sha1.hex_len() => ObjectFormat::Sha1,
            length if length == ObjectFormat::Sha256.hex_len() => ObjectFormat::Sha256,
            actual => return Err(ObjectIdError::UnsupportedHexLength { actual }),
        };

        let mut digest = [0; SHA256_BYTES];
        for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            let high = decode_hex(pair[0]).ok_or(ObjectIdError::InvalidHex {
                index: index * 2,
                byte: pair[0],
            })?;
            let low = decode_hex(pair[1]).ok_or(ObjectIdError::InvalidHex {
                index: index * 2 + 1,
                byte: pair[1],
            })?;
            digest[index] = (high << 4) | low;
        }
        Self::from_bytes(format, &digest[..format.byte_len()])
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.as_bytes() {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectId")
            .field("format", &self.format)
            .field("hex", &self.to_string())
            .finish()
    }
}

impl Serialize for ObjectId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ObjectId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

/// Validation errors for full Git object IDs.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ObjectIdError {
    /// A hexadecimal ID was neither a full SHA-1 nor a full SHA-256 digest.
    #[error("unsupported full object ID length {actual}; expected 40 or 64 hexadecimal characters")]
    UnsupportedHexLength { actual: usize },

    /// A digest byte length did not match its declared algorithm.
    #[error("{format} object ID requires {expected} bytes, got {actual}")]
    AlgorithmLengthMismatch {
        format: ObjectFormat,
        expected: usize,
        actual: usize,
    },

    /// A non-hexadecimal byte occurred in an object ID.
    #[error("invalid hexadecimal byte 0x{byte:02x} at index {index}")]
    InvalidHex { index: usize, byte: u8 },
}

const fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA1_HEX: &str = "0123456789abcdef0123456789abcdef01234567";
    const SHA256_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn parses_sha1_and_sha256_without_losing_the_algorithm() {
        let sha1: ObjectId = SHA1_HEX.parse().expect("SHA-1 OID should parse");
        let sha256: ObjectId = SHA256_HEX.parse().expect("SHA-256 OID should parse");

        assert_eq!(sha1.format(), ObjectFormat::Sha1);
        assert_eq!(sha1.as_bytes().len(), SHA1_BYTES);
        assert_eq!(sha1.to_string(), SHA1_HEX);
        assert_eq!(sha256.format(), ObjectFormat::Sha256);
        assert_eq!(sha256.as_bytes().len(), SHA256_BYTES);
        assert_eq!(sha256.to_string(), SHA256_HEX);
    }

    #[test]
    fn accepts_uppercase_hex_and_serializes_canonically() {
        let oid: ObjectId = SHA1_HEX
            .to_ascii_uppercase()
            .parse()
            .expect("uppercase OID should parse");
        let json = serde_json::to_string(&oid).expect("OID should serialize");

        assert_eq!(json, format!("\"{SHA1_HEX}\""));
        assert_eq!(
            serde_json::from_str::<ObjectId>(&json).expect("OID should deserialize"),
            oid
        );
    }

    #[test]
    fn rejects_invalid_lengths_algorithms_and_hex() {
        assert_eq!(
            "abcd".parse::<ObjectId>(),
            Err(ObjectIdError::UnsupportedHexLength { actual: 4 })
        );
        assert_eq!(
            ObjectId::from_bytes(ObjectFormat::Sha1, &[0; SHA256_BYTES]),
            Err(ObjectIdError::AlgorithmLengthMismatch {
                format: ObjectFormat::Sha1,
                expected: SHA1_BYTES,
                actual: SHA256_BYTES,
            })
        );

        let invalid = format!("{}g", &SHA1_HEX[..SHA1_HEX.len() - 1]);
        assert!(matches!(
            invalid.parse::<ObjectId>(),
            Err(ObjectIdError::InvalidHex {
                index: 39,
                byte: b'g'
            })
        ));
    }
}
