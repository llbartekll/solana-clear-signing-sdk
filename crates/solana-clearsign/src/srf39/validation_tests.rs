//! Load-time resource limits and refusal of ambiguous IDLs.

use serde_json::{json, Value};

use crate::InstructionContext;

use super::{Srf39Engine, Srf39IdlError};

const PROGRAM: &str = "11111111111111111111111111111111";

fn u8_type() -> Value {
    json!({ "kind": "numberTypeNode", "format": "u8" })
}

fn link(name: &str) -> Value {
    json!({ "kind": "definedTypeLinkNode", "name": name })
}

fn field(name: &str, ty: Value) -> Value {
    json!({ "kind": "structFieldTypeNode", "name": name, "type": ty })
}

fn structure(fields: Vec<Value>) -> Value {
    json!({ "kind": "structTypeNode", "fields": fields })
}

fn array(item: Value, count: usize) -> Value {
    json!({
        "kind": "arrayTypeNode", "item": item,
        "count": { "kind": "fixedCountNode", "value": count }
    })
}

fn argument(name: &str, ty: Value) -> Value {
    json!({ "kind": "instructionArgumentNode", "name": name, "type": ty })
}

fn defined(name: &str, ty: Value) -> Value {
    json!({ "kind": "definedTypeNode", "name": name, "type": ty })
}

fn root(defined_types: Vec<Value>, instruction: Value) -> Value {
    json!({
        "kind": "rootNode", "standard": "codama", "version": "1.0.0",
        "program": {
            "kind": "programNode", "name": "test", "publicKey": PROGRAM,
            "definedTypes": defined_types, "instructions": [instruction]
        }
    })
}

fn instruction(arguments: Vec<Value>) -> Value {
    json!({ "kind": "instructionNode", "name": "run", "arguments": arguments })
}

fn load(root: &Value) -> Result<Srf39Engine, Srf39IdlError> {
    Srf39Engine::from_json(&root.to_string())
}

fn schema_error(root: &Value) -> String {
    match load(root).expect_err("must be refused") {
        Srf39IdlError::InvalidSchema { detail } => detail,
        other => panic!("expected a schema error, got {other}"),
    }
}

fn on_small_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(work)
        .expect("spawn")
        .join()
        .expect("no panic")
}

/// `t0 -> t1 -> … -> u8`, each level one array.
fn chain(levels: usize) -> Value {
    let mut types: Vec<Value> = (0..levels)
        .map(|index| {
            defined(
                &format!("t{index}"),
                array(link(&format!("t{}", index + 1)), 1),
            )
        })
        .collect();
    types.push(defined(&format!("t{levels}"), u8_type()));
    root(types, instruction(vec![argument("value", link("t0"))]))
}

#[test]
fn exponential_link_expansion_is_refused_before_it_is_built() {
    // 2^28 leaves behind a few KB of JSON, within the nesting limit.
    let mut types: Vec<Value> = (0..14)
        .map(|index| {
            let next = format!("t{}", index + 1);
            defined(
                &format!("t{index}"),
                structure(
                    ["a", "b", "c", "d"]
                        .map(|name| field(name, link(&next)))
                        .to_vec(),
                ),
            )
        })
        .collect();
    types.push(defined("t14", u8_type()));
    let detail = schema_error(&root(types, instruction(vec![argument("v", link("t0"))])));
    assert!(
        detail.contains("defined type expansion exceeds"),
        "{detail}"
    );
}

#[test]
fn deep_link_chains_are_refused_without_exhausting_a_small_stack() {
    let detail = on_small_stack(|| schema_error(&chain(1_500)));
    assert!(detail.contains("type nesting exceeds"), "{detail}");
}

#[test]
fn deepest_accepted_type_loads_and_renders_on_a_small_stack() {
    let display = on_small_stack(|| {
        // Each level is an array plus a link, plus the final number.
        let engine = load(&chain(15)).expect("within the limit");
        let instruction = InstructionContext {
            program_id: PROGRAM,
            instruction_data: &[7],
            accounts: &[],
            fee_payer: None,
        };
        pollster::block_on(engine.display_instruction(&instruction, None)).expect("render")
    });
    assert_eq!(display.expect("rendered").fields[0].value, "7");
    assert!(load(&chain(16)).is_err());
}

#[test]
fn decoded_value_limit_covers_the_whole_instruction() {
    let empty = || array(structure(vec![]), 32_767);
    assert!(load(&root(
        vec![],
        instruction(vec![argument("a", empty()), argument("b", empty())])
    ))
    .is_ok());
    let detail = schema_error(&root(
        vec![],
        instruction(vec![
            argument("a", empty()),
            argument("b", empty()),
            argument("c", u8_type()),
        ]),
    ));
    assert!(
        detail.contains("decoded value count")
            && detail.contains("$.program.instructions[0].arguments"),
        "{detail}"
    );
}

#[test]
fn long_field_names_count_towards_the_decoded_value_limit() {
    let named = |length: usize| {
        let item = structure(vec![field(&"n".repeat(length), structure(vec![]))]);
        root(vec![], instruction(vec![argument("a", array(item, 1_000))]))
    };
    assert!(load(&named(63)).is_ok());
    assert!(schema_error(&named(100_000)).contains("decoded value count"));
}

