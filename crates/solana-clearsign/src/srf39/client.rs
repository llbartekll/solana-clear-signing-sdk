//! The high-level client: IDL lookup, binding verification, engine caching,
//! canonical rendering and presentation enrichment in one call.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use thiserror::Error;

use crate::provider::Srf39AccountProvider;
use crate::InstructionContext;

use super::binding::{sha256_hex, verify_binding, IdlBinding, IdlRejection};
use super::diagnostics::FormatDiagnostic;
use super::diagnostics::Srf39DiagnosticKind;
use super::hints::InstructionDisplayHints;
use super::presentation::{
    enrich, EmptyPresentationProvider, PresentationMetadataProvider, PresentationOverlay,
};
use super::source::{IdlOrigin, IdlProvenance, IdlResolution, Srf39IdlSource, StaticIdlSource};
use super::Srf39IdlError;
use super::{DisplayMiss, DisplayResult, InstructionDisplay, Srf39DisplayError, Srf39Engine};

/// Upper bound on cached `NotFound` entries. On overflow every `NotFound`
/// entry is dropped; `Ready` and `Rejected` entries are kept.
const NOT_FOUND_CAP: usize = 256;

/// Everything rendered for one instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedInstruction {
    /// Byte-for-byte what the strict renderer produced.
    pub canonical: InstructionDisplay,
    pub idl: IdlBinding,
    pub hints: InstructionDisplayHints,
    pub presentation: PresentationOverlay,
    /// In emission order; branch on `code`, never on `message`.
    pub diagnostics: Vec<FormatDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderOutcome {
    Rendered(Box<RenderedInstruction>),
    Unsupported { reason: UnsupportedReason },
}

/// Expected "not mine" outcomes. None of these is a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsupportedReason {
    IdlNotFound {
        program_id: String,
    },
    InstructionNotRecognized {
        program_id: String,
    },
    InstructionDecodeFailed {
        program_id: String,
        instruction: String,
    },
}

/// Failures. Hosts should fail closed on every variant.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RenderFailure {
    #[error("invalid input: {detail}")]
    InvalidInput { detail: String },
    #[error("IDL for {program_id} rejected: {rejection}")]
    IdlRejected {
        program_id: String,
        rejection: IdlRejection,
    },
    #[error("IDL source failed: {detail}")]
    IdlSourceFailed { detail: String, retryable: bool },
    #[error("could not decode linked account '{account}': {detail}")]
    AccountDecode { account: String, detail: String },
    #[error("internal error: {detail}")]
    Internal { detail: String },
}

impl From<Srf39DisplayError> for RenderFailure {
    fn from(value: Srf39DisplayError) -> Self {
        match value {
            Srf39DisplayError::AccountDecode { account, detail } => {
                Self::AccountDecode { account, detail }
            }
            Srf39DisplayError::Internal { detail } => Self::Internal { detail },
        }
    }
}

pub(crate) struct LoadedIdl {
    pub(crate) engine: Arc<Srf39Engine>,
    pub(crate) binding: IdlBinding,
}

enum CacheEntry {
    Ready(Arc<LoadedIdl>),
    NotFound,
    Rejected(IdlRejection),
}

enum Loaded {
    Ready(Arc<LoadedIdl>),
    NotFound,
}

pub struct Srf39Client {
    source: Arc<dyn Srf39IdlSource>,
    account_provider: Option<Arc<dyn Srf39AccountProvider>>,
    presentation: Arc<dyn PresentationMetadataProvider>,
    cache: Mutex<BTreeMap<String, CacheEntry>>,
}

impl Srf39Client {
    pub fn new(
        source: Arc<dyn Srf39IdlSource>,
        account_provider: Option<Arc<dyn Srf39AccountProvider>>,
        presentation: Option<Arc<dyn PresentationMetadataProvider>>,
    ) -> Self {
        Self {
            source,
            account_provider,
            presentation: presentation.unwrap_or_else(|| Arc::new(EmptyPresentationProvider)),
            cache: Mutex::new(BTreeMap::new()),
        }
    }

