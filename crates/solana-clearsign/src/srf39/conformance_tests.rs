use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Deserialize;

use crate::provider::AccountData;
use crate::provider::Srf39AccountProvider;
use crate::AccountMeta;
use crate::InstructionContext;

use super::get_instruction_display;
use super::DisplayResult;
use super::LoadedSrf39Idl;
use super::Srf39DisplayError;
use super::Srf39Engine;
use super::Srf39IdlError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Fixture {
    pub(super) program_address: String,
    pub(super) data_base64: String,
    pub(super) accounts: Vec<FixtureAccount>,
    #[serde(default)]
    pub(super) account_data: BTreeMap<String, FixtureAccountData>,
    pub(super) scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
pub(super) struct FixtureAccount {
    pub(super) address: String,
    pub(super) role: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FixtureAccountData {
    pub(super) program_address: String,
    pub(super) data_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Scenario {
    pub(super) name: String,
    pub(super) program_address: Option<String>,
    pub(super) data_base64: Option<String>,
    pub(super) accounts: Option<Vec<FixtureAccount>>,
    pub(super) account_data: Option<BTreeMap<String, FixtureAccountData>>,
    #[serde(default)]
    pub(super) fetch_accounts: bool,
    pub(super) expected_error: Option<String>,
}

#[derive(Default)]
pub(super) struct FixtureProvider {
    pub(super) accounts: BTreeMap<String, AccountData>,
}

impl Srf39AccountProvider for FixtureProvider {
    fn resolve_account<'a>(
        &'a self,
        address: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<AccountData>> + Send + 'a>> {
        let account = self.accounts.get(address).cloned();
        Box::pin(async move { account })
    }
}

#[test]
fn rust_matches_reference_transfer_checked_fixture() {
    run_fixture("transfer-checked");
}

#[test]
fn rust_matches_reference_default_display_fixture() {
    run_fixture("default-display");
}

#[test]
fn rust_matches_reference_real_spl_token_fixture() {
    run_fixture("spl-token-transfer-checked");
}

#[test]
fn rust_matches_reference_spl_token_fixture() {
    run_fixture("spl-token-instructions");
}

#[test]
fn rust_matches_reference_struct_argument_fixture() {
    run_fixture("struct-argument");
}

#[test]
fn rust_matches_reference_inert_nodes_fixture() {
    run_fixture("inert-nodes");
}

#[test]
fn rust_matches_reference_defined_types_fixture() {
    run_fixture("defined-types");
}

#[test]
fn rust_matches_reference_signed_numbers_fixture() {
    run_fixture("signed-numbers");
}

#[test]
fn rust_matches_reference_date_time_display_fixture() {
    run_fixture("date-time-display");
}

#[test]
fn rust_matches_reference_duration_display_fixture() {
    run_fixture("duration-display");
}

#[test]
fn rust_matches_reference_enum_display_fixture() {
    run_fixture("enum-display");
}

#[test]
fn rust_matches_reference_fixed_string_fixture() {
    run_fixture("fixed-string");
}

#[test]
fn rust_matches_reference_fixed_array_fixture() {
    run_fixture("fixed-array");
}

/// The pristine Subscriptions program IDL (as emitted by the program's own
/// build, without any display metadata) must load whole: every type node it
/// uses is implemented and its PDA, error, and default-value nodes are inert.
#[test]
fn pristine_subscriptions_idl_loads_without_display_metadata() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/idl/subscriptions.json");
    let json = std::fs::read_to_string(path).expect("read vendored Subscriptions IDL");
    let idl = LoadedSrf39Idl::from_json(&json).expect("pristine IDL loads");
    let program = idl.primary_program();
    assert_eq!(program.instructions.len(), 16);
    assert_eq!(program.accounts.len(), 6);
    assert_eq!(program.defined_types.len(), 10);
    let engine = Srf39Engine::from_json(&json).expect("engine loads");
    assert_eq!(engine.program_name(), "subscriptions");
}

#[test]
fn rejects_string_and_array_shapes_outside_the_fixed_layout_subset() {
    let cases = [
        (
            r#"{ "kind": "fixedSizeTypeNode", "size": 8,
                "type": { "kind": "stringTypeNode", "encoding": "base58" } }"#,
            "$.program.instructions[0].arguments[0].type.type",
            "stringTypeNode(encoding=base58)",
        ),
        (
            r#"{ "kind": "fixedSizeTypeNode", "size": 8,
                "type": { "kind": "numberTypeNode", "format": "u8", "endian": "le" } }"#,
            "$.program.instructions[0].arguments[0].type",
            "fixedSizeTypeNode(non-string)",
        ),
        (
            r#"{ "kind": "arrayTypeNode", "item": { "kind": "publicKeyTypeNode" },
                "count": { "kind": "fixedCountNode", "value": 0 } }"#,
            "$.program.instructions[0].arguments[0].type.count",
            "arrayTypeNode(zero count)",
        ),
        (
            r#"{ "kind": "arrayTypeNode", "item": { "kind": "publicKeyTypeNode" },
                "count": { "kind": "prefixedCountNode",
                           "prefix": { "kind": "numberTypeNode", "format": "u32", "endian": "le" } } }"#,
            "$.program.instructions[0].arguments[0].type.count",
            "prefixedCountNode",
        ),
    ];
    for (ty, path, kind) in cases {
        let instruction = format!(
            r#"{{
              "kind": "instructionNode",
              "name": "label",
              "arguments": [{{ "kind": "instructionArgumentNode", "name": "value", "type": {ty} }}]
            }}"#
        );
        let error = LoadedSrf39Idl::from_json(&root_with_instruction(&instruction))
            .expect_err("shape must be rejected");
        assert_eq!(
            error,
            Srf39IdlError::UnsupportedIdlNode {
                path: path.to_string(),
                kind: kind.to_string(),
            }
        );
    }
}

