use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;

use crate::provider::AccountData;
use crate::provider::Srf39AccountProvider;
use crate::InstructionContext;

use super::decode;
use super::decode::DecodedArguments;
use super::decode::DecodedValue;
use super::hints::AccountHint;
use super::hints::AmountHint;
use super::hints::DecimalsSource;
use super::hints::InstructionDisplayHints;
use super::hints::LinkedAccountRead;
use super::hints::LinkedAccountStatus;
use super::hints::PublicKeyArgumentHint;
use super::hints::TimeDisplay;
use super::hints::TimeHint;
use super::hints::TokenHint;
use super::hints::UnitSource;
use super::model::display_type;
use super::model::ticks_per_second;
use super::model::DisplaySkip;
use super::model::InstructionArgumentNode;
use super::model::InstructionNode;
use super::model::NumberDisplayNode;
use super::model::ProgramNode;
use super::model::RemainingAccountsValueNode;
use super::model::StructFieldTypeNode;
use super::model::TokenMintReference;
use super::model::TypeNode;
use super::model::ValueNode;
use super::DisplayField;
use super::InstructionDisplay;
use super::LoadedSrf39Idl;
use super::Srf39DisplayError;

#[derive(Debug, Clone)]
enum ResolvedScalar {
    Number(i128),
    String(String),
}

#[derive(Debug, Clone)]
struct ResolvedValue {
    value: ResolvedScalar,
    consumed_account: Option<String>,
}

#[derive(Debug, Clone)]
struct FormattedValue {
    text: String,
    degraded: bool,
}

#[derive(Debug, Clone)]
struct RemainingAccount {
    address: String,
    is_signer: bool,
}

/// Which IDL member produced a fallback field. Used only to derive hints; the
/// canonical field list never depends on it.
#[derive(Debug, Clone)]
enum FieldOrigin {
    /// `member` names the direct field of a flattened struct argument;
    /// `element` is set when a public-key array expanded into one field per
    /// element.
    Argument {
        name: String,
        member: Option<String>,
        element: Option<usize>,
    },
    Account {
        name: String,
    },
    Remaining {
        group_name: String,
    },
}

/// Formatted values are keyed by `(argument, flattened field)`; the top-level
/// argument itself has no member name.
type MemberKey = (String, Option<String>);

/// One entry of the argument list as the reference renders it: a top-level
/// argument, or one direct field of a flattened struct argument.
struct Member<'a> {
    argument: &'a InstructionArgumentNode,
    /// The struct field when the argument was flattened.
    field: Option<&'a StructFieldTypeNode>,
    ty: &'a TypeNode,
    value: Option<&'a DecodedValue>,
    label: String,
}

impl Member<'_> {
    fn key(&self) -> MemberKey {
        (
            self.argument.name.clone(),
            self.field.map(|field| field.name.clone()),
        )
    }

    fn origin(&self, element: Option<usize>) -> FieldOrigin {
        FieldOrigin::Argument {
            name: self.argument.name.clone(),
            member: self.field.map(|field| field.name.clone()),
            element,
        }
    }

    /// The reference hides a flattened field by its own `skip` and the whole
    /// argument by the argument's `skip`; both are keyed by plain name.
    fn is_hidden(&self, consumed: &HashSet<String>) -> bool {
        let argument = &self.argument;
        if is_skipped(
            argument.display.as_ref().and_then(|display| display.skip),
            &argument.name,
            consumed,
        ) {
            return true;
        }
        self.field.is_some_and(|field| {
            is_skipped(
                field.display.as_ref().and_then(|display| display.skip),
                &field.name,
                consumed,
            )
        })
    }
}

/// The reference `argumentFields` flatten gate: the argument asks to flatten,
/// its display type is a struct, and its value (after unwrapping options) is a
/// present struct.
fn flattened_struct<'a>(
    argument: &'a InstructionArgumentNode,
    value: Option<&'a DecodedValue>,
) -> Option<(&'a [StructFieldTypeNode], &'a DecodedValue)> {
    if !argument
        .display
        .as_ref()
        .is_some_and(|display| display.flatten)
    {
        return None;
    }
    let TypeNode::Struct { fields } = display_type(&argument.ty) else {
        return None;
    };
    let inner = unwrap_option_value(value?)?;
    matches!(inner, DecodedValue::Struct(_)).then_some((fields.as_slice(), inner))
}

/// The canonical display plus the structural hints derived alongside it.
pub(crate) struct RenderOutput {
    pub(crate) display: InstructionDisplay,
    pub(crate) hints: InstructionDisplayHints,
}

struct RenderContext<'a> {
    idl: &'a LoadedSrf39Idl,
    program: &'a ProgramNode,
    instruction: &'a InstructionNode,
    data: &'a DecodedArguments,
    account_addresses: BTreeMap<String, String>,
    remaining_accounts: Vec<RemainingAccount>,
    provider: Option<&'a dyn Srf39AccountProvider>,
    account_cache: HashMap<String, Option<AccountData>>,
    decoded_account_cache: HashMap<(String, String), Option<DecodedValue>>,
}

pub(crate) async fn render(
    idl: &LoadedSrf39Idl,
    program: &ProgramNode,
    instruction: &InstructionNode,
    data: &DecodedArguments,
    input: &InstructionContext<'_>,
    provider: Option<&dyn Srf39AccountProvider>,
) -> Result<RenderOutput, Srf39DisplayError> {
    let account_addresses = bind_account_addresses(instruction, input);
    let remaining_accounts = input
        .accounts
        .iter()
        .skip(instruction.accounts.len())
        .map(|meta| RemainingAccount {
            address: meta.pubkey.to_string(),
            is_signer: meta.is_signer,
        })
        .collect();
    let mut context = RenderContext {
        idl,
        program,
        instruction,
        data,
        account_addresses,
        remaining_accounts,
        provider,
        account_cache: HashMap::new(),
        decoded_account_cache: HashMap::new(),
    };

    let consumed = context.resolve_consumed_accounts().await?;
    let formatted_arguments = context.format_arguments().await?;
    let fields_with_origins = context.list_fields(&formatted_arguments, &consumed);
    let interpolated_intent = context.interpolate(&formatted_arguments).await?;
    let intent = instruction
        .display
        .as_ref()
        .and_then(|display| display.intent.clone())
        .unwrap_or_else(|| title_case(&instruction.name));

    let has_template = instruction
        .display
        .as_ref()
        .is_some_and(|display| display.interpolated_intent.is_some());
    let interpolated_intent_suppressed = has_template && interpolated_intent.is_none();

    let hints = context
        .build_hints(
            &formatted_arguments,
            &consumed,
            &fields_with_origins,
            interpolated_intent_suppressed,
        )
        .await?;

    let fields = fields_with_origins
        .into_iter()
        .map(|(field, _)| field)
        .collect();

    Ok(RenderOutput {
        display: InstructionDisplay {
            intent,
            interpolated_intent,
            fields,
        },
        hints,
    })
}

