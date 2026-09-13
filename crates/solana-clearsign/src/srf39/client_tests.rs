//! Client behaviour: lookup, binding, caching, outcomes and failures.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::provider::Srf39AccountProvider;
use crate::InstructionContext;

use super::binding::IdlRejection;
use super::client::{RenderFailure, RenderOutcome, Srf39Client, UnsupportedReason};
use super::conformance_tests::{fixture_provider, load_fixture, OwnedInstruction};
use super::hints::DecimalsSource;
use super::presentation::{PresentationMetadataProvider, TokenMetadata};
use super::source::{
    Fut, IdlOrigin, IdlProvenance, IdlResolution, IdlSourceError, ResolvedSrf39Idl, Srf39IdlSource,
    StaticIdlSource,
};

const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const SPL_TOKEN_ROOT_SHA256: &str =
    "472f41c79165064ba7bd7cd8623d0b730a227b6bfc12faca77dc8e4702b7bdc1";
const OTHER_PROGRAM: &str = "11111111111111111111111111111111";
const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

struct CountingSource {
    inner: StaticIdlSource,
    calls: AtomicUsize,
}

impl CountingSource {
    fn new(idls: Vec<ResolvedSrf39Idl>) -> Arc<Self> {
        Arc::new(Self {
            inner: StaticIdlSource::new(idls),
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Srf39IdlSource for CountingSource {
    fn idl_for_program<'a>(
        &'a self,
        program_id: &'a str,
    ) -> Fut<'a, Result<IdlResolution, IdlSourceError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.idl_for_program(program_id)
    }
}

/// Returns the IDL under whatever key it is asked for, claiming a fixed program.
struct LyingSource {
    claimed_program: String,
    json: String,
}

impl Srf39IdlSource for LyingSource {
    fn idl_for_program<'a>(
        &'a self,
        _program_id: &'a str,
    ) -> Fut<'a, Result<IdlResolution, IdlSourceError>> {
        let resolved = ResolvedSrf39Idl {
            program_id: self.claimed_program.clone(),
            json: self.json.clone(),
            provenance: provenance(None),
        };
        Box::pin(async move { Ok(IdlResolution::Found(resolved)) })
    }
}

struct FlakySource {
    fail: AtomicBool,
    inner: StaticIdlSource,
}

impl Srf39IdlSource for FlakySource {
    fn idl_for_program<'a>(
        &'a self,
        program_id: &'a str,
    ) -> Fut<'a, Result<IdlResolution, IdlSourceError>> {
        if self.fail.load(Ordering::SeqCst) {
            return Box::pin(async {
                Err(IdlSourceError {
                    detail: "disk unavailable".to_string(),
                    retryable: true,
                })
            });
        }
        self.inner.idl_for_program(program_id)
    }
}

struct KnownTokens(BTreeMap<String, TokenMetadata>);

impl PresentationMetadataProvider for KnownTokens {
    fn token_metadata<'a>(&'a self, mint: &'a str) -> Fut<'a, Option<TokenMetadata>> {
        let metadata = self.0.get(mint).cloned();
        Box::pin(async move { metadata })
    }
}

fn provenance(expected_sha256_hex: Option<&str>) -> IdlProvenance {
    IdlProvenance {
        source_id: "test".to_string(),
        origin: IdlOrigin::Bundled,
        expected_sha256_hex: expected_sha256_hex.map(str::to_string),
        version: None,
        reference: None,
    }
}

fn spl_token_instruction_idl(expected_sha256_hex: Option<&str>) -> (String, ResolvedSrf39Idl) {
    let (root_json, _, _) = load_fixture("spl-token-instructions");
    let resolved = ResolvedSrf39Idl {
        program_id: TOKEN_PROGRAM.to_string(),
        json: root_json.clone(),
        provenance: provenance(expected_sha256_hex),
    };
    (root_json, resolved)
}