#[test]
fn rejects_enum_shapes_the_pinned_decoder_cannot_index() {
    let cases = [
        (
            r#"{ "kind": "enumTypeNode", "size": { "kind": "numberTypeNode", "format": "u32", "endian": "le" },
                "variants": [{ "kind": "enumEmptyVariantTypeNode", "name": "a" }] }"#,
            "$.program.instructions[0].arguments[0].type.size",
            "enumTypeNode(non-u8 size)",
        ),
        (
            r#"{ "kind": "enumTypeNode", "size": { "kind": "numberTypeNode", "format": "u8", "endian": "le" },
                "variants": [] }"#,
            "$.program.instructions[0].arguments[0].type",
            "enumTypeNode(no variants)",
        ),
        (
            r#"{ "kind": "enumTypeNode", "size": { "kind": "numberTypeNode", "format": "u8", "endian": "le" },
                "variants": [{ "kind": "enumEmptyVariantTypeNode", "name": "a", "discriminator": 5 }] }"#,
            "$.program.instructions[0].arguments[0].type.variants[0]",
            "enumTypeNode(discriminator mismatch)",
        ),
        (
            r#"{ "kind": "enumTypeNode", "size": { "kind": "numberTypeNode", "format": "u8", "endian": "le" },
                "variants": [{ "kind": "enumEmptyVariantTypeNode", "name": "a" },
                             { "kind": "enumEmptyVariantTypeNode", "name": "a" }] }"#,
            "$.program.instructions[0].arguments[0].type.variants[1]",
            "enumTypeNode(duplicate variant)",
        ),
    ];
    for (ty, path, kind) in cases {
        let instruction = format!(
            r#"{{
              "kind": "instructionNode",
              "name": "trade",
              "arguments": [{{ "kind": "instructionArgumentNode", "name": "side", "type": {ty} }}]
            }}"#
        );
        let error = LoadedSrf39Idl::from_json(&root_with_instruction(&instruction))
            .expect_err("enum shape must be rejected");
        assert_eq!(
            error,
            Srf39IdlError::UnsupportedIdlNode {
                path: path.to_string(),
                kind: kind.to_string(),
            }
        );
    }
}