/// Positional binding of declared instruction accounts to supplied metas.
fn bind_account_addresses(
    instruction: &InstructionNode,
    input: &InstructionContext<'_>,
) -> BTreeMap<String, String> {
    instruction
        .accounts
        .iter()
        .zip(input.accounts.iter())
        .map(|(node, meta)| (node.name.clone(), meta.pubkey.to_string()))
        .collect()
}

/// Parity with the reference `getRequiredAccountsForDisplay`: the addresses
/// whose account state the display would read, derived statically from the
/// provide/inject graph. Deduplicated in first-occurrence order. No
/// `accountLink` filter is applied, matching the reference (the renderer may
/// therefore fetch fewer accounts than listed here).
pub(crate) fn required_accounts(
    instruction: &InstructionNode,
    input: &InstructionContext<'_>,
) -> Vec<String> {
    let account_addresses = bind_account_addresses(instruction, input);
    let mut addresses = Vec::new();
    for node in amount_injections(instruction) {
        let Some(ValueNode::AccountField { account, .. }) =
            select_injection_target(instruction, &node)
        else {
            continue;
        };
        let Some(address) = account_addresses.get(&account) else {
            continue;
        };
        if !addresses.contains(address) {
            addresses.push(address.clone());
        }
    }
    addresses
}

/// The reference `collectInjectedNodes`: the injected `decimals`/`unit`
/// inputs of every rendered amount, in IDL order (`decimals` before `unit`).
/// A flattened struct argument surfaces its direct fields, so the walk
/// descends exactly one level into it; nested structs stay opaque.
fn amount_injections(instruction: &InstructionNode) -> Vec<ValueNode> {
    let mut injections = Vec::new();
    for argument in &instruction.arguments {
        let flatten = argument
            .display
            .as_ref()
            .is_some_and(|display| display.flatten);
        collect_member_injections(&argument.ty, flatten, &mut injections);
    }
    injections
}

fn collect_member_injections(ty: &TypeNode, flatten: bool, injections: &mut Vec<ValueNode>) {
    match display_type(ty) {
        TypeNode::Number {
            display: Some(NumberDisplayNode::Amount { decimals, unit }),
            ..
        } => {
            if let Some(value @ ValueNode::Injected { .. }) = decimals {
                injections.push(value.clone());
            }
            if let Some(value @ ValueNode::Injected { .. }) = unit {
                injections.push(value.clone());
            }
        }
        TypeNode::Struct { fields } if flatten => {
            for field in fields {
                collect_member_injections(&field.ty, false, injections);
            }
        }
        _ => {}
    }
}

/// Follows the provide/inject protocol statically: a matching provider wins,
/// otherwise the injection's own fallback; chains collapse to their terminal
/// node and cycles resolve to `None`.
fn select_injection_target(instruction: &InstructionNode, node: &ValueNode) -> Option<ValueNode> {
    let mut current = node.clone();
    let mut seen = HashSet::new();
    loop {
        match current {
            ValueNode::Injected { key, fallback } => {
                if !seen.insert(key.clone()) {
                    return None;
                }
                current = instruction
                    .provides
                    .iter()
                    .rev()
                    .find(|provided| provided.name == key)
                    .map(|provided| provided.node.clone())
                    .or_else(|| fallback.map(|fallback| *fallback))?;
            }
            target => return Some(target),
        }
    }
}

