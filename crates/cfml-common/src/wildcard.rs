//! Lucee 7.1.3.85 (commit 3a76ded7) wildcard pattern matching for directory
//! filters, ported from `lucee.commons.io.res.util.WildcardPattern` and
//! `WildcardPatternFilter`. Used by directoryList, directoryCopy, and cfdirectory.
//!
//! Semantics:
//! - Matches the entry's **name only**, never the full path.
//! - Patterns are delimited (default `|`), trimmed, and joined with OR logic.
//! - `*` = any run, `?` = exactly one char, everything else is literal.
//! - Leading `!` negates the whole pattern list.
//! - `?` alone (no other wildcards) is a literal compare quirk.
//! - Matching is **leftmost, non-backtracking** (so `a*b` does NOT match `abab`).
//! - Case-INSENSITIVE on Windows, case-SENSITIVE on Linux/macOS.

/// A compiled wildcard pattern, ready to match names.
#[derive(Debug, Clone)]
pub struct WildcardPattern {
    patterns: Vec<ParsedPattern>,
    is_include: bool, // false when negated with leading `!`
    was_empty: bool, // true if the original pattern was empty/whitespace only
}

#[derive(Debug, Clone)]
struct ParsedPattern {
    parts: Vec<PatternPart>,
}

#[derive(Debug, Clone)]
enum PatternPart {
    Literal(String),
    Star,
    Question,
}

impl WildcardPattern {
    /// Build a pattern from the given text and delimiter characters.
    ///
    /// The `delimiters` argument is a string of delimiter characters (e.g. `"|"`
    /// or `",|"`). Each delimiter splits the pattern into tokens. Tokens are
    /// trimmed and empty tokens are dropped. A match on ANY token includes the
    /// entry (OR semantics).
    ///
    /// A leading `!` in the pattern text (before trimming the first token)
    /// negates the whole pattern list. So `!*.txt` means everything except *.txt.
    ///
    /// An empty or whitespace-only pattern matches EVERYTHING (no filter). A
    /// pattern with delimiters but no tokens (e.g. `"|"`) matches NOTHING.
    pub fn new(pattern: &str, delimiters: &str) -> Self {
        let trimmed = pattern.trim();
        let was_empty = trimmed.is_empty();
        
        let (is_include, text) = if let Some(rest) = trimmed.strip_prefix('!') {
            (false, rest)
        } else {
            (true, trimmed)
        };

        let mut patterns = Vec::new();
        for token in Self::split_by_delimiters(text, delimiters) {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            patterns.push(Self::parse_token(token));
        }

        Self { patterns, is_include, was_empty }
    }