#[test]
fn rejects_fractional_or_non_positive_ticks_per_second() {
    for (kind, ticks) in [
        ("dateTimeNumberDisplayNode", "0.5"),
        ("durationNumberDisplayNode", "0"),
        ("dateTimeNumberDisplayNode", "-1"),
        ("durationNumberDisplayNode", "4294967296"),
    ] {
        let instruction = format!(
            r#"{{
              "kind": "instructionNode",
              "name": "stamp",
              "arguments": [{{
                "kind": "instructionArgumentNode",
                "name": "at",
                "type": {{
                  "kind": "numberTypeNode", "format": "i64", "endian": "le",
                  "display": {{ "kind": "{kind}", "ticksPerSecond": {ticks} }}
                }}
              }}]
            }}"#
        );
        let error = LoadedSrf39Idl::from_json(&root_with_instruction(&instruction))
            .expect_err("only positive integer ticksPerSecond load");
        assert_eq!(
            error,
            Srf39IdlError::UnsupportedIdlNode {
                path: "$.program.instructions[0].arguments[0].type.display.ticksPerSecond"
                    .to_string(),
                kind: format!("{kind}(ticksPerSecond)"),
            }
        );
    }
}

/// Wraps defined types and one instruction into a minimal root.
fn root_with_defined_types(defined_types: &str, instruction: &str) -> String {
    format!(
        r#"{{
          "kind": "rootNode",
          "standard": "codama",
          "version": "1.8.0",
          "program": {{
            "kind": "programNode",
            "name": "test",
            "publicKey": "11111111111111111111111111111111",
            "definedTypes": [{defined_types}],
            "instructions": [{instruction}]
          }}
        }}"#
    )
}

const U8_TYPE: &str = r#"{ "kind": "numberTypeNode", "format": "u8", "endian": "le" }"#;

#[test]
fn rejects_unresolved_defined_type_link_with_path() {
    let instruction = r#"{
      "kind": "instructionNode",
      "name": "pay",
      "arguments": [{
        "kind": "instructionArgumentNode",
        "name": "params",
        "type": { "kind": "definedTypeLinkNode", "name": "missing" }
      }]
    }"#;
    let error = LoadedSrf39Idl::from_json(&root_with_defined_types("", instruction))
        .expect_err("dangling link must be rejected");
    assert_eq!(
        error,
        Srf39IdlError::UnsupportedIdlNode {
            path: "$.program.instructions[0].arguments[0].type".to_string(),
            kind: "definedTypeLinkNode(unresolved)".to_string(),
        }
    );
}

#[test]
fn rejects_unresolved_program_link_with_path() {
    let instruction = r#"{
      "kind": "instructionNode",
      "name": "pay",
      "arguments": [{
        "kind": "instructionArgumentNode",
        "name": "params",
        "type": {
          "kind": "definedTypeLinkNode",
          "name": "params",
          "program": { "kind": "programLinkNode", "name": "nope" }
        }
      }]
    }"#;
    let error = LoadedSrf39Idl::from_json(&root_with_defined_types("", instruction))
        .expect_err("unknown program must be rejected");
    assert_eq!(
        error,
        Srf39IdlError::UnsupportedIdlNode {
            path: "$.program.instructions[0].arguments[0].type.program".to_string(),
            kind: "programLinkNode(unresolved)".to_string(),
        }
    );
}