impl<'a> RenderContext<'a> {
    /// The argument list as members, in IDL order: each argument as one
    /// member, or, when it flattens, one member per direct struct field with
    /// the prefixed label. Borrows only the instruction and decoded data.
    fn members(&self) -> Vec<Member<'a>> {
        let mut members = Vec::new();
        for argument in &self.instruction.arguments {
            let display = argument.display.as_ref();
            let value = self.data.get(&argument.name);
            if let Some((fields, inner)) = flattened_struct(argument, value) {
                let prefix = display
                    .and_then(|display| display.flatten_prefix.as_deref())
                    .unwrap_or("");
                for field in fields {
                    let label = field
                        .display
                        .as_ref()
                        .and_then(|display| display.label.clone())
                        .unwrap_or_else(|| title_case(&field.name));
                    members.push(Member {
                        argument,
                        field: Some(field),
                        ty: &field.ty,
                        value: inner.field(&field.name),
                        label: format!("{prefix}{label}"),
                    });
                }
            } else {
                members.push(Member {
                    argument,
                    field: None,
                    ty: &argument.ty,
                    value,
                    label: display
                        .and_then(|display| display.label.clone())
                        .unwrap_or_else(|| title_case(&argument.name)),
                });
            }
        }
        members
    }

    async fn resolve_consumed_accounts(&mut self) -> Result<HashSet<String>, Srf39DisplayError> {
        let injections = amount_injections(self.instruction);

        let mut consumed = HashSet::new();
        for injection in injections {
            if let Some(value) = self.resolve_value(&injection).await? {
                if let Some(account) = value.consumed_account {
                    consumed.insert(account);
                }
            }
        }
        Ok(consumed)
    }

    /// Formats every top-level argument (interpolation always sees the whole
    /// argument) and, for a flattened struct, each direct field.
    async fn format_arguments(
        &mut self,
    ) -> Result<BTreeMap<MemberKey, FormattedValue>, Srf39DisplayError> {
        let mut formatted = BTreeMap::new();
        for argument in &self.instruction.arguments {
            let Some(value) = self.data.get(&argument.name) else {
                continue;
            };
            let text = self.format_argument(&argument.ty, value).await?;
            formatted.insert((argument.name.clone(), None), text);
        }
        for member in self.members() {
            let (Some(_), Some(value)) = (member.field, member.value) else {
                continue;
            };
            let text = self.format_argument(member.ty, value).await?;
            formatted.insert(member.key(), text);
        }
        Ok(formatted)
    }

    /// The reference `formatArgumentValue`: option wrappers are unwrapped on
    /// both the type and the value so presentation applies to the inner value
    /// (`None` renders as `none`); numbers and enums follow their display
    /// metadata; everything else falls back to the raw form.
    async fn format_argument(
        &mut self,
        ty: &TypeNode,
        value: &DecodedValue,
    ) -> Result<FormattedValue, Srf39DisplayError> {
        let Some(inner) = unwrap_option_value(value) else {
            return Ok(FormattedValue {
                text: "none".to_string(),
                degraded: false,
            });
        };
        match (display_type(ty), inner) {
            (
                TypeNode::Number {
                    display: Some(display),
                    ..
                },
                DecodedValue::Number(raw),
            ) => self.format_number(display, *raw).await,
            (TypeNode::Enum { variants, .. }, DecodedValue::Enum { index }) => Ok(FormattedValue {
                text: enum_label(variants, *index),
                degraded: false,
            }),
            // Arrays render as one comma-joined line whose elements each carry
            // their own presentation (and their own ` (raw)` marker).
            (TypeNode::Array { item, .. }, DecodedValue::Array(elements)) => {
                let mut texts = Vec::with_capacity(elements.len());
                let mut degraded = false;
                for element in elements {
                    let formatted = Box::pin(self.format_argument(item, element)).await?;
                    degraded |= formatted.degraded;
                    texts.push(formatted.text);
                }
                Ok(FormattedValue {
                    text: texts.join(", "),
                    degraded,
                })
            }
            _ => Ok(FormattedValue {
                text: raw_value(inner),
                degraded: false,
            }),
        }
    }

    async fn format_number(
        &mut self,
        display: &NumberDisplayNode,
        raw: decode::DecodedNumber,
    ) -> Result<FormattedValue, Srf39DisplayError> {
        let (decimals, unit) = match display {
            NumberDisplayNode::Amount { decimals, unit } => (decimals, unit),
            // Only an unresolved amount scale degrades; a date or duration the
            // reference cannot format falls back to the bare integer.
            NumberDisplayNode::DateTime { ticks_per_second } => {
                let text = format_date_time(raw.value, ticks_per_second_of(ticks_per_second))
                    .unwrap_or_else(|| raw.value.to_string());
                return Ok(FormattedValue {
                    text,
                    degraded: false,
                });
            }
            NumberDisplayNode::Duration { ticks_per_second } => {
                let text = format_duration(raw.value, ticks_per_second_of(ticks_per_second))
                    .unwrap_or_else(|| raw.value.to_string());
                return Ok(FormattedValue {
                    text,
                    degraded: false,
                });
            }
        };

        let decimals = match decimals {
            Some(node) => match self.resolve_value(node).await? {
                Some(ResolvedValue {
                    value: ResolvedScalar::Number(value),
                    ..
                }) => Some(value),
                _ => None,
            },
            None => Some(0),
        };
        let Some(decimals) = decimals
            .and_then(|value| u8::try_from(value).ok())
            .map(usize::from)
        else {
            return Ok(FormattedValue {
                text: format!("{} (raw)", raw.value),
                degraded: true,
            });
        };

        let scaled = scale_by_decimals(raw.value, decimals);
        let suffix = match unit {
            Some(node) => match self.resolve_value(node).await? {
                Some(ResolvedValue {
                    value: ResolvedScalar::String(value),
                    ..
                }) if !value.is_empty() => Some(value),
                _ => None,
            },
            None => None,
        };
        let text = suffix.map_or(scaled.clone(), |unit| format!("{scaled} {unit}"));
        Ok(FormattedValue {
            text,
            degraded: false,
        })
    }

    fn list_fields(
        &self,
        formatted: &BTreeMap<MemberKey, FormattedValue>,
        consumed: &HashSet<String>,
    ) -> Vec<(DisplayField, FieldOrigin)> {
        let mut fields = Vec::new();
        for member in self.members() {
            if member.is_hidden(consumed) {
                continue;
            }
            let Some(value) = formatted.get(&member.key()) else {
                continue;
            };
            // The reference `memberFields`: an array of addresses expands into
            // one field per element (numbered when there are several), since
            // each address is verified on its own; other members are one field.
            if let Some(elements) = public_key_elements(member.ty, member.value) {
                for (index, element) in elements.iter().enumerate() {
                    fields.push((
                        DisplayField {
                            label: if elements.len() > 1 {
                                format!("{} #{}", member.label, index + 1)
                            } else {
                                member.label.clone()
                            },
                            value: unwrap_option_value(element)
                                .map_or_else(|| "none".to_string(), raw_value),
                        },
                        member.origin(Some(index)),
                    ));
                }
                continue;
            }
            fields.push((
                DisplayField {
                    label: member.label.clone(),
                    value: value.text.clone(),
                },
                member.origin(None),
            ));
        }

        for account in &self.instruction.accounts {
            if is_skipped(
                account.display.as_ref().and_then(|display| display.skip),
                &account.name,
                consumed,
            ) {
                continue;
            }
            let Some(address) = self.account_addresses.get(&account.name) else {
                continue;
            };
            fields.push((
                DisplayField {
                    label: self.account_label(&account.name),
                    value: address.clone(),
                },
                FieldOrigin::Account {
                    name: account.name.clone(),
                },
            ));
        }
        self.list_remaining_account_fields(&mut fields, consumed);
        fields
    }

    fn account_label(&self, account_name: &str) -> String {
        self.instruction
            .accounts
            .iter()
            .find(|account| account.name == account_name)
            .and_then(|account| account.display.as_ref())
            .and_then(|display| display.label.clone())
            .unwrap_or_else(|| title_case(account_name))
    }

    fn list_remaining_account_fields(
        &self,
        fields: &mut Vec<(DisplayField, FieldOrigin)>,
        consumed: &HashSet<String>,
    ) {
        let mut cursor = 0;
        for (group_index, group) in self.instruction.remaining_accounts.iter().enumerate() {
            let start = cursor;
            let is_last = group_index + 1 == self.instruction.remaining_accounts.len();
            while cursor < self.remaining_accounts.len()
                && (is_last
                    || group
                        .is_signer
                        .as_ref()
                        .map_or(!self.remaining_accounts[cursor].is_signer, |rule| {
                            rule.matches(self.remaining_accounts[cursor].is_signer)
                        }))
            {
                cursor += 1;
            }

            let RemainingAccountsValueNode::Argument { name } = &group.value;
            if is_skipped(
                group.display.as_ref().and_then(|display| display.skip),
                name,
                consumed,
            ) {
                continue;
            }
            let label = group
                .display
                .as_ref()
                .and_then(|display| display.label.clone())
                .unwrap_or_else(|| title_case(name));
            let accounts = &self.remaining_accounts[start..cursor];
            for (index, account) in accounts.iter().enumerate() {
                fields.push((
                    DisplayField {
                        label: if accounts.len() > 1 {
                            format!("{label} #{}", index + 1)
                        } else {
                            label.clone()
                        },
                        value: account.address.clone(),
                    },
                    FieldOrigin::Remaining {
                        group_name: name.clone(),
                    },
                ));
            }
        }
    }

    async fn interpolate(
        &mut self,
        formatted_arguments: &BTreeMap<MemberKey, FormattedValue>,
    ) -> Result<Option<String>, Srf39DisplayError> {
        let Some(template) = self
            .instruction
            .display
            .as_ref()
            .and_then(|display| display.interpolated_intent.as_deref())
        else {
            return Ok(None);
        };

        let mut output = String::with_capacity(template.len() + 32);
        let mut rest = template;
        while let Some(start) = rest.find("${") {
            output.push_str(&rest[..start]);
            let token_start = start + 2;
            let Some(relative_end) = rest[token_start..].find('}') else {
                output.push_str(&rest[start..]);
                return Ok(Some(output));
            };
            let token_end = token_start + relative_end;
            let token = rest[token_start..token_end].trim();
            let replacement =
                resolve_placeholder(token, formatted_arguments, &self.account_addresses);
            match replacement {
                Placeholder::Resolved(value) => output.push_str(value),
                Placeholder::Missing => return Ok(None),
                Placeholder::Unrecognized => output.push_str(&rest[start..=token_end]),
            }
            rest = &rest[token_end + 1..];
        }
        output.push_str(rest);
        Ok(Some(output))
    }

    /// Derives the hints from the resolution state accumulated by the
    /// canonical pass. Every `resolve_value` call here hits the account and
    /// decode caches populated earlier, so no new provider request is made.
    async fn build_hints(
        &mut self,
        formatted_arguments: &BTreeMap<MemberKey, FormattedValue>,
        consumed: &HashSet<String>,
        fields: &[(DisplayField, FieldOrigin)],
        interpolated_intent_suppressed: bool,
    ) -> Result<InstructionDisplayHints, Srf39DisplayError> {
        let members = self.members();
        let field_position = |origin_of: &Member<'_>, element: Option<usize>| {
            fields.iter().position(|(_, origin)| {
                matches!(
                    origin,
                    FieldOrigin::Argument { name, member, element: at }
                        if *name == origin_of.argument.name
                            && member.as_deref() == origin_of.field.map(|field| field.name.as_str())
                            && *at == element
                )
            })
        };

        let mut amounts = Vec::new();
        for member in &members {
            let TypeNode::Number {
                display: Some(NumberDisplayNode::Amount { decimals, unit }),
                ..
            } = display_type(member.ty)
            else {
                continue;
            };
            let Some(DecodedValue::Number(raw)) = member.value.and_then(unwrap_option_value) else {
                continue;
            };
            let degraded = formatted_arguments
                .get(&member.key())
                .is_some_and(|value| value.degraded);
            let field_index = field_position(member, None);
            let decimals = self.decimals_source(decimals.as_ref()).await?;
            let unit = self.unit_source(unit.as_ref()).await?;
            amounts.push(AmountHint {
                field_index,
                argument: member.argument.name.clone(),
                member: member.field.map(|field| field.name.clone()),
                raw_value: raw.value.to_string(),
                degraded,
                decimals,
                unit,
                token: self
                    .instruction
                    .display
                    .as_ref()
                    .and_then(|display| display.metadata.as_ref())
                    .and_then(|metadata| {
                        metadata.token_amounts.iter().find(|binding| {
                            member.field.is_none() && binding.amount == member.argument.name
                        })
                    })
                    .map(|binding| TokenHint {
                        mint: match &binding.mint {
                            TokenMintReference::Argument { name } => {
                                self.data.get(name).and_then(unwrap_option_value).and_then(
                                    |value| match value {
                                        DecodedValue::PublicKey(address) => Some(address.clone()),
                                        _ => None,
                                    },
                                )
                            }
                            TokenMintReference::Account { name } => {
                                self.account_addresses.get(name).cloned()
                            }
                        },
                    }),
            });
        }

        let mut times = Vec::new();
        for member in &members {
            let TypeNode::Number {
                display: Some(display),
                ..
            } = display_type(member.ty)
            else {
                continue;
            };
            let Some(DecodedValue::Number(raw)) = member.value.and_then(unwrap_option_value) else {
                continue;
            };
            let (display, text) = match display {
                NumberDisplayNode::Amount { .. } => continue,
                NumberDisplayNode::DateTime { ticks_per_second } => {
                    let ticks = ticks_per_second_of(ticks_per_second);
                    (
                        TimeDisplay::DateTime {
                            ticks_per_second: ticks,
                        },
                        format_date_time(raw.value, ticks),
                    )
                }
                NumberDisplayNode::Duration { ticks_per_second } => {
                    let ticks = ticks_per_second_of(ticks_per_second);
                    (
                        TimeDisplay::Duration {
                            ticks_per_second: ticks,
                        },
                        format_duration(raw.value, ticks),
                    )
                }
            };
            let ticks = match display {
                TimeDisplay::DateTime { ticks_per_second }
                | TimeDisplay::Duration { ticks_per_second } => i128::from(ticks_per_second),
            };
            let formatted = text.is_some();
            let seconds = formatted
                .then(|| i64::try_from(raw.value.div_euclid(ticks)).ok())
                .flatten();
            times.push(TimeHint {
                field_index: field_position(member, None),
                argument: member.argument.name.clone(),
                member: member.field.map(|field| field.name.clone()),
                raw_value: raw.value.to_string(),
                display,
                seconds,
                formatted,
            });
        }

        let mut public_key_arguments = Vec::new();
        for member in &members {
            let Some(value) = member.value else {
                continue;
            };
            if let Some(elements) = public_key_elements(member.ty, Some(value)) {
                for (element, item) in elements.iter().enumerate() {
                    let Some(DecodedValue::PublicKey(address)) = unwrap_option_value(item) else {
                        continue;
                    };
                    public_key_arguments.push(PublicKeyArgumentHint {
                        field_index: field_position(member, Some(element)),
                        argument: member.argument.name.clone(),
                        member: member.field.map(|field| field.name.clone()),
                        element: Some(element),
                        address: address.clone(),
                    });
                }
                continue;
            }
            if !matches!(display_type(member.ty), TypeNode::PublicKey) {
                continue;
            }
            let Some(DecodedValue::PublicKey(address)) = unwrap_option_value(value) else {
                continue;
            };
            public_key_arguments.push(PublicKeyArgumentHint {
                field_index: field_position(member, None),
                argument: member.argument.name.clone(),
                member: member.field.map(|field| field.name.clone()),
                element: None,
                address: address.clone(),
            });
        }

        let mut accounts = Vec::new();
        for account in &self.instruction.accounts {
            let field_index = fields.iter().position(|(_, origin)| {
                matches!(origin, FieldOrigin::Account { name } if *name == account.name)
            });
            accounts.push(AccountHint {
                field_index,
                name: account.name.clone(),
                address: self.account_addresses.get(&account.name).cloned(),
                label: self.account_label(&account.name),
                linked_account: account.account_link.as_ref().map(|link| link.name.clone()),
                consumed: consumed.contains(&account.name),
            });
        }
        for (index, (field, origin)) in fields.iter().enumerate() {
            let FieldOrigin::Remaining { group_name } = origin else {
                continue;
            };
            accounts.push(AccountHint {
                field_index: Some(index),
                name: group_name.clone(),
                address: Some(field.value.clone()),
                label: field.label.clone(),
                linked_account: None,
                consumed: false,
            });
        }

        let linked_account_reads = self
            .account_cache
            .iter()
            .collect::<BTreeMap<_, _>>()
            .into_iter()
            .map(|(address, account)| LinkedAccountRead {
                address: address.clone(),
                status: match account {
                    Some(account) => LinkedAccountStatus::Fetched {
                        owner: account.owner.clone(),
                        length: account.data.len(),
                    },
                    None => LinkedAccountStatus::Missing,
                },
            })
            .collect();

        Ok(InstructionDisplayHints {
            instruction_name: self.instruction.name.clone(),
            interpolated_intent_suppressed,
            amounts,
            times,
            public_key_arguments,
            accounts,
            linked_account_reads,
        })
    }

    async fn decimals_source(
        &mut self,
        node: Option<&ValueNode>,
    ) -> Result<DecimalsSource, Srf39DisplayError> {
        let Some(node) = node else {
            return Ok(DecimalsSource::ImplicitZero);
        };
        match select_injection_target(self.instruction, node) {
            None => Ok(DecimalsSource::Unsatisfied),
            Some(ValueNode::Number { number }) => Ok(DecimalsSource::Literal { value: number }),
            Some(ValueNode::AccountField { account, path }) => {
                let resolved = match self.resolve_value(node).await? {
                    Some(ResolvedValue {
                        value: ResolvedScalar::Number(value),
                        ..
                    }) => u8::try_from(value).ok(),
                    _ => None,
                };
                Ok(DecimalsSource::AccountField {
                    address: self.account_addresses.get(&account).cloned(),
                    linked_account: self.linked_account_name(&account),
                    account,
                    path,
                    resolved,
                })
            }
            Some(_) => Ok(DecimalsSource::Unsatisfied),
        }
    }

    async fn unit_source(
        &mut self,
        node: Option<&ValueNode>,
    ) -> Result<UnitSource, Srf39DisplayError> {
        let Some(node) = node else {
            return Ok(UnitSource::None);
        };
        match select_injection_target(self.instruction, node) {
            None => Ok(UnitSource::Unsatisfied),
            Some(ValueNode::String { string }) => Ok(UnitSource::Literal { value: string }),
            Some(ValueNode::AccountField { account, path }) => {
                let resolved = match self.resolve_value(node).await? {
                    Some(ResolvedValue {
                        value: ResolvedScalar::String(value),
                        ..
                    }) if !value.is_empty() => Some(value),
                    _ => None,
                };
                Ok(UnitSource::AccountField {
                    address: self.account_addresses.get(&account).cloned(),
                    linked_account: self.linked_account_name(&account),
                    account,
                    path,
                    resolved,
                })
            }
            Some(_) => Ok(UnitSource::Unsatisfied),
        }
    }

    fn linked_account_name(&self, account_name: &str) -> Option<String> {
        self.instruction
            .accounts
            .iter()
            .find(|account| account.name == account_name)
            .and_then(|account| account.account_link.as_ref())
            .map(|link| link.name.clone())
    }

    async fn resolve_value(
        &mut self,
        node: &ValueNode,
    ) -> Result<Option<ResolvedValue>, Srf39DisplayError> {
        let Some(target) = select_injection_target(self.instruction, node) else {
            return Ok(None);
        };
        match target {
            ValueNode::Argument { .. } | ValueNode::Identity => Ok(None),
            ValueNode::Number { number } => Ok(Some(ResolvedValue {
                value: ResolvedScalar::Number(i128::from(number)),
                consumed_account: None,
            })),
            ValueNode::String { string } => Ok(Some(ResolvedValue {
                value: ResolvedScalar::String(string),
                consumed_account: None,
            })),
            ValueNode::AccountField { account, path } => {
                self.resolve_account_field(&account, &path).await
            }
            ValueNode::Injected { .. } => Ok(None),
        }
    }

    async fn resolve_account_field(
        &mut self,
        account_name: &str,
        field_path: &str,
    ) -> Result<Option<ResolvedValue>, Srf39DisplayError> {
        let Some(instruction_account) = self
            .instruction
            .accounts
            .iter()
            .find(|account| account.name == account_name)
        else {
            return Ok(None);
        };
        let Some(link) = &instruction_account.account_link else {
            return Ok(None);
        };
        // The link names an account of an additional program or, by default,
        // of the surrounding program. An unresolvable link never yields a
        // value (the reference's `resolveAccountData` returns null).
        let target_program = match &link.program {
            Some(program) => self.idl.root.program_by_name(&program.name),
            None => Some(self.program),
        };
        let Some(account_node) = target_program
            .and_then(|program| {
                program
                    .accounts
                    .iter()
                    .find(|account| account.name == link.name)
            })
            .cloned()
        else {
            return Ok(None);
        };
        let program_name = target_program
            .map(|program| program.name.as_str())
            .unwrap_or_default();
        let Some(address) = self.account_addresses.get(account_name).cloned() else {
            return Ok(None);
        };

        if !self.account_cache.contains_key(&address) {
            let account = match self.provider {
                Some(provider) => provider.resolve_account(&address).await,
                None => None,
            };
            self.account_cache.insert(address.clone(), account);
        }
        let Some(account_data) = self.account_cache.get(&address).and_then(Option::as_ref) else {
            return Ok(None);
        };
        let decode_key = (address, format!("{program_name}::{}", account_node.name));
        if !self.decoded_account_cache.contains_key(&decode_key) {
            let decoded = decode::decode_account(&account_node.data, &account_data.data);
            self.decoded_account_cache
                .insert(decode_key.clone(), decoded);
        }
        let decoded = self
            .decoded_account_cache
            .get(&decode_key)
            .and_then(Option::as_ref)
            .ok_or_else(|| Srf39DisplayError::AccountDecode {
                account: account_name.to_string(),
                detail: "bytes do not match the linked account type".to_string(),
            })?;
        if !matches!(decoded, DecodedValue::Struct(_)) {
            return Err(Srf39DisplayError::AccountDecode {
                account: account_name.to_string(),
                detail: "linked account is not a struct".to_string(),
            });
        }
        let Some(value) = decoded.field(field_path) else {
            return Ok(None);
        };
        let scalar = match value {
            DecodedValue::Number(number) => ResolvedScalar::Number(number.value),
            // The reference decodes a scalar enum field to its index and
            // treats that number like any other.
            DecodedValue::Enum { index } => ResolvedScalar::Number(*index as i128),
            DecodedValue::PublicKey(address) | DecodedValue::String(address) => {
                ResolvedScalar::String(address.clone())
            }
            DecodedValue::Boolean(_)
            | DecodedValue::Option(_)
            | DecodedValue::Array(_)
            | DecodedValue::Struct(_) => return Ok(None),
        };
        Ok(Some(ResolvedValue {
            value: scalar,
            consumed_account: Some(account_name.to_string()),
        }))
    }
}

