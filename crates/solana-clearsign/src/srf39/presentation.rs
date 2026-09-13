//! Host token metadata and address labels, keyed by decoded addresses.
//!
//! Canonical output remains the pinned Codama result. Explicit IDL token
//! bindings may produce separately presented amounts using callback decimals.
//! Neither argument names nor labels are used to guess token relationships.

use std::collections::BTreeMap;

use super::diagnostics::FormatDiagnostic;
use super::diagnostics::Srf39DiagnosticKind;
use super::hints::{DecimalsSource, InstructionDisplayHints, LinkedAccountStatus, UnitSource};
use super::source::Fut;
use super::InstructionDisplay;

/// Host-curated token metadata for a mint address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenMetadata {
    pub symbol: String,
    pub name: Option<String>,
    /// Scales explicitly bound token amounts; otherwise only cross-checks the IDL scale.
    pub decimals: Option<u8>,
    /// Used only to cross-check the fetched account owner.
    pub token_program: Option<String>,
}

/// A human label for an address, with its source when known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressLabel {
    pub label: String,
    pub source: Option<String>,
}

/// The host side of presentation enrichment. Both methods default to "unknown".
pub trait PresentationMetadataProvider: Send + Sync {
    fn token_metadata<'a>(&'a self, mint: &'a str) -> Fut<'a, Option<TokenMetadata>> {
        let _ = mint;
        Box::pin(async { None })
    }

    fn address_label<'a>(&'a self, address: &'a str) -> Fut<'a, Option<AddressLabel>> {
        let _ = address;
        Box::pin(async { None })
    }
}

/// Provider that knows nothing. Every render then carries
/// `token_metadata_not_found` for each scale-source account.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyPresentationProvider;

impl PresentationMetadataProvider for EmptyPresentationProvider {}

/// Annotations laid over the canonical field list. Sorted by
/// `(field_index, kind order)` and deduplicated.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PresentationOverlay {
    pub annotations: Vec<FieldAnnotation>,
    /// Values for the IDL's explicit token bindings, in field order. Consumers
    /// use these in the preview and retain `canonical` for technical inspection.
    pub token_amounts: Vec<PresentedTokenAmount>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentedTokenAmount {
    pub field_index: usize,
    /// Numeric text, or `<integer> (raw)` when metadata is missing or conflicts.
    /// The symbol remains a separate annotation.
    pub value: String,
    pub mint: Option<String>,
    /// `None` means the presented value is raw. A resolved value uses callback
    /// decimals, after comparison with any explicit IDL/account scale.
    pub decimals: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldAnnotation {
    /// Index into `InstructionDisplay::fields`.
    pub field_index: usize,
    pub kind: AnnotationKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnnotationKind {
    /// The field is an amount whose scale came from `source_address`, which
    /// the provider recognises as a token.
    TokenAmount {
        symbol: String,
        name: Option<String>,
        source_address: String,
        /// Applies to the presented token amount when present, otherwise to
        /// the canonical value. `false` means the amount remains raw.
        applies_to_value: bool,
    },
    /// The field holds an address the provider knows as a token mint: the
    /// account that supplied an amount's scale (after the cross-checks above),
    /// any other rendered account, or a public-key argument.
    TokenMint {
        symbol: String,
        name: Option<String>,
    },
    /// A host label for the address in this field.
    AddressLabel {
        label: String,
        source: Option<String>,
    },
}

impl AnnotationKind {
    fn order(&self) -> u8 {
        match self {
            Self::TokenAmount { .. } => 0,
            Self::TokenMint { .. } => 1,
            Self::AddressLabel { .. } => 2,
        }
    }
}

/// Character policy for symbols shown next to amounts: 1..=16 characters,
/// first `[A-Za-z0-9]`, rest `[A-Za-z0-9$._-]`. ASCII-only by design so
/// homoglyph tickers are rejected rather than displayed.
pub fn symbol_is_acceptable(symbol: &str) -> bool {
    let mut characters = symbol.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if symbol.len() > 16 || !first.is_ascii_alphanumeric() {
        return false;
    }
    characters.all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '$' | '.' | '_' | '-')
    })
}