#[test]
fn rejects_recursive_defined_types_at_their_definition() {
    let defined_types = format!(
        r#"{{
          "kind": "definedTypeNode",
          "name": "node",
          "type": {{
            "kind": "structTypeNode",
            "fields": [
              {{ "kind": "structFieldTypeNode", "name": "value", "type": {U8_TYPE} }},
              {{ "kind": "structFieldTypeNode", "name": "next",
                 "type": {{ "kind": "definedTypeLinkNode", "name": "node" }} }}
            ]
          }}
        }}"#
    );
    let error = LoadedSrf39Idl::from_json(&root_with_defined_types(&defined_types, ""))
        .expect_err("recursive type must be rejected");
    assert_eq!(
        error,
        Srf39IdlError::UnsupportedIdlNode {
            path: "$.program.definedTypes[0].type.fields[1].type".to_string(),
            kind: "definedTypeLinkNode(cycle)".to_string(),
        }
    );

    let mutual = format!(
        r#"{{ "kind": "definedTypeNode", "name": "a",
             "type": {{ "kind": "structTypeNode", "fields": [
               {{ "kind": "structFieldTypeNode", "name": "b",
                  "type": {{ "kind": "definedTypeLinkNode", "name": "b" }} }} ] }} }},
           {{ "kind": "definedTypeNode", "name": "b",
             "type": {{ "kind": "structTypeNode", "fields": [
               {{ "kind": "structFieldTypeNode", "name": "size", "type": {U8_TYPE} }},
               {{ "kind": "structFieldTypeNode", "name": "a",
                  "type": {{ "kind": "definedTypeLinkNode", "name": "a" }} }} ] }} }}"#
    );
    let error = LoadedSrf39Idl::from_json(&root_with_defined_types(&mutual, ""))
        .expect_err("mutually recursive types must be rejected");
    assert!(matches!(
        error,
        Srf39IdlError::UnsupportedIdlNode { kind, .. } if kind == "definedTypeLinkNode(cycle)"
    ));
}

#[test]
fn rejects_duplicate_defined_type_names() {
    let defined_types = format!(
        r#"{{ "kind": "definedTypeNode", "name": "twice", "type": {U8_TYPE} }},
           {{ "kind": "definedTypeNode", "name": "twice", "type": {U8_TYPE} }}"#
    );
    let error = LoadedSrf39Idl::from_json(&root_with_defined_types(&defined_types, ""))
        .expect_err("duplicate names must be rejected");
    assert!(matches!(error, Srf39IdlError::InvalidSchema { .. }));
}

#[test]
fn reports_unsupported_nodes_inside_defined_types_at_their_definition() {
    let defined_types = r#"{
      "kind": "definedTypeNode",
      "name": "params",
      "type": {
        "kind": "structTypeNode",
        "fields": [{
          "kind": "structFieldTypeNode", "name": "count",
          "type": { "kind": "numberTypeNode", "format": "u16", "endian": "le" }
        }]
      }
    }"#;
    let instruction = r#"{
      "kind": "instructionNode",
      "name": "pay",
      "arguments": [{
        "kind": "instructionArgumentNode",
        "name": "params",
        "type": { "kind": "definedTypeLinkNode", "name": "params" }
      }]
    }"#;
    let error = LoadedSrf39Idl::from_json(&root_with_defined_types(defined_types, instruction))
        .expect_err("u16 is unsupported");
    assert_eq!(
        error,
        Srf39IdlError::UnsupportedIdlNode {
            path: "$.program.definedTypes[0].type.fields[0].type".to_string(),
            kind: "numberTypeNode(format=u16, endian=le)".to_string(),
        }
    );
}

/// Wraps one instruction into a minimal root so tests can probe validation
/// paths under `$.program.instructions[0]`.
fn root_with_instruction(instruction: &str) -> String {
    format!(
        r#"{{
          "kind": "rootNode",
          "standard": "codama",
          "version": "1.8.0",
          "program": {{
            "kind": "programNode",
            "name": "test",
            "publicKey": "11111111111111111111111111111111",
            "instructions": [{instruction}]
          }}
        }}"#
    )
}