enum Placeholder<'a> {
    Resolved(&'a str),
    Missing,
    Unrecognized,
}

fn resolve_placeholder<'a>(
    token: &str,
    arguments: &'a BTreeMap<MemberKey, FormattedValue>,
    accounts: &'a BTreeMap<String, String>,
) -> Placeholder<'a> {
    let Some((root, name)) = token.split_once('.') else {
        return Placeholder::Unrecognized;
    };
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Placeholder::Unrecognized;
    }
    match root {
        // Only top-level arguments are addressable; flattened fields are not.
        "data" => match arguments.get(&(name.to_string(), None)) {
            Some(value) if !value.degraded => Placeholder::Resolved(&value.text),
            _ => Placeholder::Missing,
        },
        "accounts" => accounts
            .get(name)
            .map_or(Placeholder::Missing, |value| Placeholder::Resolved(value)),
        _ => Placeholder::Unrecognized,
    }
}

fn is_skipped(skip: Option<DisplaySkip>, name: &str, consumed: &HashSet<String>) -> bool {
    match skip {
        Some(DisplaySkip::Always) => true,
        Some(DisplaySkip::WhenInjected) => consumed.contains(name),
        Some(DisplaySkip::Never) | None => false,
    }
}

fn ticks_per_second_of(number: &Option<serde_json::Number>) -> u32 {
    ticks_per_second(number.as_ref())
}

