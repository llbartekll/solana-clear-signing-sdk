use std::collections::BTreeMap;

use super::model::fixed_size;
use super::model::CountNode;
use super::model::DiscriminatorNode;
use super::model::InstructionNode;
use super::model::ProgramNode;
use super::model::TypeNode;
use super::model::ValueNode;

/// The wire format of a decoded integer. The reference decoder yields a JS
/// `number` for narrow formats and a `bigint` for wide ones, which changes how
/// the value is quoted inside struct JSON, so the format travels with the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumberFormat {
    U8,
    U32,
    U64,
    I64,
}

impl NumberFormat {
    pub(crate) fn parse(format: &str) -> Option<Self> {
        match format {
            "u8" => Some(Self::U8),
            "u32" => Some(Self::U32),
            "u64" => Some(Self::U64),
            "i64" => Some(Self::I64),
            _ => None,
        }
    }

    pub(crate) fn byte_len(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U32 => 4,
            Self::U64 | Self::I64 => 8,
        }
    }

    /// `true` when the reference decoder represents the format as a `bigint`,
    /// which `JSON.stringify` renders as a quoted decimal string.
    pub(crate) fn is_wide(self) -> bool {
        matches!(self, Self::U64 | Self::I64)
    }
}

/// One decoded integer: the value widened to `i128` so unsigned and signed
/// formats share one arithmetic path, plus the format it was decoded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecodedNumber {
    pub(crate) value: i128,
    pub(crate) format: NumberFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DecodedValue {
    Boolean(bool),
    Number(DecodedNumber),
    Option(Option<Box<DecodedValue>>),
    PublicKey(String),
    /// A scalar enum variant by index. The pinned reference decoder yields the
    /// bare index (a JS `number`), so that is also what struct JSON shows.
    Enum {
        index: usize,
    },
    /// A fixed-size UTF-8 string after the reference's decoding: lossy,
    /// leading BOM dropped, every NUL removed.
    String(String),
    Array(Vec<DecodedValue>),
    /// Struct fields in IDL declaration order, which the reference preserves
    /// when it renders a struct as JSON.
    Struct(Vec<(String, DecodedValue)>),
}

impl DecodedValue {
    /// The named field of a decoded struct, or `None` for any other shape.
    pub(crate) fn field(&self, name: &str) -> Option<&DecodedValue> {
        let Self::Struct(fields) = self else {
            return None;
        };
        fields
            .iter()
            .find(|(field_name, _)| field_name == name)
            .map(|(_, value)| value)
    }
}

pub(crate) type DecodedArguments = BTreeMap<String, DecodedValue>;

pub(crate) fn identify_instruction<'a>(
    program: &'a ProgramNode,
    bytes: &[u8],
) -> Option<&'a InstructionNode> {
    if let Some(instruction) = program.instructions.iter().find(|instruction| {
        !instruction.discriminators.is_empty()
            && instruction
                .discriminators
                .iter()
                .all(|discriminator| matches_discriminator(instruction, discriminator, bytes))
    }) {
        return Some(instruction);
    }

    if program.instructions.len() == 1 && program.instructions[0].discriminators.is_empty() {
        return program.instructions.first();
    }
    None
}

pub(crate) fn decode_instruction(
    instruction: &InstructionNode,
    bytes: &[u8],
) -> Option<DecodedArguments> {
    let mut cursor = 0;
    let mut values = BTreeMap::new();
    for argument in &instruction.arguments {
        let value = decode_type(&argument.ty, bytes, &mut cursor)?;
        values.insert(argument.name.clone(), value);
    }
    Some(values)
}

pub(crate) fn decode_account(node: &TypeNode, bytes: &[u8]) -> Option<DecodedValue> {
    let mut cursor = 0;
    let value = decode_type(node, bytes, &mut cursor)?;
    Some(value)
}