#[test]
fn inert_kinds_are_rejected_outside_inert_positions() {
    let cases = [
        (
            r#"{
              "kind": "instructionNode",
              "name": "pay",
              "arguments": [{
                "kind": "instructionArgumentNode",
                "name": "amount",
                "type": {
                  "kind": "numberTypeNode", "format": "u64", "endian": "le",
                  "display": {
                    "kind": "amountNumberDisplayNode",
                    "unit": { "kind": "injectedValueNode", "key": "unit" }
                  }
                }
              }],
              "provides": [{
                "kind": "providedNode",
                "name": "unit",
                "node": { "kind": "accountValueNode", "name": "mint" }
              }]
            }"#,
            "$.program.instructions[0].provides[0].node",
            "accountValueNode",
        ),
        (
            r#"{
              "kind": "instructionNode",
              "name": "pay",
              "arguments": [{
                "kind": "instructionArgumentNode",
                "name": "seed",
                "type": { "kind": "pdaNode", "name": "vault", "seeds": [] }
              }]
            }"#,
            "$.program.instructions[0].arguments[0].type",
            "pdaNode",
        ),
        (
            r#"{
              "kind": "instructionNode",
              "name": "pay",
              "arguments": [{
                "kind": "instructionArgumentNode",
                "name": "authority",
                "type": { "kind": "publicKeyTypeNode" },
                "defaultValue": {
                  "kind": "publicKeyValueNode",
                  "publicKey": "11111111111111111111111111111111"
                }
              }]
            }"#,
            "$.program.instructions[0].arguments[0].defaultValue",
            "publicKeyValueNode",
        ),
    ];
    for (instruction, path, kind) in cases {
        let error = LoadedSrf39Idl::from_json(&root_with_instruction(instruction))
            .expect_err("inert kinds stay unsupported outside inert positions");
        assert_eq!(
            error,
            Srf39IdlError::UnsupportedIdlNode {
                path: path.to_string(),
                kind: kind.to_string(),
            }
        );
    }
}

#[test]
fn unsupported_node_reports_its_json_path() {
    let json = r#"
    {
      "kind": "rootNode",
      "standard": "codama",
      "version": "1.8.0",
      "program": {
        "kind": "programNode",
        "name": "test",
        "publicKey": "11111111111111111111111111111111",
        "instructions": [{
          "kind": "instructionNode",
          "name": "memo",
          "arguments": [{
            "kind": "instructionArgumentNode",
            "name": "message",
            "type": { "kind": "stringTypeNode", "encoding": "utf8" }
          }]
        }]
      }
    }
    "#;
    let error = LoadedSrf39Idl::from_json(json).expect_err("bare strings have no fixed layout");
    assert_eq!(
        error,
        Srf39IdlError::UnsupportedIdlNode {
            path: "$.program.instructions[0].arguments[0].type".to_string(),
            kind: "stringTypeNode(variable size)".to_string(),
        }
    );
}

#[test]
fn rejects_non_v1_roots() {
    let json = r#"
    {
      "kind": "rootNode",
      "standard": "codama",
      "version": "2.0.0",
      "program": {
        "kind": "programNode",
        "name": "test",
        "publicKey": "11111111111111111111111111111111"
      }
    }
    "#;
    let error = LoadedSrf39Idl::from_json(json).expect_err("v2 must be rejected explicitly");
    assert!(matches!(error, Srf39IdlError::InvalidRoot { .. }));
}

#[test]
fn rejects_non_fixed_options_with_a_json_path() {
    let json = r#"
    {
      "kind": "rootNode",
      "standard": "codama",
      "version": "1.0.0",
      "program": {
        "kind": "programNode",
        "name": "test",
        "publicKey": "11111111111111111111111111111111",
        "instructions": [{
          "kind": "instructionNode",
          "name": "optionalOwner",
          "arguments": [{
            "kind": "instructionArgumentNode",
            "name": "owner",
            "type": {
              "kind": "optionTypeNode",
              "item": { "kind": "publicKeyTypeNode" },
              "prefix": { "kind": "numberTypeNode", "format": "u32", "endian": "le" }
            }
          }]
        }]
      }
    }
    "#;
    let error = LoadedSrf39Idl::from_json(json).expect_err("non-fixed option must be rejected");
    assert_eq!(
        error,
        Srf39IdlError::UnsupportedIdlNode {
            path: "$.program.instructions[0].arguments[0].type".to_string(),
            kind: "optionTypeNode(fixed=false)".to_string(),
        }
    );
}

#[test]
fn cross_program_account_links_load_and_dangling_links_degrade() {
    let json = r#"
    {
      "kind": "rootNode",
      "standard": "codama",
      "version": "1.0.0",
      "program": {
        "kind": "programNode",
        "name": "test",
        "publicKey": "11111111111111111111111111111111",
        "instructions": [{
          "kind": "instructionNode",
          "name": "readForeign",
          "accounts": [{
            "kind": "instructionAccountNode",
            "name": "foreign",
            "accountLink": {
              "kind": "accountLinkNode",
              "name": "state",
              "program": { "kind": "programLinkNode", "name": "otherProgram" }
            }
          }]
        }]
      }
    }
    "#;
    LoadedSrf39Idl::from_json(json).expect("a link to an unknown program loads and degrades");
}