/// The elements of an argument that the fallback list expands into one field
/// each: its display type is an array whose item displays as a public key,
/// and the value (after unwrapping options) is present.
fn public_key_elements<'v>(
    ty: &TypeNode,
    value: Option<&'v DecodedValue>,
) -> Option<&'v [DecodedValue]> {
    let TypeNode::Array { item, .. } = display_type(ty) else {
        return None;
    };
    if !matches!(display_type(item), TypeNode::PublicKey) {
        return None;
    }
    match unwrap_option_value(value?)? {
        DecodedValue::Array(elements) => Some(elements),
        _ => None,
    }
}

/// The reference `unwrapOptionValue`: nested `Some`s collapse, `None`
/// anywhere yields `None`, and non-option values pass through.
fn unwrap_option_value(value: &DecodedValue) -> Option<&DecodedValue> {
    match value {
        DecodedValue::Option(Some(inner)) => unwrap_option_value(inner),
        DecodedValue::Option(None) => None,
        other => Some(other),
    }
}

/// The reference `variantLabel`: the variant's display label, else its
/// title-cased name; an index without a variant prints as the bare number.
fn enum_label(variants: &[super::model::EnumVariantNode], index: usize) -> String {
    variants.get(index).map_or_else(
        || index.to_string(),
        |variant| {
            variant
                .display
                .as_ref()
                .and_then(|display| display.label.clone())
                .unwrap_or_else(|| title_case(&variant.name))
        },
    )
}