fn matches_discriminator(
    instruction: &InstructionNode,
    discriminator: &DiscriminatorNode,
    bytes: &[u8],
) -> bool {
    match discriminator {
        DiscriminatorNode::Field { name, offset } => {
            let Some(argument) = instruction
                .arguments
                .iter()
                .find(|argument| argument.name == *name)
            else {
                return false;
            };
            let Some(ValueNode::Number { number }) = argument.default_value.as_ref() else {
                return false;
            };
            matches_number_at(&argument.ty, *number, bytes, *offset)
        }
        DiscriminatorNode::Size { size } => bytes.len() == *size,
    }
}

fn decode_type(node: &TypeNode, bytes: &[u8], cursor: &mut usize) -> Option<DecodedValue> {
    match node {
        TypeNode::Boolean { size } => {
            let value = decode_number(size, bytes, cursor)?;
            Some(DecodedValue::Boolean(value.value == 1))
        }
        TypeNode::Number { .. } => decode_number(node, bytes, cursor).map(DecodedValue::Number),
        TypeNode::Option {
            item,
            prefix,
            fixed,
        } => {
            let tag = decode_number(prefix, bytes, cursor)?;
            if tag.value == 1 {
                let value = decode_type(item, bytes, cursor)?;
                Some(DecodedValue::Option(Some(Box::new(value))))
            } else if *fixed {
                let item_size = fixed_size(item)?;
                let end = cursor.checked_add(item_size)?;
                bytes.get(*cursor..end)?;
                *cursor = end;
                Some(DecodedValue::Option(None))
            } else {
                Some(DecodedValue::Option(None))
            }
        }
        TypeNode::PublicKey => {
            let end = cursor.checked_add(32)?;
            let value = bytes.get(*cursor..end)?;
            *cursor = end;
            Some(DecodedValue::PublicKey(bs58::encode(value).into_string()))
        }
        TypeNode::Struct { fields } => {
            let mut values = Vec::with_capacity(fields.len());
            for field in fields {
                values.push((field.name.clone(), decode_type(&field.ty, bytes, cursor)?));
            }
            Some(DecodedValue::Struct(values))
        }
        // Links are inlined when the IDL loads; none can reach the decoder.
        TypeNode::DefinedTypeLink { .. } => None,
        TypeNode::Enum { size, variants } => {
            let index = usize::try_from(decode_number(size, bytes, cursor)?.value).ok()?;
            // The reference codec throws on an out-of-range variant, which
            // surfaces as an instruction miss or an account decode failure.
            (index < variants.len()).then_some(DecodedValue::Enum { index })
        }
        TypeNode::FixedSize { size, ty } => match ty.as_ref() {
            TypeNode::String { .. } => decode_fixed_utf8(*size, bytes, cursor),
            // Validation admits only strings inside a fixed-size wrapper.
            _ => None,
        },
        // A bare string never passes validation.
        TypeNode::String { .. } => None,
        TypeNode::Array { item, count } => {
            let CountNode::Fixed { value: count } = count;
            // Check the complete wire span before allocating the value tree.
            let end = cursor.checked_add(fixed_size(node)?)?;
            bytes.get(*cursor..end)?;
            let mut items = Vec::new();
            items.try_reserve_exact(*count).ok()?;
            for _ in 0..*count {
                items.push(decode_type(item, bytes, cursor)?);
            }
            Some(DecodedValue::Array(items))
        }
    }
}

/// `fixCodecSize(getUtf8Codec(), size)` as the pinned Kit decodes it: exactly
/// `size` bytes, a WHATWG `TextDecoder` (one leading BOM consumed, invalid
/// sequences replaced by U+FFFD per maximal subpart), then every NUL removed.
fn decode_fixed_utf8(size: usize, bytes: &[u8], cursor: &mut usize) -> Option<DecodedValue> {
    let end = cursor.checked_add(size)?;
    let slice = bytes.get(*cursor..end)?;
    *cursor = end;
    let slice = slice.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(slice);
    let text: String = String::from_utf8_lossy(slice)
        .chars()
        .filter(|character| *character != '\0')
        .collect();
    Some(DecodedValue::String(text))
}

