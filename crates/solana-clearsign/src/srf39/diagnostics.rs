//! Diagnostics emitted by the high-level client. Codes are the stable
//! contract; messages are free text.

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
    /// An amount rendered raw because its scale could not be resolved.
    AmountScaleUnresolved { argument: String },
    /// The presentation provider knows nothing about the scale-source address.
    TokenMetadataNotFound { address: String },
    /// The provider's symbol failed the character policy.
    TokenSymbolRejected { address: String },
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
}

impl Srf39DiagnosticKind {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::IdlDigestUnpinned => "idl_digest_unpinned",
            Self::LinkedAccountUnavailable { .. } => "linked_account_unavailable",
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
            Self::AmountScaleUnresolved { argument } => {
                format!("amount '{argument}' is shown raw because its scale could not be resolved")
            }
            Self::TokenMetadataNotFound { address } => {
                format!("no token metadata for scale-source account {address}")
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
                "the IDL renders unit '{unit}' for {address} but token metadata says '{symbol}'"
            ),
            Self::InterpolatedIntentUnavailable => {
                "the interpolated sentence was suppressed; use the field list".to_string()
            }
        }
    }

    pub(crate) fn diagnostic(&self) -> FormatDiagnostic {
        FormatDiagnostic {
            code: self.code().to_string(),
            severity: self.severity(),
            message: self.message(),
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