/// Applies the enrichment rules. Deterministic for equal inputs and equal
/// provider answers. Emits no `idl_digest_unpinned`: that needs the binding,
/// which the client owns.
pub(crate) async fn enrich(
    display: &InstructionDisplay,
    hints: &InstructionDisplayHints,
    provider: &dyn PresentationMetadataProvider,
) -> (PresentationOverlay, Vec<FormatDiagnostic>) {
    let mut diagnostics = Vec::new();
    let mut annotations = Vec::new();
    let mut token_amounts = Vec::new();
    let field_count = display.fields.len();

    for read in &hints.linked_account_reads {
        if read.status == LinkedAccountStatus::Missing {
            diagnostics.push(
                Srf39DiagnosticKind::LinkedAccountUnavailable {
                    address: read.address.clone(),
                }
                .diagnostic(),
            );
        }
    }

    let mut token_cache: BTreeMap<String, Option<TokenMetadata>> = BTreeMap::new();
    for amount in &hints.amounts {
        if let Some(token) = &amount.token {
            if let Some(field_index) = amount.field_index.filter(|index| *index < field_count) {
                let presented = super::token_amounts::present(
                    amount,
                    token,
                    field_index,
                    super::token_amounts::TokenAmountContext {
                        hints,
                        provider,
                        token_cache: &mut token_cache,
                        annotations: &mut annotations,
                        diagnostics: &mut diagnostics,
                    },
                )
                .await;
                token_amounts.push(presented);
            }
            continue;
        }
        if amount.degraded {
            diagnostics.push(
                Srf39DiagnosticKind::AmountScaleUnresolved {
                    argument: amount.argument.clone(),
                }
                .diagnostic(),
            );
        }
        let DecimalsSource::AccountField {
            account,
            address: Some(address),
            ..
        } = &amount.decimals
        else {
            continue;
        };
        if !token_cache.contains_key(address) {
            let metadata = provider.token_metadata(address).await;
            token_cache.insert(address.clone(), metadata);
        }
        let Some(metadata) = token_cache.get(address).and_then(Option::as_ref) else {
            diagnostics.push(
                Srf39DiagnosticKind::TokenMetadataNotFound {
                    address: address.clone(),
                }
                .diagnostic(),
            );
            continue;
        };
        if !metadata_matches(amount, address, metadata, hints, &mut diagnostics) {
            continue;
        }
        if let Some(field_index) = amount.field_index.filter(|index| *index < field_count) {
            annotations.push(FieldAnnotation {
                field_index,
                kind: AnnotationKind::TokenAmount {
                    symbol: metadata.symbol.clone(),
                    name: metadata.name.clone(),
                    source_address: address.clone(),
                    applies_to_value: !amount.degraded,
                },
            });
        }
        let mint_field = hints
            .accounts
            .iter()
            .find(|hint| hint.name == *account && hint.address.as_deref() == Some(address))
            .and_then(|hint| hint.field_index)
            .filter(|index| *index < field_count);
        if let Some(field_index) = mint_field {
            annotations.push(FieldAnnotation {
                field_index,
                kind: AnnotationKind::TokenMint {
                    symbol: metadata.symbol.clone(),
                    name: metadata.name.clone(),
                },
            });
        }
    }

    if hints.interpolated_intent_suppressed {
        diagnostics.push(Srf39DiagnosticKind::InterpolatedIntentUnavailable.diagnostic());
    }

    // Accounts that supplied an amount's scale are governed by the strict
    // rules above (which fail closed on any mismatch); every other rendered
    // account is an address the provider may simply recognise as a mint.
    let scale_sources: std::collections::BTreeSet<&str> = hints
        .amounts
        .iter()
        .filter_map(|amount| {
            if let Some(token) = &amount.token {
                token.mint.as_deref()
            } else {
                match &amount.decimals {
                    DecimalsSource::AccountField {
                        address: Some(address),
                        ..
                    } => Some(address.as_str()),
                    _ => None,
                }
            }
        })
        .collect();

    let mut label_cache: BTreeMap<String, Option<AddressLabel>> = BTreeMap::new();
    for hint in &hints.accounts {
        let (Some(field_index), Some(address)) = (hint.field_index, hint.address.as_ref()) else {
            continue;
        };
        if field_index >= field_count {
            continue;
        }
        if !label_cache.contains_key(address) {
            let label = provider.address_label(address).await;
            label_cache.insert(address.clone(), label);
        }
        if let Some(label) = label_cache.get(address).and_then(Option::as_ref) {
            annotations.push(FieldAnnotation {
                field_index,
                kind: AnnotationKind::AddressLabel {
                    label: label.label.clone(),
                    source: label.source.clone(),
                },
            });
        }
        if scale_sources.contains(address.as_str()) {
            continue;
        }
        if let Some(kind) =
            known_mint_annotation(provider, &mut token_cache, &mut diagnostics, address).await
        {
            annotations.push(FieldAnnotation { field_index, kind });
        }
    }

    // Public-key arguments are typed as addresses by the IDL, so they get the
    // same label lookup as account fields and, when the provider knows the
    // address as a mint, a `TokenMint` annotation. Nothing was fetched for
    // them, so no `token_metadata_not_found` and no on-chain cross-checks.
    for hint in &hints.public_key_arguments {
        let Some(field_index) = hint.field_index.filter(|index| *index < field_count) else {
            continue;
        };
        let address = &hint.address;
        if !label_cache.contains_key(address) {
            let label = provider.address_label(address).await;
            label_cache.insert(address.clone(), label);
        }
        if let Some(label) = label_cache.get(address).and_then(Option::as_ref) {
            annotations.push(FieldAnnotation {
                field_index,
                kind: AnnotationKind::AddressLabel {
                    label: label.label.clone(),
                    source: label.source.clone(),
                },
            });
        }
        if scale_sources.contains(address.as_str()) {
            continue;
        }
        if let Some(kind) =
            known_mint_annotation(provider, &mut token_cache, &mut diagnostics, address).await
        {
            annotations.push(FieldAnnotation { field_index, kind });
        }
    }

    annotations.sort_by(|a, b| {
        a.field_index
            .cmp(&b.field_index)
            .then(a.kind.order().cmp(&b.kind.order()))
    });
    annotations.dedup();

    token_amounts.sort_by_key(|amount| amount.field_index);
    (
        PresentationOverlay {
            annotations,
            token_amounts,
        },
        diagnostics,
    )
}