#[test]
fn duplicate_names_are_refused_with_a_path() {
    let account = |name: &str| json!({ "kind": "instructionAccountNode", "name": name });
    let mut accounts = root(vec![], instruction(vec![]));
    accounts["program"]["instructions"][0]["accounts"] =
        json!([account("dest"), account("other"), account("dest")]);
    let mut layouts = root(vec![], instruction(vec![]));
    let layout = json!({ "kind": "accountNode", "name": "mint", "data": structure(vec![]) });
    layouts["program"]["accounts"] = json!([layout, layout]);
    let cases = [
        (
            accounts,
            "duplicate instruction account name 'dest' at $.program.instructions[0].accounts[2]",
        ),
        (
            layouts,
            "duplicate account name 'mint' at $.program.accounts[1]",
        ),
        (
            root(
                vec![],
                instruction(vec![argument("x", u8_type()), argument("x", u8_type())]),
            ),
            "duplicate argument name 'x' at $.program.instructions[0].arguments[1]",
        ),
        (
            root(
                vec![],
                instruction(vec![argument(
                    "s",
                    structure(vec![field("x", u8_type()), field("x", u8_type())]),
                )]),
            ),
            "duplicate struct field name 'x' at $.program.instructions[0].arguments[0].type.fields[1]",
        ),
    ];
    for (root, expected) in cases {
        assert_eq!(schema_error(&root), expected);
    }
}

#[test]
fn field_discriminators_require_a_number_that_holds_the_default() {
    let discriminated = |ty: Value, default: u64| {
        let mut argument = argument("tag", ty);
        argument["defaultValue"] = json!({ "kind": "numberValueNode", "number": default });
        let mut instruction = instruction(vec![argument]);
        instruction["discriminators"] =
            json!([{ "kind": "fieldDiscriminatorNode", "name": "tag", "offset": 0 }]);
        root(vec![], instruction)
    };
    let number = |format: &str| json!({ "kind": "numberTypeNode", "format": format });
    for (format, maximum) in [
        ("u8", u64::from(u8::MAX)),
        ("u32", u64::from(u32::MAX)),
        ("i64", i64::MAX as u64),
    ] {
        assert!(
            load(&discriminated(number(format), maximum)).is_ok(),
            "{format}"
        );
        let detail = schema_error(&discriminated(number(format), maximum + 1));
        assert!(
            detail.contains("$.program.instructions[0].arguments[0].defaultValue"),
            "{detail}"
        );
    }
    assert!(load(&discriminated(number("u64"), u64::MAX)).is_ok());
    assert!(matches!(
        load(&discriminated(
            json!({ "kind": "publicKeyTypeNode" }),
            1
        )),
        Err(Srf39IdlError::UnsupportedIdlNode { path, .. })
            if path == "$.program.instructions[0].discriminators[0]"
    ));
}

#[test]
fn literal_amount_decimals_above_255_are_refused() {
    let amount = |decimals: Value| {
        root(
            vec![],
            instruction(vec![argument(
                "amount",
                json!({ "kind": "numberTypeNode", "format": "u64",
                        "display": { "kind": "amountNumberDisplayNode", "decimals": decimals } }),
            )]),
        )
    };
    let literal = |number: u64| json!({ "kind": "numberValueNode", "number": number });
    assert!(load(&amount(literal(255))).is_ok());
    for (decimals, suffix) in [
        (literal(256), "decimals"),
        (
            json!({ "kind": "injectedValueNode", "key": "d", "fallback": literal(300) }),
            "decimals.fallback",
        ),
    ] {
        assert_eq!(
            load(&amount(decimals)).expect_err("refused"),
            Srf39IdlError::UnsupportedIdlNode {
                path: format!("$.program.instructions[0].arguments[0].type.display.{suffix}"),
                kind: "numberValueNode(amount decimals above 255)".to_string(),
            }
        );
    }
}

#[test]
fn errors_inside_a_linked_type_cite_its_definition() {
    let mut idl = root(
        vec![],
        instruction(vec![argument(
            "config",
            json!({ "kind": "definedTypeLinkNode", "name": "config",
                    "program": { "kind": "programLinkNode", "name": "other" } }),
        )]),
    );
    idl["additionalPrograms"] = json!([{
        "kind": "programNode", "name": "other", "publicKey": PROGRAM,
        "definedTypes": [defined("config", structure(vec![
            field("a", u8_type()),
            field("b", json!({ "kind": "numberTypeNode", "format": "u16" })),
        ]))]
    }]);
    assert!(matches!(
        load(&idl),
        Err(Srf39IdlError::UnsupportedIdlNode { path, .. })
            if path == "$.additionalPrograms[0].definedTypes[0].type.fields[1].type"
    ));
}

#[test]
fn any_node_kind_is_accepted_in_inert_positions_only() {
    let mut idl = root(vec![], instruction(vec![]));
    idl["program"]["pdas"] = json!([{ "kind": "pdaNode", "name": "p", "seeds": [
        { "kind": "constantPdaSeedNode", "type": { "kind": "bytesTypeNode" },
          "value": { "kind": "bytesValueNode", "data": "00", "encoding": "base16" } }
    ]}]);
    idl["program"]["instructions"][0]["accounts"] = json!([{
        "kind": "instructionAccountNode", "name": "payer",
        "defaultValue": { "kind": "payerValueNode" }
    }]);
    load(&idl).expect("inert subtrees are opaque");
    idl["program"]["instructions"][0]["accounts"][0]["display"] =
        json!({ "kind": "payerValueNode" });
    assert!(load(&idl).is_err());
}
