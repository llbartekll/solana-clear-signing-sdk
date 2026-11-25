//! Character policy for text the IDL author did not write: decoded strings
//! and host-supplied names and labels.

/// Longest host-supplied name, label or source, in characters.
const MAX_HOST_TEXT: usize = 128;

/// Characters that can move, hide or split surrounding text: controls, line
/// and paragraph separators, bidi controls, and invisible formatting.
fn is_unsafe(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'..='\u{1160}'
                | '\u{17B4}'..='\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{3164}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF0}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E0FFF}'
        )
}

pub(crate) fn has_unsafe_characters(text: &str) -> bool {
    text.chars().any(is_unsafe)
}

/// Makes every unsafe character visible as `\u{XXXX}`.
pub(crate) fn escape_unsafe(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        if is_unsafe(character) {
            output.extend(character.escape_unicode());
        } else {
            output.push(character);
        }
    }
    output
}

pub(crate) fn host_text_is_acceptable(text: &str) -> bool {
    !text.trim().is_empty() && text.chars().count() <= MAX_HOST_TEXT && !has_unsafe_characters(text)
}

/// Additionally refuses a label that reads as another address.
pub(crate) fn label_is_acceptable(label: &str) -> bool {
    host_text_is_acceptable(label)
        && !bs58::decode(label.trim())
            .into_vec()
            .is_ok_and(|bytes| bytes.len() == 32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_characters_are_detected_and_made_visible() {
        for text in [
            "a\nb",
            "a\u{85}b",
            "a\u{202E}b",
            "a\u{2066}b",
            "a\u{200B}b",
            "a\u{2028}b",
            "\u{FEFF}a",
            "a\u{E0041}",
        ] {
            assert!(has_unsafe_characters(text), "{text:?}");
            assert!(!has_unsafe_characters(&escape_unsafe(text)), "{text:?}");
        }
        assert_eq!(escape_unsafe("a\n\u{202E}b"), "a\\u{a}\\u{202e}b");
        assert!(!has_unsafe_characters("Zażółć 漢字 ❤️ \u{FFFD}"));
    }

    #[test]
    fn host_text_policy_refuses_spoofs() {
        assert!(label_is_acceptable("Treasury (cold)"));
        assert!(!label_is_acceptable(" "));
        assert!(!label_is_acceptable("Treasury\nAmount"));
        assert!(!label_is_acceptable(
            "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM"
        ));
        assert!(host_text_is_acceptable(&"a".repeat(128)));
        assert!(!host_text_is_acceptable(&"a".repeat(129)));
    }
}
