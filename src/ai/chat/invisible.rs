//! Classification of invisible Unicode characters in tool output.
//!
//! This is the pure logic behind the `InvisibleCharFilter` middleware. It
//! inspects a string and decides whether the invisible characters it
//! contains are benign (structural artifacts we can strip and pass through),
//! or a sign of smuggling (reject).
//!
//! The key distinction is *what a character can do*:
//!
//! - **Can hide meaning** — control chars, bidirectional overrides, and the
//!   Unicode tag block (`U+E0000..U+E007F`). These let an attacker make the
//!   model read something a human never sees. Any occurrence is a hard reject.
//!
//! - **Can only interleave** — zero-width spaces, joiners, BOM, variation
//!   selectors, and Hangul fillers. These are invisible but cannot map to
//!   hidden letters, so they can't *hide* an instruction. Their only use in
//!   an attack is to be slipped between letters of a word to defeat
//!   substring filters while keeping the text readable to the model. So they
//!   are stripped when they sit at structural boundaries (next to whitespace,
//!   punctuation, or markup) but rejected when interleaved mid-word.

/// The set of characters that can *hide* meaning and are always rejected.
///
/// - C0 controls, DEL, C1 controls (`char::is_control`), plus soft hyphen
///   `U+00AD`
/// - Bidirectional overrides `U+202A..U+202E`, `U+2066..U+2069`
/// - Unicode tag block `U+E0000..U+E007F`
fn is_hard_reject(c: char) -> Option<RejectReason> {
    let code = c as u32;
    if c.is_control() || code == 0xAD {
        return Some(RejectReason::ControlChar);
    }
    if (0x202A..=0x202E).contains(&code) || (0x2066..=0x2069).contains(&code) {
        return Some(RejectReason::BidiFormatting);
    }
    if (0xE0000..=0xE007F).contains(&code) {
        return Some(RejectReason::TagBlock);
    }
    None
}

/// The set of characters that can only interleave and are safe to strip when
/// they appear at structural boundaries: zero-width chars, joiners, BOM,
/// variation selectors, and Hangul fillers.
fn is_strippable(c: char) -> bool {
    let code = c as u32;
    matches!(code, 0x200B | 0x200C | 0x200D | 0x2060 | 0xFEFF)
        || (0xFE00..=0xFE0F).contains(&code)
        || (0xE0100..=0xE01EF).contains(&code)
        || matches!(code, 0x3164 | 0xFFA0)
}

/// Reject when stripped characters make up more than ~1/3 of the input *and*
/// there are at least a handful of them. A handful of structural artifacts
/// (e.g. Docusaurus heading anchors) lands around 10-15%; a single emoji ZWJ
/// sequence is short but legitimate. Only a string that is mostly invisible
/// junk trips the density check.
const MIN_STRIPPABLE_COUNT: usize = 5;
const MAX_STRIPPABLE_DIVISOR: usize = 3; // allow up to 1 / 3 = 33%

/// Why a string was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// Control characters or soft hyphen — can corrupt or confuse output.
    ControlChar,
    /// Bidirectional overrides — can make text render differently than it reads.
    BidiFormatting,
    /// Unicode tag block chars — the primary prompt-injection vector.
    TagBlock,
    /// A strippable char interleaved between two letters — filter evasion.
    MidWordInterleaving,
    /// Too many strippable chars for the size of the input.
    ExcessiveInvisibleChars,
}

/// Outcome of scanning a string for invisible characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvisibleSanitization {
    /// No invisible characters — safe to pass through unchanged.
    Clean,
    /// Only benign, boundary-adjacent invisible characters were found; the
    /// cleaned string (with them removed) is provided.
    Cleaned(String),
    /// The input contains a character that can hide meaning or indicates
    /// smuggling. It must be rejected, not modified.
    Reject { reason: RejectReason },
}