fn scenario(
    fixture_name: &str,
    scenario_name: &str,
) -> (OwnedInstruction, Arc<dyn Srf39AccountProvider>) {
    let (_, fixture, _) = load_fixture(fixture_name);
    let scenario = fixture
        .scenarios
        .iter()
        .find(|scenario| scenario.name == scenario_name)
        .unwrap_or_else(|| panic!("scenario {scenario_name}"));
    let owned = OwnedInstruction::from_scenario(&fixture, scenario);
    let provider = fixture_provider(
        scenario
            .account_data
            .as_ref()
            .unwrap_or(&fixture.account_data),
    );
    (owned, Arc::new(provider))
}

fn usdc_metadata() -> Arc<dyn PresentationMetadataProvider> {
    let mut tokens = BTreeMap::new();
    tokens.insert(
        USDC.to_string(),
        TokenMetadata {
            symbol: "USDC".to_string(),
            name: Some("USD Coin".to_string()),
            decimals: Some(6),
            token_program: Some(TOKEN_PROGRAM.to_string()),
        },
    );
    Arc::new(KnownTokens(tokens))
}

fn render(client: &Srf39Client, owned: &OwnedInstruction) -> Result<RenderOutcome, RenderFailure> {
    let metas = owned.metas();
    let instruction = InstructionContext {
        program_id: &owned.program_id,
        instruction_data: &owned.data,
        accounts: &metas,
        fee_payer: None,
    };
    pollster::block_on(client.render(&instruction))
}

/// The derived Subscriptions IDL binds under its program id with the digest
/// recorded by the authoring script, and renders with exactly the diagnostics
/// the two-hop mint gap implies.
#[test]
fn subscriptions_idl_binds_and_pins_digest() {
    const PROGRAM: &str = "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44";
    let (root_json, _, _) = load_fixture("subscriptions");
    let provenance_json = std::fs::read_to_string(
        super::conformance_tests::fixture_directory("subscriptions").join("provenance.json"),
    )
    .expect("provenance");
    let provenance_value: serde_json::Value =
        serde_json::from_str(&provenance_json).expect("provenance json");
    let root_sha256 = provenance_value["rootSha256"]
        .as_str()
        .expect("rootSha256")
        .to_string();
    let idl = ResolvedSrf39Idl {
        program_id: PROGRAM.to_string(),
        json: root_json,
        provenance: provenance(Some(&root_sha256)),
    };
    let (owned, accounts) = scenario("subscriptions", "createRecurringDelegation-vendor-bytes");
    let client = Srf39Client::new(
        Arc::new(StaticIdlSource::new(vec![idl])),
        Some(accounts),
        None,
    );
    let RenderOutcome::Rendered(rendered) = render(&client, &owned).expect("render") else {
        panic!("rendered");
    };
    assert_eq!(rendered.idl.program_id, PROGRAM);
    assert_eq!(rendered.idl.program_name, "subscriptions");
    assert_eq!(rendered.idl.sha256_hex, root_sha256);
    assert!(rendered.idl.digest_pinned);
    assert_eq!(rendered.canonical.intent, "Authorize Recurring Spending");
    assert_eq!(rendered.canonical.fields[0].value, "5000000 (raw)");
    assert_eq!(
        rendered
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>(),
        vec!["amount_scale_unresolved", "interpolated_intent_unavailable"]
    );
    assert!(rendered.hints.linked_account_reads.is_empty());
}

#[test]
fn transfer_checked_renders_with_pinned_binding_and_known_token() {
    let (_, idl) = spl_token_instruction_idl(Some(SPL_TOKEN_ROOT_SHA256));
    let (owned, provider) = scenario("spl-token-instructions", "transferChecked-online");
    let source = CountingSource::new(vec![idl]);
    let client = Srf39Client::new(source.clone(), Some(provider), Some(usdc_metadata()));

    let RenderOutcome::Rendered(rendered) = render(&client, &owned).expect("render") else {
        panic!("expected a rendered instruction");
    };
    let (_, _, expected) = load_fixture("spl-token-instructions");
    assert_eq!(
        serde_json::to_value(&rendered.canonical).expect("serialize"),
        expected["transferChecked-online"]
    );
    assert_eq!(rendered.idl.program_id, TOKEN_PROGRAM);
    assert_eq!(rendered.idl.program_name, "token");
    assert_eq!(rendered.idl.sha256_hex, SPL_TOKEN_ROOT_SHA256);
    assert!(rendered.idl.digest_pinned);
    assert!(matches!(
        &rendered.hints.amounts[0].decimals,
        DecimalsSource::AccountField { account, address: Some(address), resolved: Some(6), .. }
            if account == "mint" && address == USDC
    ));
    assert!(
        rendered.diagnostics.is_empty(),
        "diagnostics: {:?}",
        rendered.diagnostics
    );
    assert_eq!(rendered.presentation.annotations.len(), 2);
    assert_eq!(source.calls(), 1);
}

