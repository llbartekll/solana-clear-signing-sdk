//! Diagnostics emitted by the high-level client. Codes are the stable
//! contract; messages are free text.

use super::text::escape_unsafe;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FormatDiagnostic {
    /// Stable, machine-readable code (snake_case). The contract.
    pub code: String,
    pub severity: DiagnosticSeverity,
    /// Human-readable text. May change between versions; never branch on it.
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Srf39DiagnosticKind {
    /// The source supplied no expected digest for the IDL.
    IdlDigestUnpinned,
    /// A linked account the display needed was not available.
    LinkedAccountUnavailable { address: String },
    /// A linked account's length contradicts the layout's size discriminator.
    LinkedAccountLayoutMismatch { address: String },
    /// An amount rendered raw because its scale could not be resolved.
    AmountScaleUnresolved { argument: String },
    /// The presentation provider supplied no token metadata for the address.
    TokenMetadataNotFound { address: String },
    /// The provider's symbol failed the character policy.
    TokenSymbolRejected { address: String },
    /// The provider's token name failed the text policy.
    TokenNameRejected { address: String },
    /// The provider's label or label source failed the text policy.
    AddressLabelRejected { address: String },
    /// The provider's decimals disagree with the on-chain scale.
    TokenRegistryDecimalsMismatch {
        address: String,
        registry: u8,
        on_chain: u8,
    },
    /// Callback decimals conflict with an explicit literal scale in the IDL.
    TokenAmountScaleConflict {
        address: String,
        metadata: u8,
        idl: u64,
    },
    /// The provider's token program disagrees with the fetched owner.
    TokenRegistryOwnerMismatch {
        address: String,
        registry: String,
        owner: String,
    },
    /// The IDL's own unit disagrees with the provider's symbol.
    UnitLiteralConflictsWithMetadata {
        address: String,
        unit: String,
        symbol: String,
    },
    /// The IDL has an interpolated sentence but it could not be rendered.
    InterpolatedIntentUnavailable,
    /// The sentence rendered, but an amount in the preview is presented at another scale.
    InterpolatedIntentScaleConflict,
    /// A field value contains control, bidi or invisible characters.
    UnsafeDisplayText { label: String },
}

impl Srf39DiagnosticKind {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::IdlDigestUnpinned => "idl_digest_unpinned",
            Self::LinkedAccountUnavailable { .. } => "linked_account_unavailable",
            Self::LinkedAccountLayoutMismatch { .. } => "linked_account_layout_mismatch",
            Self::TokenNameRejected { .. } => "token_name_rejected",
            Self::AddressLabelRejected { .. } => "address_label_rejected",
            Self::InterpolatedIntentScaleConflict => "interpolated_intent_scale_conflict",
            Self::UnsafeDisplayText { .. } => "unsafe_display_text",
            Self::AmountScaleUnresolved { .. } => "amount_scale_unresolved",
            Self::TokenMetadataNotFound { .. } => "token_metadata_not_found",
            Self::TokenSymbolRejected { .. } => "token_symbol_rejected",
            Self::TokenRegistryDecimalsMismatch { .. } => "token_registry_decimals_mismatch",
            Self::TokenAmountScaleConflict { .. } => "token_amount_scale_conflict",
            Self::TokenRegistryOwnerMismatch { .. } => "token_registry_owner_mismatch",
            Self::UnitLiteralConflictsWithMetadata { .. } => "unit_literal_conflicts_with_metadata",
            Self::InterpolatedIntentUnavailable => "interpolated_intent_unavailable",
        }
    }

    pub(crate) fn severity(&self) -> DiagnosticSeverity {
        match self {
            Self::InterpolatedIntentUnavailable => DiagnosticSeverity::Info,
            _ => DiagnosticSeverity::Warning,
        }
    }

    fn message(&self) -> String {
        match self {
            Self::IdlDigestUnpinned => {
                "the IDL source supplied no expected digest; provenance is unpinned".to_string()
            }
            Self::LinkedAccountUnavailable { address } => {
                format!("linked account {address} was not available to the renderer")
            }
            Self::LinkedAccountLayoutMismatch { address } => {
                format!("The data of linked account {address} does not have the size its layout declares.")
            }
            Self::TokenNameRejected { address } => {
                format!("The token name for {address} failed the text policy and was ignored.")
            }
            Self::AddressLabelRejected { address } => {
                format!("The label for {address} failed the text policy and was ignored.")
            }
            Self::InterpolatedIntentScaleConflict => {
                "The IDL summary may show an amount whose scale is contradicted or unconfirmed. Review the individual fields.".to_string()
            }
            Self::UnsafeDisplayText { label } => {
                format!("Field '{label}' contains control or invisible characters.")
            }
            Self::AmountScaleUnresolved { argument } => {
                format!("Amount '{argument}' has no usable scale. Its value is shown in base units (raw).")
            }
            Self::TokenMetadataNotFound { address } => {
                format!("No token metadata was supplied for address {address}.")
            }
            Self::TokenSymbolRejected { address } => {
                format!("token symbol for {address} failed the character policy and was ignored")
            }
            Self::TokenRegistryDecimalsMismatch {
                address,
                registry,
                on_chain,
            } => format!(
                "token metadata for {address} declares {registry} decimals but the account holds {on_chain}"
            ),
            Self::TokenRegistryOwnerMismatch {
                address,
                registry,
                owner,
            } => format!(
                "token metadata for {address} expects program {registry} but the account is owned by {owner}"
            ),
            Self::TokenAmountScaleConflict { address, metadata, idl } => format!(
                "token metadata for {address} declares {metadata} decimals but the IDL declares {idl}; the presented amount stays raw"
            ),
            Self::UnitLiteralConflictsWithMetadata {
                address,
                unit,
                symbol,
            } => format!(
                "the IDL declares unit '{unit}' for {address} but token metadata says '{symbol}'"
            ),
            Self::InterpolatedIntentUnavailable => {
                "The IDL summary could not be rendered. Review the individual fields.".to_string()
            }
        }
    }

    pub(crate) fn diagnostic(&self) -> FormatDiagnostic {
        FormatDiagnostic {
            code: self.code().to_string(),
            severity: self.severity(),
            message: escape_unsafe(&self.message()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_kinds() -> Vec<Srf39DiagnosticKind> {
        let address = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string();
        vec![
            Srf39DiagnosticKind::IdlDigestUnpinned,
            Srf39DiagnosticKind::LinkedAccountUnavailable {
                address: address.clone(),
            },
            Srf39DiagnosticKind::AmountScaleUnresolved {
                argument: "amount".into(),
            },
            Srf39DiagnosticKind::TokenMetadataNotFound {
                address: address.clone(),
            },
            Srf39DiagnosticKind::TokenSymbolRejected {
                address: address.clone(),
            },
            Srf39DiagnosticKind::TokenRegistryDecimalsMismatch {
                address: address.clone(),
                registry: 9,
                on_chain: 6,
            },
            Srf39DiagnosticKind::TokenAmountScaleConflict {
                address: address.clone(),
                metadata: 6,
                idl: 9,
            },
            Srf39DiagnosticKind::TokenRegistryOwnerMismatch {
                address: address.clone(),
                registry: "a".into(),
                owner: "b".into(),
            },
            Srf39DiagnosticKind::UnitLiteralConflictsWithMetadata {
                address: address.clone(),
                unit: "USDT".into(),
                symbol: "USDC".into(),
            },
            Srf39DiagnosticKind::InterpolatedIntentUnavailable,
            Srf39DiagnosticKind::LinkedAccountLayoutMismatch {
                address: address.clone(),
            },
            Srf39DiagnosticKind::TokenNameRejected {
                address: address.clone(),
            },
            Srf39DiagnosticKind::AddressLabelRejected { address },
            Srf39DiagnosticKind::InterpolatedIntentScaleConflict,
            Srf39DiagnosticKind::UnsafeDisplayText {
                label: "Memo".into(),
            },
        ]
    }

    #[test]
    fn codes_are_stable_and_message_independent() {
        let codes: Vec<&str> = all_kinds().iter().map(|kind| kind.code()).collect();
        assert_eq!(
            codes,
            [
                "idl_digest_unpinned",
                "linked_account_unavailable",
                "amount_scale_unresolved",
                "token_metadata_not_found",
                "token_symbol_rejected",
                "token_registry_decimals_mismatch",
                "token_amount_scale_conflict",
                "token_registry_owner_mismatch",
                "unit_literal_conflicts_with_metadata",
                "interpolated_intent_unavailable",
                "linked_account_layout_mismatch",
                "token_name_rejected",
                "address_label_rejected",
                "interpolated_intent_scale_conflict",
                "unsafe_display_text",
            ]
        );
        for kind in all_kinds() {
            let diagnostic = kind.diagnostic();
            assert_eq!(diagnostic.code, kind.code());
            assert!(!diagnostic.message.is_empty());
            assert!(diagnostic
                .code
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_'));
        }
    }

    #[test]
    fn only_the_suppressed_sentence_is_informational() {
        for kind in all_kinds() {
            let expected = if matches!(kind, Srf39DiagnosticKind::InterpolatedIntentUnavailable) {
                DiagnosticSeverity::Info
            } else {
                DiagnosticSeverity::Warning
            };
            assert_eq!(kind.severity(), expected, "{}", kind.code());
        }
    }
}
