use serde::Deserialize;
use serde_json::Value;

use super::Srf39IdlError;

// A host-side resource limit, not a Codama schema rule. Count containers as
// well as leaves so nested arrays of zero-byte values cannot amplify work.
const MAX_DECODED_VALUES: usize = 65_536;

const SUPPORTED_KINDS: &[&str] = &[
    "accountFieldValueNode",
    "accountLinkNode",
    "accountNode",
    "amountNumberDisplayNode",
    "argumentValueNode",
    "arrayTypeNode",
    "booleanTypeNode",
    "dateTimeNumberDisplayNode",
    "definedTypeLinkNode",
    "definedTypeNode",
    "durationNumberDisplayNode",
    "enumEmptyVariantTypeNode",
    "enumTypeNode",
    "enumVariantDisplayNode",
    "fieldDiscriminatorNode",
    "fixedCountNode",
    "fixedSizeTypeNode",
    "identityValueNode",
    "injectedValueNode",
    "instructionAccountDisplayNode",
    "instructionAccountNode",
    "instructionArgumentNode",
    "instructionDisplayNode",
    "instructionNode",
    "instructionRemainingAccountsNode",
    "numberTypeNode",
    "numberValueNode",
    "optionTypeNode",
    "programLinkNode",
    "programNode",
    "providedNode",
    "publicKeyTypeNode",
    "rootNode",
    "sizeDiscriminatorNode",
    "stringTypeNode",
    "stringValueNode",
    "structFieldDisplayNode",
    "structFieldTypeNode",
    "structTypeNode",
];

/// Node kinds the reference display runtime never consults. They are accepted
/// only inside [`INERT_POSITIONS`], where they describe PDA derivation,
/// program errors, or client-side account defaults; anywhere else they are
/// unsupported like any other unknown kind.
const INERT_KINDS: &[&str] = &[
    "accountValueNode",
    "constantPdaSeedNode",
    "errorNode",
    "pdaLinkNode",
    "pdaNode",
    "pdaSeedValueNode",
    "pdaValueNode",
    "publicKeyValueNode",
    "stringTypeNode",
    "variablePdaSeedNode",
];

/// `(enclosing node kind, key)` pairs whose subtree is inert for display.
const INERT_POSITIONS: &[(&str, &str)] = &[
    ("programNode", "errors"),
    ("programNode", "pdas"),
    ("accountNode", "pda"),
    ("instructionAccountNode", "defaultValue"),
];

#[derive(Debug, Clone)]
pub struct LoadedSrf39Idl {
    pub(crate) root: RootNode,
}

impl LoadedSrf39Idl {
    pub fn from_json(json: &str) -> Result<Self, Srf39IdlError> {
        let value: Value =
            serde_json::from_str(json).map_err(|error| Srf39IdlError::InvalidJson {
                detail: error.to_string(),
            })?;
        validate_known_kinds(&value, "$", false)?;
        let mut root: RootNode =
            serde_json::from_value(value).map_err(|error| Srf39IdlError::InvalidSchema {
                detail: error.to_string(),
            })?;
        check_links(&root)?;
        resolve_links(&mut root)?;
        validate_root(&root)?;
        Ok(Self { root })
    }

    pub(crate) fn primary_program(&self) -> &ProgramNode {
        &self.root.program
    }

    pub(crate) fn additional_programs(&self) -> &[ProgramNode] {
        &self.root.additional_programs
    }

    pub(crate) fn program_by_address(&self, address: &str) -> Option<&ProgramNode> {
        self.root
            .programs()
            .find(|program| program.public_key == address)
    }
}

impl RootNode {
    /// The primary program followed by the additional programs.
    fn programs(&self) -> impl Iterator<Item = &ProgramNode> {
        std::iter::once(&self.program).chain(self.additional_programs.iter())
    }

    /// Link targets are addressed by program *name*, as in the reference
    /// `LinkableDictionary`.
    pub(crate) fn program_by_name(&self, name: &str) -> Option<&ProgramNode> {
        self.programs().find(|program| program.name == name)
    }