#[test]
fn rust_matches_reference_cross_program_link_fixture() {
    run_fixture("cross-program-link");
}

#[test]
fn rust_matches_reference_flatten_struct_fixture() {
    run_fixture("flatten-struct");
}

#[test]
fn rust_matches_reference_subscriptions_fixture() {
    run_fixture("subscriptions");
}

#[test]
fn subscriptions_scenario_preserves_the_original_devnet_capture() {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Capture {
        program_id: String,
        instruction_data_hex: String,
        accounts: Vec<String>,
        authority_account_owner: String,
        authority_account_data_b64: String,
    }

    let capture: Capture = serde_json::from_str(include_str!(
        "../../../../vendor/fixtures/recurring_delegation_devnet.json"
    ))
    .expect("original Devnet capture");
    let (_, fixture, _) = load_fixture("subscriptions");
    let scenario = fixture
        .scenarios
        .iter()
        .find(|scenario| scenario.name == "createRecurringDelegation-vendor-bytes")
        .expect("captured transaction must remain in the conformance corpus");
    let instruction = OwnedInstruction::from_scenario(&fixture, scenario);

    assert_eq!(instruction.program_id, capture.program_id);
    assert_eq!(
        instruction
            .data
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        capture.instruction_data_hex
    );
    assert_eq!(
        instruction
            .accounts
            .iter()
            .map(|(address, _, _)| address)
            .collect::<Vec<_>>(),
        capture.accounts.iter().collect::<Vec<_>>()
    );
    assert!(scenario.fetch_accounts);
    let provider = fixture_provider(
        scenario
            .account_data
            .as_ref()
            .unwrap_or(&fixture.account_data),
    );
    let authority = provider
        .accounts
        .get(&capture.accounts[1])
        .expect("authority");
    assert_eq!(authority.owner, capture.authority_account_owner);
    assert_eq!(
        authority.data,
        STANDARD
            .decode(capture.authority_account_data_b64)
            .expect("captured authority base64")
    );
}

#[test]
fn rejects_flatten_metadata_the_reference_would_ignore() {
    let struct_type = format!(
        r#"{{ "kind": "structTypeNode", "fields": [
             {{ "kind": "structFieldTypeNode", "name": "a", "type": {U8_TYPE},
                "display": {{ "kind": "structFieldDisplayNode", "flatten": true }} }} ] }}"#
    );
    let cases = [
        (
            format!(
                r#"{{ "kind": "instructionArgumentNode", "name": "value", "type": {U8_TYPE},
                        "display": {{ "kind": "structFieldDisplayNode", "flatten": true }} }}"#
            ),
            "$.program.instructions[0].arguments[0].display",
            "structFieldDisplayNode(flatten on non-struct)",
        ),
        (
            format!(
                r#"{{ "kind": "instructionArgumentNode", "name": "value", "type": {U8_TYPE},
                        "display": {{ "kind": "structFieldDisplayNode", "flattenPrefix": "x." }} }}"#
            ),
            "$.program.instructions[0].arguments[0].display",
            "structFieldDisplayNode(flattenPrefix without flatten)",
        ),
        (
            format!(
                r#"{{ "kind": "instructionArgumentNode", "name": "value", "type": {struct_type} }}"#
            ),
            "$.program.instructions[0].arguments[0].type.fields[0].display",
            "structFieldDisplayNode(nested flatten)",
        ),
    ];
    for (argument, path, kind) in cases {
        let instruction =
            format!(r#"{{ "kind": "instructionNode", "name": "set", "arguments": [{argument}] }}"#);
        let error = LoadedSrf39Idl::from_json(&root_with_instruction(&instruction))
            .expect_err("flatten metadata must be rejected");
        assert_eq!(
            error,
            Srf39IdlError::UnsupportedIdlNode {
                path: path.to_string(),
                kind: kind.to_string(),
            }
        );
    }
}