#[test]
fn offline_render_reports_diagnostics_in_a_fixed_order() {
    let (_, idl) = spl_token_instruction_idl(Some(SPL_TOKEN_ROOT_SHA256));
    let (owned, _) = scenario("spl-token-instructions", "transferChecked-offline");
    let client = Srf39Client::new(CountingSource::new(vec![idl]), None, Some(usdc_metadata()));

    let RenderOutcome::Rendered(rendered) = render(&client, &owned).expect("render") else {
        panic!("expected a rendered instruction");
    };
    assert_eq!(rendered.canonical.fields[0].value, "1469 (raw)");
    assert_eq!(rendered.canonical.interpolated_intent, None);
    assert!(rendered.hints.interpolated_intent_suppressed);
    let codes: Vec<&str> = rendered
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert_eq!(
        codes,
        [
            "linked_account_unavailable",
            "amount_scale_unresolved",
            "interpolated_intent_unavailable",
        ]
    );
    let amount = rendered
        .presentation
        .annotations
        .iter()
        .find(|annotation| annotation.field_index == 0)
        .expect("amount annotation");
    assert!(matches!(
        &amount.kind,
        super::presentation::AnnotationKind::TokenAmount {
            applies_to_value: false,
            ..
        }
    ));
}

#[test]
fn source_is_consulted_once_per_program_for_sequential_calls() {
    let (_, idl) = spl_token_instruction_idl(Some(SPL_TOKEN_ROOT_SHA256));
    let (owned, provider) = scenario("spl-token-instructions", "transferChecked-online");
    let source = CountingSource::new(vec![idl]);
    let client = Srf39Client::new(source.clone(), Some(provider), None);

    for _ in 0..3 {
        assert!(matches!(
            render(&client, &owned).expect("render"),
            RenderOutcome::Rendered(_)
        ));
    }
    let metas = owned.metas();
    let instruction = InstructionContext {
        program_id: &owned.program_id,
        instruction_data: &owned.data,
        accounts: &metas,
        fee_payer: None,
    };
    assert!(pollster::block_on(client.display(&instruction, None))
        .expect("display")
        .is_some());
    assert_eq!(
        pollster::block_on(client.required_accounts(&instruction)).expect("required"),
        Some(vec![USDC.to_string()])
    );
    assert_eq!(source.calls(), 1);

    client.invalidate(TOKEN_PROGRAM);
    assert!(matches!(
        render(&client, &owned).expect("render"),
        RenderOutcome::Rendered(_)
    ));
    assert_eq!(source.calls(), 2);
}

#[test]
fn unknown_program_is_unsupported_and_cached() {
    let source = CountingSource::new(vec![]);
    let client = Srf39Client::new(source.clone(), None, None);
    let (mut owned, _) = scenario("spl-token-instructions", "transferChecked-online");
    owned.program_id = OTHER_PROGRAM.to_string();

    for _ in 0..2 {
        assert_eq!(
            render(&client, &owned).expect("render"),
            RenderOutcome::Unsupported {
                reason: UnsupportedReason::IdlNotFound {
                    program_id: OTHER_PROGRAM.to_string()
                }
            }
        );
    }
    let metas = owned.metas();
    let instruction = InstructionContext {
        program_id: &owned.program_id,
        instruction_data: &owned.data,
        accounts: &metas,
        fee_payer: None,
    };
    assert_eq!(
        pollster::block_on(client.display(&instruction, None)).expect("display"),
        None
    );
    assert_eq!(
        pollster::block_on(client.required_accounts(&instruction)).expect("required"),
        None
    );
    assert_eq!(source.calls(), 1);
}