/// Scan `input` and classify its invisible characters.
pub fn sanitize_invisible_chars(input: &str) -> InvisibleSanitization {
    let mut out = String::with_capacity(input.len());
    let mut prev: Option<char> = None;
    let mut stripped = 0usize;
    let mut total = 0usize;

    let mut it = input.chars();
    while let Some(c) = it.next() {
        total += 1;
        // Common whitespace is legitimate content, not invisible.
        if matches!(c, '\t' | '\n' | '\r') {
            out.push(c);
            prev = Some(c);
            continue;
        }

        if let Some(reason) = is_hard_reject(c) {
            return InvisibleSanitization::Reject { reason };
        }

        if is_strippable(c) {
            let next = it.clone().next();
            let prev_alpha = prev.map_or(false, char::is_alphanumeric);
            let next_alpha = next.map_or(false, char::is_alphanumeric);
            // Interleaved between two letters: the only reason to do this is
            // to hide a word from substring filters while keeping it readable
            // to the model.
            if prev_alpha && next_alpha {
                return InvisibleSanitization::Reject {
                    reason: RejectReason::MidWordInterleaving,
                };
            }
            stripped += 1;
            prev = Some(c);
            continue;
        }

        out.push(c);
        prev = Some(c);
    }

    if stripped >= MIN_STRIPPABLE_COUNT && stripped * MAX_STRIPPABLE_DIVISOR > total {
        return InvisibleSanitization::Reject {
            reason: RejectReason::ExcessiveInvisibleChars,
        };
    }

    if stripped == 0 {
        InvisibleSanitization::Clean
    } else {
        InvisibleSanitization::Cleaned(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_cleaned(input: &str) -> String {
        match sanitize_invisible_chars(input) {
            InvisibleSanitization::Cleaned(out) => out,
            other => panic!("Expected Cleaned for {input:?}, got {other:?}"),
        }
    }

    fn assert_reject(input: &str, expected: RejectReason) {
        match sanitize_invisible_chars(input) {
            InvisibleSanitization::Reject { reason } => {
                assert_eq!(reason, expected, "for input {input:?}");
            }
            other => panic!("Expected Reject({expected:?}) for {input:?}, got {other:?}"),
        }
    }

    #[test]
    fn clean_text_is_clean() {
        assert_eq!(
            sanitize_invisible_chars("Hello, world!"),
            InvisibleSanitization::Clean
        );
    }

    #[test]
    fn common_whitespace_is_allowed() {
        assert_eq!(
            sanitize_invisible_chars("line1\n\tindented\r\nmore"),
            InvisibleSanitization::Clean
        );
    }

    #[test]
    fn plain_emoji_without_variation_selector_is_clean() {
        assert_eq!(
            sanitize_invisible_chars("Hello \u{1F600}"),
            InvisibleSanitization::Clean
        );
    }

    #[test]
    fn normal_unicode_is_clean() {
        assert_eq!(
            sanitize_invisible_chars("café — naïve 🎉"),
            InvisibleSanitization::Clean
        );
    }

    #[test]
    fn boundary_zero_width_space_is_stripped() {
        // Docusaurus heading anchor: `[text​](#anchor)` — ZWSP sits
        // between text and the closing `]`, not between two letters.
        let input = "## Heading [More intelligence​](#-anchor \"Direct link\")";
        let out = assert_cleaned(input);
        assert!(!out.contains('\u{200B}'), "ZWSP should be stripped: {out:?}");
        assert!(out.contains("More intelligence"));
        assert!(out.contains("#-anchor"));
    }

    #[test]
    fn multiple_boundary_zero_width_spaces_are_stripped() {
        // Mirrors the real DeepSeek page: several headings each carrying a
        // structural ZWSP in their anchor.
        let input = "## One​](#a)\n\n## Two​](#b)\n\n## Three​](#c)";
        let out = assert_cleaned(input);
        assert_eq!(out.matches('\u{200B}').count(), 0);
        assert!(out.contains("## One"));
        assert!(out.contains("## Two"));
        assert!(out.contains("## Three"));
    }

    #[test]
    fn bom_at_start_is_stripped() {
        assert_eq!(assert_cleaned("\u{FEFF}content"), "content");
    }

    #[test]
    fn variation_selector_after_emoji_is_stripped() {
        // ❤️ = U+2764 + variation selector U+FE0F. The VS is not between two
        // letters, so it's a benign glyph-variant marker and gets stripped.
        assert_eq!(assert_cleaned("I \u{2764}\u{FE0F} you"), "I \u{2764} you");
    }

    #[test]
    fn emoji_zwj_sequence_is_not_rejected() {
        // 👨👩👧 = man ZWJ woman ZWJ girl. ZWJs sit between non-alphanumeric
        // emoji, so they're stripped, not rejected.
        assert_cleaned("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}");
    }

    #[test]
    fn mid_word_zero_width_space_is_rejected() {
        // Interleaved between letters — filter evasion.
        assert_reject("i\u{200B}g\u{200B}n\u{200B}o", RejectReason::MidWordInterleaving);
    }

    #[test]
    fn mid_word_hangul_filler_is_rejected() {
        assert_reject("a\u{3164}b", RejectReason::MidWordInterleaving);
    }

    #[test]
    fn tag_block_is_rejected() {
        // Tag letters spelling "delete" — hidden instruction vector.
        assert_reject(
            "text\u{E0064}\u{E0065}\u{E006C}\u{E0065}\u{E0074}\u{E0065}",
            RejectReason::TagBlock,
        );
    }

    #[test]
    fn tag_block_wins_over_benign_zero_width() {
        // A benign boundary ZWSP plus a tag-block char: hard reject wins.
        assert_reject("heading\u{200B}]\u{E007F}", RejectReason::TagBlock);
    }

    #[test]
    fn bidi_override_is_rejected() {
        assert_reject("\u{202A}text\u{202C}", RejectReason::BidiFormatting);
    }

    #[test]
    fn control_chars_are_rejected() {
        for bad in ["result\u{0007}", "result\u{0085}", "a\u{007F}b", "a\u{00AD}b"] {
            assert_reject(bad, RejectReason::ControlChar);
        }
    }

    #[test]
    fn excessive_density_is_rejected() {
        // Many ZWSPs crammed into a short string.
        assert_reject(
            "a\u{200B}\u{200B}\u{200B}\u{200B}\u{200B}b",
            RejectReason::ExcessiveInvisibleChars,
        );
    }

    #[test]
    fn sparse_boundary_zero_width_is_not_excessive() {
        // A few ZWSPs in a long document stay under the density threshold.
        let mut input = String::from("## Heading\u{200B}](#a)");
        for _ in 0..20 {
            input.push_str("\n\nSome normal prose padding to keep density low.");
        }
        assert_cleaned(&input);
    }
}