#[test]
fn rejects_array_allocation_overflow_and_nested_zero_byte_expansion() {
    let array = |item: serde_json::Value, count: usize| {
        serde_json::json!({
            "kind": "arrayTypeNode", "item": item,
            "count": { "kind": "fixedCountNode", "value": count }
        })
    };
    let byte = serde_json::json!({ "kind": "numberTypeNode", "format": "u8" });
    let empty = serde_json::json!({ "kind": "structTypeNode", "fields": [] });
    let cases = [
        (array(byte.clone(), usize::MAX), false),
        (array(byte.clone(), 65_536), false),
        (array(array(empty, 256), 256), false),
        (array(byte, 65_535), true),
    ];
    for (ty, accepted) in cases {
        let instruction = serde_json::json!({
            "kind": "instructionNode", "name": "set",
            "arguments": [{ "kind": "instructionArgumentNode", "name": "values", "type": ty }]
        });
        let result = Srf39Engine::from_json(&root_with_instruction(&instruction.to_string()));
        if accepted {
            let engine = result.expect("the resource limit boundary is supported");
            let instruction = InstructionContext {
                program_id: engine.program_id(),
                instruction_data: &[1],
                accounts: &[],
                fee_payer: None,
            };
            assert_eq!(
                pollster::block_on(engine.display_instruction(&instruction, None)),
                Ok(None),
                "truncated input must produce a controlled miss"
            );
        } else {
            let error = result.expect_err("oversized value trees must be rejected at IDL load");
            assert!(matches!(error, Srf39IdlError::InvalidSchema { detail }
                if detail.contains("decoded value count")
                && detail.contains("$.program.instructions[0].arguments[0].type")));
        }
    }
}

fn run_fixture(name: &str) {
    let directory = fixture_directory(name);
    let root_json =
        std::fs::read_to_string(directory.join("root.json")).expect("read root fixture");
    let cases_json =
        std::fs::read_to_string(directory.join("cases.json")).expect("read cases fixture");
    let expected_json =
        std::fs::read_to_string(directory.join("expected.json")).expect("read expected fixture");
    let idl = LoadedSrf39Idl::from_json(&root_json).expect("load fixture IDL");
    let engine = Srf39Engine::from_json(&root_json).expect("load fixture engine");
    let fixture: Fixture = serde_json::from_str(&cases_json).expect("parse cases fixture");
    let expected: serde_json::Value =
        serde_json::from_str(&expected_json).expect("parse expected fixture");
    for scenario in &fixture.scenarios {
        let data = STANDARD
            .decode(
                scenario
                    .data_base64
                    .as_deref()
                    .unwrap_or(&fixture.data_base64),
            )
            .expect("valid instruction base64");
        let program_id = scenario
            .program_address
            .as_deref()
            .unwrap_or(&fixture.program_address);
        let fixture_accounts = scenario.accounts.as_ref().unwrap_or(&fixture.accounts);
        let accounts: Vec<AccountMeta<'_>> = fixture_accounts
            .iter()
            .map(|account| {
                let (is_signer, is_writable) = role_flags(&account.role);
                AccountMeta {
                    pubkey: &account.address,
                    is_signer,
                    is_writable,
                }
            })
            .collect();
        let instruction = InstructionContext {
            program_id,
            instruction_data: &data,
            accounts: &accounts,
            fee_payer: None,
        };
        let fixture_account_data = scenario
            .account_data
            .as_ref()
            .unwrap_or(&fixture.account_data);
        let provider = fixture_provider(fixture_account_data);
        let source = scenario
            .fetch_accounts
            .then_some(&provider as &dyn Srf39AccountProvider);
        let result = pollster::block_on(get_instruction_display(&idl, &instruction, source));
        let simple = pollster::block_on(engine.display_instruction(&instruction, source));
        let actual = match (scenario.expected_error.as_deref(), result) {
            (None, Ok(result)) => {
                // Parity: the simple API must be exactly the display half of
                // the detailed result, for every miss reason alike.
                let display = match result {
                    DisplayResult::Rendered { display, .. } => Some(display),
                    DisplayResult::Miss(_) => None,
                };
                assert_eq!(
                    simple.expect("simple API succeeds when detailed does"),
                    display,
                    "fixture {name}, scenario {}: simple/detailed parity",
                    scenario.name
                );
                serde_json::to_value(display).expect("serialize display")
            }
            (None, Err(error)) => panic!(
                "unexpected error for fixture {name}, scenario {}: {error}",
                scenario.name
            ),
            (Some("accountDecode"), Err(Srf39DisplayError::AccountDecode { .. })) => {
                serde_json::json!({ "error": "accountDecode" })
            }
            (Some(expected), Err(error)) => panic!(
                "expected {expected} for fixture {name}, scenario {}, got {error}",
                scenario.name
            ),
            (Some(expected), Ok(display)) => panic!(
                "expected {expected} for fixture {name}, scenario {}, got {display:?}",
                scenario.name
            ),
        };
        assert_eq!(
            actual, expected[&scenario.name],
            "fixture {name}, scenario {}",
            scenario.name
        );
    }
}