#[test]
fn unrecognized_and_undecodable_instructions_are_unsupported() {
    let (root_json, _, _) = load_fixture("spl-token-transfer-checked");
    let idl = ResolvedSrf39Idl {
        program_id: TOKEN_PROGRAM.to_string(),
        json: root_json,
        provenance: provenance(None),
    };
    let client = Srf39Client::new(CountingSource::new(vec![idl]), None, None);

    let (unknown, _) = scenario("spl-token-transfer-checked", "unknownDiscriminator");
    assert_eq!(
        render(&client, &unknown).expect("render"),
        RenderOutcome::Unsupported {
            reason: UnsupportedReason::InstructionNotRecognized {
                program_id: TOKEN_PROGRAM.to_string()
            }
        }
    );
    let (truncated, _) = scenario("spl-token-transfer-checked", "truncated");
    assert_eq!(
        render(&client, &truncated).expect("render"),
        RenderOutcome::Unsupported {
            reason: UnsupportedReason::InstructionDecodeFailed {
                program_id: TOKEN_PROGRAM.to_string(),
                instruction: "transferChecked".to_string(),
            }
        }
    );
}

#[test]
fn digest_mismatch_is_rejected_cached_and_retried_after_invalidate() {
    let mut wrong = SPL_TOKEN_ROOT_SHA256.to_string();
    wrong.replace_range(0..1, if wrong.starts_with('4') { "5" } else { "4" });
    let (_, idl) = spl_token_instruction_idl(Some(&wrong));
    let (owned, _) = scenario("spl-token-instructions", "transferChecked-online");
    let source = CountingSource::new(vec![idl]);
    let client = Srf39Client::new(source.clone(), None, None);

    for _ in 0..2 {
        let error = render(&client, &owned).expect_err("digest mismatch");
        assert_eq!(
            error,
            RenderFailure::IdlRejected {
                program_id: TOKEN_PROGRAM.to_string(),
                rejection: IdlRejection::DigestMismatch {
                    expected: wrong.clone(),
                    actual: SPL_TOKEN_ROOT_SHA256.to_string(),
                },
            }
        );
    }
    assert_eq!(source.calls(), 1);
    client.invalidate_all();
    render(&client, &owned).expect_err("still rejected");
    assert_eq!(source.calls(), 2);
}

#[test]
fn malformed_pinned_digest_is_rejected() {
    let (_, idl) = spl_token_instruction_idl(Some("not-a-digest"));
    let (owned, _) = scenario("spl-token-instructions", "transferChecked-online");
    let client = Srf39Client::new(CountingSource::new(vec![idl]), None, None);
    let error = render(&client, &owned).expect_err("malformed digest");
    assert!(matches!(
        error,
        RenderFailure::IdlRejected {
            rejection: IdlRejection::DigestMismatch { expected, .. },
            ..
        } if expected == "not-a-digest"
    ));
}

#[test]
fn unpinned_idl_renders_with_a_warning() {
    let (_, idl) = spl_token_instruction_idl(None);
    let (owned, provider) = scenario("spl-token-instructions", "transferChecked-online");
    let client = Srf39Client::new(
        CountingSource::new(vec![idl]),
        Some(provider),
        Some(usdc_metadata()),
    );
    let RenderOutcome::Rendered(rendered) = render(&client, &owned).expect("render") else {
        panic!("expected a rendered instruction");
    };
    assert!(!rendered.idl.digest_pinned);
    assert_eq!(rendered.diagnostics.len(), 1);
    assert_eq!(rendered.diagnostics[0].code, "idl_digest_unpinned");
}