    /// Eagerly parses one IDL and registers it under its primary program id
    /// and every `additionalPrograms` id, sharing a single parsed engine.
    /// The binding is unpinned (`digest_pinned == false`, origin `inline`).
    pub fn from_idl_json(
        json: &str,
        account_provider: Option<Arc<dyn Srf39AccountProvider>>,
        presentation: Option<Arc<dyn PresentationMetadataProvider>>,
    ) -> Result<Self, Srf39IdlError> {
        let engine = Arc::new(Srf39Engine::from_json(json)?);
        let sha256 = sha256_hex(json.as_bytes());
        let provenance = IdlProvenance {
            source_id: "inline".to_string(),
            origin: IdlOrigin::Other("inline".to_string()),
            expected_sha256_hex: None,
            version: None,
            reference: None,
        };
        let mut cache = BTreeMap::new();
        let mut program_ids = vec![engine.program_id().to_string()];
        program_ids.extend(engine.additional_program_ids());
        for program_id in program_ids {
            let loaded = LoadedIdl {
                engine: Arc::clone(&engine),
                binding: IdlBinding {
                    program_id: program_id.clone(),
                    program_name: engine.program_name().to_string(),
                    sha256_hex: sha256.clone(),
                    digest_pinned: false,
                    provenance: provenance.clone(),
                },
            };
            cache.insert(program_id, CacheEntry::Ready(Arc::new(loaded)));
        }
        Ok(Self {
            source: Arc::new(StaticIdlSource::empty()),
            account_provider,
            presentation: presentation.unwrap_or_else(|| Arc::new(EmptyPresentationProvider)),
            cache: Mutex::new(cache),
        })
    }

    pub async fn render(
        &self,
        instruction: &InstructionContext<'_>,
    ) -> Result<RenderOutcome, RenderFailure> {
        let program_id = instruction.program_id.to_string();
        let loaded = match self.load(&program_id).await? {
            Loaded::Ready(loaded) => loaded,
            Loaded::NotFound => {
                return Ok(RenderOutcome::Unsupported {
                    reason: UnsupportedReason::IdlNotFound { program_id },
                })
            }
        };
        let provider = self.account_provider.as_deref();
        let result = loaded
            .engine
            .display_instruction_detailed(instruction, provider)
            .await?;
        let (canonical, hints) = match result {
            DisplayResult::Rendered { display, hints } => (display, hints),
            DisplayResult::Miss(DisplayMiss::ProgramNotInIdl) => {
                return Err(RenderFailure::Internal {
                    detail: format!("cached IDL for {program_id} does not contain the program"),
                })
            }
            DisplayResult::Miss(DisplayMiss::InstructionNotIdentified) => {
                return Ok(RenderOutcome::Unsupported {
                    reason: UnsupportedReason::InstructionNotRecognized { program_id },
                })
            }
            DisplayResult::Miss(DisplayMiss::InstructionDecodeFailed { instruction }) => {
                return Ok(RenderOutcome::Unsupported {
                    reason: UnsupportedReason::InstructionDecodeFailed {
                        program_id,
                        instruction,
                    },
                })
            }
        };

        let mut diagnostics = Vec::new();
        if !loaded.binding.digest_pinned {
            diagnostics.push(Srf39DiagnosticKind::IdlDigestUnpinned.diagnostic());
        }
        let (presentation, enrichment_diagnostics) =
            enrich(&canonical, &hints, self.presentation.as_ref()).await;
        diagnostics.extend(enrichment_diagnostics);

        Ok(RenderOutcome::Rendered(Box::new(RenderedInstruction {
            canonical,
            idl: loaded.binding.clone(),
            hints,
            presentation,
            diagnostics,
        })))
    }