pub(crate) fn metadata_matches(
    amount: &super::hints::AmountHint,
    address: &String,
    metadata: &TokenMetadata,
    hints: &InstructionDisplayHints,
    diagnostics: &mut Vec<FormatDiagnostic>,
) -> bool {
    if !symbol_is_acceptable(&metadata.symbol) {
        diagnostics.push(
            Srf39DiagnosticKind::TokenSymbolRejected {
                address: address.clone(),
            }
            .diagnostic(),
        );
        return false;
    }
    let resolved = match &amount.decimals {
        DecimalsSource::AccountField { resolved, .. } => *resolved,
        _ => None,
    };
    if let (Some(registry), Some(on_chain)) = (metadata.decimals, resolved) {
        if registry != on_chain {
            diagnostics.push(
                Srf39DiagnosticKind::TokenRegistryDecimalsMismatch {
                    address: address.clone(),
                    registry,
                    on_chain,
                }
                .diagnostic(),
            );
            return false;
        }
    }
    if let Some(registry) = &metadata.token_program {
        let owner = hints
            .linked_account_reads
            .iter()
            .find(|read| read.address == *address)
            .and_then(|read| match &read.status {
                LinkedAccountStatus::Fetched { owner, .. } => Some(owner.clone()),
                LinkedAccountStatus::Missing => None,
            });
        if let Some(owner) = owner {
            if owner != *registry {
                diagnostics.push(
                    Srf39DiagnosticKind::TokenRegistryOwnerMismatch {
                        address: address.clone(),
                        registry: registry.clone(),
                        owner,
                    }
                    .diagnostic(),
                );
                return false;
            }
        }
    }
    let rendered_unit = match &amount.unit {
        UnitSource::Literal { value } => Some(value.clone()),
        UnitSource::AccountField {
            resolved: Some(value),
            ..
        } => Some(value.clone()),
        _ => None,
    };
    if let Some(unit) = rendered_unit {
        if unit != metadata.symbol {
            diagnostics.push(
                Srf39DiagnosticKind::UnitLiteralConflictsWithMetadata {
                    address: address.clone(),
                    unit,
                    symbol: metadata.symbol.clone(),
                }
                .diagnostic(),
            );
            return false;
        }
    }
    if let (Some(registry), DecimalsSource::Literal { value }) =
        (metadata.decimals, &amount.decimals)
    {
        if u64::from(registry) != *value {
            diagnostics.push(
                Srf39DiagnosticKind::TokenAmountScaleConflict {
                    address: address.clone(),
                    metadata: registry,
                    idl: *value,
                }
                .diagnostic(),
            );
            return false;
        }
    }
    true
}

/// A `TokenMint` annotation for an address the provider knows as a mint,
/// applying only the symbol policy: nothing was fetched for the address, so no
/// `token_metadata_not_found` and no on-chain cross-checks apply.
async fn known_mint_annotation(
    provider: &dyn PresentationMetadataProvider,
    token_cache: &mut BTreeMap<String, Option<TokenMetadata>>,
    diagnostics: &mut Vec<FormatDiagnostic>,
    address: &String,
) -> Option<AnnotationKind> {
    if !token_cache.contains_key(address) {
        let metadata = provider.token_metadata(address).await;
        token_cache.insert(address.clone(), metadata);
    }
    let metadata = token_cache.get(address).and_then(Option::as_ref)?;
    if !symbol_is_acceptable(&metadata.symbol) {
        diagnostics.push(
            Srf39DiagnosticKind::TokenSymbolRejected {
                address: address.clone(),
            }
            .diagnostic(),
        );
        return None;
    }
    Some(AnnotationKind::TokenMint {
        symbol: metadata.symbol.clone(),
        name: metadata.name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_policy_accepts_ordinary_tickers_and_rejects_spoofs() {
        for ok in ["USDC", "wSOL", "BTC.b", "USD-1", "A", "x_y$"] {
            assert!(symbol_is_acceptable(ok), "{ok}");
        }
        for bad in ["", "USDC ", " USDC", "USD\u{0421}", "-x", ".x", "$x"] {
            assert!(!symbol_is_acceptable(bad), "{bad:?}");
        }
        assert!(symbol_is_acceptable(&"A".repeat(16)));
        assert!(!symbol_is_acceptable(&"A".repeat(17)));
    }
}
