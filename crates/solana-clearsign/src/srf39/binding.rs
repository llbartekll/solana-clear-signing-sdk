//! Binding a resolved IDL to the program it is used for.
//!
//! sRFC 39 requires implementations to "validate that the IDL being used
//! corresponds to the correct program". Locally that means: the document
//! parses, its primary program is the requested one, and — when the source
//! pinned a digest — the bytes are the expected bytes. On-chain authority
//! checks need a network and are out of scope here.

use sha2::Digest;
use thiserror::Error;

use super::source::{IdlProvenance, ResolvedSrf39Idl};
use super::{Srf39Engine, Srf39IdlError};

/// What the client verified about the IDL it rendered with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdlBinding {
    pub program_id: String,
    /// `root.program.name` from the IDL.
    pub program_name: String,
    /// Actual SHA-256 of the JSON bytes, lowercase hex.
    pub sha256_hex: String,
    /// `true` when the source supplied an expected digest and it matched.
    pub digest_pinned: bool,
    pub provenance: IdlProvenance,
}

/// Why an IDL was refused. Rejections are deterministic for the same bytes
/// and are cached by the client until invalidated.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdlRejection {
    #[error("IDL declares program {declared} but was requested for {requested}")]
    ProgramMismatch { requested: String, declared: String },
    #[error("IDL digest mismatch: expected {expected}, actual {actual}")]
    DigestMismatch { expected: String, actual: String },
    #[error("IDL invalid: {0}")]
    Invalid(Srf39IdlError),
}

pub(crate) fn verify_binding(
    requested: &str,
    resolved: &ResolvedSrf39Idl,
) -> Result<(Srf39Engine, IdlBinding), IdlRejection> {
    if resolved.program_id != requested {
        return Err(IdlRejection::ProgramMismatch {
            requested: requested.to_string(),
            declared: resolved.program_id.clone(),
        });
    }
    let engine = Srf39Engine::from_json(&resolved.json).map_err(IdlRejection::Invalid)?;
    if engine.program_id() != requested {
        return Err(IdlRejection::ProgramMismatch {
            requested: requested.to_string(),
            declared: engine.program_id().to_string(),
        });
    }
    let actual = sha256_hex(resolved.json.as_bytes());
    let digest_pinned = match &resolved.provenance.expected_sha256_hex {
        Some(expected) => {
            if !is_hex_digest(expected) || expected.to_ascii_lowercase() != actual {
                return Err(IdlRejection::DigestMismatch {
                    expected: expected.clone(),
                    actual,
                });
            }
            true
        }
        None => false,
    };
    let binding = IdlBinding {
        program_id: requested.to_string(),
        program_name: engine.program_name().to_string(),
        sha256_hex: actual,
        digest_pinned,
        provenance: resolved.provenance.clone(),
    };
    Ok((engine, binding))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = sha2::Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn is_hex_digest(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_a_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hex_digest_shape_is_strict() {
        assert!(is_hex_digest(&"a".repeat(64)));
        assert!(!is_hex_digest(&"a".repeat(63)));
        assert!(!is_hex_digest(&"g".repeat(64)));
    }
}
