//! Enrichment rules, isolation from the canonical output, and determinism.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

use crate::provider::Srf39AccountProvider;
use crate::InstructionContext;

use super::client::{RenderOutcome, Srf39Client};
use super::conformance_tests::{fixture_provider, load_fixture, OwnedInstruction};
use super::presentation::{
    AddressLabel, AnnotationKind, PresentationMetadataProvider, TokenMetadata,
};
use super::source::{Fut, IdlOrigin, IdlProvenance, ResolvedSrf39Idl, StaticIdlSource};
use super::Srf39Engine;
use super::UnsupportedReason;

const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

const FIXTURES: &[&str] = &[
    "default-display",
    "transfer-checked",
    "spl-token-transfer-checked",
    "spl-token-instructions",
    "struct-argument",
    "inert-nodes",
    "defined-types",
    "signed-numbers",
    "date-time-display",
    "duration-display",
    "enum-display",
    "fixed-string",
    "fixed-array",
    "cross-program-link",
    "flatten-struct",
    "subscriptions",
];

#[derive(Default)]
struct MapProvider {
    tokens: BTreeMap<String, TokenMetadata>,
    labels: BTreeMap<String, AddressLabel>,
    token_calls: Mutex<Vec<String>>,
    label_calls: Mutex<Vec<String>>,
}

impl PresentationMetadataProvider for MapProvider {
    fn token_metadata<'a>(&'a self, mint: &'a str) -> Fut<'a, Option<TokenMetadata>> {
        self.token_calls
            .lock()
            .expect("lock")
            .push(mint.to_string());
        let metadata = self.tokens.get(mint).cloned();
        Box::pin(async move { metadata })
    }

    fn address_label<'a>(&'a self, address: &'a str) -> Fut<'a, Option<AddressLabel>> {
        self.label_calls
            .lock()
            .expect("lock")
            .push(address.to_string());
        let label = self.labels.get(address).cloned();
        Box::pin(async move { label })
    }
}

/// Answers every lookup, to prove the canonical output ignores the provider.
struct AdversarialProvider;

impl PresentationMetadataProvider for AdversarialProvider {
    fn token_metadata<'a>(&'a self, mint: &'a str) -> Fut<'a, Option<TokenMetadata>> {
        let metadata = TokenMetadata {
            symbol: "EVIL".to_string(),
            name: Some(format!("Evil token {mint}")),
            decimals: None,
            token_program: None,
        };
        Box::pin(async move { Some(metadata) })
    }

    fn address_label<'a>(&'a self, address: &'a str) -> Fut<'a, Option<AddressLabel>> {
        let label = AddressLabel {
            label: format!("label for {address}"),
            source: Some("adversary".to_string()),
        };
        Box::pin(async move { Some(label) })
    }
}

fn usdc(decimals: Option<u8>, token_program: Option<&str>) -> TokenMetadata {
    TokenMetadata {
        symbol: "USDC".to_string(),
        name: Some("USD Coin".to_string()),
        decimals,
        token_program: token_program.map(str::to_string),
    }
}

fn client_for(
    fixture_name: &str,
    provider: Arc<dyn PresentationMetadataProvider>,
) -> (Srf39Client, super::conformance_tests::Fixture) {
    let (root_json, fixture, _) = load_fixture(fixture_name);
    let engine = Srf39Engine::from_json(&root_json).expect("engine");
    let idl = ResolvedSrf39Idl {
        program_id: engine.program_id().to_string(),
        json: root_json,
        provenance: IdlProvenance {
            source_id: "test".to_string(),
            origin: IdlOrigin::Bundled,
            expected_sha256_hex: Some(super::binding::sha256_hex(
                std::fs::read(
                    super::conformance_tests::fixture_directory(fixture_name).join("root.json"),
                )
                .expect("root bytes")
                .as_slice(),
            )),
            version: None,
            reference: None,
        },
    };
    let scenario_provider = fixture_provider(&fixture.account_data);
    let client = Srf39Client::new(
        Arc::new(StaticIdlSource::new(vec![idl])),
        Some(Arc::new(scenario_provider)),
        Some(provider),
    );
    (client, fixture)
}

fn render_scenario(
    fixture_name: &str,
    scenario_name: &str,
    provider: Arc<dyn PresentationMetadataProvider>,
) -> RenderOutcome {
    let (root_json, fixture, _) = load_fixture(fixture_name);
    let engine = Srf39Engine::from_json(&root_json).expect("engine");
    let scenario = fixture
        .scenarios
        .iter()
        .find(|scenario| scenario.name == scenario_name)
        .unwrap_or_else(|| panic!("scenario {scenario_name}"));
    let owned = OwnedInstruction::from_scenario(&fixture, scenario);
    let account_provider: Option<Arc<dyn Srf39AccountProvider>> =
        scenario.fetch_accounts.then(|| {
            Arc::new(fixture_provider(
                scenario
                    .account_data
                    .as_ref()
                    .unwrap_or(&fixture.account_data),
            )) as Arc<dyn Srf39AccountProvider>
        });
    let idl = ResolvedSrf39Idl {
        program_id: engine.program_id().to_string(),
        json: root_json,
        provenance: IdlProvenance {
            source_id: "test".to_string(),
            origin: IdlOrigin::Bundled,
            expected_sha256_hex: None,
            version: None,
            reference: None,
        },
    };
    let client = Srf39Client::new(
        Arc::new(StaticIdlSource::new(vec![idl])),
        account_provider,
        Some(provider),
    );
    let metas = owned.metas();
    let instruction = InstructionContext {
        program_id: &owned.program_id,
        instruction_data: &owned.data,
        accounts: &metas,
        fee_payer: None,
    };
    pollster::block_on(client.render(&instruction)).expect("render")
}