    /// Canonical display only. `provider_override` replaces the client's
    /// account provider for this call.
    pub async fn display(
        &self,
        instruction: &InstructionContext<'_>,
        provider_override: Option<&dyn Srf39AccountProvider>,
    ) -> Result<Option<InstructionDisplay>, RenderFailure> {
        let loaded = match self.load(instruction.program_id).await? {
            Loaded::Ready(loaded) => loaded,
            Loaded::NotFound => return Ok(None),
        };
        let provider = provider_override.or(self.account_provider.as_deref());
        loaded
            .engine
            .display_instruction(instruction, provider)
            .await
            .map_err(Into::into)
    }

    /// Parity with the reference `getRequiredAccountsForDisplay`. `None`
    /// when no IDL is known for the program or the instruction is not
    /// recognised.
    pub async fn required_accounts(
        &self,
        instruction: &InstructionContext<'_>,
    ) -> Result<Option<Vec<String>>, RenderFailure> {
        let loaded = match self.load(instruction.program_id).await? {
            Loaded::Ready(loaded) => loaded,
            Loaded::NotFound => return Ok(None),
        };
        Ok(loaded.engine.required_accounts_for_display(instruction))
    }

    /// Drops the cached entry for one program so the source is consulted again.
    pub fn invalidate(&self, program_id: &str) {
        self.lock_cache().remove(program_id);
    }

    pub fn invalidate_all(&self) {
        self.lock_cache().clear();
    }

    async fn load(&self, program_id: &str) -> Result<Loaded, RenderFailure> {
        validate_program_id(program_id)?;
        if let Some(entry) = self.lookup(program_id)? {
            return Ok(entry);
        }
        let resolved = self
            .source
            .idl_for_program(program_id)
            .await
            .map_err(|error| RenderFailure::IdlSourceFailed {
                detail: error.detail,
                retryable: error.retryable,
            })?;
        let entry = match resolved {
            IdlResolution::NotFound => CacheEntry::NotFound,
            IdlResolution::Found(resolved) => match verify_binding(program_id, &resolved) {
                Ok((engine, binding)) => CacheEntry::Ready(Arc::new(LoadedIdl {
                    engine: Arc::new(engine),
                    binding,
                })),
                Err(rejection) => CacheEntry::Rejected(rejection),
            },
        };
        let mut cache = self.lock_cache();
        if !cache.contains_key(program_id) {
            if matches!(entry, CacheEntry::NotFound) {
                let not_found = cache
                    .values()
                    .filter(|entry| matches!(entry, CacheEntry::NotFound))
                    .count();
                if not_found >= NOT_FOUND_CAP {
                    cache.retain(|_, entry| !matches!(entry, CacheEntry::NotFound));
                }
            }
            cache.insert(program_id.to_string(), entry);
        }
        drop(cache);
        self.lookup(program_id)?
            .ok_or_else(|| RenderFailure::Internal {
                detail: "cache entry vanished after insertion".to_string(),
            })
    }

    fn lookup(&self, program_id: &str) -> Result<Option<Loaded>, RenderFailure> {
        let cache = self.lock_cache();
        Ok(match cache.get(program_id) {
            Some(CacheEntry::Ready(loaded)) => Some(Loaded::Ready(Arc::clone(loaded))),
            Some(CacheEntry::NotFound) => Some(Loaded::NotFound),
            Some(CacheEntry::Rejected(rejection)) => {
                return Err(RenderFailure::IdlRejected {
                    program_id: program_id.to_string(),
                    rejection: rejection.clone(),
                })
            }
            None => None,
        })
    }

    fn lock_cache(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, CacheEntry>> {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn validate_program_id(program_id: &str) -> Result<(), RenderFailure> {
    let decoded = bs58::decode(program_id)
        .into_vec()
        .map_err(|_| RenderFailure::InvalidInput {
            detail: format!("program id '{program_id}' is not base58"),
        })?;
    if decoded.len() != 32 {
        return Err(RenderFailure::InvalidInput {
            detail: format!(
                "program id '{program_id}' decodes to {} bytes, expected 32",
                decoded.len()
            ),
        });
    }
    Ok(())
}
