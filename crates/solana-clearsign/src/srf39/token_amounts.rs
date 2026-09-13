//! SDK token-metadata presentation for explicit IDL amount/mint bindings.
//! The strict display and its required-account planner do not use this path.

use std::collections::BTreeMap;

use super::diagnostics::{FormatDiagnostic, Srf39DiagnosticKind};
use super::display::scale_by_decimals;
use super::hints::{AmountHint, InstructionDisplayHints, TokenHint};
use super::presentation::{
    metadata_matches, AnnotationKind, FieldAnnotation, PresentationMetadataProvider,
    PresentedTokenAmount, TokenMetadata,
};

pub(crate) struct TokenAmountContext<'a> {
    pub hints: &'a InstructionDisplayHints,
    pub provider: &'a dyn PresentationMetadataProvider,
    pub token_cache: &'a mut BTreeMap<String, Option<TokenMetadata>>,
    pub annotations: &'a mut Vec<FieldAnnotation>,
    pub diagnostics: &'a mut Vec<FormatDiagnostic>,
}

pub(crate) async fn present(
    amount: &AmountHint,
    token: &TokenHint,
    field_index: usize,
    context: TokenAmountContext<'_>,
) -> PresentedTokenAmount {
    let TokenAmountContext {
        hints,
        provider,
        token_cache,
        annotations,
        diagnostics,
    } = context;
    let mut result = PresentedTokenAmount {
        field_index,
        value: format!("{} (raw)", amount.raw_value),
        mint: token.mint.clone(),
        decimals: None,
    };
    let Some(address) = &token.mint else {
        unresolved(amount, diagnostics);
        return result;
    };
    if !token_cache.contains_key(address) {
        token_cache.insert(address.clone(), provider.token_metadata(address).await);
    }
    let Some(metadata) = token_cache.get(address).and_then(Option::as_ref) else {
        diagnostics.push(
            Srf39DiagnosticKind::TokenMetadataNotFound {
                address: address.clone(),
            }
            .diagnostic(),
        );
        unresolved(amount, diagnostics);
        return result;
    };
    if !metadata_matches(amount, address, metadata, hints, diagnostics) {
        unresolved(amount, diagnostics);
        return result;
    }
    if let (Some(decimals), Ok(raw)) = (metadata.decimals, amount.raw_value.parse::<i128>()) {
        result.value = scale_by_decimals(raw, usize::from(decimals));
        result.decimals = Some(decimals);
    } else {
        unresolved(amount, diagnostics);
    }
    annotations.push(FieldAnnotation {
        field_index,
        kind: AnnotationKind::TokenAmount {
            symbol: metadata.symbol.clone(),
            name: metadata.name.clone(),
            source_address: address.clone(),
            applies_to_value: result.decimals.is_some(),
        },
    });
    // A matching mint field may be hidden by the IDL; the amount's resolution
    // never depends on that field being visible.
    let mint_fields = hints
        .accounts
        .iter()
        .filter(|hint| hint.address.as_ref() == Some(address))
        .filter_map(|hint| hint.field_index)
        .chain(
            hints
                .public_key_arguments
                .iter()
                .filter(|hint| hint.address == *address)
                .filter_map(|hint| hint.field_index),
        );
    for field_index in mint_fields {
        annotations.push(FieldAnnotation {
            field_index,
            kind: AnnotationKind::TokenMint {
                symbol: metadata.symbol.clone(),
                name: metadata.name.clone(),
            },
        });
    }
    result
}

fn unresolved(amount: &AmountHint, diagnostics: &mut Vec<FormatDiagnostic>) {
    diagnostics.push(
        Srf39DiagnosticKind::AmountScaleUnresolved {
            argument: amount.argument.clone(),
        }
        .diagnostic(),
    );
}