fn codes(outcome: &RenderOutcome) -> Vec<String> {
    let RenderOutcome::Rendered(rendered) = outcome else {
        panic!("expected rendered, got {outcome:?}");
    };
    rendered
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code != "idl_digest_unpinned")
        .map(|diagnostic| diagnostic.code.clone())
        .collect()
}

fn token_annotations(outcome: &RenderOutcome) -> Vec<AnnotationKind> {
    let RenderOutcome::Rendered(rendered) = outcome else {
        panic!("expected rendered, got {outcome:?}");
    };
    rendered
        .presentation
        .annotations
        .iter()
        .filter(|annotation| !matches!(annotation.kind, AnnotationKind::AddressLabel { .. }))
        .map(|annotation| annotation.kind.clone())
        .collect()
}

fn provider_with(metadata: Option<TokenMetadata>) -> Arc<dyn PresentationMetadataProvider> {
    let mut provider = MapProvider::default();
    if let Some(metadata) = metadata {
        provider.tokens.insert(USDC.to_string(), metadata);
    }
    Arc::new(provider)
}

#[test]
fn unknown_token_yields_no_annotation_and_a_warning() {
    let outcome = render_scenario(
        "spl-token-instructions",
        "transferChecked-online",
        provider_with(None),
    );
    assert_eq!(codes(&outcome), ["token_metadata_not_found"]);
    assert!(token_annotations(&outcome).is_empty());
}

#[test]
fn known_token_annotates_amount_and_mint() {
    let outcome = render_scenario(
        "spl-token-instructions",
        "transferChecked-online",
        provider_with(Some(usdc(Some(6), Some(TOKEN_PROGRAM)))),
    );
    assert!(codes(&outcome).is_empty());
    assert_eq!(
        token_annotations(&outcome),
        vec![
            AnnotationKind::TokenAmount {
                symbol: "USDC".to_string(),
                name: Some("USD Coin".to_string()),
                source_address: USDC.to_string(),
                applies_to_value: true,
            },
            AnnotationKind::TokenMint {
                symbol: "USDC".to_string(),
                name: Some("USD Coin".to_string()),
            },
        ]
    );
    let RenderOutcome::Rendered(rendered) = &outcome else {
        unreachable!()
    };
    assert_eq!(rendered.presentation.annotations[0].field_index, 0);
    assert_eq!(rendered.presentation.annotations[1].field_index, 1);
}

#[test]
fn metadata_without_optional_cross_checks_still_annotates() {
    let outcome = render_scenario(
        "spl-token-instructions",
        "transferChecked-online",
        provider_with(Some(usdc(None, None))),
    );
    assert!(codes(&outcome).is_empty());
    assert!(matches!(
        &token_annotations(&outcome)[0],
        AnnotationKind::TokenAmount { symbol, applies_to_value: true, .. } if symbol == "USDC"
    ));
}

#[test]
fn rejected_symbol_is_treated_as_unknown() {
    let mut metadata = usdc(None, None);
    metadata.symbol = "USD\u{0421}".to_string();
    let outcome = render_scenario(
        "spl-token-instructions",
        "transferChecked-online",
        provider_with(Some(metadata)),
    );
    assert_eq!(codes(&outcome), ["token_symbol_rejected"]);
    assert!(token_annotations(&outcome).is_empty());
}

#[test]
fn registry_decimals_mismatch_suppresses_the_annotation() {
    let outcome = render_scenario(
        "spl-token-instructions",
        "transferChecked-online",
        provider_with(Some(usdc(Some(9), None))),
    );
    assert_eq!(codes(&outcome), ["token_registry_decimals_mismatch"]);
    assert!(token_annotations(&outcome).is_empty());
}

#[test]
fn registry_owner_mismatch_suppresses_the_annotation() {
    let outcome = render_scenario(
        "spl-token-instructions",
        "transferChecked-online",
        provider_with(Some(usdc(None, Some("11111111111111111111111111111111")))),
    );
    assert_eq!(codes(&outcome), ["token_registry_owner_mismatch"]);
    assert!(token_annotations(&outcome).is_empty());
}

#[test]
fn owner_check_is_skipped_when_the_account_was_not_fetched() {
    let outcome = render_scenario(
        "spl-token-instructions",
        "transferChecked-offline",
        provider_with(Some(usdc(
            Some(6),
            Some("11111111111111111111111111111111"),
        ))),
    );
    assert_eq!(
        codes(&outcome),
        [
            "linked_account_unavailable",
            "amount_scale_unresolved",
            "interpolated_intent_unavailable",
        ]
    );
    assert!(matches!(
        &token_annotations(&outcome)[0],
        AnnotationKind::TokenAmount {
            applies_to_value: false,
            ..
        }
    ));
}

#[test]
fn idl_unit_conflicting_with_metadata_fails_closed() {
    // The synthetic fixture's mint is not USDC; key the provider by that mint.
    const SYNTHETIC_MINT: &str = "86xCnPeV69n6t3DnyGvkKobf9FdN2H9oiVDdaMpo2MMY";
    let provider_for = |metadata: TokenMetadata| -> Arc<dyn PresentationMetadataProvider> {
        let mut provider = MapProvider::default();
        provider.tokens.insert(SYNTHETIC_MINT.to_string(), metadata);
        Arc::new(provider)
    };
    let mut metadata = usdc(None, None);
    metadata.symbol = "USDT".to_string();
    let outcome = render_scenario("transfer-checked", "online", provider_for(metadata));
    assert_eq!(codes(&outcome), ["unit_literal_conflicts_with_metadata"]);
    assert!(token_annotations(&outcome).is_empty());

    let agreeing = render_scenario("transfer-checked", "online", provider_for(usdc(None, None)));
    assert!(codes(&agreeing).is_empty());
    // The mint field is hidden (`whenInjected`), so only the amount is annotated.
    assert_eq!(token_annotations(&agreeing).len(), 1);
}