/// The reference `toSeconds`: `Number(value) / ticksPerSecond` in IEEE
/// doubles. `i128 as f64` rounds to nearest-even exactly like `Number(bigint)`.
fn to_seconds(value: i128, ticks_per_second: u32) -> f64 {
    (value as f64) / f64::from(ticks_per_second)
}

/// The reference `formatDateTimeValue`: `new Date(seconds * 1000)
/// .toISOString()`, including ECMAScript's time clip (`|ms| <= 8.64e15`,
/// truncated toward zero) and expanded `±YYYYYY` years outside 0000–9999.
fn format_date_time(value: i128, ticks_per_second: u32) -> Option<String> {
    let millis = to_seconds(value, ticks_per_second) * 1000.0;
    if !millis.is_finite() || millis.abs() > 8.64e15 {
        return None;
    }
    Some(iso_utc_from_epoch_millis(millis.trunc() as i64))
}

fn iso_utc_from_epoch_millis(millis: i64) -> String {
    let days = millis.div_euclid(86_400_000);
    let millis_of_day = millis.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    let hours = millis_of_day / 3_600_000;
    let minutes = (millis_of_day % 3_600_000) / 60_000;
    let seconds = (millis_of_day % 60_000) / 1_000;
    let fraction = millis_of_day % 1_000;
    let year_text = if (0..=9999).contains(&year) {
        format!("{year:04}")
    } else if year < 0 {
        format!("-{:06}", -year)
    } else {
        format!("+{year:06}")
    };
    format!("{year_text}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{fraction:03}Z")
}