#[test]
fn program_mismatch_is_rejected_for_both_claim_and_content() {
    let (root_json, _) = spl_token_instruction_idl(None);
    let (mut owned, _) = scenario("spl-token-instructions", "transferChecked-online");
    owned.program_id = OTHER_PROGRAM.to_string();

    // The source claims the requested program but the IDL says otherwise.
    let lying = Arc::new(LyingSource {
        claimed_program: OTHER_PROGRAM.to_string(),
        json: root_json.clone(),
    });
    let client = Srf39Client::new(lying, None, None);
    assert_eq!(
        render(&client, &owned).expect_err("content mismatch"),
        RenderFailure::IdlRejected {
            program_id: OTHER_PROGRAM.to_string(),
            rejection: IdlRejection::ProgramMismatch {
                requested: OTHER_PROGRAM.to_string(),
                declared: TOKEN_PROGRAM.to_string(),
            },
        }
    );

    // The source returns an entry for a different program than requested.
    let lying = Arc::new(LyingSource {
        claimed_program: TOKEN_PROGRAM.to_string(),
        json: root_json,
    });
    let client = Srf39Client::new(lying, None, None);
    assert_eq!(
        render(&client, &owned).expect_err("claim mismatch"),
        RenderFailure::IdlRejected {
            program_id: OTHER_PROGRAM.to_string(),
            rejection: IdlRejection::ProgramMismatch {
                requested: OTHER_PROGRAM.to_string(),
                declared: TOKEN_PROGRAM.to_string(),
            },
        }
    );
}

#[test]
fn invalid_idl_from_source_is_rejected_as_invalid() {
    let idl = ResolvedSrf39Idl {
        program_id: TOKEN_PROGRAM.to_string(),
        json: "{".to_string(),
        provenance: provenance(None),
    };
    let (owned, _) = scenario("spl-token-instructions", "transferChecked-online");
    let client = Srf39Client::new(CountingSource::new(vec![idl]), None, None);
    assert!(matches!(
        render(&client, &owned).expect_err("invalid"),
        RenderFailure::IdlRejected {
            rejection: IdlRejection::Invalid(super::Srf39IdlError::InvalidJson { .. }),
            ..
        }
    ));
}

#[test]
fn source_failures_are_surfaced_and_never_cached() {
    let (_, idl) = spl_token_instruction_idl(Some(SPL_TOKEN_ROOT_SHA256));
    let (owned, _) = scenario("spl-token-instructions", "transferChecked-online");
    let source = Arc::new(FlakySource {
        fail: AtomicBool::new(true),
        inner: StaticIdlSource::new(vec![idl]),
    });
    let client = Srf39Client::new(source.clone(), None, None);
    assert_eq!(
        render(&client, &owned).expect_err("source down"),
        RenderFailure::IdlSourceFailed {
            detail: "disk unavailable".to_string(),
            retryable: true,
        }
    );
    source.fail.store(false, Ordering::SeqCst);
    assert!(matches!(
        render(&client, &owned).expect("render after recovery"),
        RenderOutcome::Rendered(_)
    ));
}

#[test]
fn invalid_program_ids_are_rejected_on_every_entry_point() {
    let client = Srf39Client::new(CountingSource::new(vec![]), None, None);
    for bad in ["", "not base58 0OIl", "abc"] {
        let instruction = InstructionContext {
            program_id: bad,
            instruction_data: &[],
            accounts: &[],
            fee_payer: None,
        };
        assert!(matches!(
            pollster::block_on(client.render(&instruction)),
            Err(RenderFailure::InvalidInput { .. })
        ));
        assert!(matches!(
            pollster::block_on(client.display(&instruction, None)),
            Err(RenderFailure::InvalidInput { .. })
        ));
        assert!(matches!(
            pollster::block_on(client.required_accounts(&instruction)),
            Err(RenderFailure::InvalidInput { .. })
        ));
    }
}