#[test]
fn labels_are_applied_to_named_and_remaining_account_fields() {
    let mut provider = MapProvider::default();
    provider.labels.insert(
        "2V47kNnc5hpvPDuZjVKvktfZnPdk5Dac96BZkLJDYNsR".to_string(),
        AddressLabel {
            label: "Alice".to_string(),
            source: Some("address-book".to_string()),
        },
    );
    let outcome = render_scenario(
        "spl-token-instructions",
        "transferChecked-online",
        Arc::new(provider),
    );
    let RenderOutcome::Rendered(rendered) = &outcome else {
        unreachable!()
    };
    let labels: Vec<_> = rendered
        .presentation
        .annotations
        .iter()
        .filter(|annotation| matches!(annotation.kind, AnnotationKind::AddressLabel { .. }))
        .collect();
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].field_index, 2);

    let outcome = render_scenario(
        "spl-token-transfer-checked",
        "multisig",
        Arc::new(AdversarialProvider),
    );
    let RenderOutcome::Rendered(rendered) = &outcome else {
        unreachable!()
    };
    let labelled: Vec<usize> = rendered
        .presentation
        .annotations
        .iter()
        .filter(|annotation| matches!(annotation.kind, AnnotationKind::AddressLabel { .. }))
        .map(|annotation| annotation.field_index)
        .collect();
    assert_eq!(labelled, vec![1, 2, 3, 4]);
    assert_eq!(rendered.canonical.fields.len(), 5);
}

#[test]
fn canonical_output_is_independent_of_the_presentation_provider() {
    for fixture_name in FIXTURES {
        let (root_json, fixture, expected) = load_fixture(fixture_name);
        let engine = Srf39Engine::from_json(&root_json).expect("engine");
        for scenario in &fixture.scenarios {
            if scenario.expected_error.is_some() {
                continue;
            }
            let outcome =
                render_scenario(fixture_name, &scenario.name, Arc::new(AdversarialProvider));
            let owned = OwnedInstruction::from_scenario(&fixture, scenario);
            let metas = owned.metas();
            let instruction = InstructionContext {
                program_id: &owned.program_id,
                instruction_data: &owned.data,
                accounts: &metas,
                fee_payer: None,
            };
            let provider = fixture_provider(
                scenario
                    .account_data
                    .as_ref()
                    .unwrap_or(&fixture.account_data),
            );
            let source = scenario
                .fetch_accounts
                .then_some(&provider as &dyn Srf39AccountProvider);
            let direct = pollster::block_on(engine.display_instruction(&instruction, source))
                .expect("direct render");
            match (&outcome, direct) {
                (RenderOutcome::Rendered(rendered), Some(display)) => {
                    assert_eq!(
                        rendered.canonical, display,
                        "{fixture_name}/{}",
                        scenario.name
                    );
                    assert_eq!(
                        serde_json::to_value(&rendered.canonical).expect("serialize"),
                        expected[&scenario.name],
                        "{fixture_name}/{}",
                        scenario.name
                    );
                    for field in &rendered.canonical.fields {
                        assert!(
                            !field.value.contains("EVIL"),
                            "{fixture_name}/{}",
                            scenario.name
                        );
                        assert!(
                            !field.value.contains("label for"),
                            "{fixture_name}/{}",
                            scenario.name
                        );
                    }
                }
                (RenderOutcome::Unsupported { .. }, None) => {}
                // A source-backed client binds an IDL to its primary program;
                // instructions addressed to an additional program are only
                // reachable through `from_idl_json`, so the engine renders
                // them while the client reports the IDL as not found.
                (
                    RenderOutcome::Unsupported {
                        reason: UnsupportedReason::IdlNotFound { .. },
                    },
                    Some(_),
                ) if scenario
                    .program_address
                    .as_deref()
                    .is_some_and(|address| address != fixture.program_address) => {}
                (outcome, direct) => panic!(
                    "{fixture_name}/{}: client {outcome:?} vs engine {direct:?}",
                    scenario.name
                ),
            }
        }
    }
}

#[test]
fn provider_is_asked_only_about_hinted_addresses_and_once_each() {
    let provider = Arc::new(MapProvider::default());
    let (client, fixture) = client_for("spl-token-transfer-checked", provider.clone());
    let scenario = fixture
        .scenarios
        .iter()
        .find(|scenario| scenario.name == "multisig")
        .expect("multisig");
    let owned = OwnedInstruction::from_scenario(&fixture, scenario);
    let metas = owned.metas();
    let instruction = InstructionContext {
        program_id: &owned.program_id,
        instruction_data: &owned.data,
        accounts: &metas,
        fee_payer: None,
    };
    let RenderOutcome::Rendered(rendered) =
        pollster::block_on(client.render(&instruction)).expect("render")
    else {
        panic!("rendered");
    };
    let mut expected_labels: Vec<String> = rendered
        .hints
        .accounts
        .iter()
        .filter(|hint| hint.field_index.is_some())
        .filter_map(|hint| hint.address.clone())
        .collect();
    expected_labels.dedup();
    assert_eq!(*provider.label_calls.lock().expect("lock"), expected_labels);
    assert_eq!(expected_labels.len(), 4);
    // The scale account is looked up first; every other rendered account is
    // then asked about once, in field order.
    let mut expected_tokens = vec![USDC.to_string()];
    expected_tokens.extend(
        expected_labels
            .iter()
            .filter(|address| *address != USDC)
            .cloned(),
    );
    assert_eq!(*provider.token_calls.lock().expect("lock"), expected_tokens);
}