/// Gregorian calendar date from days since the Unix epoch.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The reference `formatDurationValue`: negative durations are unformatted;
/// otherwise whole seconds split into `HH:mm:ss` with uncapped hours. The
/// arithmetic stays in doubles so rounding artefacts match the reference.
fn format_duration(value: i128, ticks_per_second: u32) -> Option<String> {
    let total = to_seconds(value, ticks_per_second);
    if !total.is_finite() || total < 0.0 {
        return None;
    }
    let whole = total.floor();
    let hours = (whole / 3600.0).floor();
    let minutes = ((whole % 3600.0) / 60.0).floor();
    let seconds = whole % 60.0;
    Some(format!(
        "{:02}:{:02}:{:02}",
        hours as u64, minutes as u64, seconds as u64
    ))
}

/// Integer-only fixed-point scaling, matching the reference `scaleByDecimals`:
/// the magnitude is left-padded, split, and trailing fractional zeros trimmed;
/// a negative value keeps a single leading `-`.
pub(crate) fn scale_by_decimals(value: i128, decimals: usize) -> String {
    if decimals == 0 {
        return value.to_string();
    }
    let sign = if value < 0 { "-" } else { "" };
    let digits = value.unsigned_abs().to_string();
    let padded = if digits.len() <= decimals {
        format!("{}{}", "0".repeat(decimals + 1 - digits.len()), digits)
    } else {
        digits
    };
    let split = padded.len() - decimals;
    let integer = &padded[..split];
    let fraction = padded[split..].trim_end_matches('0');
    if fraction.is_empty() {
        format!("{sign}{integer}")
    } else {
        format!("{sign}{integer}.{fraction}")
    }
}

/// The reference `rawValue`: scalars print plainly, an absent option prints
/// `none`, and a struct prints as the JSON the reference decoder would
/// stringify (see [`raw_json`]).
fn raw_value(value: &DecodedValue) -> String {
    match value {
        DecodedValue::Boolean(value) => value.to_string(),
        DecodedValue::Number(number) => number.value.to_string(),
        DecodedValue::Option(Some(value)) => raw_value(value),
        DecodedValue::Option(None) => "none".to_string(),
        DecodedValue::PublicKey(address) => address.clone(),
        DecodedValue::Enum { index } => index.to_string(),
        DecodedValue::String(text) => text.clone(),
        DecodedValue::Array(_) | DecodedValue::Struct(_) => raw_json(value),
    }
}

/// Replicates `JSON.stringify(value, (_, v) => typeof v === 'bigint' ?
/// v.toString() : v)` over the shapes the reference decoder produces: struct
/// fields in declaration order, wide integers quoted, narrow integers bare,
/// options as `{"__option":"Some","value":…}` / `{"__option":"None"}`.
fn raw_json(value: &DecodedValue) -> String {
    match value {
        DecodedValue::Boolean(value) => value.to_string(),
        DecodedValue::Number(number) => {
            if number.format.is_wide() {
                json_string(&number.value.to_string())
            } else {
                number.value.to_string()
            }
        }
        DecodedValue::Option(Some(value)) => {
            format!("{{\"__option\":\"Some\",\"value\":{}}}", raw_json(value))
        }
        DecodedValue::Option(None) => "{\"__option\":\"None\"}".to_string(),
        DecodedValue::PublicKey(address) => json_string(address),
        // The pinned reference decodes scalar enums with `getEnumCodec`, so a
        // nested enum appears as its bare index.
        DecodedValue::Enum { index } => index.to_string(),
        DecodedValue::String(text) => json_string(text),
        DecodedValue::Array(elements) => {
            let rendered = elements.iter().map(raw_json).collect::<Vec<_>>().join(",");
            format!("[{rendered}]")
        }
        DecodedValue::Struct(fields) => {
            let rendered = fields
                .iter()
                .map(|(name, value)| format!("{}:{}", json_string(name), raw_json(value)))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{rendered}}}")
        }
    }
}

/// A JSON string literal with the same escapes `JSON.stringify` applies to
/// well-formed text.
fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""))
}

/// The first character upper-cased and the rest lower-cased, as the reference
/// `capitalize` does.
fn capitalize(word: &str) -> String {
    let mut characters = word.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };
    let mut output: String = first.to_uppercase().collect();
    output.push_str(&characters.as_str().to_lowercase());
    output
}

