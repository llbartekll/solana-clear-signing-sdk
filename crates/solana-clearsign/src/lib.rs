//! Native Solana clear-signing primitives.
//!
//! [`Srf39Engine`] is the native sRFC 39 path. It consumes an app-supplied IDL
//! and renders one raw Solana instruction as an [`InstructionDisplay`].
//!
//! The primary engine performs no I/O. Hosts can supply raw linked-account
//! bytes through [`Srf39AccountProvider`]. See `docs/architecture.md` in the
//! repository for the complete trust boundary and supported node subset.

mod provider;
mod srf39;

#[cfg(feature = "uniffi")]
pub mod uniffi_compat;

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

pub use provider::AccountData;
pub use provider::Srf39AccountProvider;
pub use srf39::{
    symbol_is_acceptable, AccountHint, AddressLabel, AmountHint, AnnotationKind, DecimalsSource,
    DiagnosticSeverity, DisplayField, DisplayMiss, DisplayResult, EmptyPresentationProvider,
    FieldAnnotation, FormatDiagnostic, IdlBinding, IdlOrigin, IdlProvenance, IdlRejection,
    IdlResolution, IdlSourceError, InstructionDisplay, InstructionDisplayHints, LinkedAccountRead,
    LinkedAccountStatus, PresentationMetadataProvider, PresentationOverlay, PresentedTokenAmount,
    PublicKeyArgumentHint, RenderFailure, RenderOutcome, RenderedInstruction, ResolvedSrf39Idl,
    Srf39Client, Srf39DisplayError, Srf39Engine, Srf39IdlError, Srf39IdlSource, StaticIdlSource,
    TimeDisplay, TimeHint, TokenHint, TokenMetadata, UnitSource, UnsupportedReason,
};

/// One compiled instruction, as the host sees it. Account order is
/// **positional and load-bearing**; trailing metas can be consumed by an IDL's
/// remaining-account declarations.
#[derive(Debug, Clone, Copy)]
pub struct InstructionContext<'a> {
    /// Base58 program id.
    pub program_id: &'a str,
    /// Full raw instruction data. The sRFC 39 path derives discriminator layout
    /// from the IDL.
    pub instruction_data: &'a [u8],
    pub accounts: &'a [AccountMeta<'a>],
    /// Reserved for future fee display; unused by MVP decode/render.
    pub fee_payer: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub struct AccountMeta<'a> {
    pub pubkey: &'a str,
    pub is_signer: bool,
    pub is_writable: bool,
}