#[test]
fn public_key_arguments_get_labels_and_mint_annotations_without_not_found() {
    const A1: &str = "3Wnd5Df69KitZfUoPYZU438eFRNwGHkhLnSAWL65PxJX";
    const A2: &str = "86xCnPeV69n6t3DnyGvkKobf9FdN2H9oiVDdaMpo2MMY";
    let mut provider = MapProvider::default();
    provider.tokens.insert(
        A1.to_string(),
        TokenMetadata {
            symbol: "TKN".to_string(),
            name: Some("Token".to_string()),
            decimals: None,
            token_program: None,
        },
    );
    provider.labels.insert(
        A2.to_string(),
        AddressLabel {
            label: "Treasury".to_string(),
            source: Some("test".to_string()),
        },
    );
    let provider = Arc::new(provider);
    let outcome = render_scenario("fixed-array", "expanded", provider.clone());
    let RenderOutcome::Rendered(rendered) = &outcome else {
        panic!("rendered");
    };
    let addresses: Vec<(Option<usize>, Option<usize>, &str)> = rendered
        .hints
        .public_key_arguments
        .iter()
        .map(|hint| (hint.field_index, hint.element, hint.address.as_str()))
        .collect();
    assert_eq!(
        addresses,
        vec![
            (Some(0), Some(0), A1),
            (Some(1), Some(1), A2),
            (
                Some(2),
                Some(2),
                "SysvarRent111111111111111111111111111111111"
            ),
            (
                Some(3),
                Some(3),
                "Vote111111111111111111111111111111111111111"
            ),
            (Some(4), Some(0), A1),
            (
                Some(7),
                Some(0),
                "SysvarRent111111111111111111111111111111111"
            ),
            (
                Some(8),
                Some(1),
                "Vote111111111111111111111111111111111111111"
            ),
            (Some(9), Some(0), A1),
        ],
        "the absent optional element and non-address arguments produce no hint"
    );
    let mint_fields: Vec<usize> = rendered
        .presentation
        .annotations
        .iter()
        .filter(|annotation| matches!(annotation.kind, AnnotationKind::TokenMint { .. }))
        .map(|annotation| annotation.field_index)
        .collect();
    assert_eq!(mint_fields, vec![0, 4, 9]);
    let labels: Vec<(usize, String)> = rendered
        .presentation
        .annotations
        .iter()
        .filter_map(|annotation| match &annotation.kind {
            AnnotationKind::AddressLabel { label, .. } => {
                Some((annotation.field_index, label.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(labels, vec![(1, "Treasury".to_string())]);
    assert_eq!(
        codes(&outcome),
        Vec::<String>::new(),
        "no not-found for arguments"
    );
    let unique: Vec<String> = vec![
        A1.to_string(),
        A2.to_string(),
        "SysvarRent111111111111111111111111111111111".to_string(),
        "Vote111111111111111111111111111111111111111".to_string(),
    ];
    assert_eq!(*provider.token_calls.lock().expect("lock"), unique);
    assert_eq!(*provider.label_calls.lock().expect("lock"), unique);
}

#[test]
fn subscriptions_mint_addresses_are_annotated_without_scale_lookups() {
    const USDC_DEVNET: &str = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";
    let mut provider = MapProvider::default();
    provider.tokens.insert(
        USDC_DEVNET.to_string(),
        usdc(Some(6), Some("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")),
    );
    let provider = Arc::new(provider);

    // `expectedMint` is a public-key argument.
    let outcome = render_scenario("subscriptions", "subscribe-usdc", provider.clone());
    let RenderOutcome::Rendered(rendered) = &outcome else {
        panic!("rendered");
    };
    let mint_fields: Vec<usize> = rendered
        .presentation
        .annotations
        .iter()
        .filter(|annotation| matches!(annotation.kind, AnnotationKind::TokenMint { .. }))
        .map(|annotation| annotation.field_index)
        .collect();
    assert_eq!(mint_fields, vec![1], "the Token mint argument field");
    assert_eq!(rendered.canonical.fields[1].label, "Token mint");
    assert_eq!(codes(&outcome), vec!["interpolated_intent_unavailable"]);

    // `tokenMint` is a plain account with no link and no amount to scale.
    let outcome = render_scenario(
        "subscriptions",
        "initSubscriptionAuthority",
        provider.clone(),
    );
    let RenderOutcome::Rendered(rendered) = &outcome else {
        panic!("rendered");
    };
    assert_eq!(
        token_annotations(&outcome),
        vec![AnnotationKind::TokenMint {
            symbol: "USDC".to_string(),
            name: Some("USD Coin".to_string()),
        }]
    );
    let mint_field = rendered
        .hints
        .accounts
        .iter()
        .find(|account| account.name == "tokenMint")
        .and_then(|account| account.field_index)
        .expect("visible token mint");
    assert_eq!(rendered.canonical.fields[mint_field].label, "Token mint");
    assert_eq!(codes(&outcome), Vec::<String>::new());
    assert!(rendered.hints.amounts.is_empty());
}

#[test]
fn subscriptions_transfers_scale_from_mint_bytes_and_cross_check_metadata() {
    const DEVNET_MINT: &str = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";
    let mut provider = MapProvider::default();
    provider
        .tokens
        .insert(DEVNET_MINT.to_string(), usdc(Some(6), Some(TOKEN_PROGRAM)));
    let provider = Arc::new(provider);
    for (scenario, expected) in [
        ("transferFixed-online", "5"),
        ("transferRecurring-online", "5"),
        ("transferSubscription-online", "5"),
        ("transferRecurring-devnet", "0.1"),
        ("transferRecurring-max-u64", "18446744073709.551615"),
        ("transferRecurring-zero", "0"),
    ] {
        let outcome = render_scenario("subscriptions", scenario, provider.clone());
        let RenderOutcome::Rendered(rendered) = &outcome else {
            panic!("expected rendered for {scenario}");
        };
        assert_eq!(rendered.canonical.fields[0].value, expected, "{scenario}");
        assert!(matches!(
            &rendered.hints.amounts[0].decimals,
            super::hints::DecimalsSource::AccountField { address: Some(address), resolved: Some(6), .. }
                if address == DEVNET_MINT
        ));
        assert!(rendered.presentation.annotations.iter().any(|annotation| {
            annotation.field_index == 0
                && matches!(&annotation.kind, AnnotationKind::TokenAmount { symbol, applies_to_value: true, .. } if symbol == "USDC")
        }));
        assert!(
            codes(&outcome).is_empty(),
            "{scenario}: {:?}",
            codes(&outcome)
        );
    }

    for (scenario, value, diagnostic) in [
        (
            "transferRecurring-amount-raw",
            "5000000 (raw)",
            "amount_scale_unresolved",
        ),
        (
            "transferRecurring-missing-mint",
            "5000000 (raw)",
            "amount_scale_unresolved",
        ),
        (
            "transferRecurring-nine-decimals",
            "0.005",
            "token_registry_decimals_mismatch",
        ),
        (
            "transferRecurring-foreign-owner",
            "5",
            "token_registry_owner_mismatch",
        ),
    ] {
        let outcome = render_scenario("subscriptions", scenario, provider.clone());
        let RenderOutcome::Rendered(rendered) = &outcome else {
            panic!("expected rendered for {scenario}");
        };
        assert_eq!(rendered.canonical.fields[0].value, value, "{scenario}");
        assert!(
            codes(&outcome).iter().any(|code| code == diagnostic),
            "{scenario}"
        );
        assert!(
            !rendered.presentation.annotations.iter().any(|annotation| {
                matches!(
                    annotation.kind,
                    AnnotationKind::TokenAmount {
                        applies_to_value: true,
                        ..
                    }
                )
            }),
            "{scenario}: do not attach a symbol to an amount with unresolved or conflicting metadata"
        );
    }
}

#[test]
fn rendering_twice_is_deterministic() {
    let first = render_scenario(
        "spl-token-transfer-checked",
        "multisig",
        Arc::new(AdversarialProvider),
    );
    let second = render_scenario(
        "spl-token-transfer-checked",
        "multisig",
        Arc::new(AdversarialProvider),
    );
    assert_eq!(first, second);
}

#[test]
fn duplicate_annotations_collapse_when_two_amounts_share_a_scale_account() {
    let json = r#"
    {
      "kind": "rootNode",
      "standard": "codama",
      "version": "1.8.0",
      "program": {
        "kind": "programNode",
        "name": "twin",
        "publicKey": "11111111111111111111111111111111",
        "accounts": [{
          "kind": "accountNode",
          "name": "mint",
          "data": { "kind": "structTypeNode", "fields": [
            { "kind": "structFieldTypeNode", "name": "decimals", "type": { "kind": "numberTypeNode", "format": "u8", "endian": "le" } }
          ] }
        }],
        "instructions": [{
          "kind": "instructionNode",
          "name": "pay",
          "accounts": [{
            "kind": "instructionAccountNode",
            "name": "mint",
            "isWritable": false,
            "isSigner": false,
            "accountLink": { "kind": "accountLinkNode", "name": "mint" },
            "display": { "kind": "instructionAccountDisplayNode", "label": "Mint" }
          }],
          "arguments": [
            {
              "kind": "instructionArgumentNode",
              "name": "discriminator",
              "type": { "kind": "numberTypeNode", "format": "u8", "endian": "le" },
              "defaultValue": { "kind": "numberValueNode", "number": 7 },
              "display": { "kind": "structFieldDisplayNode", "skip": "always" }
            },
            {
              "kind": "instructionArgumentNode",
              "name": "principal",
              "type": { "kind": "numberTypeNode", "format": "u64", "endian": "le",
                "display": { "kind": "amountNumberDisplayNode", "decimals": { "kind": "injectedValueNode", "key": "decimals" } } }
            },
            {
              "kind": "instructionArgumentNode",
              "name": "fee",
              "type": { "kind": "numberTypeNode", "format": "u64", "endian": "le",
                "display": { "kind": "amountNumberDisplayNode", "decimals": { "kind": "injectedValueNode", "key": "decimals" } } }
            }
          ],
          "provides": [{ "kind": "providedNode", "name": "decimals", "node": { "kind": "accountFieldValueNode", "account": "mint", "path": "decimals" } }],
          "discriminators": [{ "kind": "fieldDiscriminatorNode", "name": "discriminator", "offset": 0 }]
        }]
      }
    }
    "#;
    struct Mint;
    impl crate::provider::Srf39AccountProvider for Mint {
        fn resolve_account<'a>(
            &'a self,
            _address: &'a str,
        ) -> crate::provider::Fut<'a, Option<crate::provider::AccountData>> {
            Box::pin(async {
                Some(crate::provider::AccountData {
                    owner: "owner".to_string(),
                    data: vec![2],
                })
            })
        }
    }
    let presentation = Arc::new(AdversarialProvider);
    let client =
        Srf39Client::from_idl_json(json, Some(Arc::new(Mint)), Some(presentation)).expect("client");
    let mut data = vec![7u8];
    data.extend_from_slice(&1_000u64.to_le_bytes());
    data.extend_from_slice(&25u64.to_le_bytes());
    let metas = [crate::AccountMeta {
        pubkey: USDC,
        is_signer: false,
        is_writable: false,
    }];
    let instruction = InstructionContext {
        program_id: "11111111111111111111111111111111",
        instruction_data: &data,
        accounts: &metas,
        fee_payer: None,
    };
    let RenderOutcome::Rendered(rendered) =
        pollster::block_on(client.render(&instruction)).expect("render")
    else {
        panic!("rendered");
    };
    assert_eq!(rendered.canonical.fields[0].value, "10");
    assert_eq!(rendered.canonical.fields[1].value, "0.25");
    let kinds: Vec<u8> = rendered
        .presentation
        .annotations
        .iter()
        .map(|annotation| match annotation.kind {
            AnnotationKind::TokenAmount { .. } => 0,
            AnnotationKind::TokenMint { .. } => 1,
            AnnotationKind::AddressLabel { .. } => 2,
        })
        .collect();
    // amount, amount, then exactly one mint annotation plus its label.
    assert_eq!(kinds, vec![0, 0, 1, 2]);
    assert_eq!(rendered.hints.linked_account_reads.len(), 1);
}

// A different program and arbitrary argument names prove that metadata resolution
// follows the explicit IDL binding, not Subscriptions or USDC conventions.
fn metadata_root() -> serde_json::Value {
    serde_json::json!({
        "kind": "rootNode", "standard": "codama", "version": "1.0.0",
        "program": { "kind": "programNode", "name": "billing", "publicKey": "11111111111111111111111111111111",
            "instructions": [{ "kind": "instructionNode", "name": "pay",
                "accounts": [{"kind": "instructionAccountNode", "name": "assetAccount"}],
                "arguments": [
                    {"kind": "instructionArgumentNode", "name": "quantity", "type": {
                        "kind": "numberTypeNode", "format": "u64", "endian": "le",
                        "display": {"kind": "amountNumberDisplayNode", "decimals": {"kind": "injectedValueNode", "key": "scale"}}
                    }},
                    {"kind": "instructionArgumentNode", "name": "asset", "type": {"kind": "publicKeyTypeNode"}}
                ],
                "discriminators": [{"kind": "sizeDiscriminatorNode", "size": 40}],
                "display": {"kind": "instructionDisplayNode", "x-solana-clearsign": {"tokenAmounts": [
                    {"amount": "quantity", "mint": {"source": "argument", "name": "asset"}}
                ]}}
            }]
        }
    })
}

fn render_metadata_amount(
    root: &serde_json::Value,
    raw: u64,
    mint: &str,
    accounts: &[crate::AccountMeta<'_>],
    provider: Arc<MapProvider>,
) -> super::RenderedInstruction {
    let client = Srf39Client::from_idl_json(&root.to_string(), None, Some(provider)).expect("IDL");
    let mut data = raw.to_le_bytes().to_vec();
    data.extend(bs58::decode(mint).into_vec().expect("mint"));
    let instruction = InstructionContext {
        program_id: "11111111111111111111111111111111",
        instruction_data: &data,
        accounts,
        fee_payer: None,
    };
    let RenderOutcome::Rendered(result) =
        pollster::block_on(client.render(&instruction)).expect("render")
    else {
        panic!("expected rendered");
    };
    *result
}

fn metadata_provider(mint: &str, decimals: Option<u8>) -> Arc<MapProvider> {
    let mut provider = MapProvider::default();
    provider.tokens.insert(mint.into(), usdc(decimals, None));
    Arc::new(provider)
}

#[test]
fn explicit_token_binding_scales_exact_integers_and_fetches_metadata_once() {
    for (raw, decimals, expected) in [
        (9_990_000, 6, "9.99"),
        (9_990_000, 9, "0.00999"),
        (42, 0, "42"),
        (0, 6, "0"),
        (1, 6, "0.000001"),
        (u64::MAX, 6, "18446744073709.551615"),
    ] {
        let provider = metadata_provider(USDC, Some(decimals));
        let rendered = render_metadata_amount(&metadata_root(), raw, USDC, &[], provider.clone());
        assert_eq!(rendered.canonical.fields[0].value, format!("{raw} (raw)"));
        assert_eq!(rendered.presentation.token_amounts[0].value, expected);
        assert_eq!(
            rendered.presentation.token_amounts[0].decimals,
            Some(decimals)
        );
        assert_eq!(
            rendered.presentation.token_amounts[0].mint.as_deref(),
            Some(USDC)
        );
        assert!(!rendered
            .diagnostics
            .iter()
            .any(|d| d.code == "amount_scale_unresolved"));
        assert_eq!(*provider.token_calls.lock().expect("calls"), [USDC]);
        assert!(rendered.hints.linked_account_reads.is_empty());
    }
}

#[test]
fn callback_metadata_never_bleeds_between_mints_or_renders() {
    const OTHER: &str = "So11111111111111111111111111111111111111112";
    let mut provider = MapProvider::default();
    provider.tokens.insert(USDC.into(), usdc(Some(6), None));
    provider.tokens.insert(
        OTHER.into(),
        TokenMetadata {
            symbol: "wSOL".into(),
            name: None,
            decimals: Some(9),
            token_program: None,
        },
    );
    let provider = Arc::new(provider);
    for (mint, value, symbol) in [(USDC, "9.99", "USDC"), (OTHER, "0.00999", "wSOL")] {
        let result =
            render_metadata_amount(&metadata_root(), 9_990_000, mint, &[], provider.clone());
        assert_eq!(result.presentation.token_amounts[0].value, value);
        assert!(result.presentation.annotations.iter().any(|annotation| matches!(
            &annotation.kind, AnnotationKind::TokenAmount { symbol: actual, applies_to_value: true, .. } if actual == symbol
        )));
    }
    let unknown = render_metadata_amount(&metadata_root(), 9_990_000, TOKEN_PROGRAM, &[], provider);
    assert_eq!(unknown.presentation.token_amounts[0].value, "9990000 (raw)");
    assert_eq!(unknown.presentation.token_amounts[0].decimals, None);
    assert!(unknown
        .diagnostics
        .iter()
        .any(|d| d.code == "token_metadata_not_found"));
}

#[test]
fn missing_decimals_and_rejected_symbols_leave_bound_amounts_raw() {
    for symbol in ["USDC", "USD\u{0421}"] {
        let mut provider = MapProvider::default();
        provider.tokens.insert(
            USDC.into(),
            TokenMetadata {
                symbol: symbol.into(),
                name: None,
                decimals: (symbol != "USDC").then_some(6),
                token_program: None,
            },
        );
        let rendered =
            render_metadata_amount(&metadata_root(), 9_990_000, USDC, &[], Arc::new(provider));
        assert_eq!(
            rendered.presentation.token_amounts[0].value,
            "9990000 (raw)"
        );
        assert_eq!(rendered.presentation.token_amounts[0].decimals, None);
        assert!(rendered
            .diagnostics
            .iter()
            .any(|d| d.code == "amount_scale_unresolved"));
        assert!(!rendered.presentation.annotations.iter().any(|a| matches!(
            a.kind,
            AnnotationKind::TokenAmount {
                applies_to_value: true,
                ..
            }
        )));
    }
}

#[test]
fn explicit_binding_is_required_even_when_a_mint_is_recognised() {
    let mut root = metadata_root();
    root["program"]["instructions"][0]["display"]
        .as_object_mut()
        .expect("display")
        .remove("x-solana-clearsign");
    let result = render_metadata_amount(
        &root,
        9_990_000,
        USDC,
        &[],
        metadata_provider(USDC, Some(6)),
    );
    assert_eq!(result.canonical.fields[0].value, "9990000 (raw)");
    assert!(result.presentation.token_amounts.is_empty());
    assert!(!result
        .presentation
        .annotations
        .iter()
        .any(|a| matches!(a.kind, AnnotationKind::TokenAmount { .. })));
}

#[test]
fn metadata_conflicting_with_an_idl_scale_or_unit_produces_raw_presentation() {
    for (change, code) in [
        (
            serde_json::json!({"kind":"amountNumberDisplayNode","decimals":{"kind":"numberValueNode","number":9}}),
            "token_amount_scale_conflict",
        ),
        (
            serde_json::json!({"kind":"amountNumberDisplayNode","unit":{"kind":"stringValueNode","string":"USDT"}}),
            "unit_literal_conflicts_with_metadata",
        ),
    ] {
        let mut root = metadata_root();
        root["program"]["instructions"][0]["arguments"][0]["type"]["display"] = change;
        let result = render_metadata_amount(
            &root,
            9_990_000,
            USDC,
            &[],
            metadata_provider(USDC, Some(6)),
        );
        assert_eq!(result.presentation.token_amounts[0].value, "9990000 (raw)");
        assert_eq!(result.presentation.token_amounts[0].decimals, None);
        assert!(result.diagnostics.iter().any(|d| d.code == code));
        assert!(
            result.presentation.annotations.is_empty(),
            "mint field must not bypass the failed check"
        );
    }
    let mut root = metadata_root();
    root["program"]["instructions"][0]["arguments"][0]["type"]["display"]["decimals"] =
        serde_json::json!({"kind":"numberValueNode","number":6});
    let result = render_metadata_amount(
        &root,
        9_990_000,
        USDC,
        &[],
        metadata_provider(USDC, Some(6)),
    );
    assert_eq!(result.canonical.fields[0].value, "9.99");
    assert_eq!(result.presentation.token_amounts[0].value, "9.99");
}

#[test]
fn token_binding_can_select_a_named_account_and_fails_raw_when_it_is_missing() {
    let mut root = metadata_root();
    root["program"]["instructions"][0]["display"]["x-solana-clearsign"]["tokenAmounts"][0]
        ["mint"] = serde_json::json!({"source":"account","name":"assetAccount"});
    let accounts = [crate::AccountMeta {
        pubkey: USDC,
        is_signer: false,
        is_writable: false,
    }];
    let provider = metadata_provider(USDC, Some(6));
    let present =
        render_metadata_amount(&root, 9_990_000, TOKEN_PROGRAM, &accounts, provider.clone());
    assert_eq!(present.presentation.token_amounts[0].value, "9.99");
    let absent = render_metadata_amount(&root, 9_990_000, USDC, &[], provider);
    assert_eq!(absent.presentation.token_amounts[0].decimals, None);
    assert_eq!(
        absent.presentation.token_amounts[0].mint, None,
        "must not guess from another address"
    );
}

#[test]
fn invalid_or_ambiguous_token_bindings_are_rejected_at_idl_load() {
    for binding in [
        serde_json::json!({"amount":"missing","mint":{"source":"argument","name":"asset"}}),
        serde_json::json!({"amount":"asset","mint":{"source":"argument","name":"asset"}}),
        serde_json::json!({"amount":"quantity","mint":{"source":"argument","name":"quantity"}}),
        serde_json::json!({"amount":"quantity","mint":{"source":"account","name":"missing"}}),
        serde_json::json!({"amount":"quantity","mint":{"source":"argument","name":"asset","typo":true}}),
    ] {
        let mut root = metadata_root();
        root["program"]["instructions"][0]["display"]["x-solana-clearsign"]["tokenAmounts"][0] =
            binding;
        assert!(Srf39Engine::from_json(&root.to_string()).is_err());
    }
    let mut root = metadata_root();
    let bindings = root["program"]["instructions"][0]["display"]["x-solana-clearsign"]
        ["tokenAmounts"]
        .as_array_mut()
        .expect("bindings");
    bindings.push(bindings[0].clone());
    assert!(Srf39Engine::from_json(&root.to_string()).is_err());
}

#[test]
fn hidden_mint_still_resolves_and_hidden_amount_does_not_create_a_presented_field() {
    let mut root = metadata_root();
    root["program"]["instructions"][0]["arguments"][1]["display"] =
        serde_json::json!({"kind":"structFieldDisplayNode","skip":"always"});
    let present = render_metadata_amount(
        &root,
        9_990_000,
        USDC,
        &[],
        metadata_provider(USDC, Some(6)),
    );
    assert_eq!(present.presentation.token_amounts[0].value, "9.99");
    root["program"]["instructions"][0]["arguments"][0]["display"] =
        serde_json::json!({"kind":"structFieldDisplayNode","skip":"always"});
    let provider = metadata_provider(USDC, Some(6));
    let hidden = render_metadata_amount(&root, 9_990_000, USDC, &[], provider.clone());
    assert!(hidden.presentation.token_amounts.is_empty());
    assert!(provider.token_calls.lock().expect("calls").is_empty());
}

#[test]
fn explicit_account_binding_cross_checks_mint_bytes_and_never_hides_decode_errors() {
    let (root_json, fixture, _) = load_fixture("subscriptions");
    let mut root: serde_json::Value = serde_json::from_str(&root_json).expect("root");
    let node = root["program"]["instructions"]
        .as_array_mut()
        .expect("instructions")
        .iter_mut()
        .find(|node| node["name"] == "transferRecurring")
        .expect("transferRecurring");
    node["display"]["x-solana-clearsign"] = serde_json::json!({"tokenAmounts":[
        {"amount":"amount","mint":{"source":"account","name":"tokenMint"}}
    ]});
    const MINT: &str = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";
    for (name, expected, code) in [
        ("transferRecurring-online", Some("5"), None),
        (
            "transferRecurring-missing-mint",
            Some("5000000 (raw)"),
            None,
        ),
        ("transferRecurring-amount-raw", Some("5"), None),
        (
            "transferRecurring-nine-decimals",
            Some("5000000 (raw)"),
            Some("token_registry_decimals_mismatch"),
        ),
        (
            "transferRecurring-foreign-owner",
            Some("5000000 (raw)"),
            Some("token_registry_owner_mismatch"),
        ),
        ("transferRecurring-malformed-mint", None, None),
    ] {
        let scenario = fixture
            .scenarios
            .iter()
            .find(|case| case.name == name)
            .expect("scenario");
        let owned = OwnedInstruction::from_scenario(&fixture, scenario);
        let provider = fixture_provider(
            scenario
                .account_data
                .as_ref()
                .unwrap_or(&fixture.account_data),
        );
        let mut metadata = MapProvider::default();
        metadata
            .tokens
            .insert(MINT.into(), usdc(Some(6), Some(TOKEN_PROGRAM)));
        let client = Srf39Client::from_idl_json(
            &root.to_string(),
            Some(Arc::new(provider)),
            Some(Arc::new(metadata)),
        )
        .expect("client");
        let metas = owned.metas();
        let outcome = pollster::block_on(client.render(&InstructionContext {
            program_id: &owned.program_id,
            instruction_data: &owned.data,
            accounts: &metas,
            fee_payer: None,
        }));
        let Some(value) = expected else {
            assert!(matches!(
                outcome,
                Err(super::RenderFailure::AccountDecode { .. })
            ));
            continue;
        };
        let RenderOutcome::Rendered(result) = outcome.expect("render") else {
            panic!("rendered");
        };
        assert_eq!(result.presentation.token_amounts[0].value, value, "{name}");
        if let Some(code) = code {
            assert!(result.diagnostics.iter().any(|d| d.code == code), "{name}");
            assert!(!result
                .presentation
                .annotations
                .iter()
                .any(|a| matches!(a.kind, AnnotationKind::TokenMint { .. })));
        }
    }
}

#[test]
fn callback_scaling_preserves_signed_values() {
    let mut root = metadata_root();
    root["program"]["instructions"][0]["arguments"][0]["type"]["format"] = "i64".into();
    for (raw, expected) in [
        (-9_990_000i64, "-9.99"),
        (i64::MIN, "-9223372036854.775808"),
    ] {
        let result = render_metadata_amount(
            &root,
            raw as u64,
            USDC,
            &[],
            metadata_provider(USDC, Some(6)),
        );
        assert_eq!(result.presentation.token_amounts[0].value, expected);
    }
}