/// Exact port of the reference `titleCase`: an all-caps snake-case name is
/// lower-cased and split on `_`; anything else gets a space before every
/// ASCII capital, is split on non-alphanumerics, and each word is capitalized.
pub(crate) fn title_case(value: &str) -> String {
    if is_upper_snake_case(value) {
        return value
            .to_lowercase()
            .split('_')
            .map(capitalize)
            .collect::<Vec<_>>()
            .join(" ");
    }
    let mut spaced = String::with_capacity(value.len() + 8);
    for character in value.chars() {
        if character.is_ascii_uppercase() {
            spaced.push(' ');
        }
        spaced.push(character);
    }
    spaced
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(capitalize)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The reference regex `^[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+$`.
fn is_upper_snake_case(value: &str) -> bool {
    let mut parts = value.split('_');
    let Some(first) = parts.next() else {
        return false;
    };
    let upper_or_digit = |word: &str| {
        !word.is_empty()
            && word
                .chars()
                .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
    };
    if !first.starts_with(|character: char| character.is_ascii_uppercase())
        || !upper_or_digit(first)
    {
        return false;
    }
    let mut rest = 0;
    for part in parts {
        if !upper_or_digit(part) {
            return false;
        }
        rest += 1;
    }
    rest > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::decode::{DecodedNumber, NumberFormat};

    fn number(value: i128, format: NumberFormat) -> DecodedValue {
        DecodedValue::Number(DecodedNumber { value, format })
    }

    #[test]
    fn decimal_scaling_is_integer_only_and_trims_zeros() {
        assert_eq!(scale_by_decimals(1_500_000, 6), "1.5");
        assert_eq!(scale_by_decimals(1, 6), "0.000001");
        assert_eq!(scale_by_decimals(1_000_000, 6), "1");
    }

    #[test]
    fn decimal_scaling_keeps_the_sign_like_the_reference() {
        assert_eq!(scale_by_decimals(-1_500_000, 6), "-1.5");
        assert_eq!(scale_by_decimals(-5, 0), "-5");
        assert_eq!(scale_by_decimals(-1, 6), "-0.000001");
        assert_eq!(scale_by_decimals(-1_000_000, 6), "-1");
    }

    #[test]
    fn title_case_is_an_exact_port_of_the_reference() {
        assert_eq!(title_case("transferTokens"), "Transfer Tokens");
        assert_eq!(title_case("token_amount"), "Token Amount");
        assert_eq!(title_case("USDCAmount"), "U S D C Amount");
        assert_eq!(title_case("MAX_SUPPLY"), "Max Supply");
        assert_eq!(title_case("someURL"), "Some U R L");
        assert_eq!(title_case("periodLengthS"), "Period Length S");
        assert_eq!(title_case("a__b"), "A B");
        assert_eq!(title_case("maxURLSize"), "Max U R L Size");
        assert_eq!(title_case("ALLCAPS"), "A L L C A P S");
        assert_eq!(title_case("v2Upgrade"), "V2 Upgrade");
        assert_eq!(title_case(""), "");
    }

    #[test]
    fn raw_values_match_reference_option_boolean_and_public_key_text() {
        assert_eq!(raw_value(&DecodedValue::Boolean(true)), "true");
        assert_eq!(raw_value(&DecodedValue::Boolean(false)), "false");
        assert_eq!(raw_value(&DecodedValue::Option(None)), "none");
        assert_eq!(
            raw_value(&DecodedValue::Option(Some(Box::new(number(
                42,
                NumberFormat::U64
            ))))),
            "42"
        );
        assert_eq!(
            raw_value(&DecodedValue::PublicKey(
                "11111111111111111111111111111111".to_string()
            )),
            "11111111111111111111111111111111"
        );
    }

    #[test]
    fn date_time_formatting_matches_ecmascript_to_iso_string() {
        let cases: [(i128, u32, Option<&str>); 14] = [
            (0, 1, Some("1970-01-01T00:00:00.000Z")),
            (1_761_365_183, 1, Some("2025-10-25T04:06:23.000Z")),
            (1_761_365_183_000, 1000, Some("2025-10-25T04:06:23.000Z")),
            (-1, 1, Some("1969-12-31T23:59:59.000Z")),
            (-1, 1000, Some("1969-12-31T23:59:59.999Z")),
            (8_640_000_000_000, 1, Some("+275760-09-13T00:00:00.000Z")),
            (8_640_000_000_001, 1, None),
            (-8_640_000_000_000, 1, Some("-271821-04-20T00:00:00.000Z")),
            (-8_640_000_000_001, 1, None),
            (253_402_300_800, 1, Some("+010000-01-01T00:00:00.000Z")),
            (-62_167_219_200, 1, Some("0000-01-01T00:00:00.000Z")),
            (-62_167_219_201, 1, Some("-000001-12-31T23:59:59.000Z")),
            (
                9_007_199_254_740_993,
                10_000,
                Some("+030512-09-05T03:17:54.099Z"),
            ),
            (i128::from(i64::MAX), 1, None),
        ];
        for (value, ticks, expected) in cases {
            assert_eq!(
                format_date_time(value, ticks).as_deref(),
                expected,
                "value {value} at {ticks} ticks/s"
            );
        }
        assert_eq!(
            format_date_time(15, 10_000).as_deref(),
            Some("1970-01-01T00:00:00.001Z")
        );
        assert_eq!(format_date_time(999_999_999_999_999, 1), None);
    }

    #[test]
    fn duration_formatting_matches_the_reference_double_arithmetic() {
        let cases: [(i128, u32, Option<&str>); 11] = [
            (3600, 1, Some("01:00:00")),
            (90_000, 1000, Some("00:01:30")),
            (0, 1, Some("00:00:00")),
            (1_209_600, 1, Some("336:00:00")),
            (31_536_000, 1, Some("8760:00:00")),
            (i128::from(u64::MAX), 1, Some("5124095576030431:00:16")),
            (i128::from(u64::MAX), 1000, Some("5124095576030:25:52")),
            (i128::from(i64::MAX), 1, Some("2562047788015215:30:08")),
            (-1, 1, None),
            (1500, 1000, Some("00:00:01")),
            (999, 1000, Some("00:00:00")),
        ];
        for (value, ticks, expected) in cases {
            assert_eq!(
                format_duration(value, ticks).as_deref(),
                expected,
                "value {value} at {ticks} ticks/s"
            );
        }
        assert_eq!(
            format_duration(9_007_199_254_740_993, 1).as_deref(),
            Some("2501999792983:36:32")
        );
        assert_eq!(format_duration(i128::from(i64::MIN), 1), None);
    }

    #[test]
    fn struct_raw_value_replicates_json_stringify_with_bigint_replacer() {
        let value = DecodedValue::Struct(vec![
            ("a".to_string(), number(1, NumberFormat::U64)),
            ("b".to_string(), number(7, NumberFormat::U32)),
            ("c".to_string(), DecodedValue::Option(None)),
            (
                "d".to_string(),
                DecodedValue::Option(Some(Box::new(number(5, NumberFormat::U64)))),
            ),
            (
                "e".to_string(),
                DecodedValue::PublicKey("3Wnd5Df69KitZfUoPYZU438eFRNwGHkhLnSAWL65PxJX".to_string()),
            ),
            ("f".to_string(), DecodedValue::Boolean(true)),
            (
                "g".to_string(),
                DecodedValue::Struct(vec![("z".to_string(), number(0, NumberFormat::U8))]),
            ),
        ]);
        assert_eq!(
            raw_value(&value),
            "{\"a\":\"1\",\"b\":7,\"c\":{\"__option\":\"None\"},\"d\":{\"__option\":\"Some\",\"value\":\"5\"},\"e\":\"3Wnd5Df69KitZfUoPYZU438eFRNwGHkhLnSAWL65PxJX\",\"f\":true,\"g\":{\"z\":0}}"
        );
        assert_eq!(
            raw_value(&DecodedValue::Option(Some(Box::new(value.clone())))),
            raw_value(&value),
            "an option around a struct unwraps before stringifying"
        );
    }
}