    /// Split the text by any character in `delimiters`.
    fn split_by_delimiters<'a>(text: &'a str, delimiters: &str) -> Vec<&'a str> {
        if delimiters.is_empty() {
            return vec![text];
        }
        let mut parts = Vec::new();
        let mut start = 0;
        for (i, ch) in text.char_indices() {
            if delimiters.contains(ch) {
                parts.push(&text[start..i]);
                start = i + ch.len_utf8();
            }
        }
        parts.push(&text[start..]);
        parts
    }

    /// Parse a single trimmed token into a pattern.
    fn parse_token(token: &str) -> ParsedPattern {
        let mut parts = Vec::new();
        let mut literal = String::new();

        for ch in token.chars() {
            match ch {
                '*' => {
                    if !literal.is_empty() {
                        parts.push(PatternPart::Literal(literal.clone()));
                        literal.clear();
                    }
                    parts.push(PatternPart::Star);
                }
                '?' => {
                    if !literal.is_empty() {
                        parts.push(PatternPart::Literal(literal.clone()));
                        literal.clear();
                    }
                    parts.push(PatternPart::Question);
                }
                _ => literal.push(ch),
            }
        }
        if !literal.is_empty() {
            parts.push(PatternPart::Literal(literal));
        }

        ParsedPattern { parts }
    }

    /// Match a name against this pattern.
    ///
    /// The name is matched AFTER case normalization (lowercased on Windows,
    /// unchanged on Linux/macOS).
    pub fn matches(&self, name: &str) -> bool {
        if self.patterns.is_empty() {
            // Empty/whitespace-only input: no filter, matches everything
            if self.was_empty {
                return true;
            }
            // Non-empty input that yielded no tokens (e.g. "|"): matches nothing
            return !self.is_include;
        }

        let matched = self
            .patterns
            .iter()
            .any(|p| Self::matches_single(name, p));

        // XOR: if is_include, return matched; if negated, return !matched.
        matched == self.is_include
    }

    /// Match a single pattern against a name.
    ///
    /// Lucee quirk: a pattern of exactly `?` (parsed to a single Question part)
    /// is treated as a literal compare, not a wildcard. This is implemented
    /// by checking if the pattern is a single Question with no other parts.
    fn matches_single(name: &str, pattern: &ParsedPattern) -> bool {
        // Lucee quirk: `?` alone is literal.
        if pattern.parts.len() == 1 {
            if matches!(pattern.parts[0], PatternPart::Question) {
                return Self::eq_ignore_case_maybe(name, "?");
            }
        }

        // Fast path: no wildcards (all literals).
        if pattern.parts.iter().all(|p| matches!(p, PatternPart::Literal(_))) {
            let full: String = pattern
                .parts
                .iter()
                .map(|p| match p {
                    PatternPart::Literal(s) => s.as_str(),
                    _ => unreachable!(),
                })
                .collect();
            return Self::eq_ignore_case_maybe(name, &full);
        }

        // Fast path: single `*suffix` pattern.
        if pattern.parts.len() == 2 {
            if matches!(pattern.parts[0], PatternPart::Star) {
                if let PatternPart::Literal(suffix) = &pattern.parts[1] {
                    return Self::ends_with_ignore_case_maybe(name, suffix);
                }
            }
        }

        // Fast path: single `prefix*` pattern.
        if pattern.parts.len() == 2 {
            if matches!(pattern.parts[1], PatternPart::Star) {
                if let PatternPart::Literal(prefix) = &pattern.parts[0] {
                    return Self::starts_with_ignore_case_maybe(name, prefix);
                }
            }
        }

        // General case: leftmost, non-backtracking match.
        Self::matches_complex(name, &pattern.parts)
    }

    /// Leftmost, non-backtracking match for complex patterns.
    ///
    /// This is Lucee's algorithm: for each part, we either match it exactly
    /// (literal or `?`) or skip ahead to find the next literal (for `*`).
    /// There is no backtracking, so `a*b` does NOT match `abab`.
    fn matches_complex(name: &str, parts: &[PatternPart]) -> bool {
        let mut pos = 0;

        for (i, part) in parts.iter().enumerate() {
            match part {
                PatternPart::Literal(lit) => {
                    if pos + lit.len() > name.len() {
                        return false;
                    }
                    let slice = &name[pos..pos + lit.len()];
                    if !Self::eq_ignore_case_maybe(slice, lit) {
                        return false;
                    }
                    pos += lit.len();
                }
                PatternPart::Question => {
                    if pos >= name.len() {
                        return false;
                    }
                    // Skip exactly one character (could be multi-byte UTF-8).
                    if let Some(ch) = name[pos..].chars().next() {
                        pos += ch.len_utf8();
                    } else {
                        return false;
                    }
                }
                PatternPart::Star => {
                    // Star: consume characters until the next part matches (leftmost).
                    // If there's no next part, star matches the rest of the string.
                    if i + 1 >= parts.len() {
                        // Star at the end: matches everything remaining.
                        return true;
                    }

                    // Look ahead to see what comes after the star.
                    let next = &parts[i + 1];
                    match next {
                        PatternPart::Literal(lit) => {
                            // Find the literal starting from pos (leftmost).
                            if let Some(offset) = Self::index_of_ignore_case_maybe(&name[pos..], lit) {
                                pos += offset;
                            } else {
                                return false;
                            }
                            // The literal will be matched in the next iteration.
                        }
                        PatternPart::Question => {
                            // Star followed by `?`: the star can match zero chars,
                            // and the `?` will match one char. So do nothing here;
                            // the `?` will be processed next.
                        }
                        PatternPart::Star => {
                            // Two stars in a row: the first is redundant.
                        }
                    }
                }
            }
        }

        // After processing all parts, we must have consumed the entire name.
        pos >= name.len()
    }

    /// Case-sensitive or case-insensitive equality depending on platform.
    #[cfg(target_os = "windows")]
    fn eq_ignore_case_maybe(a: &str, b: &str) -> bool {
        a.eq_ignore_ascii_case(b)
    }

    #[cfg(not(target_os = "windows"))]
    fn eq_ignore_case_maybe(a: &str, b: &str) -> bool {
        a == b
    }

    /// Case-sensitive or case-insensitive starts_with depending on platform.
    #[cfg(target_os = "windows")]
    fn starts_with_ignore_case_maybe(haystack: &str, needle: &str) -> bool {
        if haystack.len() < needle.len() {
            return false;
        }
        haystack[..needle.len()].eq_ignore_ascii_case(needle)
    }

    #[cfg(not(target_os = "windows"))]
    fn starts_with_ignore_case_maybe(haystack: &str, needle: &str) -> bool {
        haystack.starts_with(needle)
    }

    /// Case-sensitive or case-insensitive ends_with depending on platform.
    #[cfg(target_os = "windows")]
    fn ends_with_ignore_case_maybe(haystack: &str, needle: &str) -> bool {
        if haystack.len() < needle.len() {
            return false;
        }
        let start = haystack.len() - needle.len();
        haystack[start..].eq_ignore_ascii_case(needle)
    }

    #[cfg(not(target_os = "windows"))]
    fn ends_with_ignore_case_maybe(haystack: &str, needle: &str) -> bool {
        haystack.ends_with(needle)
    }

    /// Find the first occurrence of `needle` in `haystack`, case-sensitively
    /// or case-insensitively depending on platform. Returns the byte offset.
    #[cfg(target_os = "windows")]
    fn index_of_ignore_case_maybe(haystack: &str, needle: &str) -> Option<usize> {
        let lower_haystack = haystack.to_lowercase();
        let lower_needle = needle.to_lowercase();
        lower_haystack.find(&lower_needle)
    }

    #[cfg(not(target_os = "windows"))]
    fn index_of_ignore_case_maybe(haystack: &str, needle: &str) -> Option<usize> {
        haystack.find(needle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let pat = WildcardPattern::new("test.txt", "|");
        assert!(pat.matches("test.txt"));
        assert!(!pat.matches("other.txt"));
    }

    #[test]
    fn test_star_wildcard() {
        let pat = WildcardPattern::new("*.txt", "|");
        assert!(pat.matches("test.txt"));
        assert!(pat.matches("a.txt"));
        assert!(!pat.matches("test.cfm"));
    }

    #[test]
    fn test_question_wildcard() {
        let pat = WildcardPattern::new("?.txt", "|");
        assert!(pat.matches("a.txt"));
        assert!(!pat.matches("ab.txt"));
        assert!(!pat.matches(".txt"));
    }

    #[test]
    fn test_question_alone_is_literal() {
        let pat = WildcardPattern::new("?", "|");
        assert!(pat.matches("?"));
        assert!(!pat.matches("a"));
    }

    #[test]
    fn test_multiple_patterns() {
        let pat = WildcardPattern::new("*.txt|*.cfm", "|");
        assert!(pat.matches("test.txt"));
        assert!(pat.matches("test.cfm"));
        assert!(!pat.matches("test.cfc"));
    }

    #[test]
    fn test_negation() {
        let pat = WildcardPattern::new("!*.txt", "|");
        assert!(!pat.matches("test.txt"));
        assert!(pat.matches("test.cfm"));
    }

    #[test]
    fn test_empty_pattern() {
        // "|" has no tokens, so matches nothing
        let pat1 = WildcardPattern::new("|", "|");
        assert!(!pat1.matches("anything"));

        // "" or whitespace-only matches everything (no filter)
        let pat2 = WildcardPattern::new("", "|");
        assert!(pat2.matches("anything"));

        let pat3 = WildcardPattern::new("  ", "|");
        assert!(pat3.matches("anything"));
    }

    #[test]
    fn test_non_backtracking() {
        // `a*b` should NOT match `abab` due to leftmost, non-backtracking.
        let pat = WildcardPattern::new("a*b", "|");
        assert!(!pat.matches("abab"));
        // But it should match `ab` and `axb`.
        assert!(pat.matches("ab"));
        assert!(pat.matches("axb"));
        assert!(pat.matches("axxxb"));
    }

    #[test]
    fn test_trimming() {
        let pat = WildcardPattern::new(" *.txt | *.cfm ", "|");
        assert!(pat.matches("test.txt"));
        assert!(pat.matches("test.cfm"));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_case_sensitive_on_linux() {
        let pat = WildcardPattern::new("*.TXT", "|");
        assert!(pat.matches("test.TXT"));
        assert!(!pat.matches("test.txt"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_case_insensitive_on_windows() {
        let pat = WildcardPattern::new("*.TXT", "|");
        assert!(pat.matches("test.TXT"));
        assert!(pat.matches("test.txt"));
    }
}