pub(super) fn fixture_provider(
    account_data: &BTreeMap<String, FixtureAccountData>,
) -> FixtureProvider {
    let accounts = account_data
        .iter()
        .map(|(address, account)| {
            let data = STANDARD
                .decode(&account.data_base64)
                .expect("valid account base64");
            (
                address.clone(),
                AccountData {
                    owner: account.program_address.clone(),
                    data,
                },
            )
        })
        .collect();
    FixtureProvider { accounts }
}

pub(super) fn role_flags(role: &str) -> (bool, bool) {
    match role {
        "readonly" => (false, false),
        "writable" => (false, true),
        "readonlySigner" => (true, false),
        "writableSigner" => (true, true),
        other => panic!("unknown fixture account role: {other}"),
    }
}

pub(super) fn fixture_directory(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/fixtures")
        .join(name)
}

/// Owned form of one scenario's instruction, so tests outside this module can
/// build an `InstructionContext` without repeating the fixture plumbing.
pub(super) struct OwnedInstruction {
    pub(super) program_id: String,
    pub(super) data: Vec<u8>,
    pub(super) accounts: Vec<(String, bool, bool)>,
}

impl OwnedInstruction {
    pub(super) fn from_scenario(fixture: &Fixture, scenario: &Scenario) -> Self {
        let data = STANDARD
            .decode(
                scenario
                    .data_base64
                    .as_deref()
                    .unwrap_or(&fixture.data_base64),
            )
            .expect("valid instruction base64");
        let program_id = scenario
            .program_address
            .clone()
            .unwrap_or_else(|| fixture.program_address.clone());
        let accounts = scenario
            .accounts
            .as_ref()
            .unwrap_or(&fixture.accounts)
            .iter()
            .map(|account| {
                let (is_signer, is_writable) = role_flags(&account.role);
                (account.address.clone(), is_signer, is_writable)
            })
            .collect();
        Self {
            program_id,
            data,
            accounts,
        }
    }

    pub(super) fn metas(&self) -> Vec<AccountMeta<'_>> {
        self.accounts
            .iter()
            .map(|(pubkey, is_signer, is_writable)| AccountMeta {
                pubkey,
                is_signer: *is_signer,
                is_writable: *is_writable,
            })
            .collect()
    }
}

pub(super) fn load_fixture(name: &str) -> (String, Fixture, serde_json::Value) {
    let directory = fixture_directory(name);
    let root_json =
        std::fs::read_to_string(directory.join("root.json")).expect("read root fixture");
    let cases_json =
        std::fs::read_to_string(directory.join("cases.json")).expect("read cases fixture");
    let expected_json =
        std::fs::read_to_string(directory.join("expected.json")).expect("read expected fixture");
    let fixture: Fixture = serde_json::from_str(&cases_json).expect("parse cases fixture");
    let expected: serde_json::Value =
        serde_json::from_str(&expected_json).expect("parse expected fixture");
    (root_json, fixture, expected)
}
