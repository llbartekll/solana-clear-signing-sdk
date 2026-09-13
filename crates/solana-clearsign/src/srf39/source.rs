//! Where IDLs come from.
//!
//! An [`Srf39IdlSource`] answers "which IDL describes this program?" and
//! carries enough provenance for the client to verify the answer: the exact
//! JSON bytes, an optional pinned digest, and where the document came from.
//! The first shipped source is static (bundled files); the trait leaves room
//! for on-chain program-metadata lookups later.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

pub(crate) type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// An IDL document as returned by a source, before verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSrf39Idl {
    /// The program the source claims this IDL describes.
    pub program_id: String,
    /// The IDL JSON exactly as stored (the digest is computed over these
    /// UTF-8 bytes; sources must not re-serialise).
    pub json: String,
    pub provenance: IdlProvenance,
}

/// Where an IDL came from and what it was pinned to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdlProvenance {
    /// Opaque identifier of the source instance (e.g. `bundle:srf39-manifest.json`).
    pub source_id: String,
    pub origin: IdlOrigin,
    /// Expected SHA-256 of the JSON bytes, lowercase hex. `None` means the
    /// document is not pinned, which the client reports as a diagnostic.
    pub expected_sha256_hex: Option<String>,
    /// Free-form version string recorded by the source (IDL or program version).
    pub version: Option<String>,
    /// URL, commit or path that identifies the upstream document.
    pub reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdlOrigin {
    /// Shipped inside the application bundle.
    Bundled,
    /// Read from a local file outside the bundle.
    LocalFile,
    /// Reserved for the program-metadata program's canonical account.
    ProgramMetadataCanonical {
        authority: String,
    },
    Other(String),
}

/// The outcome of a source lookup. `NotFound` is not an error: it is how a
/// source says it does not know the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdlResolution {
    Found(ResolvedSrf39Idl),
    NotFound,
}

/// A source failed to answer (I/O, corrupt storage). Never cached.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("IDL source failed: {detail}")]
pub struct IdlSourceError {
    pub detail: String,
    pub retryable: bool,
}

pub trait Srf39IdlSource: Send + Sync {
    fn idl_for_program<'a>(
        &'a self,
        program_id: &'a str,
    ) -> Fut<'a, Result<IdlResolution, IdlSourceError>>;
}

/// In-memory source keyed by program id. When two entries share a program id
/// the last one wins.
#[derive(Debug, Clone, Default)]
pub struct StaticIdlSource {
    idls: BTreeMap<String, ResolvedSrf39Idl>,
}

impl StaticIdlSource {
    pub fn new(idls: impl IntoIterator<Item = ResolvedSrf39Idl>) -> Self {
        Self {
            idls: idls
                .into_iter()
                .map(|idl| (idl.program_id.clone(), idl))
                .collect(),
        }
    }

    pub fn empty() -> Self {
        Self::default()
    }
}

impl Srf39IdlSource for StaticIdlSource {
    fn idl_for_program<'a>(
        &'a self,
        program_id: &'a str,
    ) -> Fut<'a, Result<IdlResolution, IdlSourceError>> {
        let resolution = self
            .idls
            .get(program_id)
            .cloned()
            .map_or(IdlResolution::NotFound, IdlResolution::Found);
        Box::pin(async move { Ok(resolution) })
    }
}
