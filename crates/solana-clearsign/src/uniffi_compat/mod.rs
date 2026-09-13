//! UniFFI bridge for the sRFC 39 engine and high-level client.
//!
//! Hosts supply IDLs, raw account bytes and presentation metadata through
//! async callbacks. FFI records are converted at this boundary.

use std::sync::Arc;

use crate::provider::AccountData;
use crate::provider::Fut;
use crate::provider::Srf39AccountProvider;
use crate::DiagnosticSeverity;
use crate::FormatDiagnostic;
use crate::Srf39DisplayError;
use crate::Srf39Engine;
use crate::Srf39IdlError;
use crate::{
    AccountMeta, AddressLabel, AnnotationKind, DecimalsSource, IdlBinding, IdlOrigin,
    IdlProvenance, IdlRejection, IdlResolution, IdlSourceError, InstructionContext,
    InstructionDisplayHints, LinkedAccountStatus, PresentationMetadataProvider, RenderFailure,
    RenderOutcome, RenderedInstruction, ResolvedSrf39Idl, Srf39Client, Srf39IdlSource, TimeDisplay,
    TokenMetadata, UnitSource, UnsupportedReason,
};

#[derive(Debug, Clone, uniffi::Record)]
pub struct Srf39InstructionInputFfi {
    pub program_id: String,
    pub instruction_data: Vec<u8>,
    pub accounts: Vec<Srf39AccountMetaFfi>,
    pub fee_payer: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Srf39AccountMetaFfi {
    pub pubkey: String,
    pub is_signer: bool,
    pub is_writable: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Srf39AccountDataFfi {
    pub owner: String,
    pub data: Vec<u8>,
}

impl From<Srf39AccountDataFfi> for AccountData {
    fn from(value: Srf39AccountDataFfi) -> Self {
        Self {
            owner: value.owner,
            data: value.data,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Srf39DisplayFieldFfi {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Srf39InstructionDisplayFfi {
    pub intent: String,
    pub interpolated_intent: Option<String>,
    pub fields: Vec<Srf39DisplayFieldFfi>,
}

impl From<crate::InstructionDisplay> for Srf39InstructionDisplayFfi {
    fn from(value: crate::InstructionDisplay) -> Self {
        Self {
            intent: value.intent,
            interpolated_intent: value.interpolated_intent,
            fields: value
                .fields
                .into_iter()
                .map(|field| Srf39DisplayFieldFfi {
                    label: field.label,
                    value: field.value,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, thiserror::Error, uniffi::Error)]
pub enum Srf39IdlFailureFfi {
    #[error("invalid sRFC 39 IDL JSON: {detail}")]
    InvalidJson { detail: String },
    #[error("invalid sRFC 39 IDL root: {detail}")]
    InvalidRoot { detail: String },
    #[error("invalid sRFC 39 IDL schema: {detail}")]
    InvalidSchema { detail: String },
    #[error("unsupported sRFC 39 IDL node at {path}: {kind}")]
    UnsupportedIdlNode { path: String, kind: String },
}

impl From<Srf39IdlError> for Srf39IdlFailureFfi {
    fn from(value: Srf39IdlError) -> Self {
        match value {
            Srf39IdlError::InvalidJson { detail } => Self::InvalidJson { detail },
            Srf39IdlError::InvalidRoot { detail } => Self::InvalidRoot { detail },
            Srf39IdlError::InvalidSchema { detail } => Self::InvalidSchema { detail },
            Srf39IdlError::UnsupportedIdlNode { path, kind } => {
                Self::UnsupportedIdlNode { path, kind }
            }
        }
    }
}

#[derive(Debug, Clone, thiserror::Error, uniffi::Error)]
pub enum Srf39DisplayFailureFfi {
    #[error("could not decode linked account '{account}': {detail}")]
    AccountDecode { account: String, detail: String },
    #[error("sRFC 39 display invariant failed: {detail}")]
    Internal { detail: String },
}

impl From<Srf39DisplayError> for Srf39DisplayFailureFfi {
    fn from(value: Srf39DisplayError) -> Self {
        match value {
            Srf39DisplayError::AccountDecode { account, detail } => {
                Self::AccountDecode { account, detail }
            }
            Srf39DisplayError::Internal { detail } => Self::Internal { detail },
        }
    }
}

#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait Srf39AccountProviderFfi: Send + Sync {
    async fn resolve_account(&self, address: String) -> Option<Srf39AccountDataFfi>;
}

struct Srf39AccountProviderFfiProxy(Arc<dyn Srf39AccountProviderFfi>);

impl Srf39AccountProvider for Srf39AccountProviderFfiProxy {
    fn resolve_account<'a>(
        &'a self,
        address: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<AccountData>> + Send + 'a>> {
        let provider = Arc::clone(&self.0);
        let address = address.to_string();
        Box::pin(async move { provider.resolve_account(address).await.map(Into::into) })
    }
}

#[derive(Debug, uniffi::Object)]
pub struct Srf39EngineFfi {
    engine: Srf39Engine,
}

#[uniffi::export(async_runtime = "tokio")]
impl Srf39EngineFfi {
    #[uniffi::constructor]
    pub fn from_json(idl_json: String) -> Result<Arc<Self>, Srf39IdlFailureFfi> {
        let engine = Srf39Engine::from_json(&idl_json).map_err(Srf39IdlFailureFfi::from)?;
        Ok(Arc::new(Self { engine }))
    }

    pub async fn display_instruction(
        &self,
        instruction: Srf39InstructionInputFfi,
        account_provider: Option<Arc<dyn Srf39AccountProviderFfi>>,
    ) -> Result<Option<Srf39InstructionDisplayFfi>, Srf39DisplayFailureFfi> {
        let metas: Vec<AccountMeta<'_>> = instruction
            .accounts
            .iter()
            .map(|account| AccountMeta {
                pubkey: &account.pubkey,
                is_signer: account.is_signer,
                is_writable: account.is_writable,
            })
            .collect();
        let context = InstructionContext {
            program_id: &instruction.program_id,
            instruction_data: &instruction.instruction_data,
            accounts: &metas,
            fee_payer: instruction.fee_payer.as_deref(),
        };
        let provider = account_provider.map(Srf39AccountProviderFfiProxy);
        let provider = provider
            .as_ref()
            .map(|value| value as &dyn Srf39AccountProvider);

        self.engine
            .display_instruction(&context, provider)
            .await
            .map(|display| display.map(Into::into))
            .map_err(Into::into)
    }
}

// ---------------------------------------------------------------------------
// High-level client: IDL sources, presentation providers, rich outcomes.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, uniffi::Enum)]
pub enum IdlOriginFfi {
    Bundled,
    LocalFile,
    ProgramMetadataCanonical { authority: String },
    Other { detail: String },
}

impl From<IdlOriginFfi> for IdlOrigin {
    fn from(value: IdlOriginFfi) -> Self {
        match value {
            IdlOriginFfi::Bundled => Self::Bundled,
            IdlOriginFfi::LocalFile => Self::LocalFile,
            IdlOriginFfi::ProgramMetadataCanonical { authority } => {
                Self::ProgramMetadataCanonical { authority }
            }
            IdlOriginFfi::Other { detail } => Self::Other(detail),
        }
    }
}

impl From<IdlOrigin> for IdlOriginFfi {
    fn from(value: IdlOrigin) -> Self {
        match value {
            IdlOrigin::Bundled => Self::Bundled,
            IdlOrigin::LocalFile => Self::LocalFile,
            IdlOrigin::ProgramMetadataCanonical { authority } => {
                Self::ProgramMetadataCanonical { authority }
            }
            IdlOrigin::Other(detail) => Self::Other { detail },
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct IdlProvenanceFfi {
    pub source_id: String,
    pub origin: IdlOriginFfi,
    pub expected_sha256_hex: Option<String>,
    pub version: Option<String>,
    pub reference: Option<String>,
}

impl From<IdlProvenanceFfi> for IdlProvenance {
    fn from(value: IdlProvenanceFfi) -> Self {
        Self {
            source_id: value.source_id,
            origin: value.origin.into(),
            expected_sha256_hex: value.expected_sha256_hex,
            version: value.version,
            reference: value.reference,
        }
    }
}

impl From<IdlProvenance> for IdlProvenanceFfi {
    fn from(value: IdlProvenance) -> Self {
        Self {
            source_id: value.source_id,
            origin: value.origin.into(),
            expected_sha256_hex: value.expected_sha256_hex,
            version: value.version,
            reference: value.reference,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ResolvedSrf39IdlFfi {
    pub program_id: String,
    pub json: String,
    pub provenance: IdlProvenanceFfi,
}

impl From<ResolvedSrf39IdlFfi> for ResolvedSrf39Idl {
    fn from(value: ResolvedSrf39IdlFfi) -> Self {
        Self {
            program_id: value.program_id,
            json: value.json,
            provenance: value.provenance.into(),
        }
    }
}

/// The only error a foreign IDL source may raise. Any other foreign exception
/// is converted into `Failed { retryable: false }` instead of panicking.
#[derive(Debug, Clone, thiserror::Error, uniffi::Error)]
pub enum IdlSourceFailureFfi {
    #[error("IDL source failed: {detail}")]
    Failed { detail: String, retryable: bool },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for IdlSourceFailureFfi {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed {
            detail: error.reason,
            retryable: false,
        }
    }
}

#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait Srf39IdlSourceFfi: Send + Sync {
    /// `None` means the source knows no IDL for the program.
    async fn idl_for_program(
        &self,
        program_id: String,
    ) -> Result<Option<ResolvedSrf39IdlFfi>, IdlSourceFailureFfi>;
}

struct Srf39IdlSourceFfiProxy(Arc<dyn Srf39IdlSourceFfi>);

impl Srf39IdlSource for Srf39IdlSourceFfiProxy {
    fn idl_for_program<'a>(
        &'a self,
        program_id: &'a str,
    ) -> Fut<'a, Result<IdlResolution, IdlSourceError>> {
        let source = Arc::clone(&self.0);
        let program_id = program_id.to_string();
        Box::pin(async move {
            match source.idl_for_program(program_id).await {
                Ok(Some(resolved)) => Ok(IdlResolution::Found(resolved.into())),
                Ok(None) => Ok(IdlResolution::NotFound),
                Err(IdlSourceFailureFfi::Failed { detail, retryable }) => {
                    Err(IdlSourceError { detail, retryable })
                }
            }
        })
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TokenMetadataFfi {
    pub symbol: String,
    pub name: Option<String>,
    pub decimals: Option<u8>,
    pub token_program: Option<String>,
}

impl From<TokenMetadataFfi> for TokenMetadata {
    fn from(value: TokenMetadataFfi) -> Self {
        Self {
            symbol: value.symbol,
            name: value.name,
            decimals: value.decimals,
            token_program: value.token_program,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AddressLabelFfi {
    pub label: String,
    pub source: Option<String>,
}

impl From<AddressLabelFfi> for AddressLabel {
    fn from(value: AddressLabelFfi) -> Self {
        Self {
            label: value.label,
            source: value.source,
        }
    }
}

#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait PresentationMetadataProviderFfi: Send + Sync {
    async fn token_metadata(&self, mint: String) -> Option<TokenMetadataFfi>;
    async fn address_label(&self, address: String) -> Option<AddressLabelFfi>;
}

struct PresentationProviderFfiProxy(Arc<dyn PresentationMetadataProviderFfi>);

impl PresentationMetadataProvider for PresentationProviderFfiProxy {
    fn token_metadata<'a>(&'a self, mint: &'a str) -> Fut<'a, Option<TokenMetadata>> {
        let provider = Arc::clone(&self.0);
        let mint = mint.to_string();
        Box::pin(async move { provider.token_metadata(mint).await.map(Into::into) })
    }

    fn address_label<'a>(&'a self, address: &'a str) -> Fut<'a, Option<AddressLabel>> {
        let provider = Arc::clone(&self.0);
        let address = address.to_string();
        Box::pin(async move { provider.address_label(address).await.map(Into::into) })
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct IdlBindingFfi {
    pub program_id: String,
    pub program_name: String,
    pub sha256_hex: String,
    pub digest_pinned: bool,
    pub provenance: IdlProvenanceFfi,
}

impl From<IdlBinding> for IdlBindingFfi {
    fn from(value: IdlBinding) -> Self {
        Self {
            program_id: value.program_id,
            program_name: value.program_name,
            sha256_hex: value.sha256_hex,
            digest_pinned: value.digest_pinned,
            provenance: value.provenance.into(),
        }
    }
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum DecimalsSourceFfi {
    ImplicitZero,
    Literal {
        value: u64,
    },
    AccountField {
        account: String,
        address: Option<String>,
        linked_account: Option<String>,
        path: String,
        resolved: Option<u8>,
    },
    Unsatisfied,
}

impl From<DecimalsSource> for DecimalsSourceFfi {
    fn from(value: DecimalsSource) -> Self {
        match value {
            DecimalsSource::ImplicitZero => Self::ImplicitZero,
            DecimalsSource::Literal { value } => Self::Literal { value },
            DecimalsSource::AccountField {
                account,
                address,
                linked_account,
                path,
                resolved,
            } => Self::AccountField {
                account,
                address,
                linked_account,
                path,
                resolved,
            },
            DecimalsSource::Unsatisfied => Self::Unsatisfied,
        }
    }
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum UnitSourceFfi {
    Absent,
    Literal {
        value: String,
    },
    AccountField {
        account: String,
        address: Option<String>,
        linked_account: Option<String>,
        path: String,
        resolved: Option<String>,
    },
    Unsatisfied,
}

impl From<UnitSource> for UnitSourceFfi {
    fn from(value: UnitSource) -> Self {
        match value {
            UnitSource::None => Self::Absent,
            UnitSource::Literal { value } => Self::Literal { value },
            UnitSource::AccountField {
                account,
                address,
                linked_account,
                path,
                resolved,
            } => Self::AccountField {
                account,
                address,
                linked_account,
                path,
                resolved,
            },
            UnitSource::Unsatisfied => Self::Unsatisfied,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TokenHintFfi {
    pub mint: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AmountHintFfi {
    pub field_index: Option<u32>,
    pub argument: String,
    pub member: Option<String>,
    pub raw_value: String,
    pub degraded: bool,
    pub decimals: DecimalsSourceFfi,
    pub unit: UnitSourceFfi,
    pub token: Option<TokenHintFfi>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum TimeDisplayFfi {
    DateTime { ticks_per_second: u32 },
    Duration { ticks_per_second: u32 },
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TimeHintFfi {
    pub field_index: Option<u32>,
    pub argument: String,
    pub member: Option<String>,
    pub raw_value: String,
    pub display: TimeDisplayFfi,
    pub seconds: Option<i64>,
    pub formatted: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PublicKeyArgumentHintFfi {
    pub field_index: Option<u32>,
    pub argument: String,
    pub member: Option<String>,
    pub element: Option<u32>,
    pub address: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AccountHintFfi {
    pub field_index: Option<u32>,
    pub name: String,
    pub address: Option<String>,
    pub label: String,
    pub linked_account: Option<String>,
    pub consumed: bool,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum LinkedAccountStatusFfi {
    Fetched { owner: String, length: u32 },
    Missing,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct LinkedAccountReadFfi {
    pub address: String,
    pub status: LinkedAccountStatusFfi,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DisplayHintsFfi {
    pub instruction_name: String,
    pub interpolated_intent_suppressed: bool,
    pub amounts: Vec<AmountHintFfi>,
    pub times: Vec<TimeHintFfi>,
    pub public_key_arguments: Vec<PublicKeyArgumentHintFfi>,
    pub accounts: Vec<AccountHintFfi>,
    pub linked_account_reads: Vec<LinkedAccountReadFfi>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum AnnotationKindFfi {
    TokenAmount {
        symbol: String,
        name: Option<String>,
        source_address: String,
        applies_to_value: bool,
    },
    TokenMint {
        symbol: String,
        name: Option<String>,
    },
    AddressLabel {
        label: String,
        source: Option<String>,
    },
}

impl From<AnnotationKind> for AnnotationKindFfi {
    fn from(value: AnnotationKind) -> Self {
        match value {
            AnnotationKind::TokenAmount {
                symbol,
                name,
                source_address,
                applies_to_value,
            } => Self::TokenAmount {
                symbol,
                name,
                source_address,
                applies_to_value,
            },
            AnnotationKind::TokenMint { symbol, name } => Self::TokenMint { symbol, name },
            AnnotationKind::AddressLabel { label, source } => Self::AddressLabel { label, source },
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct FieldAnnotationFfi {
    pub field_index: u32,
    pub kind: AnnotationKindFfi,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PresentationOverlayFfi {
    pub annotations: Vec<FieldAnnotationFfi>,
    pub token_amounts: Vec<PresentedTokenAmountFfi>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PresentedTokenAmountFfi {
    pub field_index: u32,
    pub value: String,
    pub mint: Option<String>,
    pub decimals: Option<u8>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum DiagnosticSeverityFfi {
    Info,
    Warning,
}

impl From<DiagnosticSeverity> for DiagnosticSeverityFfi {
    fn from(value: DiagnosticSeverity) -> Self {
        match value {
            DiagnosticSeverity::Info => Self::Info,
            DiagnosticSeverity::Warning => Self::Warning,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct FormatDiagnosticFfi {
    pub code: String,
    pub severity: DiagnosticSeverityFfi,
    pub message: String,
}

impl From<FormatDiagnostic> for FormatDiagnosticFfi {
    fn from(value: FormatDiagnostic) -> Self {
        Self {
            code: value.code,
            severity: value.severity.into(),
            message: value.message,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct RenderedInstructionFfi {
    pub canonical: Srf39InstructionDisplayFfi,
    pub idl: IdlBindingFfi,
    pub hints: DisplayHintsFfi,
    pub presentation: PresentationOverlayFfi,
    pub diagnostics: Vec<FormatDiagnosticFfi>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum UnsupportedReasonFfi {
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

impl From<UnsupportedReason> for UnsupportedReasonFfi {
    fn from(value: UnsupportedReason) -> Self {
        match value {
            UnsupportedReason::IdlNotFound { program_id } => Self::IdlNotFound { program_id },
            UnsupportedReason::InstructionNotRecognized { program_id } => {
                Self::InstructionNotRecognized { program_id }
            }
            UnsupportedReason::InstructionDecodeFailed {
                program_id,
                instruction,
            } => Self::InstructionDecodeFailed {
                program_id,
                instruction,
            },
        }
    }
}

// FFI enums cannot box their payload; the size difference is irrelevant here
// because values are serialised across the boundary, never kept in Rust.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, uniffi::Enum)]
pub enum RenderOutcomeFfi {
    Rendered { instruction: RenderedInstructionFfi },
    Unsupported { reason: UnsupportedReasonFfi },
}

/// Parse-level rejection detail, flattened so it can sit inside an enum.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum IdlParseFailureFfi {
    InvalidJson { detail: String },
    InvalidRoot { detail: String },
    InvalidSchema { detail: String },
    UnsupportedIdlNode { path: String, kind: String },
}

impl From<Srf39IdlError> for IdlParseFailureFfi {
    fn from(value: Srf39IdlError) -> Self {
        match value {
            Srf39IdlError::InvalidJson { detail } => Self::InvalidJson { detail },
            Srf39IdlError::InvalidRoot { detail } => Self::InvalidRoot { detail },
            Srf39IdlError::InvalidSchema { detail } => Self::InvalidSchema { detail },
            Srf39IdlError::UnsupportedIdlNode { path, kind } => {
                Self::UnsupportedIdlNode { path, kind }
            }
        }
    }
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum IdlRejectionFfi {
    ProgramMismatch { requested: String, declared: String },
    DigestMismatch { expected: String, actual: String },
    Invalid { failure: IdlParseFailureFfi },
}

impl From<IdlRejection> for IdlRejectionFfi {
    fn from(value: IdlRejection) -> Self {
        match value {
            IdlRejection::ProgramMismatch {
                requested,
                declared,
            } => Self::ProgramMismatch {
                requested,
                declared,
            },
            IdlRejection::DigestMismatch { expected, actual } => {
                Self::DigestMismatch { expected, actual }
            }
            IdlRejection::Invalid(error) => Self::Invalid {
                failure: error.into(),
            },
        }
    }
}

#[derive(Debug, Clone, thiserror::Error, uniffi::Error)]
pub enum RenderFailureFfi {
    #[error("invalid input: {detail}")]
    InvalidInput { detail: String },
    #[error("IDL for {program_id} rejected")]
    IdlRejected {
        program_id: String,
        rejection: IdlRejectionFfi,
    },
    #[error("IDL source failed: {detail}")]
    IdlSourceFailed { detail: String, retryable: bool },
    #[error("could not decode linked account '{account}': {detail}")]
    AccountDecode { account: String, detail: String },
    #[error("internal error: {detail}")]
    Internal { detail: String },
}

impl From<RenderFailure> for RenderFailureFfi {
    fn from(value: RenderFailure) -> Self {
        match value {
            RenderFailure::InvalidInput { detail } => Self::InvalidInput { detail },
            RenderFailure::IdlRejected {
                program_id,
                rejection,
            } => Self::IdlRejected {
                program_id,
                rejection: rejection.into(),
            },
            RenderFailure::IdlSourceFailed { detail, retryable } => {
                Self::IdlSourceFailed { detail, retryable }
            }
            RenderFailure::AccountDecode { account, detail } => {
                Self::AccountDecode { account, detail }
            }
            RenderFailure::Internal { detail } => Self::Internal { detail },
        }
    }
}

fn index_to_u32(index: usize) -> Result<u32, RenderFailureFfi> {
    u32::try_from(index).map_err(|_| RenderFailureFfi::Internal {
        detail: format!("index {index} does not fit the FFI width"),
    })
}

fn convert_hints(hints: InstructionDisplayHints) -> Result<DisplayHintsFfi, RenderFailureFfi> {
    let mut amounts = Vec::with_capacity(hints.amounts.len());
    for amount in hints.amounts {
        amounts.push(AmountHintFfi {
            field_index: amount.field_index.map(index_to_u32).transpose()?,
            argument: amount.argument,
            member: amount.member,
            raw_value: amount.raw_value,
            degraded: amount.degraded,
            decimals: amount.decimals.into(),
            unit: amount.unit.into(),
            token: amount.token.map(|token| TokenHintFfi { mint: token.mint }),
        });
    }
    let mut times = Vec::with_capacity(hints.times.len());
    for time in hints.times {
        times.push(TimeHintFfi {
            field_index: time.field_index.map(index_to_u32).transpose()?,
            argument: time.argument,
            member: time.member,
            raw_value: time.raw_value,
            display: match time.display {
                TimeDisplay::DateTime { ticks_per_second } => {
                    TimeDisplayFfi::DateTime { ticks_per_second }
                }
                TimeDisplay::Duration { ticks_per_second } => {
                    TimeDisplayFfi::Duration { ticks_per_second }
                }
            },
            seconds: time.seconds,
            formatted: time.formatted,
        });
    }
    let mut public_key_arguments = Vec::with_capacity(hints.public_key_arguments.len());
    for hint in hints.public_key_arguments {
        public_key_arguments.push(PublicKeyArgumentHintFfi {
            field_index: hint.field_index.map(index_to_u32).transpose()?,
            argument: hint.argument,
            member: hint.member,
            element: hint.element.map(index_to_u32).transpose()?,
            address: hint.address,
        });
    }
    let mut accounts = Vec::with_capacity(hints.accounts.len());
    for account in hints.accounts {
        accounts.push(AccountHintFfi {
            field_index: account.field_index.map(index_to_u32).transpose()?,
            name: account.name,
            address: account.address,
            label: account.label,
            linked_account: account.linked_account,
            consumed: account.consumed,
        });
    }
    let mut linked_account_reads = Vec::with_capacity(hints.linked_account_reads.len());
    for read in hints.linked_account_reads {
        linked_account_reads.push(LinkedAccountReadFfi {
            address: read.address,
            status: match read.status {
                LinkedAccountStatus::Fetched { owner, length } => LinkedAccountStatusFfi::Fetched {
                    owner,
                    length: index_to_u32(length)?,
                },
                LinkedAccountStatus::Missing => LinkedAccountStatusFfi::Missing,
            },
        });
    }
    Ok(DisplayHintsFfi {
        instruction_name: hints.instruction_name,
        interpolated_intent_suppressed: hints.interpolated_intent_suppressed,
        amounts,
        times,
        public_key_arguments,
        accounts,
        linked_account_reads,
    })
}

fn convert_rendered(
    rendered: RenderedInstruction,
) -> Result<RenderedInstructionFfi, RenderFailureFfi> {
    let mut annotations = Vec::with_capacity(rendered.presentation.annotations.len());
    for annotation in rendered.presentation.annotations {
        annotations.push(FieldAnnotationFfi {
            field_index: index_to_u32(annotation.field_index)?,
            kind: annotation.kind.into(),
        });
    }
    let token_amounts = rendered
        .presentation
        .token_amounts
        .into_iter()
        .map(|amount| {
            Ok(PresentedTokenAmountFfi {
                field_index: index_to_u32(amount.field_index)?,
                value: amount.value,
                mint: amount.mint,
                decimals: amount.decimals,
            })
        })
        .collect::<Result<Vec<_>, RenderFailureFfi>>()?;
    Ok(RenderedInstructionFfi {
        canonical: rendered.canonical.into(),
        idl: rendered.idl.into(),
        hints: convert_hints(rendered.hints)?,
        presentation: PresentationOverlayFfi {
            annotations,
            token_amounts,
        },
        diagnostics: rendered.diagnostics.into_iter().map(Into::into).collect(),
    })
}

fn build_account_provider(
    provider: Option<Arc<dyn Srf39AccountProviderFfi>>,
) -> Option<Arc<dyn Srf39AccountProvider>> {
    provider.map(|provider| {
        let proxy: Arc<dyn Srf39AccountProvider> = Arc::new(Srf39AccountProviderFfiProxy(provider));
        proxy
    })
}

fn build_presentation_provider(
    provider: Option<Arc<dyn PresentationMetadataProviderFfi>>,
) -> Option<Arc<dyn PresentationMetadataProvider>> {
    provider.map(|provider| {
        let proxy: Arc<dyn PresentationMetadataProvider> =
            Arc::new(PresentationProviderFfiProxy(provider));
        proxy
    })
}

/// The high-level client. One instance owns the engine cache; create one per
/// IDL source and share it.
#[derive(uniffi::Object)]
pub struct Srf39ClientFfi {
    inner: Srf39Client,
}

#[uniffi::export(async_runtime = "tokio")]
impl Srf39ClientFfi {
    #[uniffi::constructor]
    pub fn new(
        idl_source: Arc<dyn Srf39IdlSourceFfi>,
        account_provider: Option<Arc<dyn Srf39AccountProviderFfi>>,
        presentation_provider: Option<Arc<dyn PresentationMetadataProviderFfi>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Srf39Client::new(
                Arc::new(Srf39IdlSourceFfiProxy(idl_source)),
                build_account_provider(account_provider),
                build_presentation_provider(presentation_provider),
            ),
        })
    }

    /// Eagerly parses one inline IDL; errors exactly like `Srf39EngineFfi::from_json`.
    #[uniffi::constructor]
    pub fn from_idl_json(
        idl_json: String,
        account_provider: Option<Arc<dyn Srf39AccountProviderFfi>>,
        presentation_provider: Option<Arc<dyn PresentationMetadataProviderFfi>>,
    ) -> Result<Arc<Self>, Srf39IdlFailureFfi> {
        let inner = Srf39Client::from_idl_json(
            &idl_json,
            build_account_provider(account_provider),
            build_presentation_provider(presentation_provider),
        )
        .map_err(Srf39IdlFailureFfi::from)?;
        Ok(Arc::new(Self { inner }))
    }

    pub async fn render(
        &self,
        instruction: Srf39InstructionInputFfi,
    ) -> Result<RenderOutcomeFfi, RenderFailureFfi> {
        let metas = ffi_metas(&instruction);
        let context = ffi_context(&instruction, &metas);
        match self
            .inner
            .render(&context)
            .await
            .map_err(RenderFailureFfi::from)?
        {
            RenderOutcome::Rendered(rendered) => Ok(RenderOutcomeFfi::Rendered {
                instruction: convert_rendered(*rendered)?,
            }),
            RenderOutcome::Unsupported { reason } => Ok(RenderOutcomeFfi::Unsupported {
                reason: reason.into(),
            }),
        }
    }

    pub async fn display(
        &self,
        instruction: Srf39InstructionInputFfi,
        account_provider: Option<Arc<dyn Srf39AccountProviderFfi>>,
    ) -> Result<Option<Srf39InstructionDisplayFfi>, RenderFailureFfi> {
        let metas = ffi_metas(&instruction);
        let context = ffi_context(&instruction, &metas);
        let provider = account_provider.map(Srf39AccountProviderFfiProxy);
        let provider = provider
            .as_ref()
            .map(|value| value as &dyn Srf39AccountProvider);
        self.inner
            .display(&context, provider)
            .await
            .map(|display| display.map(Into::into))
            .map_err(Into::into)
    }

    pub async fn required_accounts(
        &self,
        instruction: Srf39InstructionInputFfi,
    ) -> Result<Option<Vec<String>>, RenderFailureFfi> {
        let metas = ffi_metas(&instruction);
        let context = ffi_context(&instruction, &metas);
        self.inner
            .required_accounts(&context)
            .await
            .map_err(Into::into)
    }

    pub fn invalidate(&self, program_id: String) {
        self.inner.invalidate(&program_id);
    }

    pub fn invalidate_all(&self) {
        self.inner.invalidate_all();
    }
}

fn ffi_metas(instruction: &Srf39InstructionInputFfi) -> Vec<AccountMeta<'_>> {
    instruction
        .accounts
        .iter()
        .map(|account| AccountMeta {
            pubkey: &account.pubkey,
            is_signer: account.is_signer,
            is_writable: account.is_writable,
        })
        .collect()
}

fn ffi_context<'a>(
    instruction: &'a Srf39InstructionInputFfi,
    metas: &'a [AccountMeta<'a>],
) -> InstructionContext<'a> {
    InstructionContext {
        program_id: &instruction.program_id,
        instruction_data: &instruction.instruction_data,
        accounts: metas,
        fee_payer: instruction.fee_payer.as_deref(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The high-level object driven entirely through foreign-style trait
    /// objects, as Swift would supply them.
    struct FixtureIdlSource {
        json: String,
    }
    #[async_trait::async_trait]
    impl Srf39IdlSourceFfi for FixtureIdlSource {
        async fn idl_for_program(
            &self,
            program_id: String,
        ) -> Result<Option<ResolvedSrf39IdlFfi>, IdlSourceFailureFfi> {
            if program_id != "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" {
                return Ok(None);
            }
            Ok(Some(ResolvedSrf39IdlFfi {
                program_id,
                json: self.json.clone(),
                provenance: IdlProvenanceFfi {
                    source_id: "test".to_string(),
                    origin: IdlOriginFfi::Bundled,
                    expected_sha256_hex: Some(
                        "472f41c79165064ba7bd7cd8623d0b730a227b6bfc12faca77dc8e4702b7bdc1"
                            .to_string(),
                    ),
                    version: None,
                    reference: None,
                },
            }))
        }
    }

    struct FailingIdlSource;
    #[async_trait::async_trait]
    impl Srf39IdlSourceFfi for FailingIdlSource {
        async fn idl_for_program(
            &self,
            _program_id: String,
        ) -> Result<Option<ResolvedSrf39IdlFfi>, IdlSourceFailureFfi> {
            Err(IdlSourceFailureFfi::Failed {
                detail: "boom".to_string(),
                retryable: true,
            })
        }
    }

    struct UsdcOnly;
    #[async_trait::async_trait]
    impl PresentationMetadataProviderFfi for UsdcOnly {
        async fn token_metadata(&self, mint: String) -> Option<TokenMetadataFfi> {
            (mint == "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").then(|| TokenMetadataFfi {
                symbol: "USDC".to_string(),
                name: Some("USD Coin".to_string()),
                decimals: Some(6),
                token_program: Some("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string()),
            })
        }
        async fn address_label(&self, _address: String) -> Option<AddressLabelFfi> {
            None
        }
    }

    struct MintBytes(Vec<u8>);
    #[async_trait::async_trait]
    impl Srf39AccountProviderFfi for MintBytes {
        async fn resolve_account(&self, address: String) -> Option<Srf39AccountDataFfi> {
            (address == "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").then(|| {
                Srf39AccountDataFfi {
                    owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
                    data: self.0.clone(),
                }
            })
        }
    }

    fn spl_token_transfer() -> (String, Srf39InstructionInputFfi, Vec<u8>) {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../conformance/fixtures/spl-token-instructions");
        let root = std::fs::read_to_string(dir.join("root.json")).expect("root");
        let cases: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("cases.json")).expect("cases"))
                .expect("cases json");
        let scenario = cases["scenarios"]
            .as_array()
            .expect("scenarios")
            .iter()
            .find(|scenario| scenario["name"] == "transferChecked-online")
            .expect("scenario");
        let accounts = scenario["accounts"]
            .as_array()
            .expect("accounts")
            .iter()
            .map(|account| Srf39AccountMetaFfi {
                pubkey: account["address"].as_str().expect("address").to_string(),
                is_signer: account["role"].as_str().expect("role").contains("Signer"),
                is_writable: account["role"]
                    .as_str()
                    .expect("role")
                    .starts_with("writable"),
            })
            .collect();
        let data = STANDARD
            .decode(scenario["dataBase64"].as_str().expect("data"))
            .expect("base64");
        let account_data = if scenario["accountData"].is_object() {
            &scenario["accountData"]
        } else {
            &cases["accountData"]
        };
        let mint = STANDARD
            .decode(
                account_data["EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"]["dataBase64"]
                    .as_str()
                    .expect("mint data"),
            )
            .expect("mint base64");
        (
            root,
            Srf39InstructionInputFfi {
                program_id: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
                instruction_data: data,
                accounts,
                fee_payer: None,
            },
            mint,
        )
    }

    #[tokio::test]
    async fn high_level_client_renders_through_foreign_traits() {
        let (root, input, mint) = spl_token_transfer();
        let client = Srf39ClientFfi::new(
            Arc::new(FixtureIdlSource { json: root }),
            Some(Arc::new(MintBytes(mint))),
            Some(Arc::new(UsdcOnly)),
        );
        let outcome = client.render(input.clone()).await.expect("render");
        let RenderOutcomeFfi::Rendered { instruction } = outcome else {
            panic!("expected rendered, got {outcome:?}");
        };
        assert_eq!(instruction.canonical.fields[0].value, "0.001469");
        assert!(instruction.idl.digest_pinned);
        assert!(instruction.diagnostics.is_empty());
        assert!(matches!(
            &instruction.presentation.annotations[0].kind,
            AnnotationKindFfi::TokenAmount { symbol, applies_to_value: true, .. } if symbol == "USDC"
        ));
        assert!(matches!(
            &instruction.hints.amounts[0].decimals,
            DecimalsSourceFfi::AccountField {
                resolved: Some(6),
                ..
            }
        ));
        assert_eq!(
            client
                .required_accounts(input.clone())
                .await
                .expect("required"),
            Some(vec![
                "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string()
            ])
        );
        assert!(client
            .display(input.clone(), None)
            .await
            .expect("display")
            .is_some());

        let mut foreign = input;
        foreign.program_id = "11111111111111111111111111111111".to_string();
        assert!(matches!(
            client.render(foreign).await.expect("render"),
            RenderOutcomeFfi::Unsupported {
                reason: UnsupportedReasonFfi::IdlNotFound { .. }
            }
        ));
    }

    #[tokio::test]
    async fn high_level_client_surfaces_source_failures() {
        let (_, input, _) = spl_token_transfer();
        let client = Srf39ClientFfi::new(Arc::new(FailingIdlSource), None, None);
        assert!(matches!(
            client.render(input).await,
            Err(RenderFailureFfi::IdlSourceFailed {
                retryable: true,
                ..
            })
        ));
    }
}