fn decode_number(node: &TypeNode, bytes: &[u8], cursor: &mut usize) -> Option<DecodedNumber> {
    let TypeNode::Number { format, .. } = node else {
        return None;
    };
    let format = NumberFormat::parse(format)?;
    let end = cursor.checked_add(format.byte_len())?;
    let slice = bytes.get(*cursor..end)?;
    let value = match format {
        NumberFormat::U8 => i128::from(slice[0]),
        NumberFormat::U32 => i128::from(u32::from_le_bytes(slice.try_into().ok()?)),
        NumberFormat::U64 => i128::from(u64::from_le_bytes(slice.try_into().ok()?)),
        NumberFormat::I64 => i128::from(i64::from_le_bytes(slice.try_into().ok()?)),
    };
    *cursor = end;
    Some(DecodedNumber { value, format })
}

fn matches_number_at(node: &TypeNode, number: u64, bytes: &[u8], offset: usize) -> bool {
    let TypeNode::Number { format, .. } = node else {
        return false;
    };
    match format.as_str() {
        "u8" => bytes.get(offset).copied() == u8::try_from(number).ok(),
        "u32" => u32::try_from(number).ok().is_some_and(|number| {
            bytes
                .get(offset..offset.saturating_add(4))
                .is_some_and(|slice| slice == number.to_le_bytes())
        }),
        "u64" => bytes
            .get(offset..offset.saturating_add(8))
            .is_some_and(|slice| slice == number.to_le_bytes()),
        "i64" => i64::try_from(number).ok().is_some_and(|number| {
            bytes
                .get(offset..offset.saturating_add(8))
                .is_some_and(|slice| slice == number.to_le_bytes())
        }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::srf39::model::{EnumVariantNode, StructFieldTypeNode};
    use crate::srf39::LoadedSrf39Idl;

    fn number_type(format: &str) -> TypeNode {
        TypeNode::Number {
            format: format.to_string(),
            endian: Some("le".to_string()),
            display: None,
        }
    }

    fn fixed_public_key_option() -> TypeNode {
        TypeNode::Option {
            item: Box::new(TypeNode::PublicKey),
            prefix: Box::new(number_type("u32")),
            fixed: true,
        }
    }

    #[test]
    fn truncated_matching_instruction_does_not_decode() {
        let json = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../conformance/fixtures/transfer-checked/root.json"
        ));
        let idl = LoadedSrf39Idl::from_json(json).expect("fixture IDL");
        let program = idl
            .program_by_address("11111111111111111111111111111111")
            .expect("fixture program");
        let instruction = identify_instruction(program, &[12]).expect("matching discriminator");
        assert_eq!(decode_instruction(instruction, &[12]), None);
    }

    #[test]
    fn decodes_u32_little_endian() {
        let mut cursor = 0;
        assert_eq!(
            decode_type(&number_type("u32"), &[0x78, 0x56, 0x34, 0x12], &mut cursor),
            Some(DecodedValue::Number(DecodedNumber {
                value: 0x1234_5678,
                format: NumberFormat::U32,
            }))
        );
        assert_eq!(cursor, 4);
    }

    #[test]
    fn decodes_i64_little_endian_with_sign() {
        let mut cursor = 0;
        assert_eq!(
            decode_type(&number_type("i64"), &(-5i64).to_le_bytes(), &mut cursor),
            Some(DecodedValue::Number(DecodedNumber {
                value: -5,
                format: NumberFormat::I64,
            }))
        );
        assert_eq!(cursor, 8);
        for extreme in [i64::MIN, i64::MAX] {
            let mut cursor = 0;
            assert_eq!(
                decode_type(&number_type("i64"), &extreme.to_le_bytes(), &mut cursor),
                Some(DecodedValue::Number(DecodedNumber {
                    value: i128::from(extreme),
                    format: NumberFormat::I64,
                }))
            );
        }
        let mut cursor = 0;
        assert_eq!(decode_type(&number_type("i64"), &[0; 7], &mut cursor), None);
    }

    #[test]
    fn i64_discriminators_match_non_negative_defaults_only() {
        let ty = number_type("i64");
        assert!(matches_number_at(&ty, 7, &7i64.to_le_bytes(), 0));
        assert!(!matches_number_at(&ty, 7, &8i64.to_le_bytes(), 0));
        assert!(!matches_number_at(&ty, u64::MAX, &(-1i64).to_le_bytes(), 0));
    }

    #[test]
    fn scalar_enums_decode_by_index_and_reject_out_of_range() {
        let ty = TypeNode::Enum {
            size: Box::new(number_type("u8")),
            variants: vec![
                EnumVariantNode {
                    kind: "enumEmptyVariantTypeNode".to_string(),
                    name: "buy".to_string(),
                    discriminator: Some(0),
                    display: None,
                },
                EnumVariantNode {
                    kind: "enumEmptyVariantTypeNode".to_string(),
                    name: "sell".to_string(),
                    discriminator: None,
                    display: None,
                },
            ],
        };
        let mut cursor = 0;
        assert_eq!(
            decode_type(&ty, &[1], &mut cursor),
            Some(DecodedValue::Enum { index: 1 })
        );
        assert_eq!(cursor, 1);
        let mut cursor = 0;
        assert_eq!(decode_type(&ty, &[2], &mut cursor), None);
        let mut cursor = 0;
        assert_eq!(decode_type(&ty, &[], &mut cursor), None);
    }

    #[test]
    fn fixed_utf8_strings_decode_like_the_pinned_text_decoder() {
        let ty = TypeNode::FixedSize {
            size: 8,
            ty: Box::new(TypeNode::String {
                encoding: "utf8".to_string(),
            }),
        };
        let cases: [(&[u8], &str); 6] = [
            (b"\xEF\xBB\xBFab\0\0\0", "ab"),
            (b"a\0b\0\0\0\0\0", "ab"),
            (b"\xFF\xC3a\0\0\0\0\0", "\u{FFFD}\u{FFFD}a"),
            (b"\xF0\x9F\x98\0a\0\0\0", "\u{FFFD}a"),
            (b"\xEF\xBB\xBF\xEF\xBB\xBFa\0", "\u{FEFF}a"),
            (b"\0\0\0\0\0\0\0\0", ""),
        ];
        for (bytes, expected) in cases {
            let mut cursor = 0;
            assert_eq!(
                decode_type(&ty, bytes, &mut cursor),
                Some(DecodedValue::String(expected.to_string())),
                "{bytes:?}"
            );
            assert_eq!(cursor, 8);
        }
        let mut cursor = 0;
        assert_eq!(decode_type(&ty, b"ab", &mut cursor), None);
    }

    #[test]
    fn fixed_arrays_decode_every_item_or_nothing() {
        let ty = TypeNode::Array {
            item: Box::new(number_type("u32")),
            count: CountNode::Fixed { value: 2 },
        };
        let mut cursor = 0;
        assert_eq!(
            decode_type(&ty, &[1, 0, 0, 0, 2, 0, 0, 0, 9], &mut cursor),
            Some(DecodedValue::Array(vec![
                DecodedValue::Number(DecodedNumber {
                    value: 1,
                    format: NumberFormat::U32,
                }),
                DecodedValue::Number(DecodedNumber {
                    value: 2,
                    format: NumberFormat::U32,
                }),
            ]))
        );
        assert_eq!(cursor, 8);
        let mut cursor = 0;
        assert_eq!(decode_type(&ty, &[1, 0, 0, 0, 2, 0, 0], &mut cursor), None);
    }

    #[test]
    fn oversized_and_truncated_arrays_fail_before_decoding_items() {
        for count in [usize::MAX, 65_535] {
            let ty = TypeNode::Array {
                item: Box::new(number_type("u32")),
                count: CountNode::Fixed { value: count },
            };
            let mut cursor = 0;
            assert_eq!(decode_type(&ty, &[1, 0, 0, 0], &mut cursor), None);
            assert_eq!(cursor, 0);
        }
    }

    #[test]
    fn struct_fields_keep_declaration_order() {
        let ty = TypeNode::Struct {
            fields: vec![
                StructFieldTypeNode {
                    kind: "structFieldTypeNode".to_string(),
                    name: "zeta".to_string(),
                    ty: number_type("u8"),
                    display: None,
                },
                StructFieldTypeNode {
                    kind: "structFieldTypeNode".to_string(),
                    name: "alpha".to_string(),
                    ty: number_type("u8"),
                    display: None,
                },
            ],
        };
        let mut cursor = 0;
        let decoded = decode_type(&ty, &[1, 2], &mut cursor).expect("decodes");
        let DecodedValue::Struct(fields) = &decoded else {
            panic!("expected a struct");
        };
        assert_eq!(fields[0].0, "zeta");
        assert_eq!(fields[1].0, "alpha");
        assert_eq!(
            decoded.field("alpha"),
            Some(&DecodedValue::Number(DecodedNumber {
                value: 2,
                format: NumberFormat::U8,
            }))
        );
        assert_eq!(decoded.field("missing"), None);
    }

    #[test]
    fn decodes_public_key_as_full_base58() {
        let mut cursor = 0;
        assert_eq!(
            decode_type(&TypeNode::PublicKey, &[0; 32], &mut cursor),
            Some(DecodedValue::PublicKey(
                "11111111111111111111111111111111".to_string()
            ))
        );
        assert_eq!(cursor, 32);
    }

    #[test]
    fn boolean_is_true_only_for_one() {
        let ty = TypeNode::Boolean {
            size: Box::new(number_type("u8")),
        };
        for (byte, expected) in [(0, false), (1, true), (2, false)] {
            let mut cursor = 0;
            assert_eq!(
                decode_type(&ty, &[byte], &mut cursor),
                Some(DecodedValue::Boolean(expected))
            );
            assert_eq!(cursor, 1);
        }
    }

    #[test]
    fn fixed_none_consumes_the_padded_item() {
        let mut cursor = 0;
        assert_eq!(
            decode_type(&fixed_public_key_option(), &[0; 36], &mut cursor),
            Some(DecodedValue::Option(None))
        );
        assert_eq!(cursor, 36);
    }

    #[test]
    fn fixed_some_decodes_the_item() {
        let mut bytes = vec![1, 0, 0, 0];
        bytes.extend([0; 32]);
        let mut cursor = 0;
        assert_eq!(
            decode_type(&fixed_public_key_option(), &bytes, &mut cursor),
            Some(DecodedValue::Option(Some(Box::new(
                DecodedValue::PublicKey("11111111111111111111111111111111".to_string())
            ))))
        );
        assert_eq!(cursor, 36);
    }

    #[test]
    fn new_fixed_types_reject_truncation() {
        let mut cursor = 0;
        assert_eq!(decode_type(&number_type("u32"), &[0; 3], &mut cursor), None);

        let mut cursor = 0;
        assert_eq!(
            decode_type(&TypeNode::PublicKey, &[0; 31], &mut cursor),
            None
        );

        let mut cursor = 0;
        assert_eq!(
            decode_type(&fixed_public_key_option(), &[0; 35], &mut cursor),
            None
        );
    }

    #[test]
    fn size_discriminator_requires_an_exact_match() {
        let instruction = InstructionNode {
            kind: "instructionNode".to_string(),
            name: "sized".to_string(),
            accounts: Vec::new(),
            arguments: Vec::new(),
            discriminators: Vec::new(),
            remaining_accounts: Vec::new(),
            provides: Vec::new(),
            display: None,
        };
        let discriminator = DiscriminatorNode::Size { size: 2 };
        assert!(matches_discriminator(&instruction, &discriminator, &[1, 2]));
        assert!(!matches_discriminator(&instruction, &discriminator, &[1]));
        assert!(!matches_discriminator(
            &instruction,
            &discriminator,
            &[1, 2, 3]
        ));
    }
}