#[test]
fn from_idl_json_behaves_like_the_engine_and_covers_additional_programs() {
    assert!(matches!(
        Srf39Client::from_idl_json("{", None, None),
        Err(super::Srf39IdlError::InvalidJson { .. })
    ));

    let json = r#"
    {
      "kind": "rootNode",
      "standard": "codama",
      "version": "1.8.0",
      "program": {
        "kind": "programNode",
        "name": "primary",
        "publicKey": "11111111111111111111111111111111",
        "instructions": [{
          "kind": "instructionNode",
          "name": "ping",
          "display": { "kind": "instructionDisplayNode", "intent": "Ping" },
          "arguments": [{
            "kind": "instructionArgumentNode",
            "name": "discriminator",
            "type": { "kind": "numberTypeNode", "format": "u8", "endian": "le" },
            "defaultValue": { "kind": "numberValueNode", "number": 1 },
            "display": { "kind": "structFieldDisplayNode", "skip": "always" }
          }],
          "discriminators": [{ "kind": "fieldDiscriminatorNode", "name": "discriminator", "offset": 0 }]
        }]
      },
      "additionalPrograms": [{
        "kind": "programNode",
        "name": "secondary",
        "publicKey": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
        "instructions": [{
          "kind": "instructionNode",
          "name": "pong",
          "display": { "kind": "instructionDisplayNode", "intent": "Pong" },
          "arguments": [{
            "kind": "instructionArgumentNode",
            "name": "discriminator",
            "type": { "kind": "numberTypeNode", "format": "u8", "endian": "le" },
            "defaultValue": { "kind": "numberValueNode", "number": 2 },
            "display": { "kind": "structFieldDisplayNode", "skip": "always" }
          }],
          "discriminators": [{ "kind": "fieldDiscriminatorNode", "name": "discriminator", "offset": 0 }]
        }]
      }]
    }
    "#;
    let client = Srf39Client::from_idl_json(json, None, None).expect("inline IDL");
    let ping = InstructionContext {
        program_id: OTHER_PROGRAM,
        instruction_data: &[1],
        accounts: &[],
        fee_payer: None,
    };
    let pong = InstructionContext {
        program_id: TOKEN_PROGRAM,
        instruction_data: &[2],
        accounts: &[],
        fee_payer: None,
    };
    let RenderOutcome::Rendered(rendered) = pollster::block_on(client.render(&ping)).expect("ping")
    else {
        panic!("ping renders");
    };
    assert_eq!(rendered.canonical.intent, "Ping");
    assert_eq!(rendered.idl.program_id, OTHER_PROGRAM);
    assert!(!rendered.idl.digest_pinned);
    assert_eq!(rendered.diagnostics[0].code, "idl_digest_unpinned");

    let RenderOutcome::Rendered(rendered) = pollster::block_on(client.render(&pong)).expect("pong")
    else {
        panic!("pong renders through the additional program");
    };
    assert_eq!(rendered.canonical.intent, "Pong");
    assert_eq!(rendered.idl.program_id, TOKEN_PROGRAM);
    assert_eq!(
        pollster::block_on(client.display(&pong, None)).expect("display"),
        Some(rendered.canonical)
    );
}

#[test]
fn not_found_cache_is_capped_without_evicting_ready_entries() {
    let (_, idl) = spl_token_instruction_idl(Some(SPL_TOKEN_ROOT_SHA256));
    let (owned, provider) = scenario("spl-token-instructions", "transferChecked-online");
    let source = CountingSource::new(vec![idl]);
    let client = Srf39Client::new(source.clone(), Some(provider), None);
    render(&client, &owned).expect("warm the ready entry");
    assert_eq!(source.calls(), 1);

    let mut first_unknown = String::new();
    for index in 0..257u32 {
        let mut bytes = [0u8; 32];
        bytes[..4].copy_from_slice(&(index + 1).to_le_bytes());
        let program_id = bs58::encode(bytes).into_string();
        if index == 0 {
            first_unknown = program_id.clone();
        }
        let instruction = InstructionContext {
            program_id: &program_id,
            instruction_data: &owned.data,
            accounts: &[],
            fee_payer: None,
        };
        assert!(matches!(
            pollster::block_on(client.render(&instruction)).expect("render"),
            RenderOutcome::Unsupported { .. }
        ));
    }
    assert_eq!(source.calls(), 1 + 257);

    // The overflow dropped the not-found entries, so the first unknown
    // program is looked up again; the ready entry survived untouched.
    let instruction = InstructionContext {
        program_id: &first_unknown,
        instruction_data: &owned.data,
        accounts: &[],
        fee_payer: None,
    };
    pollster::block_on(client.render(&instruction)).expect("render");
    assert_eq!(source.calls(), 1 + 257 + 1);
    render(&client, &owned).expect("ready entry");
    assert_eq!(source.calls(), 1 + 257 + 1);
}