    fn defined_type(&self, program_name: &str, type_name: &str) -> Option<&DefinedTypeNode> {
        self.program_by_name(program_name)?
            .defined_types
            .iter()
            .find(|defined| defined.name == type_name)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RootNode {
    pub kind: String,
    pub standard: String,
    pub version: String,
    pub program: ProgramNode,
    #[serde(default)]
    pub additional_programs: Vec<ProgramNode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProgramNode {
    pub kind: String,
    pub name: String,
    pub public_key: String,
    #[serde(default)]
    pub accounts: Vec<AccountNode>,
    #[serde(default)]
    pub instructions: Vec<InstructionNode>,
    #[serde(default)]
    pub defined_types: Vec<DefinedTypeNode>,
}

/// A named reusable type. Links to it are inlined by [`resolve_links`] when
/// the IDL is loaded, so the renderer never sees a `definedTypeLinkNode`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DefinedTypeNode {
    pub kind: String,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeNode,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ProgramLinkNode {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AccountNode {
    pub kind: String,
    pub name: String,
    pub data: TypeNode,
    #[serde(default)]
    pub discriminators: Vec<DiscriminatorNode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstructionNode {
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub accounts: Vec<InstructionAccountNode>,
    #[serde(default)]
    pub arguments: Vec<InstructionArgumentNode>,
    #[serde(default)]
    pub discriminators: Vec<DiscriminatorNode>,
    #[serde(default)]
    pub remaining_accounts: Vec<InstructionRemainingAccountsNode>,
    #[serde(default)]
    pub provides: Vec<ProvidedNode>,
    pub display: Option<InstructionDisplayNode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstructionAccountNode {
    pub kind: String,
    pub name: String,
    pub account_link: Option<AccountLinkNode>,
    /// Client-side resolution hint (identity, PDA, constant address). The
    /// display never substitutes it for a missing meta, so it is kept opaque.
    #[allow(dead_code)]
    pub default_value: Option<Value>,
    pub display: Option<InstructionAccountDisplayNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AccountLinkNode {
    pub kind: String,
    pub name: String,
    /// Names an additional program whose `accountNode` this link targets;
    /// absent means the surrounding program. A link that resolves to nothing
    /// simply never yields a value, as in the reference.
    pub program: Option<ProgramLinkNode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstructionArgumentNode {
    pub kind: String,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeNode,
    pub default_value: Option<ValueNode>,
    pub display: Option<StructFieldDisplayNode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstructionRemainingAccountsNode {
    pub kind: String,
    pub value: RemainingAccountsValueNode,
    pub is_signer: Option<SignerRule>,
    pub display: Option<InstructionAccountDisplayNode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum RemainingAccountsValueNode {
    #[serde(rename = "argumentValueNode")]
    Argument { name: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum SignerRule {
    Boolean(bool),
    Named(String),
}

impl SignerRule {
    pub(crate) fn matches(&self, is_signer: bool) -> bool {
        match self {
            Self::Boolean(expected) => *expected == is_signer,
            Self::Named(value) => value == "either",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum TypeNode {
    #[serde(rename = "booleanTypeNode")]
    Boolean { size: Box<TypeNode> },
    #[serde(rename = "numberTypeNode")]
    Number {
        format: String,
        endian: Option<String>,
        display: Option<NumberDisplayNode>,
    },
    #[serde(rename = "optionTypeNode")]
    Option {
        item: Box<TypeNode>,
        prefix: Box<TypeNode>,
        #[serde(default)]
        fixed: bool,
    },
    #[serde(rename = "publicKeyTypeNode")]
    PublicKey,
    #[serde(rename = "structTypeNode")]
    Struct {
        #[serde(default)]
        fields: Vec<StructFieldTypeNode>,
    },
    /// Present only between parsing and [`resolve_links`]; `program` names an
    /// additional program, otherwise the surrounding program is assumed.
    #[serde(rename = "definedTypeLinkNode")]
    DefinedTypeLink {
        name: String,
        program: Option<ProgramLinkNode>,
    },
    /// A scalar enum: the wire value is the variant index in `size`.
    #[serde(rename = "enumTypeNode")]
    Enum {
        size: Box<TypeNode>,
        #[serde(default)]
        variants: Vec<EnumVariantNode>,
    },
    /// Exactly `size` bytes of the inner type; only a UTF-8 string is accepted.
    #[serde(rename = "fixedSizeTypeNode")]
    FixedSize {
        size: usize,
        #[serde(rename = "type")]
        ty: Box<TypeNode>,
    },
    /// Variable-size on its own; accepted only inside `fixedSizeTypeNode`.
    #[serde(rename = "stringTypeNode")]
    String { encoding: String },
    /// A fixed-count sequence of `item`.
    #[serde(rename = "arrayTypeNode")]
    Array {
        item: Box<TypeNode>,
        count: CountNode,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum CountNode {
    #[serde(rename = "fixedCountNode")]
    Fixed { value: usize },
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct EnumVariantNode {
    pub kind: String,
    pub name: String,
    /// When present it must equal the variant's index; the pinned reference
    /// decoder addresses variants by position.
    pub discriminator: Option<u64>,
    pub display: Option<EnumVariantDisplayNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct EnumVariantDisplayNode {
    pub kind: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct StructFieldTypeNode {
    pub kind: String,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeNode,
    pub display: Option<StructFieldDisplayNode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum NumberDisplayNode {
    #[serde(rename = "amountNumberDisplayNode")]
    Amount {
        decimals: Option<ValueNode>,
        unit: Option<ValueNode>,
    },
    /// Ticks since the Unix epoch; `ticksPerSecond` defaults to `1`.
    #[serde(rename = "dateTimeNumberDisplayNode", rename_all = "camelCase")]
    DateTime {
        ticks_per_second: Option<serde_json::Number>,
    },
    /// Elapsed ticks; `ticksPerSecond` defaults to `1`.
    #[serde(rename = "durationNumberDisplayNode", rename_all = "camelCase")]
    Duration {
        ticks_per_second: Option<serde_json::Number>,
    },
}

/// The effective `ticksPerSecond` of a validated time display: absent means
/// the value already counts seconds.
pub(crate) fn ticks_per_second(number: Option<&serde_json::Number>) -> u32 {
    number
        .and_then(serde_json::Number::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(1)
}

/// The engine accepts only an absent or positive-integer `ticksPerSecond`.
/// The reference also divides by fractions and renders raw for `<= 0`; that
/// corner is refused at load time rather than replicated.
fn validate_ticks_per_second(
    number: Option<&serde_json::Number>,
    path: &str,
    kind: &str,
) -> Result<(), Srf39IdlError> {
    let Some(number) = number else {
        return Ok(());
    };
    let valid = number
        .as_u64()
        .is_some_and(|value| value >= 1 && u32::try_from(value).is_ok());
    if valid {
        Ok(())
    } else {
        Err(Srf39IdlError::UnsupportedIdlNode {
            path: format!("{path}.display.ticksPerSecond"),
            kind: format!("{kind}(ticksPerSecond)"),
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum ValueNode {
    #[serde(rename = "argumentValueNode")]
    Argument {
        #[serde(rename = "name")]
        _name: String,
    },
    #[serde(rename = "identityValueNode")]
    Identity,
    #[serde(rename = "numberValueNode")]
    Number { number: u64 },
    #[serde(rename = "stringValueNode")]
    String { string: String },
    #[serde(rename = "injectedValueNode")]
    Injected {
        key: String,
        fallback: Option<Box<ValueNode>>,
    },
    #[serde(rename = "accountFieldValueNode")]
    AccountField { account: String, path: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum DiscriminatorNode {
    #[serde(rename = "fieldDiscriminatorNode")]
    Field {
        name: String,
        #[serde(default)]
        offset: usize,
    },
    #[serde(rename = "sizeDiscriminatorNode")]
    Size { size: usize },
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ProvidedNode {
    pub kind: String,
    pub name: String,
    pub node: ValueNode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstructionDisplayNode {
    pub kind: String,
    pub intent: Option<String>,
    pub interpolated_intent: Option<String>,
    /// SDK extension. Ignored by the pinned Codama renderer.
    #[serde(rename = "x-solana-clearsign")]
    pub metadata: Option<MetadataDisplay>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MetadataDisplay {
    pub token_amounts: Vec<TokenAmountBinding>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TokenAmountBinding {
    pub amount: String,
    pub mint: TokenMintReference,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "source", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum TokenMintReference {
    Argument { name: String },
    Account { name: String },
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct InstructionAccountDisplayNode {
    pub kind: String,
    pub label: Option<String>,
    pub skip: Option<DisplaySkip>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StructFieldDisplayNode {
    pub kind: String,
    pub label: Option<String>,
    pub skip: Option<DisplaySkip>,
    #[serde(default)]
    pub flatten: bool,
    pub flatten_prefix: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub(crate) enum DisplaySkip {
    #[serde(rename = "always")]
    Always,
    #[serde(rename = "never")]
    Never,
    #[serde(rename = "whenInjected")]
    WhenInjected,
}

/// Walks the whole JSON tree and rejects any `kind` outside the implemented
/// set. `inert` is `true` inside an [`INERT_POSITIONS`] subtree, where the
/// reference display runtime never looks and [`INERT_KINDS`] are tolerated.
fn validate_known_kinds(value: &Value, path: &str, inert: bool) -> Result<(), Srf39IdlError> {
    match value {
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                validate_known_kinds(child, &format!("{path}[{index}]"), inert)?;
            }
        }
        Value::Object(object) => {
            let kind = object.get("kind").and_then(Value::as_str);
            if let Some(kind) = kind {
                let allowed =
                    SUPPORTED_KINDS.contains(&kind) || (inert && INERT_KINDS.contains(&kind));
                if !allowed {
                    return Err(Srf39IdlError::UnsupportedIdlNode {
                        path: path.to_string(),
                        kind: kind.to_string(),
                    });
                }
            }
            for (key, child) in object {
                let child_inert = inert
                    || kind.is_some_and(|kind| INERT_POSITIONS.contains(&(kind, key.as_str())));
                validate_known_kinds(child, &format!("{path}.{key}"), child_inert)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// `(program, JSON path)` for the primary and every additional program.
fn programs_with_paths(root: &RootNode) -> Vec<(&ProgramNode, String)> {
    std::iter::once((&root.program, "$.program".to_string()))
        .chain(
            root.additional_programs
                .iter()
                .enumerate()
                .map(|(index, program)| (program, format!("$.additionalPrograms[{index}]"))),
        )
        .collect()
}

/// Structural checks that must hold before links are followed: unique program
/// and defined-type names, and every `definedTypeLinkNode` naming an existing
/// program and type.
fn check_links(root: &RootNode) -> Result<(), Srf39IdlError> {
    let mut program_names = std::collections::HashSet::new();
    for (program, path) in programs_with_paths(root) {
        ensure_kind(&program.kind, "programNode", &path)?;
        if !program_names.insert(program.name.as_str()) {
            return Err(Srf39IdlError::InvalidSchema {
                detail: format!("duplicate program name '{}' at {path}", program.name),
            });
        }
        let mut type_names = std::collections::HashSet::new();
        for (index, defined) in program.defined_types.iter().enumerate() {
            let defined_path = format!("{path}.definedTypes[{index}]");
            ensure_kind(&defined.kind, "definedTypeNode", &defined_path)?;
            if !type_names.insert(defined.name.as_str()) {
                return Err(Srf39IdlError::InvalidSchema {
                    detail: format!(
                        "duplicate defined type name '{}' at {defined_path}",
                        defined.name
                    ),
                });
            }
            check_type_links(
                root,
                &program.name,
                &defined.ty,
                &format!("{defined_path}.type"),
            )?;
        }
        for (index, account) in program.accounts.iter().enumerate() {
            check_type_links(
                root,
                &program.name,
                &account.data,
                &format!("{path}.accounts[{index}].data"),
            )?;
        }
        for (index, instruction) in program.instructions.iter().enumerate() {
            for (argument_index, argument) in instruction.arguments.iter().enumerate() {
                check_type_links(
                    root,
                    &program.name,
                    &argument.ty,
                    &format!("{path}.instructions[{index}].arguments[{argument_index}].type"),
                )?;
            }
        }
    }
    Ok(())
}

fn check_type_links(
    root: &RootNode,
    scope: &str,
    node: &TypeNode,
    path: &str,
) -> Result<(), Srf39IdlError> {
    match node {
        TypeNode::Boolean { size } | TypeNode::Enum { size, .. } => {
            check_type_links(root, scope, size, &format!("{path}.size"))
        }
        TypeNode::FixedSize { ty, .. } => {
            check_type_links(root, scope, ty, &format!("{path}.type"))
        }
        TypeNode::Array { item, .. } => {
            check_type_links(root, scope, item, &format!("{path}.item"))
        }
        TypeNode::Number { .. } | TypeNode::PublicKey | TypeNode::String { .. } => Ok(()),
        TypeNode::Option { item, prefix, .. } => {
            check_type_links(root, scope, item, &format!("{path}.item"))?;
            check_type_links(root, scope, prefix, &format!("{path}.prefix"))
        }
        TypeNode::Struct { fields } => {
            for (index, field) in fields.iter().enumerate() {
                check_type_links(
                    root,
                    scope,
                    &field.ty,
                    &format!("{path}.fields[{index}].type"),
                )?;
            }
            Ok(())
        }
        TypeNode::DefinedTypeLink { name, program } => {
            if let Some(link) = program {
                ensure_kind(&link.kind, "programLinkNode", &format!("{path}.program"))?;
            }
            let program_name = program.as_ref().map_or(scope, |link| link.name.as_str());
            if root.program_by_name(program_name).is_none() {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: format!("{path}.program"),
                    kind: "programLinkNode(unresolved)".to_string(),
                });
            }
            if root.defined_type(program_name, name).is_none() {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: path.to_string(),
                    kind: "definedTypeLinkNode(unresolved)".to_string(),
                });
            }
            Ok(())
        }
    }
}

/// Replaces every `definedTypeLinkNode` with a copy of its target, scoping
/// nested links to the target's own program (the reference rebases the owner
/// path the same way). Defined types are inlined first, each with itself on
/// the expansion stack, so a recursive type is reported at its own definition.
fn resolve_links(root: &mut RootNode) -> Result<(), Srf39IdlError> {
    let source = root.clone();
    let mut programs: Vec<(&mut ProgramNode, String)> =
        vec![(&mut root.program, "$.program".to_string())];
    programs.extend(
        root.additional_programs
            .iter_mut()
            .enumerate()
            .map(|(index, program)| (program, format!("$.additionalPrograms[{index}]"))),
    );
    for (program, path) in programs {
        let scope = program.name.clone();
        for (index, defined) in program.defined_types.iter_mut().enumerate() {
            let mut stack = vec![(scope.clone(), defined.name.clone())];
            defined.ty = inline_type(
                &source,
                &scope,
                &defined.ty,
                &format!("{path}.definedTypes[{index}].type"),
                &mut stack,
            )?;
        }
        for (index, account) in program.accounts.iter_mut().enumerate() {
            account.data = inline_type(
                &source,
                &scope,
                &account.data,
                &format!("{path}.accounts[{index}].data"),
                &mut Vec::new(),
            )?;
        }
        for (index, instruction) in program.instructions.iter_mut().enumerate() {
            for (argument_index, argument) in instruction.arguments.iter_mut().enumerate() {
                argument.ty = inline_type(
                    &source,
                    &scope,
                    &argument.ty,
                    &format!("{path}.instructions[{index}].arguments[{argument_index}].type"),
                    &mut Vec::new(),
                )?;
            }
        }
    }
    Ok(())
}

fn inline_type(
    source: &RootNode,
    scope: &str,
    node: &TypeNode,
    path: &str,
    stack: &mut Vec<(String, String)>,
) -> Result<TypeNode, Srf39IdlError> {
    Ok(match node {
        TypeNode::Boolean { size } => TypeNode::Boolean {
            size: Box::new(inline_type(
                source,
                scope,
                size,
                &format!("{path}.size"),
                stack,
            )?),
        },
        TypeNode::Number { .. } | TypeNode::PublicKey | TypeNode::String { .. } => node.clone(),
        TypeNode::FixedSize { size, ty } => TypeNode::FixedSize {
            size: *size,
            ty: Box::new(inline_type(
                source,
                scope,
                ty,
                &format!("{path}.type"),
                stack,
            )?),
        },
        TypeNode::Array { item, count } => TypeNode::Array {
            item: Box::new(inline_type(
                source,
                scope,
                item,
                &format!("{path}.item"),
                stack,
            )?),
            count: count.clone(),
        },
        TypeNode::Enum { size, variants } => TypeNode::Enum {
            size: Box::new(inline_type(
                source,
                scope,
                size,
                &format!("{path}.size"),
                stack,
            )?),
            variants: variants.clone(),
        },
        TypeNode::Option {
            item,
            prefix,
            fixed,
        } => TypeNode::Option {
            item: Box::new(inline_type(
                source,
                scope,
                item,
                &format!("{path}.item"),
                stack,
            )?),
            prefix: Box::new(inline_type(
                source,
                scope,
                prefix,
                &format!("{path}.prefix"),
                stack,
            )?),
            fixed: *fixed,
        },
        TypeNode::Struct { fields } => {
            let mut inlined = Vec::with_capacity(fields.len());
            for (index, field) in fields.iter().enumerate() {
                inlined.push(StructFieldTypeNode {
                    ty: inline_type(
                        source,
                        scope,
                        &field.ty,
                        &format!("{path}.fields[{index}].type"),
                        stack,
                    )?,
                    ..field.clone()
                });
            }
            TypeNode::Struct { fields: inlined }
        }
        TypeNode::DefinedTypeLink { name, program } => {
            let program_name = program.as_ref().map_or(scope, |link| link.name.as_str());
            let key = (program_name.to_string(), name.clone());
            if stack.contains(&key) {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: path.to_string(),
                    kind: "definedTypeLinkNode(cycle)".to_string(),
                });
            }
            let Some(target) = source.defined_type(program_name, name) else {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: path.to_string(),
                    kind: "definedTypeLinkNode(unresolved)".to_string(),
                });
            };
            stack.push(key);
            let inlined = inline_type(source, program_name, &target.ty, path, stack)?;
            stack.pop();
            inlined
        }
    })
}

fn validate_root(root: &RootNode) -> Result<(), Srf39IdlError> {
    if root.kind != "rootNode" || root.standard != "codama" {
        return Err(Srf39IdlError::InvalidRoot {
            detail: "expected an sRFC 39 rootNode".to_string(),
        });
    }
    let major = root.version.split('.').next();
    if major != Some("1") {
        return Err(Srf39IdlError::InvalidRoot {
            detail: format!("unsupported sRFC 39 IDL version '{}'", root.version),
        });
    }

    for (program, path) in programs_with_paths(root) {
        validate_program(program, &path)?;
    }
    Ok(())
}

fn validate_program(program: &ProgramNode, path: &str) -> Result<(), Srf39IdlError> {
    ensure_kind(&program.kind, "programNode", path)?;
    for (index, defined) in program.defined_types.iter().enumerate() {
        validate_type(&defined.ty, &format!("{path}.definedTypes[{index}].type"))?;
    }
    for (index, account) in program.accounts.iter().enumerate() {
        let account_path = format!("{path}.accounts[{index}]");
        ensure_kind(&account.kind, "accountNode", &account_path)?;
        validate_type(&account.data, &format!("{account_path}.data"))?;
        for (discriminator_index, discriminator) in account.discriminators.iter().enumerate() {
            let discriminator_path =
                format!("{account_path}.discriminators[{discriminator_index}]");
            if matches!(discriminator, DiscriminatorNode::Field { .. }) {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: discriminator_path,
                    kind: "fieldDiscriminatorNode(account)".to_string(),
                });
            }
        }
    }
    for (index, instruction) in program.instructions.iter().enumerate() {
        validate_instruction(instruction, &format!("{path}.instructions[{index}]"))?;
    }
    Ok(())
}

fn validate_instruction(instruction: &InstructionNode, path: &str) -> Result<(), Srf39IdlError> {
    ensure_kind(&instruction.kind, "instructionNode", path)?;
    if let Some(display) = &instruction.display {
        ensure_kind(
            &display.kind,
            "instructionDisplayNode",
            &format!("{path}.display"),
        )?;
        if let Some(metadata) = &display.metadata {
            validate_token_amounts(instruction, metadata, path)?;
        }
    }
    for (index, account) in instruction.accounts.iter().enumerate() {
        let account_path = format!("{path}.accounts[{index}]");
        ensure_kind(&account.kind, "instructionAccountNode", &account_path)?;
        if let Some(link) = &account.account_link {
            ensure_kind(
                &link.kind,
                "accountLinkNode",
                &format!("{account_path}.accountLink"),
            )?;
            if let Some(program) = &link.program {
                ensure_kind(
                    &program.kind,
                    "programLinkNode",
                    &format!("{account_path}.accountLink.program"),
                )?;
            }
        }
        if let Some(display) = &account.display {
            ensure_kind(
                &display.kind,
                "instructionAccountDisplayNode",
                &format!("{account_path}.display"),
            )?;
        }
    }
    for (index, argument) in instruction.arguments.iter().enumerate() {
        let argument_path = format!("{path}.arguments[{index}]");
        ensure_kind(&argument.kind, "instructionArgumentNode", &argument_path)?;
        validate_type(&argument.ty, &format!("{argument_path}.type"))?;
        if let Some(display) = &argument.display {
            validate_field_display(
                display,
                &format!("{argument_path}.display"),
                &argument.ty,
                false,
            )?;
        }
    }
    for (index, discriminator) in instruction.discriminators.iter().enumerate() {
        let discriminator_path = format!("{path}.discriminators[{index}]");
        match discriminator {
            DiscriminatorNode::Field { name, .. } => {
                let Some(argument) = instruction
                    .arguments
                    .iter()
                    .find(|argument| argument.name == *name)
                else {
                    return Err(Srf39IdlError::InvalidSchema {
                        detail: format!(
                            "field discriminator at {discriminator_path} references missing argument '{name}'"
                        ),
                    });
                };
                if !matches!(
                    argument.default_value.as_ref(),
                    Some(ValueNode::Number { .. })
                ) {
                    return Err(Srf39IdlError::InvalidSchema {
                        detail: format!(
                            "field discriminator at {discriminator_path} requires a numeric default value"
                        ),
                    });
                }
            }
            DiscriminatorNode::Size { .. } => {}
        }
    }
    for (index, remaining) in instruction.remaining_accounts.iter().enumerate() {
        let remaining_path = format!("{path}.remainingAccounts[{index}]");
        ensure_kind(
            &remaining.kind,
            "instructionRemainingAccountsNode",
            &remaining_path,
        )?;
        match &remaining.value {
            RemainingAccountsValueNode::Argument { .. } => {}
        }
        if let Some(SignerRule::Named(value)) = &remaining.is_signer {
            if value != "either" {
                return Err(Srf39IdlError::InvalidSchema {
                    detail: format!(
                        "expected true, false, or 'either' at {remaining_path}.isSigner"
                    ),
                });
            }
        }
        if let Some(display) = &remaining.display {
            ensure_kind(
                &display.kind,
                "instructionAccountDisplayNode",
                &format!("{remaining_path}.display"),
            )?;
        }
    }
    for (index, provided) in instruction.provides.iter().enumerate() {
        ensure_kind(
            &provided.kind,
            "providedNode",
            &format!("{path}.provides[{index}]"),
        )?;
        validate_value(&provided.node, &format!("{path}.provides[{index}].node"))?;
    }
    Ok(())
}

fn validate_token_amounts(
    instruction: &InstructionNode,
    metadata: &MetadataDisplay,
    path: &str,
) -> Result<(), Srf39IdlError> {
    let mut amounts = std::collections::HashSet::new();
    for binding in &metadata.token_amounts {
        let valid_amount = instruction.arguments.iter().any(|argument| {
            argument.name == binding.amount
                && matches!(
                    display_type(&argument.ty),
                    TypeNode::Number {
                        format,
                        display: Some(NumberDisplayNode::Amount { .. }),
                        ..
                    } if format == "u64" || format == "i64"
                )
        });
        let valid_mint = match &binding.mint {
            TokenMintReference::Argument { name } => instruction.arguments.iter().any(|argument| {
                argument.name == *name && matches!(display_type(&argument.ty), TypeNode::PublicKey)
            }),
            TokenMintReference::Account { name } => instruction
                .accounts
                .iter()
                .any(|account| account.name == *name),
        };
        if !valid_amount || !valid_mint || !amounts.insert(&binding.amount) {
            return Err(Srf39IdlError::InvalidSchema {
                detail: format!(
                    "invalid or duplicate token amount binding for '{}' at {path}.display.x-solana-clearsign: requires an amount u64/i64 and a public-key argument or named account",
                    binding.amount
                ),
            });
        }
    }
    Ok(())
}

fn validate_type(node: &TypeNode, path: &str) -> Result<(), Srf39IdlError> {
    match node {
        TypeNode::Boolean { size } => {
            if !matches!(
                size.as_ref(),
                TypeNode::Number {
                    format,
                    endian,
                    display: None,
                } if format == "u8" && endian.as_deref().unwrap_or("le") == "le"
            ) {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: format!("{path}.size"),
                    kind: "booleanTypeNode(non-u8 size)".to_string(),
                });
            }
            validate_type(size, &format!("{path}.size"))?;
        }
        TypeNode::Number {
            format,
            endian,
            display,
        } => {
            if !matches!(format.as_str(), "u8" | "u32" | "u64" | "i64")
                || endian.as_deref().unwrap_or("le") != "le"
            {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: path.to_string(),
                    kind: format!(
                        "numberTypeNode(format={format}, endian={})",
                        endian.as_deref().unwrap_or("le")
                    ),
                });
            }
            match display {
                Some(NumberDisplayNode::Amount { decimals, unit }) => {
                    if let Some(value) = decimals {
                        if !matches!(value, ValueNode::Number { .. } | ValueNode::Injected { .. }) {
                            return Err(Srf39IdlError::UnsupportedIdlNode {
                                path: format!("{path}.display.decimals"),
                                kind: "non-numeric amount decimals".to_string(),
                            });
                        }
                        validate_value(value, &format!("{path}.display.decimals"))?;
                    }
                    if let Some(value) = unit {
                        if !matches!(value, ValueNode::String { .. } | ValueNode::Injected { .. }) {
                            return Err(Srf39IdlError::UnsupportedIdlNode {
                                path: format!("{path}.display.unit"),
                                kind: "non-string amount unit".to_string(),
                            });
                        }
                        validate_value(value, &format!("{path}.display.unit"))?;
                    }
                }
                Some(NumberDisplayNode::DateTime { ticks_per_second }) => {
                    validate_ticks_per_second(
                        ticks_per_second.as_ref(),
                        path,
                        "dateTimeNumberDisplayNode",
                    )?;
                }
                Some(NumberDisplayNode::Duration { ticks_per_second }) => {
                    validate_ticks_per_second(
                        ticks_per_second.as_ref(),
                        path,
                        "durationNumberDisplayNode",
                    )?;
                }
                None => {}
            }
        }
        TypeNode::Option {
            item,
            prefix,
            fixed,
        } => {
            validate_type(item, &format!("{path}.item"))?;
            validate_type(prefix, &format!("{path}.prefix"))?;
            if !*fixed {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: path.to_string(),
                    kind: "optionTypeNode(fixed=false)".to_string(),
                });
            }
            if !matches!(
                prefix.as_ref(),
                TypeNode::Number {
                    format,
                    endian,
                    display: None,
                } if format == "u32" && endian.as_deref().unwrap_or("le") == "le"
            ) {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: format!("{path}.prefix"),
                    kind: "optionTypeNode(non-u32 prefix)".to_string(),
                });
            }
            if fixed_size(item).is_none() {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: format!("{path}.item"),
                    kind: "optionTypeNode(variable-size item)".to_string(),
                });
            }
        }
        TypeNode::PublicKey => {}
        TypeNode::Struct { fields } => {
            for (index, field) in fields.iter().enumerate() {
                let field_path = format!("{path}.fields[{index}]");
                ensure_kind(&field.kind, "structFieldTypeNode", &field_path)?;
                validate_type(&field.ty, &format!("{field_path}.type"))?;
                if let Some(display) = &field.display {
                    validate_field_display(
                        display,
                        &format!("{field_path}.display"),
                        &field.ty,
                        true,
                    )?;
                }
            }
        }
        TypeNode::DefinedTypeLink { .. } => {
            // `resolve_links` runs first and inlines every link.
            return Err(Srf39IdlError::InvalidSchema {
                detail: format!("unresolved definedTypeLinkNode at {path}"),
            });
        }
        TypeNode::String { .. } => {
            // The reference decodes a bare string as "the rest of the bytes";
            // only the fixed-size form has a layout this engine will vouch for.
            return Err(Srf39IdlError::UnsupportedIdlNode {
                path: path.to_string(),
                kind: "stringTypeNode(variable size)".to_string(),
            });
        }
        TypeNode::FixedSize { ty, .. } => match ty.as_ref() {
            TypeNode::String { encoding } if encoding == "utf8" => {}
            TypeNode::String { encoding } => {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: format!("{path}.type"),
                    kind: format!("stringTypeNode(encoding={encoding})"),
                });
            }
            _ => {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: path.to_string(),
                    kind: "fixedSizeTypeNode(non-string)".to_string(),
                });
            }
        },
        TypeNode::Array { item, count } => {
            let CountNode::Fixed { value } = count;
            if *value == 0 {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: format!("{path}.count"),
                    kind: "arrayTypeNode(zero count)".to_string(),
                });
            }
            validate_type(item, &format!("{path}.item"))?;
        }
        TypeNode::Enum { size, variants } => {
            if !matches!(
                size.as_ref(),
                TypeNode::Number {
                    format,
                    endian,
                    display: None,
                } if format == "u8" && endian.as_deref().unwrap_or("le") == "le"
            ) {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: format!("{path}.size"),
                    kind: "enumTypeNode(non-u8 size)".to_string(),
                });
            }
            if variants.is_empty() {
                return Err(Srf39IdlError::UnsupportedIdlNode {
                    path: path.to_string(),
                    kind: "enumTypeNode(no variants)".to_string(),
                });
            }
            let mut names = std::collections::HashSet::new();
            for (index, variant) in variants.iter().enumerate() {
                let variant_path = format!("{path}.variants[{index}]");
                ensure_kind(&variant.kind, "enumEmptyVariantTypeNode", &variant_path)?;
                if variant
                    .discriminator
                    .is_some_and(|discriminator| discriminator != index as u64)
                {
                    return Err(Srf39IdlError::UnsupportedIdlNode {
                        path: variant_path,
                        kind: "enumTypeNode(discriminator mismatch)".to_string(),
                    });
                }
                if !names.insert(variant.name.as_str()) {
                    return Err(Srf39IdlError::UnsupportedIdlNode {
                        path: variant_path,
                        kind: "enumTypeNode(duplicate variant)".to_string(),
                    });
                }
                if let Some(display) = &variant.display {
                    ensure_kind(
                        &display.kind,
                        "enumVariantDisplayNode",
                        &format!("{variant_path}.display"),
                    )?;
                }
            }
        }
    }
    if !matches!(decoded_value_count(node), Some(count) if count <= MAX_DECODED_VALUES) {
        return Err(Srf39IdlError::InvalidSchema {
            detail: format!(
                "decoded value count exceeds the SDK limit of {MAX_DECODED_VALUES} at {path}"
            ),
        });
    }
    Ok(())
}

/// Upper bound on the value tree allocated when decoding one type.
fn decoded_value_count(node: &TypeNode) -> Option<usize> {
    let children = match node {
        TypeNode::Array { item, count } => {
            let CountNode::Fixed { value } = count;
            decoded_value_count(item)?.checked_mul(*value)?
        }
        TypeNode::Struct { fields } => fields.iter().try_fold(0usize, |total, field| {
            total.checked_add(decoded_value_count(&field.ty)?)
        })?,
        TypeNode::Option { item, .. } => decoded_value_count(item)?,
        _ => 0,
    };
    children.checked_add(1)
}

pub(crate) fn fixed_size(node: &TypeNode) -> Option<usize> {
    match node {
        TypeNode::Boolean { size } => fixed_size(size),
        TypeNode::Number { format, .. } => match format.as_str() {
            "u8" => Some(1),
            "u32" => Some(4),
            "u64" | "i64" => Some(8),
            _ => None,
        },
        TypeNode::Option {
            item,
            prefix,
            fixed: true,
        } => fixed_size(prefix)?.checked_add(fixed_size(item)?),
        TypeNode::Option { fixed: false, .. } => None,
        TypeNode::PublicKey => Some(32),
        TypeNode::Struct { fields } => fields.iter().try_fold(0usize, |total, field| {
            total.checked_add(fixed_size(&field.ty)?)
        }),
        TypeNode::DefinedTypeLink { .. } => None,
        TypeNode::Enum { size, .. } => fixed_size(size),
        TypeNode::FixedSize { size, .. } => Some(*size),
        TypeNode::String { .. } => None,
        TypeNode::Array { item, count } => {
            let CountNode::Fixed { value } = count;
            fixed_size(item)?.checked_mul(*value)
        }
    }
}

fn validate_value(node: &ValueNode, path: &str) -> Result<(), Srf39IdlError> {
    match node {
        ValueNode::Argument { .. } => {
            return Err(Srf39IdlError::UnsupportedIdlNode {
                path: path.to_string(),
                kind: "argumentValueNode(display value)".to_string(),
            });
        }
        ValueNode::Identity => {
            return Err(Srf39IdlError::UnsupportedIdlNode {
                path: path.to_string(),
                kind: "identityValueNode(display value)".to_string(),
            });
        }
        ValueNode::Injected {
            fallback: Some(fallback),
            ..
        } => validate_value(fallback, &format!("{path}.fallback"))?,
        ValueNode::AccountField {
            path: field_path, ..
        } if field_path.contains('.') => {
            return Err(Srf39IdlError::UnsupportedIdlNode {
                path: path.to_string(),
                kind: "accountFieldValueNode(nested path)".to_string(),
            });
        }
        _ => {}
    }
    Ok(())
}

/// `flatten` is honoured one level deep on a top-level argument whose display
/// type is a struct (the reference ignores it elsewhere; this engine refuses
/// to load such metadata rather than silently drop it).
fn validate_field_display(
    display: &StructFieldDisplayNode,
    path: &str,
    ty: &TypeNode,
    nested: bool,
) -> Result<(), Srf39IdlError> {
    ensure_kind(&display.kind, "structFieldDisplayNode", path)?;
    if nested && (display.flatten || display.flatten_prefix.is_some()) {
        return Err(Srf39IdlError::UnsupportedIdlNode {
            path: path.to_string(),
            kind: "structFieldDisplayNode(nested flatten)".to_string(),
        });
    }
    if display.flatten_prefix.is_some() && !display.flatten {
        return Err(Srf39IdlError::UnsupportedIdlNode {
            path: path.to_string(),
            kind: "structFieldDisplayNode(flattenPrefix without flatten)".to_string(),
        });
    }
    if display.flatten && !matches!(display_type(ty), TypeNode::Struct { .. }) {
        return Err(Srf39IdlError::UnsupportedIdlNode {
            path: path.to_string(),
            kind: "structFieldDisplayNode(flatten on non-struct)".to_string(),
        });
    }
    Ok(())
}

/// The reference `resolveDisplayType` for the shapes this engine accepts:
/// fixed-size wrappers and option types unwrap to their inner type so display
/// metadata attaches to the value inside.
pub(crate) fn display_type(ty: &TypeNode) -> &TypeNode {
    let mut current = ty;
    loop {
        match current {
            TypeNode::Option { item, .. } => current = item,
            TypeNode::FixedSize { ty, .. } => current = ty,
            _ => return current,
        }
    }
}

fn ensure_kind(actual: &str, expected: &str, path: &str) -> Result<(), Srf39IdlError> {
    if actual == expected {
        Ok(())
    } else {
        Err(Srf39IdlError::InvalidSchema {
            detail: format!("expected {expected} at {path}, got {actual}"),
        })
    }
}
